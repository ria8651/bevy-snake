use bevy::prelude::*;
use bevy_nfws::{NfwsEvent, NfwsHandle, NfwsPlugin, NfwsPollResult};
use bevy_snake::{GameCommands, GameUpdates};
use std::collections::VecDeque;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(NfwsPlugin)
            .add_systems(PreUpdate, receive_ws)
            .add_systems(PostUpdate, send_ws);
    }
}

#[derive(Component, Default)]
#[require(NfwsHandle(|| NfwsHandle::new("wss://misc.bink.eu.org/ws".to_string())))]
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

fn receive_ws(
    mut commands: Commands,
    mut client: Query<(Entity, &mut NfwsHandle, &mut ClientConnection)>,
) {
    for (entity, mut ws, mut client) in client.iter_mut() {
        match ws.next_event() {
            NfwsPollResult::Event(NfwsEvent::Connecting) => {
                info!("Connecting...");
            }
            NfwsPollResult::Event(NfwsEvent::Connected) => {
                info!("Connected");
            }
            NfwsPollResult::Event(NfwsEvent::TextMessage(msg)) => {
                let update: GameUpdates = serde_json::from_str(&msg).unwrap();
                client.rx.push_back(update);
            }
            NfwsPollResult::Event(NfwsEvent::BinaryMessage(_)) => {
                info!("Received binary message");
            }
            NfwsPollResult::Event(NfwsEvent::Error(err)) => {
                info!("Error: {:?}", err);
            }
            NfwsPollResult::Event(NfwsEvent::Closed(reason)) => {
                info!("Connection closed: {:?}", reason);
                commands.entity(entity).despawn();
            }
            NfwsPollResult::Closed => {
                info!("Connection closed");
                commands.entity(entity).despawn();
            }
            NfwsPollResult::Empty => {}
        }
    }
}

fn send_ws(mut ws: Query<(&mut NfwsHandle, &mut ClientConnection)>) {
    for (mut ws, mut client) in ws.iter_mut() {
        while let Some(cmd) = client.tx.pop_front() {
            ws.send_text(serde_json::to_string(&cmd).unwrap());
        }
    }
}
