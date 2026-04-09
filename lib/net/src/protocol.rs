use serde::{Deserialize, Serialize};

/// Unique identifier for a connected player.
pub type PlayerId = u32;

/// Client-to-server messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    /// Request to join the game session.
    Join {
        player_name: String,
        car_name: String,
        color: u8,
    },
    /// Per-frame input from the client.
    Input {
        sequence: u32,
        control: NetControl,
    },
    /// Client is leaving the session.
    Leave,
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
    PlayerLeft {
        player_id: PlayerId,
    },
    /// Authoritative world state snapshot broadcast every tick.
    WorldState {
        tick: u32,
        agents: Vec<AgentState>,
    },
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
        };
        let encoded = encode(&msg);
        let (decoded, consumed): (ClientMessage, _) = decode(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        match decoded {
            ClientMessage::Join {
                player_name,
                car_name,
                color,
            } => {
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
            }],
        };
        let encoded = encode(&msg);
        let (decoded, consumed): (ServerMessage, _) = decode(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        match decoded {
            ServerMessage::WorldState { tick, agents } => {
                assert_eq!(tick, 42);
                assert_eq!(agents.len(), 1);
                assert_eq!(agents[0].player_id, 1);
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
