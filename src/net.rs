//! GGRS rollback netcode over WebRTC DataChannels via matchbox.
//!
//! Architecture:
//! - GGRS runs at [`FPS`] (60Hz). Every GGRS frame each peer submits one
//!   [`u8`]-encoded input (direction bits + restart bit).
//! - The actual snake board only advances every [`FRAMES_PER_MOVEMENT`] GGRS
//!   frames. Between movement frames the [`IntendedDirections`] rollback
//!   resource is updated with each peer's most recent direction intent —
//!   that's what lets the renderer show a remote player's "I'm turning"
//!   intent within a single GGRS frame instead of waiting for the movement
//!   frame to fire.
//! - All RNG-driven sim (apple/wall spawning) runs identically on every peer
//!   via a deterministic seed derived from the sorted peer ids.

use bevy::prelude::*;
use bevy_ggrs::{
    ggrs, GgrsApp, GgrsPlugin, GgrsSchedule, LocalInputs, LocalPlayers, PlayerInputs, ReadInputs,
    Session,
};
use bevy_matchbox::prelude::*;
use bevy_snake::board::{Board, BoardSettings, Direction};
use rand::{rngs::StdRng, SeedableRng};

/// GGRS config: each player sends a `u8`-packed input, addressed by matchbox
/// `PeerId`. State checksum size is the default `u8`.
pub type GameConfig = bevy_ggrs::GgrsConfig<u8, PeerId>;

/// GGRS rollback rate. 60 Hz = 16.6 ms / frame. At this rate input feedback
/// from the remote peer is visible in ~RTT/60Hz frames, regardless of when
/// the next movement frame fires.
pub const FPS: usize = 60;
/// GGRS frames per snake movement step. 8 @ 60 Hz ≈ 7.5 Hz movement, matching
/// the feel of the old WebTransport-era tick rate.
pub const FRAMES_PER_MOVEMENT: u32 = 8;
/// Number of players in a 1v1 match. Hardcoded for now; matchbox `?next=N`
/// in [`ROOM_URL_DEFAULT`] must match.
pub const NUM_PLAYERS: usize = 2;
/// Frames the local input is delayed before being applied. Set to 0 so local
/// presses are visible immediately; GGRS will roll back when remote inputs
/// arrive late instead of forcing every local press to wait. At 60 Hz
/// rollback with a 7.5 Hz logical tick the recovered window is small enough
/// that even a 100 ms RTT only costs ~6 frames of resimulation per remote
/// input, which the snake board sim handles trivially.
pub const INPUT_DELAY: usize = 0;

pub const INPUT_UP: u8 = 1 << 0;
pub const INPUT_DOWN: u8 = 1 << 1;
pub const INPUT_LEFT: u8 = 1 << 2;
pub const INPUT_RIGHT: u8 = 1 << 3;
pub const INPUT_RESTART: u8 = 1 << 4;

/// Decode the direction bits from a raw input byte. None if no direction bit
/// is set this frame (player isn't pressing anything).
pub fn decode_direction(input: u8) -> Option<Direction> {
    if input & INPUT_UP != 0 {
        Some(Direction::Up)
    } else if input & INPUT_DOWN != 0 {
        Some(Direction::Down)
    } else if input & INPUT_LEFT != 0 {
        Some(Direction::Left)
    } else if input & INPUT_RIGHT != 0 {
        Some(Direction::Right)
    } else {
        None
    }
}

/// Counts GGRS frames since the most recent restart. Movement frames fire
/// when `frame % FRAMES_PER_MOVEMENT == 0` (and `frame > 0`).
#[derive(Resource, Default, Clone, Hash)]
pub struct MovementFrame {
    pub frame: u32,
    /// Bumped on every restart so different rounds get different deterministic
    /// RNG sequences (the seed is mixed with this).
    pub generation: u32,
}

impl MovementFrame {
    /// 0..1 fraction through the current movement step. Renderer uses this
    /// to interpolate visible snake positions between board ticks.
    pub fn movement_progress(&self) -> f32 {
        if self.frame == 0 {
            return 0.0;
        }
        ((self.frame % FRAMES_PER_MOVEMENT) as f32) / FRAMES_PER_MOVEMENT as f32
    }
}

/// Base seed for the deterministic RNG. Negotiated once at session start
/// from the sorted peer ids — every peer arrives at the same value
/// independently. `generation` is mixed in per restart so each round uses a
/// fresh RNG sequence.
#[derive(Resource, Clone, Hash, Default)]
pub struct RngState {
    pub seed: u64,
}

/// Per-player queue of upcoming turn directions. Rolled back with the rest
/// of the simulation. A press enqueues at the back (capped at
/// [`MAX_QUEUE_LEN`]); a movement frame consumes the front.
///
/// The queue is what lets quick taps land — without it, two presses between
/// movement frames would only see the most recent one applied. It also lets
/// the renderer "lean" each snake toward the front of its queue so the next
/// turn is visible before it actually fires.
#[derive(Resource, Default, Clone, Hash)]
pub struct InputQueues(pub Vec<Vec<Direction>>);

impl InputQueues {
    /// The direction this player will turn next, or `None` if their queue
    /// is empty (i.e., snake will continue straight).
    pub fn front(&self, player: usize) -> Option<Direction> {
        self.0.get(player).and_then(|q| q.first().copied())
    }
}

/// Local-only buffer that captures `just_pressed` keyboard events between
/// GGRS frames so a press survives the (frequent) Update tick where GGRS's
/// fixed-timestep accumulator doesn't fire `ReadInputs`. Drained on every
/// `ReadInputs` run.
#[derive(Resource, Default)]
pub struct PendingInput {
    pub direction: Option<Direction>,
    pub restart: bool,
}

/// Per-player input queue depth. Three is the value the pre-rollback game
/// used and works well in practice: enough for a tight S-curve, small
/// enough that long-ago presses don't surprise you.
pub const MAX_QUEUE_LEN: usize = 3;

pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(GgrsPlugin::<GameConfig>::default())
            .set_rollback_schedule_fps(FPS)
            .rollback_resource_with_clone::<Board>()
            .rollback_resource_with_clone::<MovementFrame>()
            .rollback_resource_with_clone::<RngState>()
            .rollback_resource_with_clone::<InputQueues>()
            .insert_resource(Board::new(BoardSettings::default()))
            .insert_resource(MovementFrame::default())
            .insert_resource(RngState::default())
            .insert_resource(InputQueues(vec![Vec::new(); NUM_PLAYERS]))
            .insert_resource(PendingInput::default())
            .add_systems(Startup, start_socket)
            .add_systems(Update, (wait_for_players, buffer_local_input))
            .add_systems(ReadInputs, read_local_input)
            .add_systems(
                GgrsSchedule,
                (apply_restart, enqueue_inputs, advance_board).chain(),
            );
    }
}

fn room_url() -> String {
    option_env!("MATCHBOX_ROOM_URL")
        .unwrap_or("ws://localhost:3536/snake?next=2")
        .to_string()
}

fn start_socket(mut commands: Commands) {
    let url = room_url();
    info!("opening matchbox socket: {}", url);
    commands.insert_resource(MatchboxSocket::new_unreliable(url));
}

fn wait_for_players(
    mut commands: Commands,
    mut socket: ResMut<MatchboxSocket>,
    session: Option<Res<Session<GameConfig>>>,
) {
    if session.is_some() {
        return;
    }
    if socket.get_channel(0).is_err() {
        return;
    }

    socket.update_peers();
    let players = socket.players();
    if players.len() < NUM_PLAYERS {
        return;
    }
    info!("got {} players, building GGRS session", players.len());

    let seed = derive_session_seed(&mut socket);
    info!("session seed: {:x}", seed);

    let mut builder = ggrs::SessionBuilder::<GameConfig>::new()
        .with_num_players(NUM_PLAYERS)
        .with_input_delay(INPUT_DELAY)
        .with_desync_detection_mode(ggrs::DesyncDetection::On { interval: 10 });
    for (i, player) in players.into_iter().enumerate() {
        builder = builder.add_player(player, i).expect("add_player");
    }
    let channel = socket.take_channel(0).unwrap();
    let session = builder.start_p2p_session(channel).unwrap();

    commands.insert_resource(RngState { seed });
    commands.insert_resource(Session::P2P(session));
}

fn derive_session_seed(socket: &mut MatchboxSocket) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut ids: Vec<String> = socket
        .connected_peers()
        .map(|id| id.to_string())
        .collect();
    if let Some(self_id) = socket.id() {
        ids.push(self_id.to_string());
    }
    ids.sort();
    let mut h = DefaultHasher::new();
    for id in ids {
        id.hash(&mut h);
    }
    h.finish()
}

/// Captures local key presses into `PendingInput` every `Update`. Runs every
/// Bevy frame regardless of whether GGRS is going to step this frame, so a
/// press survives the (common) Update where the GGRS accumulator hasn't
/// crossed a step boundary. `read_local_input` drains the buffer on the
/// next `ReadInputs`.
fn buffer_local_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut pending: ResMut<PendingInput>,
) {
    if keys.just_pressed(KeyCode::ArrowUp) || keys.just_pressed(KeyCode::KeyW) {
        pending.direction = Some(Direction::Up);
    } else if keys.just_pressed(KeyCode::ArrowDown) || keys.just_pressed(KeyCode::KeyS) {
        pending.direction = Some(Direction::Down);
    } else if keys.just_pressed(KeyCode::ArrowLeft) || keys.just_pressed(KeyCode::KeyA) {
        pending.direction = Some(Direction::Left);
    } else if keys.just_pressed(KeyCode::ArrowRight) || keys.just_pressed(KeyCode::KeyD) {
        pending.direction = Some(Direction::Right);
    }
    if keys.just_pressed(KeyCode::Space) {
        pending.restart = true;
    }
}

fn read_local_input(
    mut commands: Commands,
    local_players: Res<LocalPlayers>,
    mut pending: ResMut<PendingInput>,
) {
    let mut input: u8 = 0;
    if let Some(dir) = pending.direction.take() {
        input |= match dir {
            Direction::Up => INPUT_UP,
            Direction::Down => INPUT_DOWN,
            Direction::Left => INPUT_LEFT,
            Direction::Right => INPUT_RIGHT,
        };
    }
    if pending.restart {
        input |= INPUT_RESTART;
        pending.restart = false;
    }
    let mut inputs = bevy::utils::HashMap::new();
    for handle in local_players.0.iter() {
        inputs.insert(*handle, input);
    }
    commands.insert_resource(LocalInputs::<GameConfig>(inputs));
}

fn apply_restart(
    mut board: ResMut<Board>,
    mut frame: ResMut<MovementFrame>,
    mut queues: ResMut<InputQueues>,
    inputs: Res<PlayerInputs<GameConfig>>,
) {
    let any_restart = inputs.iter().any(|(raw, _)| raw & INPUT_RESTART != 0);
    if !any_restart {
        return;
    }
    info!("restart (frame={}, generation={})", frame.frame, frame.generation);
    *board = Board::new(BoardSettings::default());
    frame.frame = 0;
    frame.generation = frame.generation.wrapping_add(1);
    *queues = InputQueues(vec![Vec::new(); NUM_PLAYERS]);
}

/// Each GGRS frame, push any newly-pressed direction onto the matching
/// player's queue. Deduped against the queue's back (so an Update that
/// fires `just_pressed` once but spans two GGRS frames doesn't enqueue the
/// same direction twice) and capped at [`MAX_QUEUE_LEN`].
fn enqueue_inputs(
    mut queues: ResMut<InputQueues>,
    inputs: Res<PlayerInputs<GameConfig>>,
) {
    if queues.0.len() < NUM_PLAYERS {
        queues.0.resize(NUM_PLAYERS, Vec::new());
    }
    for (i, (raw, _status)) in inputs.iter().enumerate() {
        if i >= NUM_PLAYERS {
            break;
        }
        let Some(dir) = decode_direction(*raw) else {
            continue;
        };
        let q = &mut queues.0[i];
        if q.last().copied() == Some(dir) {
            continue;
        }
        if q.len() >= MAX_QUEUE_LEN {
            continue;
        }
        q.push(dir);
    }
}

fn advance_board(
    mut board: ResMut<Board>,
    mut frame: ResMut<MovementFrame>,
    mut queues: ResMut<InputQueues>,
    rng_state: Res<RngState>,
) {
    frame.frame = frame.frame.wrapping_add(1);

    if frame.frame % FRAMES_PER_MOVEMENT != 0 {
        return;
    }

    // Per-movement-frame RNG seed: deterministic on rollback because all
    // operands live in rollback state.
    let mix = rng_state
        .seed
        .wrapping_add(frame.frame as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (frame.generation as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let mut rng = StdRng::seed_from_u64(mix);

    // Drain one direction per player from the front of their queue. An empty
    // queue passes `None`, which `Board::tick` interprets as "keep going
    // straight".
    let mut dirs: Vec<Option<Direction>> = Vec::with_capacity(NUM_PLAYERS);
    for i in 0..NUM_PLAYERS {
        let dir = queues
            .0
            .get_mut(i)
            .and_then(|q| if q.is_empty() { None } else { Some(q.remove(0)) });
        dirs.push(dir);
    }

    if let Err(e) = board.tick(&dirs, &mut rng) {
        warn!("board tick error: {}", e);
    }
}
