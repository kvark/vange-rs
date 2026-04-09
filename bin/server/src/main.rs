use vangers::{
    config::{self, settings},
    level,
    physics::{self, CarPhysicsData, Dynamo},
    space,
};
use vangers_net::{
    decode, encode, AgentState, ClientMessage, NetControl, NetDynamo, NetTransform, PlayerId,
    ServerMessage,
};

use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use glam::Vec3;
use log::{error, info, warn};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
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

    /// Level name to host (use "test" for procedural test level)
    #[arg(short, long, default_value = "test")]
    level: String,
}

/// Events from client connection tasks to the main game loop.
enum SessionEvent {
    Connected {
        player_id: PlayerId,
        sender: mpsc::UnboundedSender<Vec<u8>>,
    },
    Message {
        player_id: PlayerId,
        msg: ClientMessage,
    },
    Disconnected {
        player_id: PlayerId,
    },
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
            let angle = self.dynamo.rudder
                + common.car.rudder_step * 2.0 * dt * control.rudder;
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

    // Load level
    info!("Loading level: {}", cli.level);
    let level_config = if cli.level == "test" {
        level::LevelConfig::new_test()
    } else {
        level::LevelConfig::load(std::path::Path::new(&cli.level))
    };
    let geometry = settings::Geometry::default();
    let level = level::load(&level_config, &geometry);
    info!(
        "Level loaded: {}x{} (test={})",
        level.size.0,
        level.size.1,
        cli.level == "test"
    );

    // Physics constants
    let common = config::common::Common::test_default();
    info!("Using test physics constants (gravity={}, frame_rate={})",
        common.nature.gravity, common.speed.standard_frame_rate);

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
                    info!("WebSocket connection from {} assigned player_id={}", peer, id);
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
    let mut tick: u32 = 0;
    let level_name = cli.level.clone();
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

    info!("Starting game loop (physics_dt={:.4}, input_factor={:.2})", physics_dt, input_factor);

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
                    );

                    // Wrap coordinates
                    let size = level.size;
                    agent.transform.disp.x =
                        agent.transform.disp.x.rem_euclid(size.0 as f32);
                    agent.transform.disp.y =
                        agent.transform.disp.y.rem_euclid(size.1 as f32);
                }

                // Collect agent states and broadcast
                let agents: Vec<AgentState> = players
                    .iter()
                    .filter(|(_, a)| a.joined)
                    .map(|(&id, agent)| agent.to_agent_state(id))
                    .collect();

                let msg = encode(&ServerMessage::WorldState {
                    tick,
                    agents,
                });

                let mut disconnected = Vec::new();
                for (&id, agent) in &players {
                    if agent.sender.send(msg.clone()).is_err() {
                        disconnected.push(id);
                    }
                }
                for id in disconnected {
                    remove_player(&mut players, id);
                }
            }

            Some(event) = event_rx.recv() => {
                match event {
                    SessionEvent::Connected { player_id, sender } => {
                        if players.len() >= max_players {
                            warn!("Rejecting player_id={}: server full", player_id);
                            drop(sender);
                            continue;
                        }
                        // Create agent with placeholder state, wait for Join
                        players.insert(player_id, ServerAgent {
                            name: String::new(),
                            car_name: String::new(),
                            color: 0,
                            control: NetControl::default(),
                            transform: space::Transform::IDENTITY,
                            dynamo: Dynamo::default(),
                            phys_data: CarPhysicsData::test_default(),
                            sender,
                            joined: false,
                        });
                    }

                    SessionEvent::Message { player_id, msg } => {
                        match msg {
                            ClientMessage::Join { player_name, car_name, color } => {
                                let spawn_index = players.len();
                                let coords = find_spawn_point(&level, spawn_index);
                                let height = level.get(coords).high() + 5.0;

                                info!(
                                    "Player {} ({}) joined with car={}, color={}, spawn=({},{})",
                                    player_id, player_name, car_name, color, coords.0, coords.1
                                );

                                if let Some(agent) = players.get_mut(&player_id) {
                                    agent.name = player_name.clone();
                                    agent.car_name = car_name.clone();
                                    agent.color = color;
                                    agent.joined = true;
                                    agent.transform = space::Transform {
                                        scale: 1.0,
                                        disp: Vec3::new(
                                            coords.0 as f32,
                                            coords.1 as f32,
                                            height,
                                        ),
                                        rot: glam::Quat::from_rotation_z(std::f32::consts::PI),
                                    };

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
                                if let Some(agent) = players.get_mut(&player_id) {
                                    agent.control = control;
                                }
                            }

                            ClientMessage::Leave => {
                                info!("Player {} leaving", player_id);
                                remove_player(&mut players, player_id);
                            }
                        }
                    }

                    SessionEvent::Disconnected { player_id } => {
                        info!("Player {} disconnected", player_id);
                        remove_player(&mut players, player_id);
                    }
                }
            }
        }
    }
}

fn remove_player(players: &mut HashMap<PlayerId, ServerAgent>, player_id: PlayerId) {
    if let Some(removed) = players.remove(&player_id) {
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
        player_id,
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
                    let _ = event_tx.send(SessionEvent::Message {
                        player_id,
                        msg,
                    });
                    buf.drain(..consumed);
                }
            }
            Err(e) => {
                warn!("Read error for player {}: {}", player_id, e);
                break;
            }
        }
    }

    let _ = event_tx.send(SessionEvent::Disconnected { player_id });
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
        player_id,
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
                    let _ = event_tx.send(SessionEvent::Message {
                        player_id,
                        msg,
                    });
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

    let _ = event_tx.send(SessionEvent::Disconnected { player_id });
    write_handle.abort();
}
