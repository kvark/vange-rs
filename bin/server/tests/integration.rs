use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant};

use vangers_net::{decode, encode, ClientMessage, ServerMessage};

static NEXT_PORT: AtomicU16 = AtomicU16::new(19876);

struct ServerProcess {
    child: Child,
    port: u16,
}

impl ServerProcess {
    fn start() -> Self {
        let port = NEXT_PORT.fetch_add(2, Ordering::Relaxed);
        let ws_port = port + 1;
        let child = Command::new(env!("CARGO_BIN_EXE_vangers-server"))
            .args([
                "--port",
                &port.to_string(),
                "--ws-port",
                &ws_port.to_string(),
                "--level",
                "test",
                "--tick-rate",
                "20",
            ])
            .env("RUST_LOG", "warn")
            .spawn()
            .expect("Failed to start server");
        std::thread::sleep(Duration::from_millis(500));
        ServerProcess { child, port }
    }

    fn connect(&self) -> Client {
        let stream = TcpStream::connect(format!("127.0.0.1:{}", self.port))
            .expect("Failed to connect to server");
        stream
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        stream.set_nonblocking(false).unwrap();
        Client {
            stream,
            buf: Vec::new(),
        }
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Client {
    stream: TcpStream,
    buf: Vec<u8>,
}

impl Client {
    fn send(&mut self, msg: &ClientMessage) {
        let data = encode(msg);
        self.stream.write_all(&data).unwrap();
    }

    /// Read messages until `deadline`, returning all collected.
    fn recv_for(&mut self, duration: Duration) -> Vec<ServerMessage> {
        let deadline = Instant::now() + duration;
        let mut msgs = Vec::new();
        let mut tmp = [0u8; 4096];

        while Instant::now() < deadline {
            match self.stream.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => {
                    self.buf.extend_from_slice(&tmp[..n]);
                }
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(_) => break,
            }
            while let Some((msg, consumed)) = decode::<ServerMessage>(&self.buf) {
                msgs.push(msg);
                self.buf.drain(..consumed);
            }
        }
        msgs
    }

    /// Block until we receive a Welcome, preserving other messages in a stash.
    fn recv_welcome(&mut self, stash: &mut Vec<ServerMessage>) -> ServerMessage {
        let msgs = self.recv_for(Duration::from_secs(3));
        let mut welcome = None;
        for msg in msgs {
            if welcome.is_none() && matches!(msg, ServerMessage::Welcome { .. }) {
                welcome = Some(msg);
            } else {
                stash.push(msg);
            }
        }
        welcome.expect("No Welcome received within timeout")
    }
}

#[test]
fn test_join_and_receive_welcome() {
    let server = ServerProcess::start();
    let mut client = server.connect();

    client.send(&ClientMessage::Join {
        player_name: "Alice".into(),
        car_name: "TestCar".into(),
        color: 21,
    });

    let mut stash = Vec::new();
    let msg = client.recv_welcome(&mut stash);
    match msg {
        ServerMessage::Welcome {
            player_id,
            level_name,
            ..
        } => {
            assert!(player_id > 0);
            assert_eq!(level_name, "test");
        }
        _ => unreachable!(),
    }
}

#[test]
fn test_two_players_see_each_other() {
    let server = ServerProcess::start();

    let mut alice = server.connect();
    let mut bob = server.connect();

    // Alice joins
    alice.send(&ClientMessage::Join {
        player_name: "Alice".into(),
        car_name: "TestCar".into(),
        color: 21,
    });
    let mut alice_extra = Vec::new();
    let alice_id = match alice.recv_welcome(&mut alice_extra) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };

    // Bob joins
    bob.send(&ClientMessage::Join {
        player_name: "Bob".into(),
        car_name: "TestCar".into(),
        color: 7,
    });
    let mut bob_extra = Vec::new();
    let bob_id = match bob.recv_welcome(&mut bob_extra) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };

    assert_ne!(alice_id, bob_id);

    // Collect more messages, combining with what was stashed during welcome
    let mut alice_msgs = alice_extra;
    alice_msgs.extend(alice.recv_for(Duration::from_secs(2)));
    let mut bob_msgs = bob_extra;
    bob_msgs.extend(bob.recv_for(Duration::from_secs(2)));

    // Alice should have received Bob's PlayerJoined
    let bob_joined = alice_msgs.iter().any(
        |m| matches!(m, ServerMessage::PlayerJoined { player_id, .. } if *player_id == bob_id),
    );
    assert!(bob_joined, "Alice didn't receive Bob's PlayerJoined");

    // Bob should have received Alice's PlayerJoined
    let alice_joined = bob_msgs.iter().any(
        |m| matches!(m, ServerMessage::PlayerJoined { player_id, .. } if *player_id == alice_id),
    );
    assert!(alice_joined, "Bob didn't receive Alice's PlayerJoined");

    // The last WorldState should have 2 agents (both joined)
    let alice_ws = alice_msgs.iter().rev().find_map(|m| match m {
        ServerMessage::WorldState { agents, .. } => Some(agents),
        _ => None,
    });
    assert!(alice_ws.is_some(), "Alice didn't receive any WorldState");
    assert_eq!(
        alice_ws.unwrap().len(),
        2,
        "Last WorldState should have 2 agents"
    );
}

#[test]
fn test_physics_updates_position() {
    let server = ServerProcess::start();
    let mut client = server.connect();

    client.send(&ClientMessage::Join {
        player_name: "Mover".into(),
        car_name: "TestCar".into(),
        color: 21,
    });
    let mut stash = Vec::new();
    let _ = client.recv_welcome(&mut stash);

    // Send input: full throttle forward
    client.send(&ClientMessage::Input {
        sequence: 1,
        control: vangers_net::NetControl {
            motor: 1.0,
            rudder: 0.0,
            roll: 0.0,
            brake: false,
            turbo: false,
            jump: None,
        },
    });

    // Collect world states for 2 seconds
    let msgs = client.recv_for(Duration::from_secs(2));
    let world_states: Vec<_> = msgs
        .iter()
        .filter_map(|m| match m {
            ServerMessage::WorldState { tick, agents, .. } => Some((tick, agents)),
            _ => None,
        })
        .collect();

    assert!(
        world_states.len() >= 5,
        "Expected at least 5 world states, got {}",
        world_states.len()
    );

    // Verify ticks are increasing
    for pair in world_states.windows(2) {
        assert!(pair[1].0 > pair[0].0, "Ticks should increase");
    }

    // The agent should have some position (physics ran on the level)
    let last_agents = &world_states.last().unwrap().1;
    assert_eq!(last_agents.len(), 1);
    let pos = &last_agents[0].transform.position;
    assert!(
        pos[0] != 0.0 || pos[1] != 0.0 || pos[2] != 0.0,
        "Agent position should not be at origin: {:?}",
        pos
    );
}

#[test]
fn test_reconnect_same_name_reclaims_player_id() {
    let server = ServerProcess::start();

    let mut alice = server.connect();
    alice.send(&ClientMessage::Join {
        player_name: "Alice".into(),
        car_name: "TestCar".into(),
        color: 21,
    });
    let mut stash = Vec::new();
    let first_id = match alice.recv_welcome(&mut stash) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };

    // Drop the TCP connection (disconnect without Leave).
    drop(alice);
    std::thread::sleep(Duration::from_millis(300));

    let mut alice2 = server.connect();
    alice2.send(&ClientMessage::Join {
        player_name: "Alice".into(),
        car_name: "TestCar".into(),
        color: 21,
    });
    let mut stash2 = Vec::new();
    let second_id = match alice2.recv_welcome(&mut stash2) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };

    assert_eq!(
        first_id, second_id,
        "same --name should reclaim the same player_id on reconnect"
    );

    // A different name must still get a distinct id.
    let mut bob = server.connect();
    bob.send(&ClientMessage::Join {
        player_name: "Bob".into(),
        car_name: "TestCar".into(),
        color: 7,
    });
    let mut bob_stash = Vec::new();
    let bob_id = match bob.recv_welcome(&mut bob_stash) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };
    assert_ne!(bob_id, second_id, "different names get distinct player_ids");
}

#[test]
fn test_reconnect_restores_last_pose() {
    let server = ServerProcess::start();

    let mut alice = server.connect();
    alice.send(&ClientMessage::Join {
        player_name: "Alice".into(),
        car_name: "TestCar".into(),
        color: 21,
    });
    let mut stash = Vec::new();
    let first_id = match alice.recv_welcome(&mut stash) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };

    // Drive away from spawn so restored pose is distinguishable.
    alice.send(&ClientMessage::Input {
        sequence: 1,
        control: vangers_net::NetControl {
            motor: 1.0,
            rudder: 0.0,
            roll: 0.0,
            brake: false,
            turbo: true,
            jump: None,
        },
    });
    let mut msgs = stash;
    msgs.extend(alice.recv_for(Duration::from_secs(2)));
    let last_pos = msgs
        .iter()
        .rev()
        .find_map(|m| match m {
            ServerMessage::WorldState { agents, .. } => agents
                .iter()
                .find(|a| a.player_id == first_id)
                .map(|a| a.transform.position),
            _ => None,
        })
        .expect("Alice should have a WorldState pose before disconnect");

    // Stop input so velocity is whatever the last tick held; then disconnect.
    drop(alice);
    std::thread::sleep(Duration::from_millis(400));

    let mut alice2 = server.connect();
    alice2.send(&ClientMessage::Join {
        player_name: "Alice".into(),
        car_name: "TestCar".into(),
        color: 21,
    });
    let mut stash2 = Vec::new();
    let second_id = match alice2.recv_welcome(&mut stash2) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };
    assert_eq!(first_id, second_id, "same name reclaims player_id");

    stash2.extend(alice2.recv_for(Duration::from_secs(1)));
    let restored_pos = stash2
        .iter()
        .find_map(|m| match m {
            ServerMessage::WorldState { agents, .. } => agents
                .iter()
                .find(|a| a.player_id == second_id)
                .map(|a| a.transform.position),
            _ => None,
        })
        .expect("Alice should receive WorldState after reconnect");

    let dx = (restored_pos[0] - last_pos[0]).abs();
    let dy = (restored_pos[1] - last_pos[1]).abs();
    assert!(
        dx < 80.0 && dy < 80.0,
        "reclaim should restore near previous XY: before={:?} after={:?} dx={} dy={}",
        last_pos,
        restored_pos,
        dx,
        dy
    );

    // Fresh name still gets a distinct id and is not forced onto Alice's pose.
    let mut bob = server.connect();
    bob.send(&ClientMessage::Join {
        player_name: "Bob".into(),
        car_name: "TestCar".into(),
        color: 7,
    });
    let mut bob_stash = Vec::new();
    let bob_id = match bob.recv_welcome(&mut bob_stash) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };
    assert_ne!(bob_id, second_id);
    bob_stash.extend(bob.recv_for(Duration::from_secs(1)));
    let bob_pos = bob_stash
        .iter()
        .find_map(|m| match m {
            ServerMessage::WorldState { agents, .. } => agents
                .iter()
                .find(|a| a.player_id == bob_id)
                .map(|a| a.transform.position),
            _ => None,
        })
        .expect("Bob should have a WorldState pose");
    let bdx = (bob_pos[0] - restored_pos[0]).abs();
    let bdy = (bob_pos[1] - restored_pos[1]).abs();
    assert!(
        bdx > 5.0 || bdy > 5.0,
        "Bob should fresh-spawn away from Alice's restored pose: bob={:?} alice={:?}",
        bob_pos,
        restored_pos
    );
}
