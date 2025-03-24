use bevy::prelude::*;
use bevy_nfws::{NfwsHandle, NfwsPlugin, NfwsPollResult};
use bevy_snake::{GameCommands, GameUpdates};
use std::collections::VecDeque;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(NfwsPlugin)
            .add_systems(Startup, start_ws)
            .add_systems(Update, poll_ws);
    }
}

#[derive(Component, Default)]
#[require(NfwsHandle(|| NfwsHandle::new("ws://localhost:1234/ws".to_string())))]
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

fn start_ws(mut commands: Commands) {
    let ws = NfwsHandle::new("ws://localhost:1234/ws".to_string());
    commands.spawn(ws);

    info!("Started websocket connection");
}

fn poll_ws(mut commands: Commands, mut ws: Query<(Entity, &mut NfwsHandle)>) {
    for (entity, mut ws) in ws.iter_mut() {
        match ws.next_event() {
            NfwsPollResult::Closed => {
                info!("Connection closed");
                commands.entity(entity).despawn();
            }
            NfwsPollResult::Empty => {}
            NfwsPollResult::Event(msg) => {
                info!("Received message: {:?}", msg);
            }
        }
    }
}
