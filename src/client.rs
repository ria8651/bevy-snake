use bevy::prelude::*;
use bevy_snake::{GameCommands, GameUpdates};
use std::{collections::VecDeque, fs::File, io::Read};
use tokio::runtime::Runtime;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        // app.add_systems(PreUpdate, receive_ws)
        //     .add_systems(PostUpdate, send_ws);
        #[cfg(not(target_arch = "wasm32"))]
        app.insert_resource(TokioRuntime::default());
        app.add_systems(Startup, test_wt);
    }
}

#[derive(Component, Default)]
pub struct ClientConnection {
    tx: VecDeque<GameCommands>,
    rx: VecDeque<GameUpdates>,
}

impl ClientConnection {
    pub fn send(&mut self, cmd: GameCommands) {
        self.tx.push_back(cmd);
    }

    pub fn recv(&mut self) -> Option<GameUpdates> {
        self.rx.pop_front()
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource, Deref, DerefMut)]
struct TokioRuntime(Runtime);

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

fn test_wt(#[cfg(not(target_arch = "wasm32"))] tokio_runtime: Res<TokioRuntime>) {
    let wt = async move {
        // read the certificate from the file
        let mut text = String::new();
        File::open("cert/localhost.hex")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        let hash = decode_hex(&text.split_whitespace().next().unwrap());

        // create a new client.
        let client = web_transport::ClientBuilder::new()
            .with_server_certificate_hashes(vec![hash])
            .unwrap();

        let url = "https://localhost:4443/";

        info!("connecting to {}", url);

        // Connect to the given URL.
        let mut session = client.connect(&url.parse().unwrap()).await.unwrap();

        log::info!("connected");

        // Create a bidirectional stream.
        let (mut send, mut recv) = session.open_bi().await.unwrap();

        log::info!("created stream");

        // Send a message.
        let msg = "hello world".to_string();
        send.write(msg.as_bytes()).await.unwrap();
        log::info!("sent: {}", msg);

        drop(send);

        // Read back the message.
        let msg = recv.read(1024).await.unwrap();
        log::info!("recv: {}", String::from_utf8_lossy(&msg.unwrap()));
    };

    #[cfg(target_arch = "wasm32")]
    {
        let task_pool = bevy::tasks::AsyncComputeTaskPool::get();
        task_pool.spawn(wt).detach();
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        tokio_runtime.spawn(wt);
    }
}

pub fn decode_hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

// fn receive_ws(
//     mut commands: Commands,
//     mut client: Query<(Entity, &mut NfwsHandle, &mut ClientConnection)>,
// ) {
//     for (entity, mut ws, mut client) in client.iter_mut() {
//         match ws.next_event() {
//             NfwsPollResult::Event(NfwsEvent::Connecting) => {
//                 info!("Connecting...");
//             }
//             NfwsPollResult::Event(NfwsEvent::Connected) => {
//                 info!("Connected");
//             }
//             NfwsPollResult::Event(NfwsEvent::TextMessage(msg)) => {
//                 let update: GameUpdates = serde_json::from_str(&msg).unwrap();
//                 client.rx.push_back(update);
//             }
//             NfwsPollResult::Event(NfwsEvent::BinaryMessage(_)) => {
//                 info!("Received binary message");
//             }
//             NfwsPollResult::Event(NfwsEvent::Error(err)) => {
//                 info!("Error: {:?}", err);
//             }
//             NfwsPollResult::Event(NfwsEvent::Closed(reason)) => {
//                 info!("Connection closed: {:?}", reason);
//                 commands.entity(entity).despawn();
//             }
//             NfwsPollResult::Closed => {
//                 info!("Connection closed");
//                 commands.entity(entity).despawn();
//             }
//             NfwsPollResult::Empty => {}
//         }
//     }
// }

// fn send_ws(mut ws: Query<(&mut NfwsHandle, &mut ClientConnection)>) {
//     for (mut ws, mut client) in ws.iter_mut() {
//         while let Some(cmd) = client.tx.pop_front() {
//             ws.send_text(serde_json::to_string(&cmd).unwrap());
//         }
//     }
// }
