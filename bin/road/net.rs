use vangers_net::{AgentState, ClientMessage, CycleState, MovingLandState, NetControl, NetTransform, PlayerId, ServerMessage, decode, encode};

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::thread;

/// Messages from the network thread to the game loop.
pub enum NetEvent {
    Welcome {
        player_id: PlayerId,
        level_name: String,
    },
    PlayerJoined {
        player_id: PlayerId,
        player_name: String,
        car_name: String,
        color: u8,
    },
    PlayerLeft {
        player_id: PlayerId,
    },
    WorldState {
        tick: u32,
        agents: Vec<AgentState>,
        cycle: Option<CycleState>,
        moving: Option<MovingLandState>,
    },
    /// Another player fired (`ServerMessage::Shot`).
    Shot {
        shooter: PlayerId,
        bay: u8,
        origin: [f32; 3],
        dir: [f32; 3],
    },
    /// A car's armour ran out (`ServerMessage::Died`).
    Died {
        player_id: PlayerId,
        killer: Option<PlayerId>,
        position: [f32; 3],
    },
    /// A wreck is back (`ServerMessage::Respawned`).
    Respawned {
        player_id: PlayerId,
        transform: NetTransform,
    },
    Disconnected,
}

/// Network client that communicates with the server via a background thread.
pub struct NetworkClient {
    /// Send messages to the server.
    send_tx: mpsc::Sender<Vec<u8>>,
    /// Receive events from the server.
    recv_rx: mpsc::Receiver<NetEvent>,
    /// Our player ID (set after Welcome).
    pub player_id: Option<PlayerId>,
}

impl NetworkClient {
    /// Connect to the server and start background I/O threads.
    pub fn connect(addr: &str, player_name: &str, car_name: &str, color: u8) -> Self {
        log::info!("Connecting to server at {}...", addr);
        let stream = TcpStream::connect(addr)
            .unwrap_or_else(|e| panic!("Failed to connect to {}: {}", addr, e));
        log::info!("Connected to server");

        let (send_tx, send_rx) = mpsc::channel::<Vec<u8>>();
        let (recv_tx, recv_rx) = mpsc::channel::<NetEvent>();

        // Send Join immediately
        let join_msg = encode(&ClientMessage::Join {
            player_name: player_name.to_string(),
            car_name: car_name.to_string(),
            color,
            protocol: vangers_net::PROTOCOL_VERSION,
        });
        let mut write_stream = stream.try_clone().expect("Failed to clone TCP stream");
        write_stream.write_all(&join_msg).ok();

        // Writer thread
        thread::spawn(move || {
            while let Ok(data) = send_rx.recv() {
                if write_stream.write_all(&data).is_err() {
                    break;
                }
            }
        });

        // Reader thread
        let mut read_stream = stream;
        thread::spawn(move || {
            let mut buf = Vec::with_capacity(8192);
            let mut tmp = [0u8; 4096];
            loop {
                match read_stream.read(&mut tmp) {
                    Ok(0) => {
                        let _ = recv_tx.send(NetEvent::Disconnected);
                        break;
                    }
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        while let Some((msg, consumed)) = decode::<ServerMessage>(&buf) {
                            let event = match msg {
                                ServerMessage::Welcome {
                                    player_id,
                                    level_name,
                                    ..
                                } => NetEvent::Welcome {
                                    player_id,
                                    level_name,
                                },
                                ServerMessage::PlayerJoined {
                                    player_id,
                                    player_name,
                                    car_name,
                                    color,
                                } => NetEvent::PlayerJoined {
                                    player_id,
                                    player_name,
                                    car_name,
                                    color,
                                },
                                ServerMessage::PlayerLeft { player_id } => {
                                    NetEvent::PlayerLeft { player_id }
                                }
                                ServerMessage::WorldState {
                                    tick,
                                    agents,
                                    cycle,
                                    moving,
                                } => NetEvent::WorldState {
                                    tick,
                                    agents,
                                    cycle,
                                    moving,
                                },
                                ServerMessage::Shot {
                                    shooter,
                                    bay,
                                    origin,
                                    dir,
                                    ..
                                } => NetEvent::Shot {
                                    shooter,
                                    bay,
                                    origin,
                                    dir,
                                },
                                ServerMessage::Died {
                                    player_id,
                                    killer,
                                    position,
                                } => NetEvent::Died {
                                    player_id,
                                    killer,
                                    position,
                                },
                                ServerMessage::Respawned {
                                    player_id,
                                    transform,
                                } => NetEvent::Respawned {
                                    player_id,
                                    transform,
                                },
                            };
                            if recv_tx.send(event).is_err() {
                                return;
                            }
                            buf.drain(..consumed);
                        }
                    }
                    Err(_) => {
                        let _ = recv_tx.send(NetEvent::Disconnected);
                        break;
                    }
                }
            }
        });

        NetworkClient {
            send_tx,
            recv_rx,
            player_id: None,
        }
    }

    /// Send control input to the server.
    /// Input for `quanta` quanta from index `first_quant`
    /// (`ClientMessage::Input`).
    pub fn send_input(&self, first_quant: u32, quanta: u16, control: &NetControl) {
        let msg = encode(&ClientMessage::Input {
            first_quant,
            quanta,
            control: control.clone(),
        });
        let _ = self.send_tx.send(msg);
    }

    /// Debug: upload local pose so the server WorldState keeps it.
    pub fn send_set_pose(&self, transform: &NetTransform, next_quant: u32) {
        let msg = encode(&ClientMessage::SetPose {
            transform: transform.clone(),
            next_quant,
        });
        let _ = self.send_tx.send(msg);
    }

    /// Upload spiral charge so WorldState / reclaim stay aligned.
    pub fn send_set_spiral(&self, charge: u8) {
        let msg = encode(&ClientMessage::SetSpiral { charge });
        let _ = self.send_tx.send(msg);
    }

    /// Report a shot from weapon bay `bay` (`ClientMessage::Fire`), with
    /// the `WorldState` tick the other cars were drawn from and how many
    /// ms ago it arrived.
    pub fn send_fire(
        &self,
        quant: u32,
        bay: u8,
        origin: [f32; 3],
        dir: [f32; 3],
        view: (u32, u16),
    ) {
        let msg = encode(&ClientMessage::Fire {
            quant,
            bay,
            origin,
            dir,
            view_tick: view.0,
            view_ms: view.1,
        });
        let _ = self.send_tx.send(msg);
    }

    /// Poll for events from the server (non-blocking).
    pub fn poll(&mut self) -> Vec<NetEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.recv_rx.try_recv() {
            if let NetEvent::Welcome { player_id, .. } = &event {
                self.player_id = Some(*player_id);
            }
            events.push(event);
        }
        events
    }
}
