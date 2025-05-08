use bevy::prelude::*;
use bevy_snake::{GameCommands, GameUpdates};
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
        // read the certificate from the file
        // let mut text = String::new();
        // File::open("cert/localhost.hex")
        //     .unwrap()
        //     .read_to_string(&mut text)
        //     .unwrap();
        let text = "ef6aaeb40dc97fc7f142fc7a4044436ebf157cb29840f508945c861fc2001c0c";
        let hash = decode_hex(&text.split_whitespace().next().unwrap());

        // create a new client
        let client = web_transport::ClientBuilder::new()
            .with_server_certificate_hashes(vec![hash])
            .unwrap();

        // connect to the given URL
        let mut session = client.connect(&url.parse().unwrap()).await.unwrap();

        update_tx.send(NetworkUpdate::Connected).await.unwrap();

        // {
        //     // create a bidirectional stream
        //     let (mut send, mut recv) = session.open_bi().await.unwrap();
        //     send.write(b"hello world").await.unwrap();
        //     drop(send);
        //     info!("sent: hello world");
        //     let msg = recv.read(1024).await.unwrap();
        //     info!("recv: {}", String::from_utf8_lossy(&msg.unwrap()));
        // }

        // send and receive messages
        loop {
            info!("waiting for command");
            tokio::select! {
                cmd = command_rx.recv() => {
                    match cmd {
                        Some(cmd) => {
                            let msg = serde_json::to_vec(&cmd).unwrap();
                            trace!("sending command: {}", String::from_utf8_lossy(&msg));
                            let mut send = session.open_uni().await.unwrap();
                            send.write(&msg).await.unwrap();
                        }
                        None => {
                            warn!("command channel closed");
                            break;
                        }
                    }
                }
                msg = session.accept_uni() => {
                    match msg {
                        Ok(mut recv) => {
                            let mut buf = Vec::new();
                            while let Some(_) = recv.read_buf(&mut buf).await.unwrap() {
                                // read until EOF
                            }
                            let update = serde_json::from_slice::<GameUpdates>(&buf).unwrap();
                            update_tx.send(NetworkUpdate::Update(update)).await.unwrap();
                        }
                        Err(e) => {
                            warn!("wt connection closed: {:?}", e);
                            break;
                        }
                    }
                }
            }
        }

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

pub fn decode_hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
