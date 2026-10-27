use serde::{Deserialize, Serialize};

/// Unique identifier for a connected player.
pub type PlayerId = u32;

/// Wire protocol revision, sent in `Join`. The server turns away a client
/// on another revision instead of misreading its messages.
///
/// 2: per-quantum input batches (`Input::first_quant` / `quanta`), the
/// input ack in `AgentState::ack`, `SetPose::next_quant`.
/// 3: the held control in `AgentState::control`.
/// 4: `ClientMessage::Fire` and `ServerMessage::Shot`.
/// 5: server-owned armour in `AgentState::armor` / `max_armor`; the
/// shooter's view of the world in `Fire::view_tick` / `view_ms`.
/// 6: `ServerMessage::Died` / `Respawned` and `AgentState::dead`.
pub const PROTOCOL_VERSION: u32 = 6;

/// Client-to-server messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    /// Request to join the game session.
    Join {
        player_name: String,
        car_name: String,
        color: u8,
        /// `PROTOCOL_VERSION` of the client.
        protocol: u32,
    },
    /// The client's input for `quanta` physics quanta starting at quantum
    /// index `first_quant` (one batch per frame). The held controls apply
    /// to all of them; `control.jump` and `control.roll` to the last one
    /// only, as the client stepped them. The server runs these quanta in
    /// order and acks them in `AgentState::ack`.
    Input {
        first_quant: u32,
        quanta: u16,
        control: NetControl,
    },
    /// Client is leaving the session.
    Leave,
    /// Debug: set this player's authoritative pose (Tweaks Position apply).
    ///
    /// Server accepts for the sender's `player_id` and uses the transform in
    /// subsequent `WorldState` snapshots. Not for normal gameplay.
    ///
    /// `next_quant` is the client's next quantum index: queued input from
    /// before the pose is dropped and the ack moves there.
    SetPose {
        transform: NetTransform,
        next_quant: u32,
    },
    /// Report spiral charge after escave / KEY_UPDATE fill or passage spend.
    ///
    /// Server stores the value and broadcasts it on `AgentState.spiral_charge`
    /// so remotes and reclaim see the same charge. Solo offline never sends this.
    SetSpiral { charge: u8 },
    /// The player fired weapon bay `bay`: a shot from `origin` (the car's
    /// predicted position) along `dir` (unit, the car's forward).
    ///
    /// `quant` is the client's next input quantum when it fired, so the
    /// server can line the shot up with this player's inputs (the shot
    /// happened once `AgentState::ack` reaches it). The server checks it
    /// against its pose of the car and relays it to the other players as
    /// `ServerMessage::Shot`; the shooter draws its own shot at once.
    ///
    /// `view_tick` is the newest `WorldState` tick the client had, and
    /// `view_ms` how long before firing it arrived: the other cars were
    /// drawn carried on from that snapshot. The server flies the shot and
    /// tests it against the other cars rewound to that view (capped).
    Fire {
        quant: u32,
        bay: u8,
        origin: [f32; 3],
        dir: [f32; 3],
        view_tick: u32,
        view_ms: u16,
    },
}

/// Server-to-client messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    /// Welcome response after a successful join.
    Welcome {
        player_id: PlayerId,
        tick: u32,
        level_name: String,
    },
    /// Another player has joined the session.
    PlayerJoined {
        player_id: PlayerId,
        player_name: String,
        car_name: String,
        color: u8,
    },
    /// A player has left the session.
    PlayerLeft { player_id: PlayerId },
    /// Authoritative world state snapshot broadcast every tick.
    WorldState {
        tick: u32,
        agents: Vec<AgentState>,
        /// Story-cycle snapshot when the hosted world runs cycles.
        /// `None` for test / bonus worlds with no bunch.
        cycle: Option<CycleState>,
        /// Moving-land playback when the hosted world has `*.vot` locations.
        /// `None` when empty / unloaded. Server owns phase; clients apply.
        moving: Option<MovingLandState>,
    },
    /// Player `shooter` fired (a relayed `ClientMessage::Fire`, sent to
    /// everyone but the shooter as soon as the server takes it). `dir` is
    /// normalized by the server. `tick`: the server tick it was taken in.
    Shot {
        shooter: PlayerId,
        tick: u32,
        quant: u32,
        bay: u8,
        origin: [f32; 3],
        dir: [f32; 3],
    },
    /// Player `player_id`'s armour ran out at `position`: the car is a
    /// wreck until `Respawned`. `killer`: whose shot did it, if the server
    /// knows. Sent to everyone, the dead player included.
    Died {
        player_id: PlayerId,
        killer: Option<PlayerId>,
        position: [f32; 3],
    },
    /// Player `player_id` is back, undamaged, at `transform` (near an
    /// escave), standing still. Sent to everyone. The owning client snaps
    /// its car there and answers with `ClientMessage::SetPose`, which
    /// lines its inputs up again; the others snap their copy of the car.
    Respawned {
        player_id: PlayerId,
        transform: NetTransform,
    },
}

/// Authoritative story-cycle snapshot (cirt banks, palette stage, carry).
///
/// Clients that are connected apply this instead of advancing gather /
/// deliver / bank decay locally, so native TCP and web WS stay aligned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CycleState {
    /// Index of the settled (or still-current-during-fade) stage.
    pub current: u32,
    /// `cirtQ` banked toward each stage's `cirt_max`.
    pub banked: Vec<i32>,
    /// World light scale for the current / fading cycle.
    pub light: f32,
    /// When fading, the stage being faded toward and quants remaining.
    pub fade: Option<CycleFade>,
    /// Per-player cirtainer contents.
    pub players: Vec<PlayerCirt>,
}

/// In-progress palette cross-fade.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CycleFade {
    pub target: u32,
    pub left: i32,
}

/// What one player is carrying in their cirtainer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerCirt {
    pub player_id: PlayerId,
    pub held: Vec<i32>,
}

/// Authoritative moving-land playback snapshot (frame / phase per location).
///
/// Clients connected to a multiplayer server apply this instead of advancing
/// `MovingWorld` locally, so Tweaks frame/phase and terrain stay aligned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MovingLandState {
    pub locations: Vec<LocationPhase>,
}

/// Playback state of one moving-land location instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocationPhase {
    /// Index of the frame currently being played.
    pub frame: u16,
    /// `cStage` — quants elapsed in this loop (`-1` before the first).
    pub stage: i32,
    /// `goPh` — park target, or `-1` (`FREE_RUNNING`).
    pub go_phase: i32,
    /// Per-frame quant counter for `frame` (progress within the period).
    pub step: i32,
}

/// Player control input, sent from client to server.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetControl {
    pub motor: f32,
    pub rudder: f32,
    pub roll: f32,
    pub brake: bool,
    pub turbo: bool,
    pub jump: Option<f32>,
}

/// Snapshot of a single agent's state, broadcast by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub player_id: PlayerId,
    pub transform: NetTransform,
    pub dynamo: NetDynamo,
    /// Spiral energy slots filled (0 = discharged). Capacity stays client-local.
    pub spiral_charge: u8,
    /// Felt car-car bumps this car has taken (wrapping counter). A change
    /// between snapshots tells the owning client to adopt the server
    /// velocity even if it skipped the snapshot of the bump itself.
    pub bumps: u32,
    /// Quanta of this player's input the server has run (the next
    /// `Input::first_quant` it expects). The owning client compares its
    /// prediction at this quantum, not its present pose.
    pub ack: u32,
    /// The held control the server last ran this car with (no jump or
    /// roll). Other clients carry the car's contact ghost on with it: a
    /// car pushing with turbo keeps pushing with turbo.
    pub control: NetControl,
    /// Armour left, as the server keeps it: shots it resolved take it
    /// down, stopping at 0. The owning client shows this one.
    pub armor: u16,
    /// Armour of the car undamaged (`car.prm`).
    pub max_armor: u16,
    /// A wreck waiting to respawn (between `Died` and `Respawned`): it
    /// drives with no control, takes no hits and fires no shots.
    pub dead: bool,
}

/// Network-serializable transform (position + rotation + scale).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetTransform {
    pub position: [f32; 3],
    pub rotation: [f32; 4], // quaternion (x, y, z, w)
    pub scale: f32,
}

/// Network-serializable physics dynamics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetDynamo {
    pub traction: f32,
    pub rudder: f32,
    pub linear_velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
}

/// Encode a message to bytes with a 4-byte length prefix.
pub fn encode<T: Serialize>(msg: &T) -> Vec<u8> {
    let payload = bincode::serialize(msg).expect("serialization failed");
    let len = payload.len() as u32;
    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(&payload);
    buf
}

/// Attempt to decode a length-prefixed message from a byte buffer.
/// Returns `Some((message, bytes_consumed))` if a complete message is available.
pub fn decode<T: for<'de> Deserialize<'de>>(buf: &[u8]) -> Option<(T, usize)> {
    if buf.len() < 4 {
        return None;
    }
    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if buf.len() < 4 + len {
        return None;
    }
    let msg = bincode::deserialize(&buf[4..4 + len]).ok()?;
    Some((msg, 4 + len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_client_message() {
        let msg = ClientMessage::Join {
            player_name: "TestPlayer".into(),
            car_name: "Torkash".into(),
            color: 21,
            protocol: PROTOCOL_VERSION,
        };
        let encoded = encode(&msg);
        let (decoded, consumed): (ClientMessage, _) = decode(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        match decoded {
            ClientMessage::Join {
                player_name,
                car_name,
                color,
                protocol,
            } => {
                assert_eq!(protocol, PROTOCOL_VERSION);
                assert_eq!(player_name, "TestPlayer");
                assert_eq!(car_name, "Torkash");
                assert_eq!(color, 21);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn round_trip_server_message() {
        let msg = ServerMessage::WorldState {
            tick: 42,
            agents: vec![AgentState {
                player_id: 1,
                transform: NetTransform {
                    position: [100.0, 200.0, 50.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: 1.0,
                },
                dynamo: NetDynamo {
                    traction: 2.0,
                    rudder: 0.1,
                    linear_velocity: [1.0, 2.0, 0.0],
                    angular_velocity: [0.0, 0.0, 0.5],
                },
                spiral_charge: 3,
                bumps: 7,
                ack: 1234,
                control: NetControl {
                    motor: 1.0,
                    turbo: true,
                    ..NetControl::default()
                },
                armor: 35,
                max_armor: 120,
                dead: true,
            }],
            cycle: None,
            moving: None,
        };
        let encoded = encode(&msg);
        let (decoded, consumed): (ServerMessage, _) = decode(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        match decoded {
            ServerMessage::WorldState { tick, agents, cycle, moving } => {
                assert_eq!(tick, 42);
                assert_eq!(agents.len(), 1);
                assert_eq!(agents[0].player_id, 1);
                assert_eq!(agents[0].spiral_charge, 3);
                assert_eq!(agents[0].ack, 1234);
                assert_eq!((agents[0].armor, agents[0].max_armor), (35, 120));
                assert!(agents[0].dead);
                assert!(cycle.is_none());
                assert!(moving.is_none());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn round_trip_world_state_with_cycle() {
        let msg = ServerMessage::WorldState {
            tick: 7,
            agents: vec![],
            cycle: Some(CycleState {
                current: 1,
                banked: vec![3, 8, 0],
                light: 0.8,
                fade: Some(CycleFade {
                    target: 2,
                    left: 40,
                }),
                players: vec![PlayerCirt {
                    player_id: 1,
                    held: vec![7, 0, 2],
                }],
            }),
            moving: Some(MovingLandState {
                locations: vec![LocationPhase {
                    frame: 2,
                    stage: 5,
                    go_phase: -1,
                    step: 3,
                }],
            }),
        };
        let encoded = encode(&msg);
        let (decoded, consumed): (ServerMessage, _) = decode(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        match decoded {
            ServerMessage::WorldState {
                cycle: Some(cycle),
                moving: Some(moving),
                ..
            } => {
                assert_eq!(cycle.current, 1);
                assert_eq!(cycle.banked, vec![3, 8, 0]);
                assert!((cycle.light - 0.8).abs() < 1e-6);
                let fade = cycle.fade.expect("fade");
                assert_eq!(fade.target, 2);
                assert_eq!(fade.left, 40);
                assert_eq!(cycle.players.len(), 1);
                assert_eq!(cycle.players[0].held, vec![7, 0, 2]);
                assert_eq!(moving.locations.len(), 1);
                assert_eq!(moving.locations[0].frame, 2);
                assert_eq!(moving.locations[0].stage, 5);
                assert_eq!(moving.locations[0].go_phase, -1);
                assert_eq!(moving.locations[0].step, 3);
            }
            _ => panic!("expected WorldState with cycle and moving"),
        }
    }

    #[test]
    fn round_trip_set_pose() {
        let msg = ClientMessage::SetPose {
            transform: NetTransform {
                position: [12.0, 34.0, 56.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: 1.0,
            },
            next_quant: 77,
        };
        let encoded = encode(&msg);
        let (decoded, consumed): (ClientMessage, _) = decode(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        match decoded {
            ClientMessage::SetPose {
                transform,
                next_quant,
            } => {
                assert_eq!(next_quant, 77);
                assert_eq!(transform.position, [12.0, 34.0, 56.0]);
                assert_eq!(transform.rotation, [0.0, 0.0, 0.0, 1.0]);
                assert!((transform.scale - 1.0).abs() < 1e-6);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn round_trip_set_spiral() {
        let msg = ClientMessage::SetSpiral { charge: 4 };
        let encoded = encode(&msg);
        let (decoded, consumed): (ClientMessage, _) = decode(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        match decoded {
            ClientMessage::SetSpiral { charge } => assert_eq!(charge, 4),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn round_trip_input_batch() {
        let msg = ClientMessage::Input {
            first_quant: 900,
            quanta: 3,
            control: NetControl {
                motor: 1.0,
                jump: Some(4.0),
                ..NetControl::default()
            },
        };
        let encoded = encode(&msg);
        let (decoded, _): (ClientMessage, _) = decode(&encoded).unwrap();
        match decoded {
            ClientMessage::Input {
                first_quant,
                quanta,
                control,
            } => {
                assert_eq!((first_quant, quanta), (900, 3));
                assert_eq!(control.jump, Some(4.0));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn round_trip_fire_and_shot() {
        let msg = ClientMessage::Fire {
            quant: 77,
            bay: 1,
            origin: [1.0, 2.0, 3.0],
            dir: [0.0, 1.0, 0.0],
            view_tick: 500,
            view_ms: 30,
        };
        let encoded = encode(&msg);
        let (decoded, consumed): (ClientMessage, _) = decode(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        match decoded {
            ClientMessage::Fire {
                quant,
                bay,
                origin,
                dir,
                view_tick,
                view_ms,
            } => {
                assert_eq!((quant, bay), (77, 1));
                assert_eq!((view_tick, view_ms), (500, 30));
                assert_eq!(origin, [1.0, 2.0, 3.0]);
                assert_eq!(dir, [0.0, 1.0, 0.0]);
            }
            _ => panic!("wrong variant"),
        }

        let msg = ServerMessage::Shot {
            shooter: 3,
            tick: 900,
            quant: 77,
            bay: 0,
            origin: [4.0, 5.0, 6.0],
            dir: [1.0, 0.0, 0.0],
        };
        let encoded = encode(&msg);
        let (decoded, consumed): (ServerMessage, _) = decode(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        match decoded {
            ServerMessage::Shot {
                shooter,
                tick,
                quant,
                bay,
                origin,
                dir,
            } => {
                assert_eq!((shooter, tick, quant, bay), (3, 900, 77, 0));
                assert_eq!(origin, [4.0, 5.0, 6.0]);
                assert_eq!(dir, [1.0, 0.0, 0.0]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn round_trip_died_and_respawned() {
        let msg = ServerMessage::Died {
            player_id: 2,
            killer: Some(1),
            position: [7.0, 8.0, 9.0],
        };
        let (decoded, _): (ServerMessage, _) = decode(&encode(&msg)).unwrap();
        match decoded {
            ServerMessage::Died {
                player_id,
                killer,
                position,
            } => {
                assert_eq!((player_id, killer), (2, Some(1)));
                assert_eq!(position, [7.0, 8.0, 9.0]);
            }
            _ => panic!("wrong variant"),
        }

        let msg = ServerMessage::Respawned {
            player_id: 2,
            transform: NetTransform {
                position: [1657.0, 353.0, 40.0],
                rotation: [0.0, 0.0, 1.0, 0.0],
                scale: 1.0,
            },
        };
        let (decoded, _): (ServerMessage, _) = decode(&encode(&msg)).unwrap();
        match decoded {
            ServerMessage::Respawned {
                player_id,
                transform,
            } => {
                assert_eq!(player_id, 2);
                assert_eq!(transform.position, [1657.0, 353.0, 40.0]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn decode_incomplete_buffer() {
        let msg = ClientMessage::Leave;
        let encoded = encode(&msg);
        // Try with incomplete data
        assert!(decode::<ClientMessage>(&encoded[..3]).is_none());
        assert!(decode::<ClientMessage>(&encoded[..5]).is_none());
        // Full data works
        assert!(decode::<ClientMessage>(&encoded).is_some());
    }
}
