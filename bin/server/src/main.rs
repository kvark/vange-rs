use vangers_net::{
    decode, encode, AgentState, ClientMessage, NetControl, NetDynamo, NetTransform, PlayerId,
    ServerMessage,
};

use clap::Parser;
use log::{error, info, warn};
use std::collections::HashMap;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    time::{self, Duration},
};

/// Vangers headless multiplayer server
#[derive(Parser)]
struct Cli {
    /// Port to listen on
    #[arg(short, long, default_value = "7800")]
    port: u16,

    /// Maximum number of players
    #[arg(long, default_value = "8")]
    max_players: usize,

    /// Server tick rate in Hz
    #[arg(long, default_value = "20")]
    tick_rate: u32,

    /// Level name to host
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

struct PlayerState {
    name: String,
    car_name: String,
    color: u8,
    control: NetControl,
    transform: NetTransform,
    dynamo: NetDynamo,
    sender: mpsc::UnboundedSender<Vec<u8>>,
}

#[tokio::main]
async fn main() {
    env_logger::init();
    let cli = Cli::parse();

    let addr = format!("0.0.0.0:{}", cli.port);
    let listener = TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind to {}: {}", addr, e));
    info!("Vangers server listening on {}", addr);
    info!(
        "Level: {}, tick rate: {} Hz, max players: {}",
        cli.level, cli.tick_rate, cli.max_players
    );

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<SessionEvent>();

    // Spawn connection acceptor
    let accept_tx = event_tx.clone();
    let max_players = cli.max_players;
    tokio::spawn(async move {
        let mut next_id: PlayerId = 1;
        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    let id = next_id;
                    next_id += 1;
                    info!("Connection from {} assigned player_id={}", peer, id);
                    let tx = accept_tx.clone();
                    tokio::spawn(handle_connection(stream, id, tx));
                }
                Err(e) => {
                    error!("Accept error: {}", e);
                }
            }
        }
    });

    // Main game loop
    let tick_duration = Duration::from_secs_f64(1.0 / cli.tick_rate as f64);
    let mut tick_interval = time::interval(tick_duration);
    let mut players: HashMap<PlayerId, PlayerState> = HashMap::new();
    let mut tick: u32 = 0;
    let level_name = cli.level.clone();

    info!("Starting game loop");

    loop {
        tokio::select! {
            _ = tick_interval.tick() => {
                if players.is_empty() {
                    continue;
                }

                tick += 1;

                // Collect agent states
                let agents: Vec<AgentState> = players
                    .iter()
                    .map(|(&id, state)| AgentState {
                        player_id: id,
                        transform: state.transform.clone(),
                        dynamo: state.dynamo.clone(),
                    })
                    .collect();

                // Broadcast world state
                let msg = encode(&ServerMessage::WorldState {
                    tick,
                    agents,
                });

                let mut disconnected = Vec::new();
                for (&id, state) in &players {
                    if state.sender.send(msg.clone()).is_err() {
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
                        // Store sender, wait for Join message to complete registration
                        players.insert(player_id, PlayerState {
                            name: String::new(),
                            car_name: String::new(),
                            color: 0,
                            control: NetControl::default(),
                            transform: NetTransform {
                                position: [0.0, 0.0, 100.0],
                                rotation: [0.0, 0.0, 0.0, 1.0],
                                scale: 1.0,
                            },
                            dynamo: NetDynamo {
                                traction: 0.0,
                                rudder: 0.0,
                                linear_velocity: [0.0, 0.0, 0.0],
                                angular_velocity: [0.0, 0.0, 0.0],
                            },
                            sender,
                        });
                    }

                    SessionEvent::Message { player_id, msg } => {
                        match msg {
                            ClientMessage::Join { player_name, car_name, color } => {
                                info!("Player {} ({}) joined with car={}, color={}",
                                    player_id, player_name, car_name, color);

                                // Send welcome
                                if let Some(state) = players.get_mut(&player_id) {
                                    state.name = player_name.clone();
                                    state.car_name = car_name.clone();
                                    state.color = color;

                                    let welcome = encode(&ServerMessage::Welcome {
                                        player_id,
                                        tick,
                                        level_name: level_name.clone(),
                                    });
                                    let _ = state.sender.send(welcome);

                                    // Notify existing players about the new player
                                    let joined_msg = encode(&ServerMessage::PlayerJoined {
                                        player_id,
                                        player_name: player_name.clone(),
                                        car_name: car_name.clone(),
                                        color,
                                    });

                                    // Send info about existing players to the new player
                                    for (&id, other) in &players {
                                        if id != player_id && !other.name.is_empty() {
                                            // Tell new player about existing player
                                            let existing = encode(&ServerMessage::PlayerJoined {
                                                player_id: id,
                                                player_name: other.name.clone(),
                                                car_name: other.car_name.clone(),
                                                color: other.color,
                                            });
                                            if let Some(new_state) = players.get(&player_id) {
                                                let _ = new_state.sender.send(existing);
                                            }
                                        }
                                    }

                                    // Tell existing players about new player
                                    for (&id, other) in &players {
                                        if id != player_id && !other.name.is_empty() {
                                            let _ = other.sender.send(joined_msg.clone());
                                        }
                                    }
                                }
                            }

                            ClientMessage::Input { control, .. } => {
                                if let Some(state) = players.get_mut(&player_id) {
                                    state.control = control;
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

fn remove_player(players: &mut HashMap<PlayerId, PlayerState>, player_id: PlayerId) {
    if let Some(removed) = players.remove(&player_id) {
        info!("Removed player {} ({})", player_id, removed.name);
        let msg = encode(&ServerMessage::PlayerLeft { player_id });
        for state in players.values() {
            let _ = state.sender.send(msg.clone());
        }
    }
}

async fn handle_connection(
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
            Ok(0) => break, // Connection closed
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                // Process all complete messages in the buffer
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
