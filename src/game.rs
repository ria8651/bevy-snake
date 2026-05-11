use crate::{
    client::{ClientConnection, NetworkUpdate},
    ClientState, ConnectionError, ErrorKind, GizmoSetting, Settings,
};
use bevy::{prelude::*, utils::HashMap};
use bevy_snake::{
    ai::{cycle_basis, AIGizmos, SnakeAI, TreeSearch},
    board::{Board, BoardEvent, Cell, Direction},
    GameCommands, GameUpdates,
};
use rand::{rngs::StdRng, SeedableRng};
use std::{collections::VecDeque, time::Duration};
use web_time::{SystemTime, UNIX_EPOCH};

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(TickTimer(Timer::from_seconds(1.0 / 7.5, TimerMode::Repeating)))
            .insert_resource(Board::empty(0, 0))
            .insert_resource(AuthoritativeBoard(Board::empty(0, 0)))
            .insert_resource(AuthoritativeTick(0))
            .insert_resource(PredictedTick(0))
            .insert_resource(LastAppliedInputs(Vec::new()))
            .insert_resource(LocalInputLog(VecDeque::new()))
            .insert_resource(LocalSnakeId(0))
            .insert_resource(Rng(StdRng::from_os_rng()))
            .insert_resource(Points(vec![0; 4]))
            .insert_resource(SnakeInputs(vec![
                SnakeInput {
                    input_map: InputMap {
                        up: KeyCode::KeyW,
                        down: KeyCode::KeyS,
                        left: KeyCode::KeyA,
                        right: KeyCode::KeyD,
                        shoot: KeyCode::Space,
                    },
                    input_queue: VecDeque::new(),
                },
                SnakeInput {
                    input_map: InputMap {
                        up: KeyCode::ArrowUp,
                        down: KeyCode::ArrowDown,
                        left: KeyCode::ArrowLeft,
                        right: KeyCode::ArrowRight,
                        shoot: KeyCode::AltRight,
                    },
                    input_queue: VecDeque::new(),
                },
                SnakeInput {
                    input_map: InputMap {
                        up: KeyCode::KeyP,
                        down: KeyCode::Semicolon,
                        left: KeyCode::KeyL,
                        right: KeyCode::Quote,
                        shoot: KeyCode::Backslash,
                    },
                    input_queue: VecDeque::new(),
                },
                SnakeInput {
                    input_map: InputMap {
                        up: KeyCode::KeyY,
                        down: KeyCode::KeyH,
                        left: KeyCode::KeyG,
                        right: KeyCode::KeyJ,
                        shoot: KeyCode::KeyB,
                    },
                    input_queue: VecDeque::new(),
                },
            ]))
            .add_systems(Startup, create_client)
            .add_systems(Update, reset_game)
            .add_systems(Update, update_game);
    }
}

#[derive(Resource, Deref, DerefMut)]
pub struct TickTimer(Timer);

#[derive(Resource, Deref, DerefMut)]
pub struct SnakeInputs(Vec<SnakeInput>);

#[derive(Resource, Deref, DerefMut)]
pub struct Points(Vec<usize>);

pub struct SnakeInput {
    pub input_map: InputMap,
    pub input_queue: VecDeque<Direction>,
}

/// The most recent fully-authoritative server snapshot. Replays of the
/// local input log start from here.
#[derive(Resource, Deref, DerefMut)]
pub struct AuthoritativeBoard(Board);

/// The tick number the authoritative board was at when the server sent it.
#[derive(Resource, Deref, DerefMut)]
pub struct AuthoritativeTick(u64);

/// How far the predicted `Board` has been advanced past the authoritative
/// snapshot. Equal to `AuthoritativeTick + log.len()` after a reconcile.
#[derive(Resource, Deref, DerefMut)]
pub struct PredictedTick(u64);

/// Per-snake direction the server applied on the most recent authoritative
/// tick. Used for opponents when we predict forward — we don't see their
/// inputs, so we assume they keep doing what the server last saw them do.
#[derive(Resource, Deref, DerefMut)]
pub struct LastAppliedInputs(Vec<Option<Direction>>);

/// Local inputs sent but not yet acknowledged by a server snapshot, in
/// submission order. Each entry is the (tick, direction) it was meant for.
/// Replayed against the authoritative board on reconcile.
#[derive(Resource, Deref, DerefMut)]
pub struct LocalInputLog(VecDeque<(u64, Direction)>);

/// Which snake id we control on this client. Today the convention is "the
/// first registered client controls snake 0," and we have no other source
/// of truth — server doesn't tell us our id. For a single player this is 0.
#[derive(Resource, Deref, DerefMut)]
pub struct LocalSnakeId(u8);

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct InputMap {
    pub up: KeyCode,
    pub down: KeyCode,
    pub left: KeyCode,
    pub right: KeyCode,
    pub shoot: KeyCode,
}

pub fn create_client(mut commands: Commands) {
    commands.spawn(ClientConnection::new(get_wt_url()));
}

#[cfg(target_arch = "wasm32")]
pub fn get_wt_url() -> String {
    use wasm_bindgen::JsValue;
    let win = web_sys::window().expect("no window");
    let val = js_sys::Reflect::get(&win, &JsValue::from_str("WT_URL"))
        .expect("failed to read window.WT_URL");
    val.as_string().expect("window.WT_URL must be a string")
}

#[cfg(not(target_arch = "wasm32"))]
pub fn get_wt_url() -> String {
    std::env::var("WT_URL").unwrap_or_else(|_| "https://localhost:1234".to_string())
}

pub fn reset_game(
    mut board: ResMut<Board>,
    mut auth_board: ResMut<AuthoritativeBoard>,
    mut auth_tick: ResMut<AuthoritativeTick>,
    mut predicted_tick: ResMut<PredictedTick>,
    mut last_applied: ResMut<LastAppliedInputs>,
    mut input_log: ResMut<LocalInputLog>,
    mut input_queues: ResMut<SnakeInputs>,
    mut client_connections: Query<&mut ClientConnection>,
    settings: Res<Settings>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    if keys.just_pressed(KeyCode::Space) {
        *board = Board::new(settings.board_settings);
        **auth_board = board.clone();
        **auth_tick = 0;
        **predicted_tick = 0;
        last_applied.clear();
        input_log.clear();

        for SnakeInput { input_queue, .. } in input_queues.iter_mut() {
            input_queue.clear();
        }

        if !client_connections.is_empty() {
            client_connections
                .single_mut()
                .send_command(GameCommands::RestartGame {
                    board_settings: settings.board_settings.clone(),
                });
        }
    }
}

#[derive(Resource, Deref, DerefMut)]
pub struct Rng(StdRng);

pub fn update_game(
    mut input_queues: ResMut<SnakeInputs>,
    mut timer: ResMut<TickTimer>,
    mut board: ResMut<Board>,
    mut auth_board: ResMut<AuthoritativeBoard>,
    mut auth_tick: ResMut<AuthoritativeTick>,
    mut predicted_tick: ResMut<PredictedTick>,
    mut last_applied: ResMut<LastAppliedInputs>,
    mut input_log: ResMut<LocalInputLog>,
    local_snake: Res<LocalSnakeId>,
    mut points: ResMut<Points>,
    mut client_connections: Query<&mut ClientConnection>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut next_state: ResMut<NextState<ClientState>>,
    mut connection_error: ResMut<ConnectionError>,
) {
    timer.tick(time.delta());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    // No connection entity right now (e.g. between Retry click and respawn).
    // Skip the network section entirely — predicted board still ticks below.
    let Ok(mut client_connection) = client_connections.get_single_mut() else {
        return;
    };

    // 1. Consume server snapshots: trust authoritative state, drop ack'd
    //    inputs from the log, and re-predict forward from the new baseline.
    if let Some(game_updates) = client_connection.receive_update() {
        match game_updates {
            NetworkUpdate::Update(GameUpdates::Ticked {
                board: new_board,
                events,
                tick,
                timestamp,
                applied_inputs,
            }) => {
                info!("received {} ({}ms ping)", tick, now - timestamp);

                **auth_board = new_board.clone();
                **auth_tick = tick;
                **last_applied = applied_inputs;

                // Drop any local inputs the server has now seen.
                while matches!(input_log.front(), Some((t, _)) if *t <= tick) {
                    input_log.pop_front();
                }

                // Reconcile: start from the authoritative snapshot and replay
                // any still-unack'd local inputs forward, ticking the board
                // without RNG (apples/walls only come from the server).
                let mut next = new_board.clone();
                let mut t = tick;
                for &(input_tick, dir) in input_log.iter() {
                    if input_tick <= t {
                        // Shouldn't happen after the drain above, but be safe.
                        continue;
                    }
                    if !apply_tick(&mut next, dir, &last_applied, **local_snake) {
                        break;
                    }
                    t += 1;
                    let _ = input_tick; // keep field for clarity
                }
                *board = next;
                **predicted_tick = t;

                for event in events {
                    match event {
                        BoardEvent::GameOver => {
                            info!("game over");
                            // No early return — we still want predicted board
                            // to display the final state and inputs cleared.
                        }
                        BoardEvent::SnakeDamaged { .. } => {
                            for (snake_id, _) in board.snakes().into_iter() {
                                points[snake_id as usize] += 1;
                            }
                        }
                        _ => {}
                    }
                }

                // Align our local tick timer to the server's cadence: a server
                // snapshot just landed, so the next local tick is in 133 ms.
                timer.reset();

                // Now that the server ack'd the head of the queue, send the
                // next queued input (visual queue, separate from input_log).
                for SnakeInput { input_queue, .. } in input_queues.iter_mut() {
                    input_queue.pop_front();
                    if let Some(&direction) = input_queue.front() {
                        let input = GameCommands::Input {
                            direction,
                            tick,
                            timestamp: now,
                        };
                        info!("sending {:?} ({})", input, tick);
                        client_connection.send_command(input);
                    }
                }
            }
            NetworkUpdate::Connected => {
                info!("connected");
                next_state.set(ClientState::Connected);
            }
            NetworkUpdate::Disconnected => {
                info!("disconnected");
                *connection_error = ConnectionError {
                    kind: ErrorKind::Disconnected,
                    detail: String::new(),
                };
                next_state.set(ClientState::Error);
            }
            NetworkUpdate::ConnectFailed(kind, detail) => {
                info!("connect failed: {:?} {}", kind, detail);
                *connection_error = ConnectionError { kind, detail };
                next_state.set(ClientState::Error);
            }
        }
    }

    // 2. Local prediction tick: between server snapshots, advance the
    //    displayed board on its own timer so the player sees instant
    //    feedback. Uses the head of the local input log for our snake, and
    //    the server's most recently applied inputs for opponents.
    if timer.just_finished() && board.width() > 0 {
        let next_tick = **predicted_tick + 1;
        let local_dir = input_log
            .iter()
            .find(|(t, _)| *t == next_tick)
            .map(|(_, d)| *d);
        let mut next = board.clone();
        if apply_tick(
            &mut next,
            local_dir.unwrap_or_else(|| local_snake_dir(&board, **local_snake)),
            &last_applied,
            **local_snake,
        ) {
            *board = next;
            **predicted_tick = next_tick;
        }
    }

    // 3. Collect local key presses, push into the visual input_queue (which
    //    gates "send next input on snapshot ack"), and into the canonical
    //    LocalInputLog used for replay. Also fire the immediate send for the
    //    head of the queue.
    for (snake_idx, SnakeInput { input_map, input_queue }) in
        input_queues.iter_mut().enumerate()
    {
        if input_queue.len() >= 3 {
            continue;
        }
        let pressed = if keys.just_pressed(input_map.up) {
            Some(Direction::Up)
        } else if keys.just_pressed(input_map.down) {
            Some(Direction::Down)
        } else if keys.just_pressed(input_map.left) {
            Some(Direction::Left)
        } else if keys.just_pressed(input_map.right) {
            Some(Direction::Right)
        } else {
            None
        };
        let Some(input) = pressed else { continue };

        let last_in_queue = input_queue.back();
        if last_in_queue.is_some_and(|&last| input == last || input == last.opposite()) {
            continue;
        }

        // Only the local snake's inputs are predicted/replayed. For
        // multi-player on the same machine the other snakes' keys still
        // queue + send; we just don't predict their snake locally.
        let is_local = snake_idx as u8 == **local_snake;

        if input_queue.is_empty() {
            // Send the immediate input now. Tick is the next predicted tick —
            // matches what we'll predict locally on the next timer fire.
            let input_tick = **predicted_tick + 1;
            let cmd = GameCommands::Input {
                direction: input,
                tick: input_tick,
                timestamp: now,
            };
            info!("sending {:?} ({})", cmd, input_tick);
            client_connection.send_command(cmd);
            if is_local {
                input_log.push_back((input_tick, input));
            }
        }
        input_queue.push_back(input);
    }
}

/// Apply one tick of deterministic simulation to `board`, treating
/// `local_dir` as the local snake's input and `last_applied[i]` as the most
/// recently-server-seen direction for other snakes. Returns `false` if the
/// tick errored (e.g., not enough inputs for the board's snake count), in
/// which case `board` is left untouched.
fn apply_tick(
    board: &mut Board,
    local_dir: Direction,
    last_applied: &[Option<Direction>],
    local_snake: u8,
) -> bool {
    // Build the inputs vec sized to the highest snake id present. We need at
    // least max(snake_id, local_snake) + 1 slots.
    let snake_count = board
        .snakes()
        .keys()
        .copied()
        .max()
        .map(|m| m as usize + 1)
        .unwrap_or(0)
        .max(local_snake as usize + 1)
        .max(last_applied.len());
    let mut inputs: Vec<Option<Direction>> = vec![None; snake_count];
    for (i, slot) in inputs.iter_mut().enumerate() {
        // For non-local snakes, reuse the server's most recently applied
        // direction. This is the standard "constant velocity" prediction —
        // it's wrong when the opponent turns, and the next snapshot will fix
        // that.
        if i as u8 == local_snake {
            *slot = Some(local_dir);
        } else if i < last_applied.len() {
            *slot = last_applied[i];
        }
    }
    let snapshot = board.clone();
    match board.tick_board_no_spawn(&inputs) {
        Ok(_) => true,
        Err(e) => {
            warn!("predicted tick failed: {}", e);
            *board = snapshot;
            false
        }
    }
}

/// What direction is the local snake heading on the displayed board? Used as
/// a fallback when the input log doesn't have anything queued for the next
/// predicted tick — predict it'll keep moving forward.
fn local_snake_dir(board: &Board, local_snake: u8) -> Direction {
    board
        .snakes()
        .get(&local_snake)
        .map(|s| s.dir)
        .unwrap_or(Direction::Right)
}

pub struct AIPlugin;

impl Plugin for AIPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, ai_system.after(update_game));
    }
}

fn ai_system(
    mut input_queues: ResMut<SnakeInputs>,
    mut gizmos: Gizmos,
    mut ai_gizmos: Local<AIGizmos>,
    settings: Res<Settings>,
    board: Res<Board>,
    tick_timer: Res<TickTimer>,
) {
    if tick_timer.just_finished() || !settings.do_game_tick {
        // let ai = RandomWalk;
        let ai = TreeSearch {
            max_depth: 100,
            max_time: Duration::from_millis(5),
        };

        let mut new_ai_gizmos = AIGizmos::default();

        if let Ok(dir) = ai.chose_move(board.as_ref(), &mut Some(&mut new_ai_gizmos)) {
            let input_queue = &mut input_queues[0].input_queue;
            if settings.ai && input_queue.is_empty() {
                input_queue.push_back(dir);
            }
        }

        *ai_gizmos = new_ai_gizmos;
    }

    if let GizmoSetting::CycleBasis = settings.gizmos {
        // find cycle basis of the board
        let mut nodes = HashMap::new();
        let mut graph = Vec::new();
        for (pos, cell) in board.cells() {
            if !matches!(cell, Cell::Wall) {
                nodes.insert(pos, nodes.len());
                graph.push(Vec::new());
            }
        }
        for (node, index) in nodes.iter() {
            for dir in Direction::ALL {
                let next_node = *node + dir.as_vec2();
                if let Some(next_index) = nodes.get(&next_node) {
                    graph[*index].push(*next_index);
                }
            }
        }
        let cycles = cycle_basis(&graph);

        // show cycles
        let points: HashMap<_, _> = nodes.iter().map(|(pos, index)| (*index, *pos)).collect();
        for (index, cycle) in cycles.iter().enumerate() {
            let color = Color::srgb(
                (index as f32 / cycles.len() as f32).min(1.0),
                0.0,
                1.0 - (index as f32 / cycles.len() as f32).min(1.0),
            );
            let com = cycle
                .iter()
                .fold(Vec2::ZERO, |acc, &index| acc + points[&index].as_vec2())
                / cycle.len() as f32;
            let points = cycle
                .iter()
                .map(|&index| points[&index].as_vec2() - (points[&index].as_vec2() - com) * 0.1)
                .collect::<Vec<_>>();
            for i in 0..cycle.len() {
                let start = points[i];
                let end = points[(i + 1) % cycle.len()];
                ai_gizmos.arrows.push((start, end, color));
            }
        }

        // combine cycles
        let mut edges = HashMap::new();
        let mut index = 0;
        for (cell, neighbors) in graph.iter().enumerate() {
            for &neighbor in neighbors {
                if neighbor < cell {
                    edges.insert((neighbor, cell), index);
                    index += 1;
                }
            }
        }
        let mut edge_cycles = Vec::new();
        for cycle in cycles.iter() {
            let mut edge_cycle = Vec::new();
            for i in 0..cycle.len() {
                let mut a = cycle[i];
                let mut b = cycle[(i + 1) % cycle.len()];
                if a > b {
                    std::mem::swap(&mut a, &mut b);
                }
                edge_cycle.push(*edges.get(&(a.min(b), a.max(b))).unwrap());
            }
            edge_cycles.push(edge_cycle);
        }
    }

    if !matches!(settings.gizmos, GizmoSetting::None) {
        let board_pos = |pos: Vec2| {
            Vec2::new(
                pos.x as f32 - board.width() as f32 / 2.0 + 0.5,
                pos.y as f32 - board.height() as f32 / 2.0 + 0.5,
            )
        };
        for (start, end, color) in ai_gizmos.lines.iter() {
            gizmos.line_2d(board_pos(start.as_vec2()), board_pos(end.as_vec2()), *color);
        }
        for (start, end, color) in ai_gizmos.arrows.iter() {
            gizmos.arrow_2d(board_pos(*start), board_pos(*end), *color);
        }
        for (pos, color) in ai_gizmos.points.iter() {
            gizmos.circle_2d(board_pos(pos.as_vec2()), 0.3, *color);
        }
    }
}
