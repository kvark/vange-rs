use vangers_net::{
    decode, encode, AgentState, ClientMessage, NetControl, PlayerId, ServerMessage,
};

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
        _tick: u32,
        agents: Vec<AgentState>,
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
                                ServerMessage::WorldState { tick, agents } => {
                                    NetEvent::WorldState { _tick: tick, agents }
                                }
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
    pub fn send_input(&self, seq: u32, control: &NetControl) {
        let msg = encode(&ClientMessage::Input {
            sequence: seq,
            control: control.clone(),
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
