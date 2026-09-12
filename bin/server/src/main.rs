use vangers::{
    config, level,
    physics::{self, CarPhysicsData, Dynamo},
    space,
};
use vangers_net::{
    decode, encode, AgentState, ClientMessage, CycleFade, CycleState, NetControl, NetDynamo,
    NetTransform, PlayerCirt, PlayerId, ServerMessage,
};

use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use glam::Vec3;
use log::{error, info, warn};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    time::{self, Duration},
};
use tokio_tungstenite::tungstenite;

/// Vangers headless multiplayer server
#[derive(Parser)]
struct Cli {
    /// TCP port to listen on
    #[arg(short, long, default_value = "7800")]
    port: u16,

    /// WebSocket port to listen on (for WASM clients)
    #[arg(long, default_value = "7801")]
    ws_port: u16,

    /// Maximum number of players
    #[arg(long, default_value = "8")]
    max_players: usize,

    /// Server tick rate in Hz
    #[arg(long, default_value = "20")]
    tick_rate: u32,

    /// Level to host: `test`, a `world.ini` path, or a world name from `wrlds.dat`.
    /// When omitted, uses `settings.game.level` (resolved to `world.ini`) if game
    /// data is present; otherwise the procedural `test` level (CI / no data).
    #[arg(short, long)]
    level: Option<String>,

    /// Path to settings file (default: config/settings.ron)
    #[arg(short, long, default_value = "config/settings.ron")]
    settings: String,
}

/// Events from client connection tasks to the main game loop.
///
/// `conn_id` is assigned at accept time. On `Join`, the server may map it to a
/// previously used `player_id` when the same `--name` reconnects (see
/// `name_ids` / `conn_to_player` in the game loop).
enum SessionEvent {
    Connected {
        conn_id: PlayerId,
        sender: mpsc::UnboundedSender<Vec<u8>>,
    },
    Message {
        conn_id: PlayerId,
        msg: ClientMessage,
    },
    Disconnected {
        conn_id: PlayerId,
    },
}

/// How long a disconnected player's last pose / cycle carry is kept for reclaim.
const RECONNECT_POSE_GRACE: Duration = Duration::from_secs(5 * 60);

/// Last per-player snapshot after leave/disconnect, restored on same-name reclaim.
///
/// Holds WorldState pose (transform/dynamo) plus story-cycle carry (`cirtainer`).
/// Shared world cycle banks stay on the server `Bunch` regardless.
struct LastPose {
    transform: space::Transform,
    dynamo: Dynamo,
    cirtainer: level::cycle::Cirtainer,
    disconnected_at: Instant,
}

/// Optional test-only cirtainer seed (`VANGERS_TEST_SEED_CIRT=7,0,2`).
///
/// Applied on **fresh** spawn only so integration tests can prove reclaim
/// restores `LastPose.cirtainer` rather than re-applying the seed.
fn test_seed_cirtainer() -> Option<level::cycle::Cirtainer> {
    let raw = std::env::var("VANGERS_TEST_SEED_CIRT").ok()?;
    let held: Vec<i32> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<i32>())
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if held.is_empty() {
        None
    } else {
        Some(level::cycle::Cirtainer::from_held(held))
    }
}

/// Server-side agent with full physics state.
struct ServerAgent {
    name: String,
    car_name: String,
    color: u8,
    control: NetControl,
    transform: space::Transform,
    dynamo: Dynamo,
    phys_data: CarPhysicsData,
    cirtainer: level::cycle::Cirtainer,
    sender: mpsc::UnboundedSender<Vec<u8>>,
    joined: bool,
}

impl ServerAgent {
    fn to_agent_state(&self, player_id: PlayerId) -> AgentState {
        AgentState {
            player_id,
            transform: NetTransform {
                position: self.transform.disp.into(),
                rotation: [
                    self.transform.rot.x,
                    self.transform.rot.y,
                    self.transform.rot.z,
                    self.transform.rot.w,
                ],
                scale: self.transform.scale,
            },
            dynamo: NetDynamo {
                traction: self.dynamo.traction,
                rudder: self.dynamo.rudder,
                linear_velocity: self.dynamo.linear_velocity.into(),
                angular_velocity: self.dynamo.angular_velocity.into(),
            },
        }
    }

    fn apply_control(&mut self, dt: f32, common: &config::common::Common) {
        let control = &self.control;
        if control.rudder != 0.0 {
            let angle = self.dynamo.rudder + common.car.rudder_step * 2.0 * dt * control.rudder;
            self.dynamo.rudder = angle.clamp(-common.car.rudder_max, common.car.rudder_max);
        }
        if control.motor != 0.0 {
            self.dynamo
                .change_traction(control.motor * dt * common.car.traction_incr);
        }
        if control.brake && self.dynamo.traction != 0.0 {
            self.dynamo.traction *= (-dt).exp2();
        }
    }
}

/// Resolved terrain + the name reported in Welcome / used for story cycles.
struct HostedLevel {
    /// Absolute or relative `world.ini`, or `None` for the procedural test level.
    ini_path: Option<std::path::PathBuf>,
    /// Welcome / cycle world name (`"test"`, `"Fostral"`, …).
    name: String,
}

impl HostedLevel {
    fn test() -> Self {
        Self {
            ini_path: None,
            name: "test".into(),
        }
    }

    fn is_test(&self) -> bool {
        self.ini_path.is_none()
    }
}

/// Resolve `--level` / settings into terrain + a name that matches what clients load.
fn resolve_hosted_level(
    cli_level: Option<&str>,
    settings: Option<&config::Settings>,
) -> HostedLevel {
    match cli_level {
        Some("test") => HostedLevel::test(),
        Some(arg) => resolve_level_arg(arg, settings),
        None => resolve_default_level(settings),
    }
}

fn resolve_default_level(settings: Option<&config::Settings>) -> HostedLevel {
    let Some(settings) = settings else {
        return HostedLevel::test();
    };
    let name = settings.game.level.trim();
    if name.is_empty() {
        return HostedLevel::test();
    }
    match resolve_world_ini(settings, name) {
        Some(hosted) => hosted,
        None => {
            warn!(
                "settings.game.level={:?} but world.ini not found under {:?} — falling back to test",
                name, settings.data_path
            );
            HostedLevel::test()
        }
    }
}

fn resolve_level_arg(arg: &str, settings: Option<&config::Settings>) -> HostedLevel {
    let path = std::path::Path::new(arg);
    let looks_like_path =
        path.is_file() || arg.ends_with(".ini") || arg.contains('/') || arg.contains('\\');
    if looks_like_path {
        let resolved = resolve_ini_filesystem_path(path, settings);
        let name = world_name_for_ini_path(&resolved, settings).unwrap_or_else(|| arg.to_string());
        return HostedLevel {
            ini_path: Some(resolved),
            name,
        };
    }
    // World name (e.g. Fostral / Glorx)
    if let Some(settings) = settings {
        if let Some(hosted) = resolve_world_ini(settings, arg) {
            return hosted;
        }
        warn!(
            "Unknown world name {:?} under {:?} — treating as world.ini path",
            arg, settings.data_path
        );
    }
    HostedLevel {
        ini_path: Some(path.to_path_buf()),
        name: arg.to_string(),
    }
}

/// Prefer an existing path as given; otherwise try under `settings.data_path`.
fn resolve_ini_filesystem_path(
    path: &std::path::Path,
    settings: Option<&config::Settings>,
) -> std::path::PathBuf {
    if path.is_file() {
        return path.to_path_buf();
    }
    if let Some(settings) = settings {
        let under_data = settings.data_path.join(path);
        if under_data.is_file() {
            return under_data;
        }
    }
    path.to_path_buf()
}

fn resolve_world_ini(settings: &config::Settings, name: &str) -> Option<HostedLevel> {
    let worlds = config::worlds::try_load_from_path(&settings.data_path)?;
    let path = config::worlds::ini_path_if_present(&settings.data_path, &worlds, name)?;
    let display = config::worlds::canonical_name(&worlds, name)
        .unwrap_or(name)
        .to_string();
    Some(HostedLevel {
        ini_path: Some(path),
        name: display,
    })
}

/// Prefer the settings / wrlds.dat key when `ini` matches a known world.
fn world_name_for_ini_path(
    ini: &std::path::Path,
    settings: Option<&config::Settings>,
) -> Option<String> {
    let settings = settings?;
    let worlds = config::worlds::try_load_from_path(&settings.data_path)?;
    // Exact match against resolved paths
    for (name, rel) in &worlds {
        let candidate = settings.data_path.join(rel);
        if paths_equal(&candidate, ini) {
            return Some(name.clone());
        }
    }
    // settings.game.level if it resolves to the same file
    let level = settings.game.level.trim();
    if !level.is_empty() {
        if let Some(path) =
            config::worlds::ini_path_if_present(&settings.data_path, &worlds, level)
        {
            if paths_equal(&path, ini) {
                return Some(
                    config::worlds::canonical_name(&worlds, level)
                        .unwrap_or(level)
                        .to_string(),
                );
            }
        }
    }
    None
}

fn paths_equal(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

/// Find a spawn point on the level terrain.
fn find_spawn_point(level: &level::Level, index: usize) -> (i32, i32) {
    let spacing = 30;
    let base_x = level.size.0 / 4;
    let base_y = level.size.1 / 4;
    let x = base_x + (index as i32 % 4) * spacing;
    let y = base_y + (index as i32 / 4) * spacing;
    (x.rem_euclid(level.size.0), y.rem_euclid(level.size.1))
}

#[tokio::main]
async fn main() {
    env_logger::init();
    let cli = Cli::parse();

    // Try to load settings (same config file as the client).
    // Falls back to test defaults when the file or game data is missing (e.g. CI).
    let settings = config::Settings::try_load(&cli.settings);
    if settings.is_none() {
        warn!(
            "Could not load settings from '{}' — using test defaults",
            cli.settings
        );
    }

    // Load level: prefer settings.game.level (same as clients) when --level
    // is omitted and game data is present; `--level test` stays for CI.
    let hosted = resolve_hosted_level(cli.level.as_deref(), settings.as_ref());
    let level_src = match &hosted.ini_path {
        None => "procedural test".to_string(),
        Some(p) => p.display().to_string(),
    };
    info!("Loading level: {} ({})", hosted.name, level_src);
    let level_config = match &hosted.ini_path {
        None => level::LevelConfig::new_test(),
        Some(path) => level::LevelConfig::load(path),
    };
    let geometry = settings
        .as_ref()
        .map(|s| s.game.geometry)
        .unwrap_or_default();
    let mut level = level::load(&level_config, &geometry);
    info!(
        "Level loaded: {}x{} (test={})",
        level.size.0,
        level.size.1,
        hosted.is_test()
    );

    // Story cycles (cirt → escave → palette). Server is authoritative so
    // native TCP and web WS clients stay on the same stage / banks.
    // Name matches the hosted terrain (Welcome + cycle), not a mismatched
    // settings.game.level when `--level test` was forced.
    let cycle_world = hosted.name.clone();
    let mut cycle = if let Some(ref settings) = settings {
        if !settings.check_path("bunches.prm") {
            None
        } else {
            let bunches = config::bunches::load(settings.open_relative("bunches.prm"));
            let mut escaves = config::escaves::load_optional(settings, "escaves.prm");
            escaves.extend(config::escaves::load_optional(settings, "spots.prm"));
            level::cycle::Bunch::load(&cycle_world, &level, &bunches, &escaves, |path| {
                settings
                    .check_path(path)
                    .then(|| std::fs::read(settings.data_path.join(path)).ok())
                    .flatten()
            })
        }
    } else {
        None
    };
    if let Some(ref bunch) = cycle {
        level.palette = *bunch.settled_palette();
        info!(
            "Story cycle loaded for '{}': {} stages, escave {}",
            cycle_world,
            bunch.stages.len(),
            bunch.escave
        );
    } else {
        info!("No story cycle for '{}' (test/bonus world or missing data)", cycle_world);
    }

    // Load physics constants and car data from game files when available.
    let (common, car_physics) = if let Some(ref settings) = settings {
        let common = config::common::load(settings.open_relative("common.prm"));
        let reg = config::game::Registry::load(settings);
        let cars = config::car::load_physics_registry(
            &settings.data_path,
            &reg,
            settings.game.physics.shape_sampling,
        );
        info!(
            "Loaded physics (gravity={}, frame_rate={}, {} cars)",
            common.nature.gravity,
            common.speed.standard_frame_rate,
            cars.len(),
        );
        (common, cars)
    } else {
        let common = config::common::Common::test_default();
        info!(
            "Using test physics (gravity={}, frame_rate={})",
            common.nature.gravity, common.speed.standard_frame_rate
        );
        (common, std::collections::HashMap::new())
    };

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<SessionEvent>();
    let next_id = Arc::new(AtomicU32::new(1));

    // TCP listener
    let tcp_addr = format!("0.0.0.0:{}", cli.port);
    let tcp_listener = TcpListener::bind(&tcp_addr)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind TCP to {}: {}", tcp_addr, e));
    info!("TCP listening on {}", tcp_addr);

    let tcp_tx = event_tx.clone();
    let tcp_next_id = next_id.clone();
    tokio::spawn(async move {
        loop {
            match tcp_listener.accept().await {
                Ok((stream, peer)) => {
                    let id = tcp_next_id.fetch_add(1, Ordering::Relaxed);
                    info!("TCP connection from {} assigned player_id={}", peer, id);
                    let tx = tcp_tx.clone();
                    tokio::spawn(handle_tcp_connection(stream, id, tx));
                }
                Err(e) => error!("TCP accept error: {}", e),
            }
        }
    });

    // WebSocket listener
    let ws_addr = format!("0.0.0.0:{}", cli.ws_port);
    let ws_listener = TcpListener::bind(&ws_addr)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind WS to {}: {}", ws_addr, e));
    info!("WebSocket listening on {}", ws_addr);

    let ws_tx = event_tx.clone();
    let ws_next_id = next_id;
    tokio::spawn(async move {
        loop {
            match ws_listener.accept().await {
                Ok((stream, peer)) => {
                    let id = ws_next_id.fetch_add(1, Ordering::Relaxed);
                    info!(
                        "WebSocket connection from {} assigned player_id={}",
                        peer, id
                    );
                    let tx = ws_tx.clone();
                    tokio::spawn(handle_ws_connection(stream, id, tx));
                }
                Err(e) => error!("WS accept error: {}", e),
            }
        }
    });

    info!(
        "Tick rate: {} Hz, max players: {}",
        cli.tick_rate, cli.max_players
    );

    // Main game loop
    let tick_duration = Duration::from_secs_f64(1.0 / cli.tick_rate as f64);
    let mut tick_interval = time::interval(tick_duration);
    let mut players: HashMap<PlayerId, ServerAgent> = HashMap::new();
    // Last player_id claimed by each Join name. Survives disconnect so a
    // client reconnecting with the same `--name` reclaims that identity.
    let mut name_ids: HashMap<String, PlayerId> = HashMap::new();
    // Accept-time conn_id → effective player_id (identity after name reclaim).
    let mut conn_to_player: HashMap<PlayerId, PlayerId> = HashMap::new();
    // Last known pose + cirtainer per player_id after leave (grace-window reclaim).
    let mut last_poses: HashMap<PlayerId, LastPose> = HashMap::new();
    let mut tick: u32 = 0;
    let level_name = cycle_world.clone();
    let max_players = cli.max_players;
    let max_quant = 0.02f32;

    // Physics timing
    let dt_fixed = 1.0 / cli.tick_rate as f32;
    let input_factor = dt_fixed / config::common::MAIN_LOOP_TIME;
    let physics_dt = dt_fixed * {
        let n = &common.nature;
        let fps = common.speed.standard_frame_rate as f32;
        fps * n.time_delta0 * n.num_calls_analysis as f32
    };

    info!(
        "Starting game loop (physics_dt={:.4}, input_factor={:.2})",
        physics_dt, input_factor
    );

    loop {
        tokio::select! {
            _ = tick_interval.tick() => {
                if players.is_empty() {
                    continue;
                }

                tick += 1;

                // Apply controls and step physics for each player
                for agent in players.values_mut() {
                    if !agent.joined {
                        continue;
                    }

                    // Apply control inputs
                    agent.apply_control(input_factor, &common);

                    let f_turbo = if agent.control.turbo {
                        common.global.k_traction_turbo
                    } else {
                        1.0
                    };
                    let f_brake = if agent.control.brake {
                        common.global.f_brake_max
                    } else {
                        0.0
                    };
                    let jump = agent.control.jump.take();

                    // Step physics (may need sub-stepping for stability)
                    let mut remaining = physics_dt;
                    while remaining > max_quant {
                        physics::step(
                            &mut agent.dynamo,
                            &mut agent.transform,
                            max_quant,
                            &agent.phys_data,
                            &level,
                            &common,
                            f_turbo,
                            f_brake,
                            None,
                            0.0,
                            None,
                            None,
                        );
                        remaining -= max_quant;
                    }
                    physics::step(
                        &mut agent.dynamo,
                        &mut agent.transform,
                        remaining,
                        &agent.phys_data,
                        &level,
                        &common,
                        f_turbo,
                        f_brake,
                        jump,
                        agent.control.roll,
                        None,
                        // The server does not deform the terrain: the edits
                        // would have to be replicated for clients to agree
                        // on the ground they are driving over.
                        None,
                    );

                    // Wrap coordinates
                    let size = level.size;
                    agent.transform.disp.x =
                        agent.transform.disp.x.rem_euclid(size.0 as f32);
                    agent.transform.disp.y =
                        agent.transform.disp.y.rem_euclid(size.1 as f32);
                }

                // Story cycle: one quant per server tick (tick_rate 20 Hz
                // matches MAIN_LOOP_TIME = 0.05). All joined players
                // gather/deliver — including those that clients only
                // keep as remote_agents.
                let cycle_state = if let Some(ref mut bunch) = cycle {
                    for agent in players.values_mut() {
                        if !agent.joined {
                            continue;
                        }
                        let coord = (
                            agent.transform.disp.x as i32,
                            agent.transform.disp.y as i32,
                        );
                        bunch.gather(coord, &mut agent.cirtainer);
                        bunch.deliver(coord, &mut agent.cirtainer);
                    }
                    let _ = bunch.quant(&mut level);
                    Some(CycleState {
                        current: bunch.current() as u32,
                        banked: bunch.banked().to_vec(),
                        light: bunch.light(),
                        fade: bunch.fade_progress().map(|(target, left)| CycleFade {
                            target: target as u32,
                            left,
                        }),
                        players: players
                            .iter()
                            .filter(|(_, a)| a.joined)
                            .map(|(&id, a)| PlayerCirt {
                                player_id: id,
                                held: a.cirtainer.held().to_vec(),
                            })
                            .collect(),
                    })
                } else {
                    None
                };

                // Collect agent states and broadcast
                let agents: Vec<AgentState> = players
                    .iter()
                    .filter(|(_, a)| a.joined)
                    .map(|(&id, agent)| agent.to_agent_state(id))
                    .collect();

                let msg = encode(&ServerMessage::WorldState {
                    tick,
                    agents,
                    cycle: cycle_state,
                });

                let mut disconnected = Vec::new();
                for (&id, agent) in &players {
                    if agent.sender.send(msg.clone()).is_err() {
                        disconnected.push(id);
                    }
                }
                for id in disconnected {
                    remove_player(&mut players, &mut conn_to_player, &mut last_poses, id);
                }
            }

            Some(event) = event_rx.recv() => {
                match event {
                    SessionEvent::Connected { conn_id, sender } => {
                        if players.len() >= max_players {
                            warn!("Rejecting conn_id={}: server full", conn_id);
                            drop(sender);
                            continue;
                        }
                        // Create agent with placeholder state, wait for Join
                        conn_to_player.insert(conn_id, conn_id);
                        players.insert(conn_id, ServerAgent {
                            name: String::new(),
                            car_name: String::new(),
                            color: 0,
                            control: NetControl::default(),
                            transform: space::Transform::IDENTITY,
                            dynamo: Dynamo::default(),
                            phys_data: CarPhysicsData::test_default(), // replaced on Join
                            cirtainer: level::cycle::Cirtainer::default(),
                            sender,
                            joined: false,
                        });
                    }

                    SessionEvent::Message { conn_id, msg } => {
                        match msg {
                            ClientMessage::Join { player_name, car_name, color } => {
                                let player_id = reclaim_player_id(
                                    conn_id,
                                    &player_name,
                                    &mut players,
                                    &mut name_ids,
                                    &mut conn_to_player,
                                );

                                purge_expired_poses(&mut last_poses);
                                let restored = last_poses.remove(&player_id).filter(|pose| {
                                    pose.disconnected_at.elapsed() <= RECONNECT_POSE_GRACE
                                });

                                let spawn_index = players.values().filter(|a| a.joined).count();
                                let coords = find_spawn_point(&level, spawn_index);
                                let height = level.get(coords).high() + 5.0;

                                let pose_note = if restored.is_some() {
                                    " (restored last pose/cirtainer)"
                                } else if player_id != conn_id {
                                    " (reclaimed id, fresh spawn)"
                                } else {
                                    ""
                                };
                                info!(
                                    "Player {} ({}) joined with car={}, color={}, spawn=({},{}){}",
                                    player_id,
                                    player_name,
                                    car_name,
                                    color,
                                    coords.0,
                                    coords.1,
                                    pose_note
                                );

                                if let Some(agent) = players.get_mut(&player_id) {
                                    if let Some(phys) = car_physics.get(&car_name) {
                                        agent.phys_data = phys.clone();
                                    } else {
                                        warn!("Unknown car '{}', using test physics", car_name);
                                    }
                                    agent.name = player_name.clone();
                                    agent.car_name = car_name.clone();
                                    agent.color = color;
                                    agent.joined = true;
                                    if let Some(pose) = restored {
                                        agent.transform = pose.transform;
                                        agent.dynamo = pose.dynamo;
                                        agent.cirtainer = pose.cirtainer;
                                        // Keep model scale in sync with the chosen car.
                                        agent.transform.scale = agent.phys_data.scale;
                                    } else {
                                        // Seed only brand-new ids so reclaim tests
                                        // cannot false-pass by re-applying the env.
                                        agent.cirtainer = if player_id == conn_id {
                                            test_seed_cirtainer().unwrap_or_default()
                                        } else {
                                            level::cycle::Cirtainer::default()
                                        };
                                        agent.dynamo = Dynamo::default();
                                        agent.transform = space::Transform {
                                            scale: agent.phys_data.scale,
                                            disp: Vec3::new(
                                                coords.0 as f32,
                                                coords.1 as f32,
                                                height,
                                            ),
                                            rot: glam::Quat::from_rotation_z(std::f32::consts::PI),
                                        };
                                    }

                                    // Send welcome
                                    let welcome = encode(&ServerMessage::Welcome {
                                        player_id,
                                        tick,
                                        level_name: level_name.clone(),
                                    });
                                    let _ = agent.sender.send(welcome);
                                }

                                // Tell new player about existing players
                                let new_sender = players.get(&player_id)
                                    .map(|a| a.sender.clone());
                                for (&id, other) in &players {
                                    if id != player_id && other.joined {
                                        if let Some(ref sender) = new_sender {
                                            let existing = encode(&ServerMessage::PlayerJoined {
                                                player_id: id,
                                                player_name: other.name.clone(),
                                                car_name: other.car_name.clone(),
                                                color: other.color,
                                            });
                                            let _ = sender.send(existing);
                                        }
                                    }
                                }

                                // Tell existing players about new player
                                let joined_msg = encode(&ServerMessage::PlayerJoined {
                                    player_id,
                                    player_name,
                                    car_name,
                                    color,
                                });
                                for (&id, other) in &players {
                                    if id != player_id && other.joined {
                                        let _ = other.sender.send(joined_msg.clone());
                                    }
                                }
                            }

                            ClientMessage::Input { control, .. } => {
                                let player_id = effective_player_id(&conn_to_player, conn_id);
                                if let Some(agent) = players.get_mut(&player_id) {
                                    agent.control = control;
                                }
                            }

                            ClientMessage::SetPose { transform } => {
                                let player_id = effective_player_id(&conn_to_player, conn_id);
                                if let Some(agent) = players.get_mut(&player_id) {
                                    if agent.joined {
                                        agent.transform.disp = Vec3::from(transform.position);
                                        agent.transform.rot = glam::Quat::from_xyzw(
                                            transform.rotation[0],
                                            transform.rotation[1],
                                            transform.rotation[2],
                                            transform.rotation[3],
                                        );
                                        // Keep car model scale authoritative.
                                        agent.transform.scale = agent.phys_data.scale;
                                        // Stop residual motion so the debug
                                        // teleport sticks in WorldState.
                                        agent.dynamo.linear_velocity = Vec3::ZERO;
                                        agent.dynamo.angular_velocity = Vec3::ZERO;
                                    }
                                }
                            }

                            ClientMessage::Leave => {
                                let player_id = effective_player_id(&conn_to_player, conn_id);
                                info!("Player {} leaving", player_id);
                                remove_player(&mut players, &mut conn_to_player, &mut last_poses, player_id);
                            }
                        }
                    }

                    SessionEvent::Disconnected { conn_id } => {
                        let player_id = effective_player_id(&conn_to_player, conn_id);
                        info!("Player {} disconnected (conn_id={})", player_id, conn_id);
                        remove_player(&mut players, &mut conn_to_player, &mut last_poses, player_id);
                    }
                }
            }
        }
    }
}

/// Resolve accept-time conn_id to the effective player_id (after name reclaim).
fn effective_player_id(conn_to_player: &HashMap<PlayerId, PlayerId>, conn_id: PlayerId) -> PlayerId {
    *conn_to_player.get(&conn_id).unwrap_or(&conn_id)
}

/// On Join, reuse a prior player_id when this `--name` is free to reclaim.
///
/// Connection tasks keep speaking `conn_id`; `conn_to_player` remaps them so
/// Welcome / WorldState / remotes see a stable identity across reconnects.
fn reclaim_player_id(
    conn_id: PlayerId,
    player_name: &str,
    players: &mut HashMap<PlayerId, ServerAgent>,
    name_ids: &mut HashMap<String, PlayerId>,
    conn_to_player: &mut HashMap<PlayerId, PlayerId>,
) -> PlayerId {
    // Empty names are not identity keys (avoid every anonymous client colliding).
    if player_name.is_empty() {
        return conn_id;
    }

    let effective = match name_ids.get(player_name).copied() {
        Some(old_id) if old_id != conn_id => {
            let live = players.get(&old_id).is_some_and(|a| a.joined);
            if live {
                // Another connected client already holds this name; keep provisional id.
                conn_id
            } else {
                old_id
            }
        }
        Some(old_id) => old_id,
        None => conn_id,
    };

    if effective != conn_id {
        if let Some(agent) = players.remove(&conn_id) {
            players.insert(effective, agent);
        }
        conn_to_player.insert(conn_id, effective);
        info!(
            "Reclaimed player_id={} for name {:?} (conn_id={})",
            effective, player_name, conn_id
        );
    }

    // Bind name → id unless a different live player already owns the binding.
    let steal = match name_ids.get(player_name).copied() {
        Some(bound) if bound != effective => {
            players.get(&bound).is_some_and(|a| a.joined)
        }
        _ => false,
    };
    if !steal {
        name_ids.insert(player_name.to_string(), effective);
    }

    effective
}

fn purge_expired_poses(last_poses: &mut HashMap<PlayerId, LastPose>) {
    last_poses.retain(|_, pose| pose.disconnected_at.elapsed() <= RECONNECT_POSE_GRACE);
}

fn remove_player(
    players: &mut HashMap<PlayerId, ServerAgent>,
    conn_to_player: &mut HashMap<PlayerId, PlayerId>,
    last_poses: &mut HashMap<PlayerId, LastPose>,
    player_id: PlayerId,
) {
    conn_to_player.retain(|_, mapped| *mapped != player_id);
    if let Some(removed) = players.remove(&player_id) {
        // Keep name→id (reclaim) and last pose/cirtainer (grace-window restore).
        if removed.joined {
            last_poses.insert(
                player_id,
                LastPose {
                    transform: removed.transform,
                    dynamo: removed.dynamo,
                    cirtainer: removed.cirtainer,
                    disconnected_at: Instant::now(),
                },
            );
        }
        info!("Removed player {} ({})", player_id, removed.name);
        let msg = encode(&ServerMessage::PlayerLeft { player_id });
        for agent in players.values() {
            let _ = agent.sender.send(msg.clone());
        }
    }
}

async fn handle_tcp_connection(
    stream: TcpStream,
    player_id: PlayerId,
    event_tx: mpsc::UnboundedSender<SessionEvent>,
) {
    let (reader, mut writer) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(reader);

    // Channel for outbound messages
    let (send_tx, mut send_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    // Register connection
    let _ = event_tx.send(SessionEvent::Connected {
        conn_id: player_id,
        sender: send_tx,
    });

    // Spawn writer task
    let write_handle = tokio::spawn(async move {
        while let Some(data) = send_rx.recv().await {
            if writer.write_all(&data).await.is_err() {
                break;
            }
        }
    });

    // Reader loop
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];

    loop {
        match reader.read(&mut tmp).await {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                while let Some((msg, consumed)) = decode::<ClientMessage>(&buf) {
                    let _ = event_tx.send(SessionEvent::Message { conn_id: player_id, msg });
                    buf.drain(..consumed);
                }
            }
            Err(e) => {
                warn!("Read error for player {}: {}", player_id, e);
                break;
            }
        }
    }

    let _ = event_tx.send(SessionEvent::Disconnected { conn_id: player_id });
    write_handle.abort();
}

async fn handle_ws_connection(
    stream: TcpStream,
    player_id: PlayerId,
    event_tx: mpsc::UnboundedSender<SessionEvent>,
) {
    let ws_stream = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            warn!("WebSocket handshake failed for player {}: {}", player_id, e);
            return;
        }
    };

    let (mut ws_writer, mut ws_reader) = ws_stream.split();

    // Channel for outbound messages
    let (send_tx, mut send_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    // Register connection
    let _ = event_tx.send(SessionEvent::Connected {
        conn_id: player_id,
        sender: send_tx,
    });

    // Writer task: send binary WebSocket frames
    let write_handle = tokio::spawn(async move {
        while let Some(data) = send_rx.recv().await {
            if ws_writer
                .send(tungstenite::Message::Binary(data.into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    // Reader loop: receive binary WebSocket frames
    let mut buf = Vec::with_capacity(4096);

    while let Some(result) = ws_reader.next().await {
        match result {
            Ok(tungstenite::Message::Binary(data)) => {
                buf.extend_from_slice(&data);
                while let Some((msg, consumed)) = decode::<ClientMessage>(&buf) {
                    let _ = event_tx.send(SessionEvent::Message { conn_id: player_id, msg });
                    buf.drain(..consumed);
                }
            }
            Ok(tungstenite::Message::Close(_)) => break,
            Err(e) => {
                warn!("WS read error for player {}: {}", player_id, e);
                break;
            }
            _ => {} // Ignore ping/pong/text
        }
    }

    let _ = event_tx.send(SessionEvent::Disconnected { conn_id: player_id });
    write_handle.abort();
}
