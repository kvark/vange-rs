use std::io::{Read, Write};
use std::path::Path;
use std::net::TcpStream;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant};

use vangers_net::{decode, encode, ClientMessage, NetTransform, ServerMessage};

static NEXT_PORT: AtomicU16 = AtomicU16::new(19876);

struct ServerProcess {
    child: Child,
    port: u16,
}

impl ServerProcess {
    fn start() -> Self {
        Self::start_with("test", &[])
    }

    fn start_with(level: &str, extra_env: &[(&str, &str)]) -> Self {
        Self::start_with_args(level, extra_env, &[])
    }

    fn start_with_args(level: &str, extra_env: &[(&str, &str)], extra_args: &[&str]) -> Self {
        let port = NEXT_PORT.fetch_add(2, Ordering::Relaxed);
        let ws_port = port + 1;
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_vangers-server"));
        // Integration tests often run with cwd = bin/server; point at the
        // workspace settings so bunches/escaves (and CycleState) load.
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let settings = workspace.join("config/settings.ron");
        cmd.current_dir(&workspace);
        cmd.args([
            "--port",
            &port.to_string(),
            "--ws-port",
            &ws_port.to_string(),
            "--level",
            level,
            "--tick-rate",
            "20",
        ]);
        if settings.is_file() {
            cmd.args(["--settings", settings.to_str().unwrap()]);
        }
        cmd.args(extra_args);
        cmd.env("RUST_LOG", "warn");
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        let child = cmd.spawn().expect("Failed to start server");
        // Fostral (and other real maps) need longer to load than `test`.
        let boot = if level == "test" {
            Duration::from_millis(500)
        } else {
            Duration::from_secs(8)
        };
        std::thread::sleep(boot);
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
            next_quant: 0,
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
    /// Next quantum index for `input` (`ClientMessage::Input::first_quant`).
    next_quant: u32,
}

/// Physics quanta a second the server runs, as a client's `QuantClock`
/// counts them: from the same settings and `common.prm` the server loads
/// (`Common::test_default` and 0.1 quanta without them: 5.6 a second).
fn quanta_per_sec() -> f32 {
    use vangers::config;
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/settings.ron");
    let settings = path
        .is_file()
        .then(|| config::Settings::try_load(path.to_str().unwrap()))
        .flatten();
    let (common, max_quant) = match settings {
        Some(ref s) => (
            config::common::load(s.open_relative("common.prm")),
            s.game.physics.max_quant,
        ),
        None => (config::common::Common::test_default(), 0.1),
    };
    let n = &common.nature;
    common.speed.standard_frame_rate as f32 * n.time_delta0 * n.num_calls_analysis as f32
        / max_quant
}

impl Client {
    /// Input for the next `quanta` quanta, as a client sends each frame's
    /// worth (held controls; a jump lands on the last quantum).
    fn input(&mut self, quanta: u16, control: vangers_net::NetControl) {
        let first_quant = self.next_quant;
        self.next_quant += quanta as u32;
        self.send(&ClientMessage::Input {
            first_quant,
            quanta,
            control,
        });
    }

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
        protocol: vangers_net::PROTOCOL_VERSION,
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
        protocol: vangers_net::PROTOCOL_VERSION,
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
        protocol: vangers_net::PROTOCOL_VERSION,
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
        protocol: vangers_net::PROTOCOL_VERSION,
    });
    let mut stash = Vec::new();
    let _ = client.recv_welcome(&mut stash);

    // Send input: full throttle forward
    client.input(
        (2.0 * quanta_per_sec()) as u16,
        vangers_net::NetControl {
            motor: 1.0,
            rudder: 0.0,
            roll: 0.0,
            brake: false,
            turbo: false,
            jump: None,
        },
    );

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
        protocol: vangers_net::PROTOCOL_VERSION,
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
        protocol: vangers_net::PROTOCOL_VERSION,
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
        protocol: vangers_net::PROTOCOL_VERSION,
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
        protocol: vangers_net::PROTOCOL_VERSION,
    });
    let mut stash = Vec::new();
    let first_id = match alice.recv_welcome(&mut stash) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };

    // Drive away from spawn so restored pose is distinguishable.
    alice.input(
        (2.0 * quanta_per_sec()) as u16,
        vangers_net::NetControl {
            motor: 1.0,
            rudder: 0.0,
            roll: 0.0,
            brake: false,
            turbo: true,
            jump: None,
        },
    );
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
        protocol: vangers_net::PROTOCOL_VERSION,
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
        protocol: vangers_net::PROTOCOL_VERSION,
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

#[test]
fn test_reconnect_restores_cirtainer() {
    // Needs game data so WorldState carries CycleState (per-player held).
    let fostral = std::path::Path::new("/workspace/vange-data/thechain/fostral/world.ini");
    if !fostral.is_file() {
        eprintln!("skip test_reconnect_restores_cirtainer: no Fostral data");
        return;
    }

    // Seed held only on fresh spawn; reclaim must restore from LastPose, not re-seed.
    let server = ServerProcess::start_with(
        fostral.to_str().expect("utf-8 path"),
        &[("VANGERS_TEST_SEED_CIRT", "7,0,2")],
    );

    let mut alice = server.connect();
    alice.send(&ClientMessage::Join {
        player_name: "Alice".into(),
        car_name: "TestCar".into(),
        color: 21,
        protocol: vangers_net::PROTOCOL_VERSION,
    });
    let mut stash = Vec::new();
    let first_id = match alice.recv_welcome(&mut stash) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };

    stash.extend(alice.recv_for(Duration::from_secs(3)));
    let held_before = stash
        .iter()
        .rev()
        .find_map(|m| match m {
            ServerMessage::WorldState {
                cycle: Some(cycle),
                ..
            } => cycle
                .players
                .iter()
                .find(|p| p.player_id == first_id)
                .map(|p| p.held.clone()),
            _ => None,
        })
        .expect("Alice should appear in CycleState.players before disconnect");
    assert_eq!(
        held_before,
        vec![7, 0, 2],
        "test seed should populate cirtainer on first join"
    );

    drop(alice);
    std::thread::sleep(Duration::from_millis(400));

    let mut alice2 = server.connect();
    alice2.send(&ClientMessage::Join {
        player_name: "Alice".into(),
        car_name: "TestCar".into(),
        color: 21,
        protocol: vangers_net::PROTOCOL_VERSION,
    });
    let mut stash2 = Vec::new();
    let second_id = match alice2.recv_welcome(&mut stash2) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };
    assert_eq!(first_id, second_id, "same name reclaims player_id");

    stash2.extend(alice2.recv_for(Duration::from_secs(3)));
    let held_after = stash2
        .iter()
        .find_map(|m| match m {
            ServerMessage::WorldState {
                cycle: Some(cycle),
                ..
            } => cycle
                .players
                .iter()
                .find(|p| p.player_id == second_id)
                .map(|p| p.held.clone()),
            _ => None,
        })
        .expect("Alice should appear in CycleState.players after reconnect");
    assert_eq!(
        held_after, held_before,
        "reclaim should restore cirtainer held, not clear it"
    );

    // Brand-new name still gets a fresh (seeded) carry, not Alice's.
    let mut bob = server.connect();
    bob.send(&ClientMessage::Join {
        player_name: "Bob".into(),
        car_name: "TestCar".into(),
        color: 7,
        protocol: vangers_net::PROTOCOL_VERSION,
    });
    let mut bob_stash = Vec::new();
    let bob_id = match bob.recv_welcome(&mut bob_stash) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };
    assert_ne!(bob_id, second_id);
    bob_stash.extend(bob.recv_for(Duration::from_secs(2)));
    let bob_held = bob_stash
        .iter()
        .find_map(|m| match m {
            ServerMessage::WorldState {
                cycle: Some(cycle),
                ..
            } => cycle
                .players
                .iter()
                .find(|p| p.player_id == bob_id)
                .map(|p| p.held.clone()),
            _ => None,
        })
        .expect("Bob should appear in CycleState.players");
    assert_eq!(bob_held, vec![7, 0, 2], "fresh name uses seed, not Alice slot");
}

#[test]
fn test_set_pose_updates_world_state() {
    let server = ServerProcess::start();
    let mut alice = server.connect();
    let mut bob = server.connect();

    alice.send(&ClientMessage::Join {
        player_name: "Alice".into(),
        car_name: "TestCar".into(),
        color: 21,
        protocol: vangers_net::PROTOCOL_VERSION,
    });
    let mut alice_stash = Vec::new();
    let alice_id = match alice.recv_welcome(&mut alice_stash) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };

    bob.send(&ClientMessage::Join {
        player_name: "Bob".into(),
        car_name: "TestCar".into(),
        color: 7,
        protocol: vangers_net::PROTOCOL_VERSION,
    });
    let mut bob_stash = Vec::new();
    let _bob_id = match bob.recv_welcome(&mut bob_stash) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };

    // Debug teleport within the 256x256 test level.
    let target = [180.0_f32, 200.0, 40.0];
    alice.send(&ClientMessage::SetPose {
        transform: NetTransform {
            position: target,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
        },
        next_quant: 0,
    });

    bob_stash.extend(bob.recv_for(Duration::from_secs(1)));
    let alice_pos = bob_stash
        .iter()
        .rev()
        .find_map(|m| match m {
            ServerMessage::WorldState { agents, .. } => agents
                .iter()
                .find(|a| a.player_id == alice_id)
                .map(|a| a.transform.position),
            _ => None,
        })
        .expect("Bob should see Alice in WorldState after SetPose");

    assert!(
        (alice_pos[0] - target[0]).abs() < 2.0 && (alice_pos[1] - target[1]).abs() < 2.0,
        "Bob should see Alice near SetPose XY {:?}, got {:?}",
        target,
        alice_pos
    );

    alice_stash.extend(alice.recv_for(Duration::from_secs(1)));
    let self_pos = alice_stash
        .iter()
        .rev()
        .find_map(|m| match m {
            ServerMessage::WorldState { agents, .. } => agents
                .iter()
                .find(|a| a.player_id == alice_id)
                .map(|a| a.transform.position),
            _ => None,
        })
        .expect("Alice should receive WorldState reflecting her SetPose");
    assert!(
        (self_pos[0] - target[0]).abs() < 2.0 && (self_pos[1] - target[1]).abs() < 2.0,
        "Alice WorldState should keep SetPose XY {:?}, got {:?}",
        target,
        self_pos
    );
}

#[test]
fn test_fire_is_relayed_to_other_players() {
    let server = ServerProcess::start();
    let mut alice = server.connect();
    let mut bob = server.connect();

    alice.send(&ClientMessage::Join {
        player_name: "Alice".into(),
        car_name: "TestCar".into(),
        color: 21,
        protocol: vangers_net::PROTOCOL_VERSION,
    });
    let mut alice_stash = Vec::new();
    let alice_id = match alice.recv_welcome(&mut alice_stash) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };
    bob.send(&ClientMessage::Join {
        player_name: "Bob".into(),
        car_name: "TestCar".into(),
        color: 7,
        protocol: vangers_net::PROTOCOL_VERSION,
    });
    let mut bob_stash = Vec::new();
    let _ = bob.recv_welcome(&mut bob_stash);

    // Fire from where the server has Alice's car.
    alice_stash.extend(alice.recv_for(Duration::from_millis(300)));
    let car = alice_stash
        .iter()
        .rev()
        .find_map(|m| match m {
            ServerMessage::WorldState { agents, .. } => agents
                .iter()
                .find(|a| a.player_id == alice_id)
                .map(|a| a.transform.position),
            _ => None,
        })
        .expect("Alice should see her car in WorldState");
    alice.send(&ClientMessage::Fire {
        quant: 42,
        bay: 1,
        origin: car,
        dir: [0.0, 2.0, 0.0],
    });
    // A muzzle far from the car and a bay that doesn't exist are dropped.
    alice.send(&ClientMessage::Fire {
        quant: 43,
        bay: 0,
        origin: [car[0], car[1], car[2] + 1000.0],
        dir: [0.0, 1.0, 0.0],
    });
    alice.send(&ClientMessage::Fire {
        quant: 44,
        bay: 200,
        origin: car,
        dir: [0.0, 1.0, 0.0],
    });

    bob_stash.extend(bob.recv_for(Duration::from_secs(1)));
    let shots: Vec<_> = bob_stash
        .iter()
        .filter_map(|m| match m {
            ServerMessage::Shot {
                shooter,
                quant,
                bay,
                origin,
                dir,
                ..
            } => Some((*shooter, *quant, *bay, *origin, *dir)),
            _ => None,
        })
        .collect();
    assert_eq!(
        shots,
        vec![(alice_id, 42, 1, car, [0.0, 1.0, 0.0])],
        "Bob should get exactly Alice's valid shot, dir normalized"
    );

    let echoed = alice
        .recv_for(Duration::from_millis(500))
        .iter()
        .chain(&alice_stash)
        .any(|m| matches!(m, ServerMessage::Shot { .. }));
    assert!(!echoed, "the shooter draws its own shot; no echo");
}

#[test]
fn test_set_spiral_broadcasts_to_peers() {
    let server = ServerProcess::start();
    let mut alice = server.connect();
    let mut bob = server.connect();

    alice.send(&ClientMessage::Join {
        player_name: "Alice".into(),
        car_name: "TestCar".into(),
        color: 21,
        protocol: vangers_net::PROTOCOL_VERSION,
    });
    let mut alice_stash = Vec::new();
    let alice_id = match alice.recv_welcome(&mut alice_stash) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };

    bob.send(&ClientMessage::Join {
        player_name: "Bob".into(),
        car_name: "TestCar".into(),
        color: 7,
        protocol: vangers_net::PROTOCOL_VERSION,
    });
    let mut bob_stash = Vec::new();
    let _ = bob.recv_welcome(&mut bob_stash);

    alice.send(&ClientMessage::SetSpiral { charge: 4 });

    bob_stash.extend(bob.recv_for(Duration::from_secs(1)));
    let alice_spiral = bob_stash
        .iter()
        .rev()
        .find_map(|m| match m {
            ServerMessage::WorldState { agents, .. } => agents
                .iter()
                .find(|a| a.player_id == alice_id)
                .map(|a| a.spiral_charge),
            _ => None,
        })
        .expect("Bob should see Alice spiral_charge in WorldState");
    assert_eq!(alice_spiral, 4, "Bob should see Alice spiral charged to 4");

    alice_stash.extend(alice.recv_for(Duration::from_secs(1)));
    let self_spiral = alice_stash
        .iter()
        .rev()
        .find_map(|m| match m {
            ServerMessage::WorldState { agents, .. } => agents
                .iter()
                .find(|a| a.player_id == alice_id)
                .map(|a| a.spiral_charge),
            _ => None,
        })
        .expect("Alice should see her own spiral_charge");
    assert_eq!(self_spiral, 4);
}

#[test]
fn test_reconnect_restores_spiral() {
    // Seed on fresh spawn only; reclaim must restore LastPose.spiral_charge.
    let server = ServerProcess::start_with("test", &[("VANGERS_TEST_SEED_SPIRAL", "3")]);
    let mut alice = server.connect();

    alice.send(&ClientMessage::Join {
        player_name: "Alice".into(),
        car_name: "TestCar".into(),
        color: 21,
        protocol: vangers_net::PROTOCOL_VERSION,
    });
    let mut stash = Vec::new();
    let first_id = match alice.recv_welcome(&mut stash) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };

    stash.extend(alice.recv_for(Duration::from_secs(1)));
    let charge_before = stash
        .iter()
        .rev()
        .find_map(|m| match m {
            ServerMessage::WorldState { agents, .. } => agents
                .iter()
                .find(|a| a.player_id == first_id)
                .map(|a| a.spiral_charge),
            _ => None,
        })
        .expect("Alice should have spiral_charge before disconnect");
    assert_eq!(charge_before, 3, "test seed should set spiral on first join");

    // Bump charge, then disconnect so LastPose keeps the updated value.
    alice.send(&ClientMessage::SetSpiral { charge: 4 });
    stash.extend(alice.recv_for(Duration::from_secs(1)));
    let charged = stash
        .iter()
        .rev()
        .find_map(|m| match m {
            ServerMessage::WorldState { agents, .. } => agents
                .iter()
                .find(|a| a.player_id == first_id)
                .map(|a| a.spiral_charge),
            _ => None,
        })
        .expect("Alice spiral after SetSpiral");
    assert_eq!(charged, 4);

    drop(alice);

    let mut alice2 = server.connect();
    alice2.send(&ClientMessage::Join {
        player_name: "Alice".into(),
        car_name: "TestCar".into(),
        color: 21,
        protocol: vangers_net::PROTOCOL_VERSION,
    });
    let mut stash2 = Vec::new();
    let second_id = match alice2.recv_welcome(&mut stash2) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };
    assert_eq!(first_id, second_id, "same name reclaims player_id");

    stash2.extend(alice2.recv_for(Duration::from_secs(1)));
    let charge_after = stash2
        .iter()
        .rev()
        .find_map(|m| match m {
            ServerMessage::WorldState { agents, .. } => agents
                .iter()
                .find(|a| a.player_id == second_id)
                .map(|a| a.spiral_charge),
            _ => None,
        })
        .expect("Alice should have spiral_charge after reconnect");
    assert_eq!(
        charge_after, 4,
        "reclaim should restore spiral_charge, not re-seed"
    );

    // Fresh name still gets the seed, not Alice's slot.
    let mut bob = server.connect();
    bob.send(&ClientMessage::Join {
        player_name: "Bob".into(),
        car_name: "TestCar".into(),
        color: 7,
        protocol: vangers_net::PROTOCOL_VERSION,
    });
    let mut bob_stash = Vec::new();
    let bob_id = match bob.recv_welcome(&mut bob_stash) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };
    bob_stash.extend(bob.recv_for(Duration::from_secs(1)));
    let bob_charge = bob_stash
        .iter()
        .rev()
        .find_map(|m| match m {
            ServerMessage::WorldState { agents, .. } => agents
                .iter()
                .find(|a| a.player_id == bob_id)
                .map(|a| a.spiral_charge),
            _ => None,
        })
        .expect("Bob should appear in WorldState");
    assert_eq!(bob_charge, 3, "fresh name uses seed, not Alice spiral");
}

/// Peak upward velocity the server reports over `duration`.
fn peak_vz(client: &mut Client, duration: Duration) -> f32 {
    client
        .recv_for(duration)
        .iter()
        .filter_map(|m| match m {
            ServerMessage::WorldState { agents, .. } => {
                agents.first().map(|a| a.dynamo.linear_velocity[2])
            }
            _ => None,
        })
        .fold(f32::MIN, f32::max)
}

#[test]
fn test_jump_reaches_server_physics() {
    let server = ServerProcess::start();
    let mut client = server.connect();
    client.send(&ClientMessage::Join {
        player_name: "Jumper".into(),
        car_name: "TestCar".into(),
        color: 21,
        protocol: vangers_net::PROTOCOL_VERSION,
    });
    let mut stash = Vec::new();
    let _ = client.recv_welcome(&mut stash);
    // Settle after the spawn drop, then measure idle vertical motion.
    let _ = client.recv_for(Duration::from_secs(1));
    let idle = peak_vz(&mut client, Duration::from_millis(600));

    // A client sends Input every frame; the jump frame is followed at once
    // by a jump-less one. The jump is on the last quantum of its batch.
    let control = |jump| vangers_net::NetControl {
        motor: 0.0,
        rudder: 0.0,
        roll: 0.0,
        brake: false,
        turbo: false,
        jump,
    };
    client.input(1, control(Some(10.0)));
    client.input(1, control(None));
    let jumped = peak_vz(&mut client, Duration::from_millis(600));
    assert!(
        jumped > idle + 5.0,
        "jump never reached server physics: idle vz {idle}, after jump {jumped}"
    );
}

#[test]
fn test_duel_spawn_places_pair_facing_each_other() {
    let server =
        ServerProcess::start_with_args("test", &[], &["--duel-spawn", "--duel-center", "100,120"]);
    let mut stash = Vec::new();
    let mut alice = server.connect();
    alice.send(&ClientMessage::Join {
        player_name: "Alice".into(),
        car_name: "TestCar".into(),
        color: 21,
        protocol: vangers_net::PROTOCOL_VERSION,
    });
    let alice_id = match alice.recv_welcome(&mut stash) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };
    let mut bob = server.connect();
    bob.send(&ClientMessage::Join {
        player_name: "Bob".into(),
        car_name: "TestCar".into(),
        color: 22,
        protocol: vangers_net::PROTOCOL_VERSION,
    });
    // `recv_welcome` keeps reading for a while; the procedural level is
    // hilly, so read the spawn pose from the earliest stashed snapshot.
    stash.clear();
    let bob_id = match bob.recv_welcome(&mut stash) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };

    // First WorldState with both: spawn pose before physics moves them far.
    let agents = stash
        .into_iter()
        .find_map(|m| match m {
            ServerMessage::WorldState { agents, .. } if agents.len() == 2 => Some(agents),
            _ => None,
        })
        .expect("WorldState with both players");
    let pose = |id| {
        let a = agents.iter().find(|a| a.player_id == id).unwrap();
        let rot = glam::Quat::from_array(a.transform.rotation);
        (glam::Vec3::from(a.transform.position), rot * glam::Vec3::Y)
    };
    let (pa, fa) = pose(alice_id);
    let (pb, fb) = pose(bob_id);
    assert!((pa.x - 65.0).abs() < 2.0 && (pa.y - 120.0).abs() < 2.0, "Alice at {pa:?}");
    assert!((pb.x - 135.0).abs() < 2.0 && (pb.y - 120.0).abs() < 2.0, "Bob at {pb:?}");
    let a_to_b = (pb - pa).truncate().normalize().extend(0.0);
    assert!(fa.dot(a_to_b) > 0.9, "Alice faces Bob: {fa:?}");
    assert!(fb.dot(-a_to_b) > 0.9, "Bob faces Alice: {fb:?}");
}

/// Slow ram through the real server tick loop, like the live duel test:
/// Fostral, `--duel-spawn`, Bob idle, Alice taps the throttle. Alice must
/// shove Bob along and never get past him (before car contact she drove
/// straight through).
#[test]
fn test_duel_slow_ram_shoves_parked_car() {
    let fostral = std::path::Path::new("/workspace/vange-data/thechain/fostral/world.ini");
    if !fostral.is_file() {
        eprintln!("skip test_duel_slow_ram_shoves_parked_car: no Fostral data");
        return;
    }
    let server = ServerProcess::start_with_args(fostral.to_str().unwrap(), &[], &["--duel-spawn"]);
    let join = |name: &str| {
        let mut c = server.connect();
        c.send(&ClientMessage::Join {
            player_name: name.into(),
            car_name: "OxidizeMonk".into(),
            color: 21,
            protocol: vangers_net::PROTOCOL_VERSION,
        });
        let mut stash = Vec::new();
        let id = match c.recv_welcome(&mut stash) {
            ServerMessage::Welcome { player_id, .. } => player_id,
            _ => unreachable!(),
        };
        (c, id)
    };
    let (mut alice, alice_id) = join("Alice");
    let (mut bob, bob_id) = join("Bob");
    let throttle = |motor: f32| vangers_net::NetControl {
        motor,
        rudder: 0.0,
        roll: 0.0,
        brake: false,
        turbo: false,
        jump: None,
    };
    // Alice streams her quanta in real time, as a client does.
    let rate = quanta_per_sec();
    let mut owed = 0.0_f32;
    let mut last: Option<Instant> = None;
    // The quanta due since the last call (the first call starts the clock
    // and sends this call's 0.1 s).
    let mut stream = |alice: &mut Client, motor: f32| {
        let now = Instant::now();
        let secs = last.map_or(0.1, |t| (now - t).as_secs_f32());
        owed += secs * rate;
        last = Some(now);
        let whole = owed.floor();
        owed -= whole;
        if whole > 0.0 {
            alice.input(whole as u16, throttle(motor));
        }
    };
    // (Alice x, Bob x) per WorldState.
    let mut trace: Vec<(f32, f32)> = Vec::new();
    let read = |alice: &mut Client, dur: Duration, trace: &mut Vec<(f32, f32)>| {
        for m in alice.recv_for(dur) {
            if let ServerMessage::WorldState { agents, .. } = m {
                let x = |id| {
                    agents
                        .iter()
                        .find(|a| a.player_id == id)
                        .map(|a| a.transform.position[0])
                };
                if let (Some(a), Some(b)) = (x(alice_id), x(bob_id)) {
                    trace.push((a, b));
                }
            }
        }
    };
    // Settle on the ground.
    read(&mut alice, Duration::from_secs(2), &mut trace);
    let (_, bob_start) = *trace.last().expect("WorldState with both players");
    // Slow ram: 0.2 s throttle taps for 4 s, then coast.
    for i in 0..40 {
        stream(&mut alice, if i % 4 < 2 { 1.0 } else { 0.0 });
        read(&mut alice, Duration::from_millis(100), &mut trace);
        let _ = bob.recv_for(Duration::from_millis(1));
    }
    for _ in 0..40 {
        stream(&mut alice, 0.0);
        read(&mut alice, Duration::from_millis(100), &mut trace);
    }

    let closest = trace.iter().map(|&(a, b)| b - a).fold(f32::MAX, f32::min);
    let (alice_end, bob_end) = *trace.last().unwrap();
    eprintln!(
        "slow ram: Bob {bob_start:.1} -> {bob_end:.1}, Alice ends {alice_end:.1}, closest centres {closest:.1}"
    );
    // A mechos contact box is ~41u long: centres closer than that overlap.
    assert!(closest > 30.0, "Alice drove into / through Bob: centres {closest:.1}u apart");
    assert!(bob_end - bob_start > 20.0, "Bob was not shoved: {bob_start:.1} -> {bob_end:.1}");
}
