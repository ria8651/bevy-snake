//! Transport abstraction for client/server wire I/O.
//!
//! Two flavours, both in terms of `GameCommands` (client→server) and
//! `GameUpdates` (server→client):
//!
//! * **reliable**: ordered, no loss. In production this is a single persistent
//!   uni-stream per direction with `u32` length-prefixed bincode frames.
//! * **datagram**: unordered, lossy, but lower-latency. Used for the input
//!   hot-path. Single bincode payload per datagram.
//!
//! The trait lets the game-loop layer treat the wire generically, so tests
//! can plug in `MockTransport` with configurable drop/reorder/delay.

#[cfg(not(target_arch = "wasm32"))]
use crate::{GameCommands, GameUpdates};
use serde::{de::DeserializeOwned, Serialize};

/// Errors from a transport operation. Kept intentionally simple: the game
/// loop only cares whether the channel is alive.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("transport closed")]
    Closed,
    #[error("codec error: {0}")]
    Codec(String),
    #[error("io error: {0}")]
    Io(String),
}

/// Length-prefix codec: `u32 BE length || bincode(payload)`.
///
/// Used for the reliable framed stream. Bincode is chosen over JSON for
/// compactness and stability of the binary form — the choice is independent
/// of any other decision.
pub fn encode_framed<T: Serialize>(value: &T) -> Result<Vec<u8>, TransportError> {
    let payload = bincode::serialize(value).map_err(|e| TransportError::Codec(e.to_string()))?;
    let len = payload.len() as u32;
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Decode a single bincode payload (no length prefix — the framer already
/// stripped it).
pub fn decode_payload<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, TransportError> {
    bincode::deserialize(bytes).map_err(|e| TransportError::Codec(e.to_string()))
}

/// Read one length-prefixed frame from a byte stream.
///
/// `read_exact` must fill the slice or return an error indicating EOF/closed.
/// Returns `Ok(None)` if the stream cleanly ended at a frame boundary.
pub async fn read_frame<R, F, Fut>(
    mut read_exact: F,
) -> Result<Option<Vec<u8>>, TransportError>
where
    F: FnMut(usize) -> Fut,
    Fut: std::future::Future<Output = Result<Option<Vec<u8>>, TransportError>>,
    R: Sized,
{
    let len_buf = match read_exact(4).await? {
        Some(b) => b,
        None => return Ok(None),
    };
    let len = u32::from_be_bytes([len_buf[0], len_buf[1], len_buf[2], len_buf[3]]) as usize;
    // Cap on a single frame size — the largest Board (24x21 Large) is ~2KB
    // bincode. 1 MiB is generous and prevents a malicious peer from making us
    // allocate gigabytes.
    if len > 1024 * 1024 {
        return Err(TransportError::Codec(format!("frame too large: {} bytes", len)));
    }
    let payload = read_exact(len)
        .await?
        .ok_or_else(|| TransportError::Io("eof mid-frame".into()))?;
    Ok(Some(payload))
}

// ---------------------------------------------------------------------------
// Mock transport for tests
// ---------------------------------------------------------------------------

// Native-only: depends on tokio's `rt` and `time` features, which are not
// enabled on wasm builds (wasm uses tokio just for `sync::mpsc`).
#[cfg(not(target_arch = "wasm32"))]
pub mod mock {
    //! In-process pair of transports with optional loss/reorder/delay on the
    //! datagram channel. The reliable channel mirrors real QUIC stream
    //! semantics: ordered, no loss.

    use super::*;
    use rand::{rngs::StdRng, Rng, SeedableRng};
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::sync::mpsc::{channel, Receiver, Sender};
    use tokio::time::sleep;

    #[derive(Clone, Debug)]
    pub struct LossConfig {
        /// 0.0..=1.0 — fraction of datagrams that disappear.
        pub datagram_loss: f32,
        /// 0.0..=1.0 — fraction of datagrams that get held back to land
        /// after the next one (one-step reordering).
        pub datagram_reorder: f32,
        /// Per-datagram fixed delay range.
        pub datagram_min_delay: Duration,
        pub datagram_max_delay: Duration,
        /// Reliable channel only gets a fixed delay — no loss, no reorder.
        pub reliable_min_delay: Duration,
        pub reliable_max_delay: Duration,
        pub seed: u64,
    }

    impl Default for LossConfig {
        fn default() -> Self {
            Self {
                datagram_loss: 0.0,
                datagram_reorder: 0.0,
                datagram_min_delay: Duration::ZERO,
                datagram_max_delay: Duration::ZERO,
                reliable_min_delay: Duration::ZERO,
                reliable_max_delay: Duration::ZERO,
                seed: 0,
            }
        }
    }

    /// The server-facing half of a MockTransport pair.
    pub struct MockServerSide {
        // Outbound: GameUpdates the server wants to send to the client.
        pub reliable_out: Sender<GameUpdates>,
        pub datagram_out: Sender<GameUpdates>,
        // Inbound: GameCommands the client sent.
        pub reliable_in: Receiver<GameCommands>,
        pub datagram_in: Receiver<GameCommands>,
    }

    /// The client-facing half of a MockTransport pair.
    pub struct MockClientSide {
        pub reliable_out: Sender<GameCommands>,
        pub datagram_out: Sender<GameCommands>,
        pub reliable_in: Receiver<GameUpdates>,
        pub datagram_in: Receiver<GameUpdates>,
    }

    /// Build a connected (client, server) pair.
    ///
    /// Loss/reorder/delay are applied by intermediary tasks: each direction
    /// has a "wire" task that forwards from the sender's outbound channel to
    /// the receiver's inbound channel, applying the policy.
    pub fn pair(cfg: LossConfig) -> (MockClientSide, MockServerSide) {
        let cfg = Arc::new(cfg);
        let rng = Arc::new(Mutex::new(StdRng::seed_from_u64(cfg.seed)));

        // client -> server
        let (c2s_reliable_tx, mut c2s_reliable_src) = channel::<GameCommands>(64);
        let (c2s_reliable_sink, c2s_reliable_rx) = channel::<GameCommands>(64);
        let (c2s_datagram_tx, mut c2s_datagram_src) = channel::<GameCommands>(64);
        let (c2s_datagram_sink, c2s_datagram_rx) = channel::<GameCommands>(64);

        // server -> client
        let (s2c_reliable_tx, mut s2c_reliable_src) = channel::<GameUpdates>(64);
        let (s2c_reliable_sink, s2c_reliable_rx) = channel::<GameUpdates>(64);
        let (s2c_datagram_tx, mut s2c_datagram_src) = channel::<GameUpdates>(64);
        let (s2c_datagram_sink, s2c_datagram_rx) = channel::<GameUpdates>(64);

        // Wire: reliable c->s (ordered, no loss, only delay).
        {
            let cfg = cfg.clone();
            let rng = rng.clone();
            tokio::spawn(async move {
                while let Some(msg) = c2s_reliable_src.recv().await {
                    let d = random_delay(&rng, cfg.reliable_min_delay, cfg.reliable_max_delay);
                    if !d.is_zero() {
                        sleep(d).await;
                    }
                    if c2s_reliable_sink.send(msg).await.is_err() {
                        break;
                    }
                }
            });
        }

        // Wire: reliable s->c.
        {
            let cfg = cfg.clone();
            let rng = rng.clone();
            tokio::spawn(async move {
                while let Some(msg) = s2c_reliable_src.recv().await {
                    let d = random_delay(&rng, cfg.reliable_min_delay, cfg.reliable_max_delay);
                    if !d.is_zero() {
                        sleep(d).await;
                    }
                    if s2c_reliable_sink.send(msg).await.is_err() {
                        break;
                    }
                }
            });
        }

        // Wire: datagram c->s. May drop, may reorder, may delay.
        {
            let cfg = cfg.clone();
            let rng = rng.clone();
            let sink = c2s_datagram_sink.clone();
            tokio::spawn(async move {
                let mut deferred: VecDeque<GameCommands> = VecDeque::new();
                while let Some(msg) = c2s_datagram_src.recv().await {
                    // Drop?
                    if {
                        let mut r = rng.lock().unwrap();
                        r.random::<f32>() < cfg.datagram_loss
                    } {
                        continue;
                    }
                    // Reorder? Hold msg until next one arrives.
                    let reorder = {
                        let mut r = rng.lock().unwrap();
                        r.random::<f32>() < cfg.datagram_reorder
                    };
                    if reorder {
                        deferred.push_back(msg);
                        continue;
                    }
                    let d = random_delay(&rng, cfg.datagram_min_delay, cfg.datagram_max_delay);
                    if !d.is_zero() {
                        sleep(d).await;
                    }
                    if sink.send(msg).await.is_err() {
                        break;
                    }
                    // Flush any deferred (one-step reorder).
                    while let Some(d_msg) = deferred.pop_front() {
                        if sink.send(d_msg).await.is_err() {
                            return;
                        }
                    }
                }
            });
        }

        // Wire: datagram s->c. Same policy.
        {
            let cfg = cfg.clone();
            let rng = rng.clone();
            let sink = s2c_datagram_sink.clone();
            tokio::spawn(async move {
                let mut deferred: VecDeque<GameUpdates> = VecDeque::new();
                while let Some(msg) = s2c_datagram_src.recv().await {
                    if {
                        let mut r = rng.lock().unwrap();
                        r.random::<f32>() < cfg.datagram_loss
                    } {
                        continue;
                    }
                    let reorder = {
                        let mut r = rng.lock().unwrap();
                        r.random::<f32>() < cfg.datagram_reorder
                    };
                    if reorder {
                        deferred.push_back(msg);
                        continue;
                    }
                    let d = random_delay(&rng, cfg.datagram_min_delay, cfg.datagram_max_delay);
                    if !d.is_zero() {
                        sleep(d).await;
                    }
                    if sink.send(msg).await.is_err() {
                        break;
                    }
                    while let Some(d_msg) = deferred.pop_front() {
                        if sink.send(d_msg).await.is_err() {
                            return;
                        }
                    }
                }
            });
        }

        (
            MockClientSide {
                reliable_out: c2s_reliable_tx,
                datagram_out: c2s_datagram_tx,
                reliable_in: s2c_reliable_rx,
                datagram_in: s2c_datagram_rx,
            },
            MockServerSide {
                reliable_out: s2c_reliable_tx,
                datagram_out: s2c_datagram_tx,
                reliable_in: c2s_reliable_rx,
                datagram_in: c2s_datagram_rx,
            },
        )
    }

    fn random_delay(rng: &Arc<Mutex<StdRng>>, min: Duration, max: Duration) -> Duration {
        if max <= min {
            return min;
        }
        let span = (max - min).as_nanos() as u64;
        let mut r = rng.lock().unwrap();
        let extra = r.random_range(0..=span);
        min + Duration::from_nanos(extra)
    }

    // ---- tests ----

    #[tokio::test]
    async fn reliable_passes_through_in_order_with_no_loss() {
        let (client, mut server) = pair(LossConfig {
            datagram_loss: 1.0, // sanity check: reliable unaffected by datagram loss
            ..Default::default()
        });

        // client -> server
        for i in 0..5u64 {
            client
                .reliable_out
                .send(GameCommands::Input {
                    tick: i,
                    direction: crate::board::Direction::Up,
                    client_send_ms: i as u32,
                })
                .await
                .unwrap();
        }
        for i in 0..5u64 {
            let cmd = server.reliable_in.recv().await.expect("recv");
            match cmd {
                GameCommands::Input { tick, .. } => assert_eq!(tick, i),
                _ => panic!("wrong variant"),
            }
        }
    }

    #[tokio::test]
    async fn datagram_loss_drops_messages() {
        let (client, mut server) = pair(LossConfig {
            datagram_loss: 1.0, // drop everything
            seed: 7,
            ..Default::default()
        });

        for i in 0..10u64 {
            client
                .datagram_out
                .send(GameCommands::Input {
                    tick: i,
                    direction: crate::board::Direction::Up,
                    client_send_ms: i as u32,
                })
                .await
                .unwrap();
        }

        let result =
            tokio::time::timeout(Duration::from_millis(50), server.datagram_in.recv()).await;
        assert!(result.is_err(), "datagram_loss=1.0 must drop everything");
    }

    #[tokio::test]
    async fn datagram_partial_loss_lets_some_through() {
        let (client, mut server) = pair(LossConfig {
            datagram_loss: 0.5,
            seed: 42,
            ..Default::default()
        });

        for i in 0..50u64 {
            client
                .datagram_out
                .send(GameCommands::Input {
                    tick: i,
                    direction: crate::board::Direction::Up,
                    client_send_ms: i as u32,
                })
                .await
                .unwrap();
        }
        // Allow some real wall time for the wire task to drain.
        tokio::time::sleep(Duration::from_millis(20)).await;

        let mut got = 0;
        while let Ok(Some(_)) =
            tokio::time::timeout(Duration::from_millis(20), server.datagram_in.recv()).await
        {
            got += 1;
        }
        assert!(got > 0, "expected at least some datagrams through");
        assert!(got < 50, "expected some loss");
    }
}

// ---------------------------------------------------------------------------
// Framing round-trip tests (no async / mock infra required)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{BoardSettings, Direction};
    use crate::{GameCommands, GameUpdates};

    #[test]
    fn framed_input_round_trips() {
        let cmd = GameCommands::Input {
            tick: 7,
            direction: Direction::Up,
            client_send_ms: 12345,
        };
        let bytes = encode_framed(&cmd).unwrap();
        // First 4 bytes are length BE.
        let len = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        assert_eq!(len + 4, bytes.len());
        let decoded: GameCommands = decode_payload(&bytes[4..]).unwrap();
        match decoded {
            GameCommands::Input { tick, direction, client_send_ms } => {
                assert_eq!(tick, 7);
                assert_eq!(direction, Direction::Up);
                assert_eq!(client_send_ms, 12345);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn framed_set_tick_rate_round_trips() {
        let cmd = GameCommands::SetTickRate { tick_interval_ms: 200 };
        let bytes = encode_framed(&cmd).unwrap();
        let decoded: GameCommands = decode_payload(&bytes[4..]).unwrap();
        match decoded {
            GameCommands::SetTickRate { tick_interval_ms } => assert_eq!(tick_interval_ms, 200),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn framed_restart_round_trips() {
        let cmd = GameCommands::RestartGame {
            board_settings: BoardSettings::default(),
        };
        let bytes = encode_framed(&cmd).unwrap();
        let decoded: GameCommands = decode_payload(&bytes[4..]).unwrap();
        matches!(decoded, GameCommands::RestartGame { .. });
    }

    #[test]
    fn framed_ticked_round_trips() {
        use crate::board::Board;
        let upd = GameUpdates::Ticked {
            tick: 5,
            board: Board::new(BoardSettings::default()),
            events: vec![],
            applied_inputs: vec![Some(Direction::Up), None],
            tick_interval_ms: 133,
            echo_client_send_ms: Some(42),
        };
        let bytes = encode_framed(&upd).unwrap();
        let decoded: GameUpdates = decode_payload(&bytes[4..]).unwrap();
        match decoded {
            GameUpdates::Ticked {
                tick,
                applied_inputs,
                tick_interval_ms,
                echo_client_send_ms,
                ..
            } => {
                assert_eq!(tick, 5);
                assert_eq!(applied_inputs, vec![Some(Direction::Up), None]);
                assert_eq!(tick_interval_ms, 133);
                assert_eq!(echo_client_send_ms, Some(42));
            }
        }
    }

    #[test]
    fn datagram_payload_round_trips() {
        // Datagrams carry a single bincode payload (no length prefix). The
        // server reads `bytes` and calls `decode_payload`. Verify that
        // bincode-encoding an Input and decoding from the same bytes works.
        let cmd = GameCommands::Input {
            tick: 13,
            direction: Direction::Left,
            client_send_ms: 999,
        };
        let bytes = bincode::serialize(&cmd).unwrap();
        // Inputs are tiny — must fit comfortably in a datagram MTU (~1200 B).
        assert!(bytes.len() < 100, "input datagram too large: {} bytes", bytes.len());
        let decoded: GameCommands = decode_payload(&bytes).unwrap();
        match decoded {
            GameCommands::Input { tick, direction, client_send_ms } => {
                assert_eq!(tick, 13);
                assert_eq!(direction, Direction::Left);
                assert_eq!(client_send_ms, 999);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn frame_too_large_is_rejected() {
        // Hand-craft a 4-byte length header claiming 2 MiB.
        let bytes = [0xFF, 0xFF, 0xFF, 0xFF];
        // Drive read_frame with a custom reader: returns Some on first call
        // (length), then we never actually need to fulfil the body because
        // the size check trips first.
        let mut step = 0;
        let result = futures::executor::block_on(read_frame::<(), _, _>(|n| {
            let step_now = step;
            step += 1;
            async move {
                match step_now {
                    0 => {
                        assert_eq!(n, 4);
                        Ok::<Option<Vec<u8>>, TransportError>(Some(bytes.to_vec()))
                    }
                    _ => Ok(None),
                }
            }
        }));
        assert!(matches!(result, Err(TransportError::Codec(_))));
    }
}
