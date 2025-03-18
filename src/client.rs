use async_tungstenite::{tokio::connect_async, tungstenite::Message};
use bevy::prelude::*;
use bevy_snake::{GameCommands, GameUpdates};
use crossbeam::channel::{unbounded, Receiver, Sender};
use futures::{SinkExt, StreamExt};
use tokio::runtime::{self, Runtime};

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(RuntimeResource::default())
            .insert_resource(PlayerConnections::default())
            .add_systems(Startup, start_ws);
    }
}

#[derive(Resource, Deref, DerefMut)]
struct RuntimeResource(&'static Runtime);

impl Default for RuntimeResource {
    fn default() -> Self {
        let runtime = Box::leak(Box::new(
            runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap(),
        ));
        RuntimeResource(runtime)
    }
}

#[derive(Resource, Deref, DerefMut, Default)]
pub struct PlayerConnections(Vec<(Receiver<GameUpdates>, Sender<GameCommands>)>);

fn start_ws(mut player_connections: ResMut<PlayerConnections>, runtime: Res<RuntimeResource>) {
    let (tx_updates, rx_updates) = unbounded();
    let (tx_commands, rx_commands) = unbounded();
    player_connections.push((rx_updates, tx_commands));

    let runtime = runtime.handle();

    runtime.spawn(async move {
        let (ws_stream, _) = connect_async("ws://localhost:1234/ws").await.unwrap();
        let (mut write, read) = ws_stream.split();
        info!("Connected to server");

        runtime.spawn(async move {
            // receive messages from server
            read.for_each(|msg| async {
                match msg {
                    Ok(Message::Text(msg)) => {
                        let game_updates: GameUpdates =
                            serde_json::from_str(&msg.to_string()).unwrap();
                        tx_updates.send(game_updates).unwrap();
                    }
                    Err(e) => {
                        error!("Error receiving message: {:?}", e);
                    }
                    _ => {}
                }
            })
            .await;
        });

        runtime.spawn_blocking(move || {
            // send messages to server
            loop {
                let game_command = rx_commands.recv().unwrap();
                info!("Sending message: {:?}", game_command);
                let msg = Message::Text(serde_json::to_string(&game_command).unwrap().into());
                runtime.block_on(write.send(msg)).unwrap();
            }
        });
    });
}
