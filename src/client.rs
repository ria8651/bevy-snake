use bevy::prelude::*;
use bevy_snake::{
    transport::{decode_payload, encode_framed},
    GameCommands, GameUpdates,
};
use bytes::Bytes;
use futures::stream::{self, StreamExt};
use tokio::sync::mpsc::{channel, Receiver, Sender};

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(not(target_arch = "wasm32"))]
        app.insert_resource(TokioRuntime::default());
        app.add_observer(start_new_wt_tasks);
    }
}

#[derive(Debug, Clone)]
pub enum NetworkUpdate {
    Connected,
    Disconnected,
    Update(GameUpdates),
}

#[derive(Debug, Component)]
pub struct ClientConnection {
    command_tx: Sender<GameCommands>,
    command_rx: Option<Receiver<GameCommands>>,
    update_tx: Option<Sender<NetworkUpdate>>,
    update_rx: Receiver<NetworkUpdate>,
    url: String,
}

impl ClientConnection {
    pub fn new(url: String) -> Self {
        let (update_tx, update_rx) = channel(100); // async_channel::unbounded();
        let (command_tx, command_rx) = channel(100); // async_channel::unbounded();
        Self {
            command_tx,
            command_rx: Some(command_rx),
            update_tx: Some(update_tx),
            update_rx,
            url,
        }
    }

    pub fn send_command(&self, command: GameCommands) {
        self.command_tx.try_send(command).unwrap();
    }

    pub fn receive_update(&mut self) -> Option<NetworkUpdate> {
        self.update_rx.try_recv().ok()
    }
}

fn start_new_wt_tasks(
    trigger: Trigger<OnAdd, ClientConnection>,
    mut q: Query<&mut ClientConnection>,
    #[cfg(not(target_arch = "wasm32"))] tokio_runtime: Res<TokioRuntime>,
) {
    let mut connection_entity = q.get_mut(trigger.entity()).unwrap();
    let mut command_rx = connection_entity.command_rx.take().unwrap();
    let update_tx = connection_entity.update_tx.take().unwrap();
    let url = connection_entity.url.clone();
    info!("Starting new wt task connecting to {}", url);
    let task = async move {
        let hash = get_cert_hash();

        // create a new client
        let client = web_transport::ClientBuilder::new()
            .with_server_certificate_hashes(vec![hash])
            .unwrap();

        // connect to the given URL
        let mut session = client.connect(&url.parse().unwrap()).await.unwrap();

        update_tx.send(NetworkUpdate::Connected).await.unwrap();

        // One persistent uni-stream per direction. This is the
        // AGENTS.md-documented fix for Firefox's WebTransport (it silently
        // stops yielding new incoming uni-streams after the first two), and
        // is required for length-prefixed framing to make sense.
        //
        // The reliable stream is used for RestartGame and other rare
        // protocol messages. Per-tick input goes via send_datagram on the
        // separate `datagram_session` clone below.
        let mut send_stream = match session.open_uni().await {
            Ok(s) => s,
            Err(e) => {
                error!("client: open_uni for send failed: {:?}", e);
                update_tx.send(NetworkUpdate::Disconnected).await.ok();
                return;
            }
        };

        let mut datagram_session = session.clone();

        // Wrap accept_uni in a Stream so its in-flight future survives
        // tokio::select! cancellations. Recreating session.accept_uni() each
        // loop iteration would leak a Web Streams reader lock on wasm: the
        // pending read prevents releaseLock from unlocking the underlying
        // ReadableStream, which silently freezes incoming traffic. Keeping
        // the future inside the unfold preserves it across cancellation.
        let accept_stream = stream::unfold(session.clone(), |mut s| async move {
            let result = s.accept_uni().await;
            Some((result, s))
        });
        tokio::pin!(accept_stream);

        // First: wait for the server's send stream to land. We hold the read
        // pump and the command-send loop separate after that.
        let recv_stream: web_transport::RecvStream = tokio::select! {
            cmd = command_rx.recv() => {
                // Drain any commands that came in before the server's stream
                // opened. They can't go anywhere yet — but the connection is
                // up, so just buffer the first one and try to send it.
                match cmd {
                    Some(cmd) => {
                        if let Err(e) =
                            send_command_routed(&mut send_stream, &mut datagram_session, &cmd).await
                        {
                            error!("client: early send failed: {:?}", e);
                        }
                        // Now wait for the server's stream the long way.
                        match accept_stream.next().await {
                            Some(Ok(rs)) => rs,
                            other => {
                                warn!("client: accept_stream ended before server send stream: {:?}",
                                    other.as_ref().map(|r| r.is_err()));
                                update_tx.send(NetworkUpdate::Disconnected).await.ok();
                                return;
                            }
                        }
                    }
                    None => {
                        warn!("command channel closed before server stream");
                        update_tx.send(NetworkUpdate::Disconnected).await.ok();
                        return;
                    }
                }
            }
            next = accept_stream.next() => match next {
                Some(Ok(rs)) => rs,
                other => {
                    warn!("client: accept_stream returned {:?}", other.is_some());
                    update_tx.send(NetworkUpdate::Disconnected).await.ok();
                    return;
                }
            }
        };

        // Build a frame stream that owns the recv_stream and a scratch buffer.
        // Wrapping in unfold preserves in-flight reads across select!
        // cancellations — the lock-leak bug AGENTS.md warns about.
        let frame_stream = stream::unfold(
            (recv_stream, Vec::<u8>::with_capacity(4096)),
            |(mut rs, mut buf)| async move {
                let frame = read_framed_wasm(&mut rs, &mut buf).await;
                Some((frame, (rs, buf)))
            },
        );
        tokio::pin!(frame_stream);

        loop {
            tokio::select! {
                cmd = command_rx.recv() => {
                    match cmd {
                        Some(cmd) => {
                            if let Err(e) =
                            send_command_routed(&mut send_stream, &mut datagram_session, &cmd).await
                        {
                                error!("client: send failed: {:?}", e);
                                break;
                            }
                        }
                        None => {
                            warn!("command channel closed");
                            break;
                        }
                    }
                }
                frame = frame_stream.next() => {
                    match frame {
                        Some(Ok(Some(update))) => {
                            if let Err(e) =
                                update_tx.send(NetworkUpdate::Update(update)).await
                            {
                                error!("client: update_tx send failed: {:?}", e);
                                break;
                            }
                        }
                        Some(Ok(None)) => {
                            warn!("client: recv stream cleanly ended");
                            break;
                        }
                        Some(Err(e)) => {
                            error!("client: frame error: {:?}", e);
                            break;
                        }
                        None => {
                            warn!("client: frame_stream ended");
                            break;
                        }
                    }
                }
            }
        }
        warn!("client wt task exiting");

        // doesn't matter if the channel is already closed
        update_tx.send(NetworkUpdate::Disconnected).await.ok();
    };

    #[cfg(target_arch = "wasm32")]
    {
        let task_pool = bevy::tasks::AsyncComputeTaskPool::get();
        task_pool.spawn(task).detach();
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        tokio_runtime.spawn(task);
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource, Deref, DerefMut)]
struct TokioRuntime(tokio::runtime::Runtime);

#[cfg(not(target_arch = "wasm32"))]
impl Default for TokioRuntime {
    fn default() -> Self {
        Self(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap(),
        )
    }
}

#[derive(Debug)]
#[allow(dead_code)] // fields used via {:?} in error! macro
enum ClientFrameError {
    Read(web_transport::Error),
    Codec(String),
    OverLimit(usize),
}

/// Send a `GameCommands` choosing the right wire path:
///
/// * `Input` → unreliable datagram (latency-sensitive, loss-tolerant; the
///   server's per-tick HashMap dedup handles out-of-order arrival).
/// * `RestartGame` and anything else → length-prefixed reliable stream.
async fn send_command_routed(
    send_stream: &mut web_transport::SendStream,
    datagram_session: &mut web_transport::Session,
    cmd: &GameCommands,
) -> Result<(), &'static str> {
    match cmd {
        GameCommands::Input { .. } => {
            // Bincode payload directly; datagram size is tiny (~20 bytes).
            let bytes = match bincode::serialize(cmd) {
                Ok(b) => Bytes::from(b),
                Err(e) => {
                    error!("encode datagram: {:?}", e);
                    return Err("encode");
                }
            };
            trace!("sending input datagram ({} bytes)", bytes.len());
            if let Err(e) = datagram_session.send_datagram(bytes).await {
                error!("client: send_datagram failed: {:?}", e);
                return Err("datagram");
            }
        }
        _ => {
            let frame = match encode_framed(cmd) {
                Ok(f) => f,
                Err(e) => {
                    error!("encode_framed: {:?}", e);
                    return Err("encode");
                }
            };
            trace!("sending reliable frame ({} bytes)", frame.len());
            if let Err(e) = send_stream.write(&frame).await {
                error!("client: write failed: {:?}", e);
                return Err("write");
            }
        }
    }
    Ok(())
}

/// Read one length-prefixed `GameUpdates` frame from the wasm RecvStream.
///
/// `scratch` is reused across calls to avoid re-allocating the 4-byte header
/// buffer. Returns `Ok(None)` if the stream cleanly ended at a frame boundary.
async fn read_framed_wasm(
    recv: &mut web_transport::RecvStream,
    scratch: &mut Vec<u8>,
) -> Result<Option<GameUpdates>, ClientFrameError> {
    scratch.clear();
    // Read until we have at least 4 bytes (the length prefix). The wasm API
    // gives us read_buf which appends to a Vec.
    while scratch.len() < 4 {
        match recv.read_buf(scratch).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                // Clean EOF. If we have nothing buffered, this is the normal
                // end of stream; otherwise it's a truncated frame.
                return if scratch.is_empty() {
                    Ok(None)
                } else {
                    Err(ClientFrameError::Codec(format!(
                        "eof in length prefix: {} bytes",
                        scratch.len()
                    )))
                };
            }
            Err(e) => return Err(ClientFrameError::Read(e)),
        }
    }
    let len = u32::from_be_bytes([scratch[0], scratch[1], scratch[2], scratch[3]]) as usize;
    if len > 1024 * 1024 {
        return Err(ClientFrameError::OverLimit(len));
    }
    // Buffer may already contain some payload bytes from the last read_buf
    // call; keep reading until we have len + 4 total.
    let needed = 4 + len;
    while scratch.len() < needed {
        match recv.read_buf(scratch).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Err(ClientFrameError::Codec(format!(
                    "eof mid-frame: have {} of {}",
                    scratch.len(),
                    needed
                )))
            }
            Err(e) => return Err(ClientFrameError::Read(e)),
        }
    }
    let update: GameUpdates =
        decode_payload(&scratch[4..needed]).map_err(|e| ClientFrameError::Codec(e.to_string()))?;
    // If we over-read, shuffle leftovers to the front for the next call.
    let leftover = scratch.len() - needed;
    if leftover > 0 {
        scratch.copy_within(needed.., 0);
    }
    scratch.truncate(leftover);
    Ok(Some(update))
}

pub fn decode_hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[cfg(target_arch = "wasm32")]
fn get_cert_hash() -> Vec<u8> {
    use wasm_bindgen::JsValue;
    let win = web_sys::window().expect("no window");
    let val = js_sys::Reflect::get(&win, &JsValue::from_str("WT_CERT_HASH"))
        .expect("failed to read window.WT_CERT_HASH");
    let hex_str = val
        .as_string()
        .expect("window.WT_CERT_HASH must be a hex string");
    decode_hex(&hex_str)
}

#[cfg(not(target_arch = "wasm32"))]
fn get_cert_hash() -> Vec<u8> {
    unimplemented!(
        "native client cert verification not wired up yet — \
         the cert hash needs to be fetched from the server's HTTP endpoint"
    )
}
