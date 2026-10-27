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
        // Scripted clients go quiet for seconds between steps, which a
        // real client (an `Input` every frame) never does: no silent
        // timeout unless a test asks for one.
        if !extra_args.contains(&"--silent-timeout-secs") {
            cmd.args(["--silent-timeout-secs", "0"]);
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
/// The guns every test car joins with (`TestCar` has no `car.prm`
/// entry, so each bay takes any size): a Machotine (bay 0), a light and a
/// heavy Ghorb (bays 1 and 2).
const GUNS: [&str; vangers_net::BAYS] = ["LightMissile", "LightFireBall", "HeavyFireBall"];

fn guns() -> vangers_net::NetGuns {
    GUNS.map(|g| Some(g.to_string()))
}

/// The damage of a shot from `bay` of `GUNS`, in hundredths of armour.
fn bay_damage(bay: usize) -> u32 {
    vangers::combat::weapons::Table::original()
        .gun(GUNS[bay])
        .expect("an original gun")
        .damage()
}

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
        guns: guns(),
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
        guns: guns(),
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
        guns: guns(),
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
        guns: guns(),
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
        guns: guns(),
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
        guns: guns(),
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
        guns: guns(),
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
        guns: guns(),
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
        guns: guns(),
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
        guns: guns(),
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
        guns: guns(),
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
        guns: guns(),
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
        guns: guns(),
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

/// The newest `WorldState` pose and ack of player `id`.
fn pose_in_world(msgs: &[ServerMessage], id: u32) -> Option<(NetTransform, u32)> {
    msgs.iter().rev().find_map(|m| match m {
        ServerMessage::WorldState { agents, .. } => agents
            .iter()
            .find(|a| a.player_id == id)
            .map(|a| (a.transform.clone(), a.ack)),
        _ => None,
    })
}

fn set_pose_to(at: [f32; 3], next_quant: u32) -> ClientMessage {
    ClientMessage::SetPose {
        transform: NetTransform {
            position: at,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
        },
        next_quant,
    }
}

#[test]
fn test_set_pose_needs_allow_setpose() {
    let server = ServerProcess::start();
    let mut alice = server.connect();
    alice.send(&ClientMessage::Join {
        player_name: "Alice".into(),
        car_name: "TestCar".into(),
        color: 21,
        protocol: vangers_net::PROTOCOL_VERSION,
        guns: guns(),
    });
    let mut stash = Vec::new();
    let alice_id = match alice.recv_welcome(&mut stash) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };
    stash.extend(alice.recv_for(Duration::from_millis(500)));
    let (before, _) = pose_in_world(&stash, alice_id).expect("WorldState");
    let target = [180.0_f32, 200.0, 40.0];
    alice.send(&set_pose_to(target, 0));
    let msgs = alice.recv_for(Duration::from_secs(1));
    let (after, _) = pose_in_world(&msgs, alice_id).expect("WorldState");
    let moved = |a: [f32; 3], b: [f32; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    assert!(
        moved(after.position, before.position) < 2.0,
        "SetPose without --allow-setpose moved the car: {:?} -> {:?}",
        before.position,
        after.position
    );
    assert!(moved(after.position, target) > 20.0);
}

#[test]
fn test_set_pose_updates_world_state() {
    let server = ServerProcess::start_with_args("test", &[], &["--allow-setpose"]);
    let mut alice = server.connect();
    let mut bob = server.connect();

    alice.send(&ClientMessage::Join {
        player_name: "Alice".into(),
        car_name: "TestCar".into(),
        color: 21,
        protocol: vangers_net::PROTOCOL_VERSION,
        guns: guns(),
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
        guns: guns(),
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
        guns: guns(),
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
        guns: guns(),
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
        view_tick: 0,
        view_ms: 0,
    });
    // A muzzle far from the car and a bay that doesn't exist are dropped.
    alice.send(&ClientMessage::Fire {
        quant: 43,
        bay: 0,
        origin: [car[0], car[1], car[2] + 1000.0],
        dir: [0.0, 1.0, 0.0],
        view_tick: 0,
        view_ms: 0,
    });
    alice.send(&ClientMessage::Fire {
        quant: 44,
        bay: 200,
        origin: car,
        dir: [0.0, 1.0, 0.0],
        view_tick: 0,
        view_ms: 0,
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

/// The newest `WorldState` in `msgs`: its tick and each agent's
/// `(player_id, position, armor, max_armor)`.
fn latest_world(msgs: &[ServerMessage]) -> Option<(u32, Vec<(u32, [f32; 3], u32, u32)>)> {
    msgs.iter().rev().find_map(|m| match m {
        ServerMessage::WorldState { tick, agents, .. } => Some((
            *tick,
            agents
                .iter()
                .map(|a| (a.player_id, a.transform.position, a.armor, a.max_armor))
                .collect(),
        )),
        _ => None,
    })
}

#[test]
fn test_shot_takes_armor_off_the_target_only() {
    let server = ServerProcess::start();
    let mut alice = server.connect();
    let mut bob = server.connect();

    alice.send(&ClientMessage::Join {
        player_name: "Alice".into(),
        car_name: "TestCar".into(),
        color: 21,
        protocol: vangers_net::PROTOCOL_VERSION,
        guns: guns(),
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
        guns: guns(),
    });
    let mut bob_stash = Vec::new();
    let bob_id = match bob.recv_welcome(&mut bob_stash) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };

    // Let both cars land first.
    alice_stash.extend(alice.recv_for(Duration::from_secs(2)));
    let (tick, agents) = latest_world(&alice_stash).expect("Alice should get a WorldState");
    let find = |agents: &[(u32, [f32; 3], u32, u32)], id| {
        *agents
            .iter()
            .find(|a| a.0 == id)
            .expect("both cars in WorldState")
    };
    let (_, a_pos, a_armor, a_max) = find(&agents, alice_id);
    let (_, b_pos, b_armor, b_max) = find(&agents, bob_id);
    assert!(a_max > 0 && b_max > 0, "armour comes with the car");
    assert_eq!((a_armor, b_armor), (a_max, b_max), "both start undamaged");

    // Bob's car sits ~90u up the slope from Alice's, just under the crest
    // of a ridge. Her muzzle is raised over the ridge's far side (within
    // `weapon::FIRE_REACH` of her car): a shot from there straight at his
    // car has to go through the ridge, and does nothing.
    let muzzle = [a_pos[0] + 64.0, a_pos[1] - 28.0, a_pos[2] + 72.0];
    let through_ridge = [
        b_pos[0] - muzzle[0],
        b_pos[1] - muzzle[1],
        b_pos[2] - muzzle[2],
    ];
    // Check the ridge is really there (the server's level, read point by
    // point): the line is deep in the rock along its middle, not grazing
    // it, and clear at both ends.
    let level = vangers::level::load(
        &vangers::level::LevelConfig::new_test(),
        &Default::default(),
    );
    let along = |f: f32| glam::Vec3::from(muzzle) + glam::Vec3::from(through_ridge) * f;
    assert!(
        !level.solid_at(along(0.0)) && !level.solid_at(along(1.0)),
        "spawns moved: {a_pos:?} {b_pos:?}"
    );
    for f in [0.3, 0.4, 0.5, 0.6] {
        let p = along(f);
        assert!(
            level.solid_at(p + glam::Vec3::Z * 5.0),
            "the ridge is not 5 deep over {p:?}; spawns moved: {a_pos:?} {b_pos:?}"
        );
    }
    alice.send(&ClientMessage::Fire {
        quant: 9,
        bay: 2,
        origin: muzzle,
        dir: through_ridge,
        view_tick: tick,
        view_ms: 0,
    });
    let msgs = bob.recv_for(Duration::from_millis(500));
    let (tick, agents) = latest_world(&msgs).expect("Bob should keep getting WorldState");
    assert_eq!(find(&agents, bob_id).2, b_max, "the ridge takes the shot");

    // Two shots at Bob's car (one bay 1, one bay 0), aimed in the view of
    // the snapshot just received; one aimed away. They leave from a muzzle
    // high over Alice's car (within `weapon::FIRE_REACH`) and come down on
    // Bob over the hill.
    let a_pos = [a_pos[0], a_pos[1], a_pos[2] + 120.0];
    let at_bob = [
        b_pos[0] - a_pos[0],
        b_pos[1] - a_pos[1],
        b_pos[2] - a_pos[2],
    ];
    for (quant, bay, dir) in [(10, 1, at_bob), (11, 0, at_bob)] {
        alice.send(&ClientMessage::Fire {
            quant,
            bay,
            origin: a_pos,
            dir,
            view_tick: tick,
            view_ms: 0,
        });
    }
    std::thread::sleep(Duration::from_millis(80));
    alice.send(&ClientMessage::Fire {
        quant: 12,
        bay: 2,
        origin: a_pos,
        dir: [-at_bob[0], -at_bob[1], -at_bob[2]],
        view_tick: tick,
        view_ms: 0,
    });

    // Shots fly for at most `SHOT_LIFE` (1.5 s); give them time to land.
    let msgs = bob.recv_for(Duration::from_secs(2));
    let (_, agents) = latest_world(&msgs).expect("Bob should keep getting WorldState");
    let (_, _, a_armor, _) = find(&agents, alice_id);
    let (_, _, b_armor, _) = find(&agents, bob_id);
    let expected = b_max.saturating_sub(bay_damage(1) + bay_damage(0));
    assert_eq!(
        b_armor, expected,
        "Bob takes both hits, the stray shot misses"
    );
    assert_eq!(a_armor, a_max, "the shooter is not hit by its own shots");
}

/// `(tick, energy, max_energy, armor)` of `id` in every `WorldState` of
/// `msgs`, in order.
fn hull_track(msgs: &[ServerMessage], id: u32) -> Vec<(u32, u32, u32, u32)> {
    msgs.iter()
        .filter_map(|m| match m {
            ServerMessage::WorldState { tick, agents, .. } => agents
                .iter()
                .find(|a| a.player_id == id)
                .map(|a| (*tick, a.energy, a.max_energy, a.armor)),
            _ => None,
        })
        .collect()
}

#[test]
fn test_hits_drain_the_shield_before_the_armour_and_it_refills() {
    // TestCar is not in a car.prm: give it the OxidizeMonk's shield
    // (MaxEnergy 120, DeltaEnergy 40), 0.24 points a quant.
    let server = ServerProcess::start_with("test", &[("VANGERS_TEST_SHIELD", "120:40")]);
    let regen = vangers::combat::shield::regen_per_quant(120, 40);
    assert_eq!(regen, 24);
    let mut alice = server.connect();
    let mut bob = server.connect();
    alice.send(&ClientMessage::Join {
        player_name: "Alice".into(),
        car_name: "TestCar".into(),
        color: 21,
        protocol: vangers_net::PROTOCOL_VERSION,
        guns: guns(),
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
        guns: guns(),
    });
    let mut bob_stash = Vec::new();
    let bob_id = match bob.recv_welcome(&mut bob_stash) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };

    alice_stash.extend(alice.recv_for(Duration::from_secs(2)));
    let (tick, agents) = latest_world(&alice_stash).expect("Alice should get a WorldState");
    let find = |id| *agents.iter().find(|a| a.0 == id).expect("both cars");
    let (_, a_pos, _, _) = find(alice_id);
    let (_, b_pos, _, b_max) = find(bob_id);
    let &(_, energy, max_energy, armor) = hull_track(&alice_stash, bob_id).last().unwrap();
    assert_eq!(max_energy, 12_000, "120 points of shield");
    assert_eq!(
        (energy, armor),
        (max_energy, b_max),
        "both full at the start"
    );

    // Heavy Ghorb shots (bay 2, 25 points each) from high over Alice's
    // car down onto Bob's, one a reload (0.55 s).
    let muzzle = [a_pos[0], a_pos[1], a_pos[2] + 120.0];
    let at_bob = [
        b_pos[0] - muzzle[0],
        b_pos[1] - muzzle[1],
        b_pos[2] - muzzle[2],
    ];
    let hit = bay_damage(2);
    assert_eq!(hit, 2500);
    let mut track = Vec::new();
    for quant in 0..8 {
        alice.send(&ClientMessage::Fire {
            quant: 20 + quant,
            bay: 2,
            origin: muzzle,
            dir: at_bob,
            view_tick: tick,
            view_ms: 0,
        });
        track.extend(hull_track(
            &bob.recv_for(Duration::from_millis(600)),
            bob_id,
        ));
        if track.last().is_some_and(|t| t.3 < b_max) {
            break;
        }
    }
    track.extend(hull_track(
        &bob.recv_for(Duration::from_millis(600)),
        bob_id,
    ));

    // Each hit a tick: the shield takes it whole while it lasts, the
    // armour untouched; the tick after, the shield refills by `regen`.
    let hits: Vec<_> = track
        .windows(2)
        .filter(|w| w[1].1 < w[0].1 || w[1].3 < w[0].3)
        .map(|w| (w[0], w[1]))
        .collect();
    assert!(
        hits.len() >= 5,
        "five hits to get through 120 points: {hits:?}"
    );
    let (last_before, last_after) = *hits.last().unwrap();
    for &(before, after) in &hits[..hits.len() - 1] {
        assert_eq!(
            after.0,
            before.0 + 1,
            "consecutive snapshots: {before:?} {after:?}"
        );
        assert_eq!(after.3, b_max, "the shield takes it: {after:?}");
        let refilled = (before.1 + regen).min(max_energy);
        assert_eq!(after.1, refilled - hit, "the whole hit off the shield");
    }
    // The hit the shield could not hold: it empties, and only the rest
    // comes off the armour.
    assert_eq!(last_after.0, last_before.0 + 1);
    assert_eq!(last_after.1, 0, "the shield is empty: {last_after:?}");
    let through = hit - (last_before.1 + regen).min(max_energy);
    assert!(through > 0 && through < hit, "part of the hit: {through}");
    assert_eq!(last_after.3, b_max - through, "the rest off the armour");

    // No more shots: the shield refills at the original's rate, 0.24
    // points a quant (4.8 a second), and the armour does not.
    let refill = hull_track(&bob.recv_for(Duration::from_secs(2)), bob_id);
    let (first, last) = (refill[0], *refill.last().unwrap());
    assert!(
        last.0 - first.0 >= 30,
        "two seconds of snapshots: {first:?} {last:?}"
    );
    assert!(last.1 < max_energy, "not full yet: {last:?}");
    assert_eq!(last.1 - first.1, (last.0 - first.0) * regen, "24 a tick");
    assert_eq!((first.3, last.3), (b_max - through, b_max - through));
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
        guns: guns(),
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
        guns: guns(),
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
        guns: guns(),
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
        guns: guns(),
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
        guns: guns(),
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
        guns: guns(),
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
        guns: guns(),
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
        guns: guns(),
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
            guns: guns(),
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

/// Bob's `(position, armor, max_armor, dead)` in the newest `WorldState`.
fn bob_in_world(msgs: &[ServerMessage], bob_id: u32) -> Option<([f32; 3], u32, u32, bool)> {
    msgs.iter().rev().find_map(|m| match m {
        ServerMessage::WorldState { agents, .. } => agents
            .iter()
            .find(|a| a.player_id == bob_id)
            .map(|a| (a.transform.position, a.armor, a.max_armor, a.dead)),
        _ => None,
    })
}

#[test]
fn test_a_kill_respawns_the_wreck_at_the_nearest_escave() {
    // Two escaves: one ~50u from where Bob's car stands, one ~150u off
    // (across the seams).
    let near = (130, 90);
    let far = (220, 220);
    // `--allow-setpose`: even the debug pose is refused from a wreck and
    // just after a respawn.
    let server = ServerProcess::start_with_args(
        "test",
        &[],
        &[
            "--escave",
            "130,90",
            "--escave",
            "220,220",
            "--allow-setpose",
        ],
    );
    let mut alice = server.connect();
    let mut bob = server.connect();
    let mut join = |client: &mut Client, name: &str, color| {
        client.send(&ClientMessage::Join {
            player_name: name.into(),
            car_name: "TestCar".into(),
            color,
            protocol: vangers_net::PROTOCOL_VERSION,
            guns: guns(),
        });
        let mut stash = Vec::new();
        match client.recv_welcome(&mut stash) {
            ServerMessage::Welcome { player_id, .. } => (player_id, stash),
            _ => unreachable!(),
        }
    };
    let (alice_id, mut alice_stash) = join(&mut alice, "Alice", 21);
    let (bob_id, _) = join(&mut bob, "Bob", 7);

    alice_stash.extend(alice.recv_for(Duration::from_secs(2)));
    let (tick, agents) = latest_world(&alice_stash).expect("Alice should get a WorldState");
    let pos = |id| agents.iter().find(|a| a.0 == id).expect("both cars").1;
    let (a_pos, b_pos) = (pos(alice_id), pos(bob_id));
    let b_max = agents.iter().find(|a| a.0 == bob_id).unwrap().3;
    let level = vangers::level::load(
        &vangers::level::LevelConfig::new_test(),
        &Default::default(),
    );
    let flat_gap = |a: [f32; 3], b: (i32, i32)| {
        level
            .shortest_xy(
                glam::Vec3::from(a),
                glam::Vec3::new(b.0 as f32, b.1 as f32, 0.0),
            )
            .length()
    };
    assert!(
        flat_gap(b_pos, near) + 50.0 < flat_gap(b_pos, far),
        "spawns moved: Bob at {b_pos:?}"
    );

    // Enough heavy Ghorb shots (bay 2) to finish Bob, one a reload,
    // from high over Alice's car down onto his.
    let muzzle = [a_pos[0], a_pos[1], a_pos[2] + 120.0];
    let at_bob = [
        b_pos[0] - muzzle[0],
        b_pos[1] - muzzle[1],
        b_pos[2] - muzzle[2],
    ];
    let shots = b_max.div_ceil(bay_damage(2));
    let fire = |client: &mut Client, quant| {
        client.send(&ClientMessage::Fire {
            quant,
            bay: 2,
            origin: muzzle,
            dir: at_bob,
            view_tick: tick,
            view_ms: 0,
        });
    };
    // One a reload (0.55 s for the heavy Ghorb).
    for quant in 0..shots {
        fire(&mut alice, 20 + quant);
        std::thread::sleep(Duration::from_millis(600));
    }

    // Everyone hears of the death, with the killer.
    let mut bob_msgs = bob.recv_for(Duration::from_millis(1500));
    let died = |msgs: &[ServerMessage]| {
        msgs.iter()
            .filter_map(|m| match m {
                ServerMessage::Died {
                    player_id,
                    killer,
                    position,
                } => Some((*player_id, *killer, *position)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let deaths = died(&bob_msgs);
    assert_eq!(deaths.len(), 1, "one death: {deaths:?}");
    assert_eq!((deaths[0].0, deaths[0].1), (bob_id, Some(alice_id)));
    assert!(
        flat_gap(deaths[0].2, (b_pos[0] as i32, b_pos[1] as i32)) < 20.0,
        "died where it stood: {:?}",
        deaths[0].2
    );
    let (_, armor, _, dead) = bob_in_world(&bob_msgs, bob_id).expect("WorldState");
    assert_eq!((armor, dead), (0, true), "a wreck in the WorldState");

    // A shot at the wreck does nothing (no second death), and neither
    // does a pose the wreck sets itself.
    fire(&mut alice, 40);
    let far_off = [b_pos[0] + 60.0, b_pos[1] - 50.0, b_pos[2]];
    bob.send(&set_pose_to(far_off, 5_000));
    bob.send(&ClientMessage::Reset {
        position: [b_pos[0] + 10.0, b_pos[1], b_pos[2]],
        next_quant: 5_000,
    });
    let more = bob.recv_for(Duration::from_millis(600));
    assert!(died(&more).is_empty(), "a wreck dies once");
    let (wreck, ack) = pose_in_world(&more, bob_id).expect("WorldState");
    assert!(
        flat_gap(wreck.position, (b_pos[0] as i32, b_pos[1] as i32)) < 5.0,
        "the wreck moved: {:?}",
        wreck.position
    );
    assert_ne!(ack, 5_000, "the wreck's SetPose reset its inputs");
    bob_msgs.extend(more);

    // About `RESPAWN_SECS` after the death: back at the near escave, whole.
    let deadline = Instant::now() + Duration::from_secs(5);
    let respawn = loop {
        let msgs = bob.recv_for(Duration::from_millis(200));
        let found = msgs.iter().find_map(|m| match m {
            ServerMessage::Respawned {
                player_id,
                transform,
                next_quant,
            } if *player_id == bob_id => Some((transform.position, *next_quant)),
            _ => None,
        });
        bob_msgs.extend(msgs);
        if let Some(at) = found {
            break at;
        }
        assert!(Instant::now() < deadline, "no respawn");
    };
    let (respawn, next_quant) = respawn;
    // Right after the respawn (a SetPose racing it): both refused.
    bob.send(&set_pose_to(far_off, next_quant + 9));
    bob.send(&ClientMessage::Reset {
        position: [respawn[0] + 10.0, respawn[1], respawn[2]],
        next_quant: next_quant + 9,
    });
    // Late inputs from the wreck, numbered below `next_quant`: dropped.
    bob.input(30, vangers_net::NetControl::default());
    assert!(
        flat_gap(respawn, near) <= 40.0,
        "respawned {respawn:?}, not by the escave at {near:?}"
    );
    let msgs = bob.recv_for(Duration::from_millis(500));
    let (at, armor, max, dead) = bob_in_world(&msgs, bob_id).expect("WorldState");
    assert_eq!((armor, dead), (max, false), "whole again");
    assert!(flat_gap(at, near) <= 50.0, "stays by the escave: {at:?}");
    let (pose, ack) = pose_in_world(&msgs, bob_id).expect("WorldState");
    assert_eq!(
        ack, next_quant,
        "inputs numbered from the Respawned quantum"
    );
    assert!(
        (pose.position[0] - respawn[0]).abs() < 2.0 && (pose.position[1] - respawn[1]).abs() < 2.0,
        "a pose just after the respawn moved it: {:?} -> {:?}",
        respawn,
        pose.position
    );

    // Bob numbers on from `next_quant`: taken and acked.
    bob.next_quant = next_quant;
    bob.input(4, vangers_net::NetControl::default());
    // A coarse `max_quant` (the test defaults) runs a quantum every few
    // ticks: give them time.
    let msgs = bob.recv_for(Duration::from_millis(1000));
    let (_, ack) = pose_in_world(&msgs, bob_id).expect("WorldState");
    assert_eq!(ack, next_quant + 4, "inputs after the respawn run");

    // Past the lock the upright key works, near the server's pose only.
    let (pose, _) = pose_in_world(&bob.recv_for(Duration::from_millis(200)), bob_id).unwrap();
    let here = pose.position;
    bob.send(&ClientMessage::Reset {
        position: [here[0] + 10.0, here[1], here[2]],
        next_quant: next_quant + 100,
    });
    let msgs = bob.recv_for(Duration::from_millis(300));
    let (upright, ack) = pose_in_world(&msgs, bob_id).expect("WorldState");
    assert_eq!(ack, next_quant + 100);
    assert!(
        (upright.position[0] - (here[0] + 10.0)).abs() < 2.0,
        "Reset within reach: {:?} from {:?}",
        upright.position,
        here
    );
    let rot = glam::Quat::from_array(upright.rotation);
    assert!(
        rot.angle_between(glam::Quat::IDENTITY) < 0.05,
        "upright: {rot:?}"
    );
    let here = upright.position;
    bob.send(&ClientMessage::Reset {
        position: [here[0] + 300.0, here[1], here[2]],
        next_quant: next_quant + 200,
    });
    let msgs = bob.recv_for(Duration::from_millis(300));
    let (kept, _) = pose_in_world(&msgs, bob_id).expect("WorldState");
    assert!(
        (kept.position[0] - here[0]).abs() < 2.0,
        "a Reset out of reach moved the car: {:?} -> {:?}",
        here,
        kept.position
    );

    // Alice saw both, in that order.
    alice_stash.extend(alice.recv_for(Duration::from_secs(1)));
    let seen: Vec<&str> = alice_stash
        .iter()
        .filter_map(|m| match m {
            ServerMessage::Died { player_id, .. } if *player_id == bob_id => Some("died"),
            ServerMessage::Respawned { player_id, .. } if *player_id == bob_id => Some("respawned"),
            _ => None,
        })
        .collect();
    assert_eq!(seen, ["died", "respawned"]);
}

/// Per player in the newest `WorldState`: (position, armour, in_escave,
/// bumps).
type CarInWorld = (u32, [f32; 3], u32, bool, u32);

fn cars_in_world(msgs: &[ServerMessage]) -> Option<(u32, Vec<CarInWorld>)> {
    msgs.iter().rev().find_map(|m| match m {
        ServerMessage::WorldState { tick, agents, .. } => Some((
            *tick,
            agents
                .iter()
                .map(|a| {
                    (
                        a.player_id,
                        a.transform.position,
                        a.armor,
                        a.in_escave,
                        a.bumps,
                    )
                })
                .collect(),
        )),
        _ => None,
    })
}

#[test]
fn test_a_car_in_an_escave_is_out_of_shots_and_contact() {
    // One escave ~50u from where Bob's car stands.
    let server =
        ServerProcess::start_with_args("test", &[], &["--escave", "130,90", "--allow-setpose"]);
    let mut alice = server.connect();
    let mut bob = server.connect();
    let join = |client: &mut Client, name: &str, color| {
        client.send(&ClientMessage::Join {
            player_name: name.into(),
            car_name: "TestCar".into(),
            color,
            protocol: vangers_net::PROTOCOL_VERSION,
            guns: guns(),
        });
        let mut stash = Vec::new();
        match client.recv_welcome(&mut stash) {
            ServerMessage::Welcome { player_id, .. } => player_id,
            _ => unreachable!(),
        }
    };
    let alice_id = join(&mut alice, "Alice", 21);
    let bob_id = join(&mut bob, "Bob", 7);
    let car = |msgs: &[ServerMessage], id| {
        let (tick, cars) = cars_in_world(msgs).expect("a WorldState");
        let c = *cars.iter().find(|c| c.0 == id).expect("both cars");
        (tick, c.1, c.2, c.3, c.4)
    };

    let msgs = alice.recv_for(Duration::from_secs(2));
    let (_, a_pos, a_max, _, _) = car(&msgs, alice_id);
    let (_, b_pos, b_max, b_inside, _) = car(&msgs, bob_id);
    assert!(!b_inside, "everyone starts on the road");
    let level = vangers::level::load(
        &vangers::level::LevelConfig::new_test(),
        &Default::default(),
    );
    let gap = |a: [f32; 3], b: (f32, f32)| {
        level
            .shortest_xy(glam::Vec3::from(a), glam::Vec3::new(b.0, b.1, 0.0))
            .length()
    };
    let reach = vangers::combat::ESCAVE_REACH;
    assert!(
        gap(b_pos, (130.0, 90.0)) < reach,
        "spawns moved: Bob at {b_pos:?}"
    );

    bob.send(&ClientMessage::Escave { inside: true });
    let msgs = alice.recv_for(Duration::from_millis(500));
    let (tick, _, _, inside, _) = car(&msgs, bob_id);
    assert!(inside, "Bob is in the escave");

    // Shots fly through Bob's car, and his own are dropped.
    let muzzle = [a_pos[0], a_pos[1], a_pos[2] + 120.0];
    let down_at =
        |from: [f32; 3], to: [f32; 3]| [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let fire = |client: &mut Client, quant, origin, dir, view_tick| {
        client.send(&ClientMessage::Fire {
            quant,
            bay: 2,
            origin,
            dir,
            view_tick,
            view_ms: 0,
        });
    };
    let b_muzzle = [b_pos[0], b_pos[1], b_pos[2] + 120.0];
    for quant in 0..3 {
        fire(&mut alice, 20 + quant, muzzle, down_at(muzzle, b_pos), tick);
        fire(
            &mut bob,
            20 + quant,
            b_muzzle,
            down_at(b_muzzle, a_pos),
            tick,
        );
        std::thread::sleep(Duration::from_millis(120));
    }
    let msgs = alice.recv_for(Duration::from_secs(2));
    let (_, _, a_armor, _, _) = car(&msgs, alice_id);
    let (tick, _, b_armor, _, _) = car(&msgs, bob_id);
    assert_eq!(b_armor, b_max, "shots fly through a car in an escave");
    assert_eq!(a_armor, a_max, "a car in an escave does not fire");

    // Out again: hit like any car.
    bob.send(&ClientMessage::Escave { inside: false });
    let msgs = alice.recv_for(Duration::from_millis(300));
    let (tick_out, _, _, inside, _) = car(&msgs, bob_id);
    assert!(!inside, "Bob came out");
    fire(
        &mut alice,
        30,
        muzzle,
        down_at(muzzle, b_pos),
        tick_out.max(tick),
    );
    let msgs = alice.recv_for(Duration::from_secs(2));
    let hit = bay_damage(2);
    let (_, _, b_armor, _, b_bumps) = car(&msgs, bob_id);
    assert_eq!(b_armor, b_max - hit, "back on the road, Bob is hit");

    // Back in, Alice parks on top of him and drives: no contact.
    bob.send(&ClientMessage::Escave { inside: true });
    let _ = alice.recv_for(Duration::from_millis(300));
    let onto = [b_pos[0] + 6.0, b_pos[1], b_pos[2] + 2.0];
    let next = alice.next_quant;
    alice.send(&set_pose_to(onto, next));
    let rate = quanta_per_sec();
    let drive = |alice: &mut Client, bob: &mut Client, secs: f32| {
        let mut msgs = Vec::new();
        for _ in 0..(secs * 10.0) as u32 {
            alice.input((rate * 0.1).ceil() as u16, Default::default());
            msgs.extend(alice.recv_for(Duration::from_millis(100)));
            let _ = bob.recv_for(Duration::from_millis(1));
        }
        msgs
    };
    let msgs = drive(&mut alice, &mut bob, 1.5);
    let (_, b_now, _, inside, bumps) = car(&msgs, bob_id);
    assert!(inside);
    assert_eq!(bumps, b_bumps, "nothing touches a car in an escave");
    assert!(
        gap(b_now, (b_pos[0], b_pos[1])) < 1.0,
        "Bob was moved in the escave: {b_pos:?} -> {b_now:?}"
    );

    // Out again with Alice on top: the cars meet.
    bob.send(&ClientMessage::Escave { inside: false });
    let msgs = drive(&mut alice, &mut bob, 1.5);
    let (_, b_now, _, inside, bumps) = car(&msgs, bob_id);
    assert!(!inside);
    assert!(
        bumps != b_bumps || gap(b_now, (b_pos[0], b_pos[1])) > 3.0,
        "Bob is back in contact: bumps {b_bumps} -> {bumps}, {b_pos:?} -> {b_now:?}"
    );

    // Across the map from the escave (the far corner of the torus), Alice
    // can't go in.
    let far = [130.0 + 128.0, 90.0 + 128.0, a_pos[2] + 60.0];
    assert!(gap(far, (130.0, 90.0)) > reach);
    let next = alice.next_quant;
    alice.send(&set_pose_to(far, next));
    alice.send(&ClientMessage::Escave { inside: true });
    let msgs = drive(&mut alice, &mut bob, 0.5);
    let (_, a_now, _, inside, _) = car(&msgs, alice_id);
    assert!(gap(a_now, (130.0, 90.0)) > reach, "Alice at {a_now:?}");
    assert!(!inside, "no escave in reach of Alice");
}

#[test]
fn test_a_car_in_an_escave_ignores_drive_input() {
    // Bob's car stands ~50u from this escave.
    let server = ServerProcess::start_with_args("test", &[], &["--escave", "130,90"]);
    let mut bob = server.connect();
    bob.send(&ClientMessage::Join {
        player_name: "Bob".into(),
        car_name: "TestCar".into(),
        color: 7,
        protocol: vangers_net::PROTOCOL_VERSION,
        guns: guns(),
    });
    let mut stash = Vec::new();
    let bob_id = match bob.recv_welcome(&mut stash) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };
    let level = vangers::level::load(
        &vangers::level::LevelConfig::new_test(),
        &Default::default(),
    );
    let pos = |msgs: &[ServerMessage]| {
        let (_, cars) = cars_in_world(msgs).expect("a WorldState");
        let c = *cars.iter().find(|c| c.0 == bob_id).expect("Bob");
        (c.1, c.3)
    };
    let gap = |a: [f32; 3], b: [f32; 3]| {
        (
            level
                .shortest_xy(glam::Vec3::from(a), glam::Vec3::from(b))
                .length(),
            (a[2] - b[2]).abs(),
        )
    };
    let floor_it = vangers_net::NetControl {
        motor: 1.0,
        rudder: 0.5,
        roll: 0.0,
        brake: false,
        turbo: true,
        jump: None,
    };
    let rate = quanta_per_sec();
    let drive = |bob: &mut Client, secs: f32, control: &vangers_net::NetControl| {
        let mut msgs = Vec::new();
        for i in 0..(secs * 10.0) as u32 {
            let control = vangers_net::NetControl {
                // A jump now and then, too.
                jump: (i % 5 == 4).then_some(1.0),
                ..control.clone()
            };
            bob.input((rate * 0.1).ceil() as u16, control);
            msgs.extend(bob.recv_for(Duration::from_millis(100)));
        }
        msgs
    };

    // Settle on the ground first.
    let parked = pos(&bob.recv_for(Duration::from_secs(2))).0;

    bob.send(&ClientMessage::Escave { inside: true });
    let msgs = drive(&mut bob, 2.0, &floor_it);
    let (inside_at, inside) = pos(&msgs);
    assert!(inside, "Bob is in the escave");
    let (xy, z) = gap(parked, inside_at);
    assert!(
        xy < 0.5 && z < 0.5,
        "drive input moved a car in an escave: {parked:?} -> {inside_at:?}"
    );
    // Nor does a Reset move it.
    bob.send(&ClientMessage::Reset {
        position: [parked[0] + 10.0, parked[1], parked[2]],
        next_quant: bob.next_quant,
    });
    let (still, _) = pos(&bob.recv_for(Duration::from_millis(500)));
    assert!(
        gap(parked, still).0 < 0.5,
        "a Reset moved a car in an escave: {parked:?} -> {still:?}"
    );

    // Out again, the same input drives it.
    bob.send(&ClientMessage::Escave { inside: false });
    let msgs = drive(&mut bob, 2.0, &floor_it);
    let (out_at, inside) = pos(&msgs);
    assert!(!inside);
    assert!(
        gap(parked, out_at).0 > 5.0,
        "out of the escave, the car drives: {parked:?} -> {out_at:?}"
    );
}

/// Each player's `(guns, armor)` in the newest `WorldState`.
fn guns_in_world(msgs: &[ServerMessage]) -> Vec<(u32, [Option<u8>; vangers_net::BAYS], u32)> {
    msgs.iter()
        .rev()
        .find_map(|m| match m {
            ServerMessage::WorldState { agents, .. } => Some(
                agents
                    .iter()
                    .map(|a| (a.player_id, a.guns, a.armor))
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

fn named_guns(names: [Option<&str>; vangers_net::BAYS]) -> vangers_net::NetGuns {
    names.map(|g| g.map(str::to_string))
}

#[test]
fn test_guns_are_checked_on_join_and_changed_only_in_an_escave() {
    // One escave ~50u from where Bob's car stands.
    let server = ServerProcess::start_with_args("test", &[], &["--escave", "130,90"]);
    let mut alice = server.connect();
    let mut bob = server.connect();
    let join = |client: &mut Client, name: &str, car: &str, guns| {
        client.send(&ClientMessage::Join {
            player_name: name.into(),
            car_name: car.into(),
            color: 7,
            protocol: vangers_net::PROTOCOL_VERSION,
            guns,
        });
        let mut stash = Vec::new();
        match client.recv_welcome(&mut stash) {
            ServerMessage::Welcome { player_id, .. } => player_id,
            _ => unreachable!(),
        }
    };
    // `TestCar` takes any gun in any bay. OxidizeMonk (`car.prm`) has two
    // bays of size 3: a Speetle (size 5) does not fit, so the whole set is
    // refused and Bob has no guns.
    let alice_id = join(&mut alice, "Alice", "TestCar", guns());
    let bob_id = join(
        &mut bob,
        "Bob",
        "OxidizeMonk",
        named_guns([Some("LightLaser"), None, None]),
    );
    let bob_guns = |client: &mut Client, wait| {
        let world = guns_in_world(&client.recv_for(Duration::from_millis(wait)));
        let find = |id| world.iter().find(|a| a.0 == id).expect("both cars").1;
        (find(alice_id), find(bob_id))
    };
    let (a_guns, b_guns) = bob_guns(&mut alice, 1000);
    assert_eq!(a_guns, [Some(11), Some(15), Some(16)], "Machotine, Ghorbs");
    assert_eq!(b_guns, [None; 3], "a Speetle does not fit OxidizeMonk");

    // Out on the road, a new set is refused.
    let fits = named_guns([Some("LightMissile"), Some("LightFireBall"), None]);
    bob.send(&ClientMessage::Weapons { guns: fits.clone() });
    assert_eq!(bob_guns(&mut alice, 400).1, [None; 3], "only in an escave");

    // In the escave it is taken.
    bob.send(&ClientMessage::Escave { inside: true });
    let _ = alice.recv_for(Duration::from_millis(300));
    bob.send(&ClientMessage::Weapons { guns: fits });
    let taken = [Some(11), Some(15), None];
    assert_eq!(bob_guns(&mut alice, 400).1, taken, "taken in an escave");

    // A set that does not fit keeps the last one: a heavy Ghorb (size 7)
    // in a size 3 bay, or a gun in the bay OxidizeMonk does not have.
    for bad in [
        [Some("LightMissile"), Some("HeavyFireBall"), None],
        [None, None, Some("LightMissile")],
        [Some("NoSuchGun"), None, None],
    ] {
        bob.send(&ClientMessage::Weapons {
            guns: named_guns(bad),
        });
        assert_eq!(
            bob_guns(&mut alice, 400).1,
            taken,
            "{bad:?} keeps the last set"
        );
    }
}

#[test]
fn test_each_gun_hits_for_its_own_damage_once_a_reload() {
    let server = ServerProcess::start();
    let mut alice = server.connect();
    let mut bob = server.connect();
    // No gun in bay 0, a light Ghorb in bay 1, a heavy one in bay 2.
    alice.send(&ClientMessage::Join {
        player_name: "Alice".into(),
        car_name: "TestCar".into(),
        color: 21,
        protocol: vangers_net::PROTOCOL_VERSION,
        guns: named_guns([None, Some(GUNS[1]), Some(GUNS[2])]),
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
        guns: guns(),
    });
    let mut bob_stash = Vec::new();
    let bob_id = match bob.recv_welcome(&mut bob_stash) {
        ServerMessage::Welcome { player_id, .. } => player_id,
        _ => unreachable!(),
    };

    alice_stash.extend(alice.recv_for(Duration::from_secs(2)));
    let (tick, agents) = latest_world(&alice_stash).expect("Alice should get a WorldState");
    let find = |agents: &[(u32, [f32; 3], u32, u32)], id| {
        *agents
            .iter()
            .find(|a| a.0 == id)
            .expect("both cars in WorldState")
    };
    let (_, a_pos, _, _) = find(&agents, alice_id);
    let (_, b_pos, b_max, _) = find(&agents, bob_id);
    // From high over Alice's car down onto Bob's (as in
    // `test_shot_takes_armor_off_the_target_only`).
    let muzzle = [a_pos[0], a_pos[1], a_pos[2] + 120.0];
    let at_bob = [
        b_pos[0] - muzzle[0],
        b_pos[1] - muzzle[1],
        b_pos[2] - muzzle[2],
    ];
    let mut quant = 10;
    let mut fire = |alice: &mut Client, bay| {
        quant += 1;
        alice.send(&ClientMessage::Fire {
            quant,
            bay,
            origin: muzzle,
            dir: at_bob,
            view_tick: tick,
            view_ms: 0,
        });
    };
    let bob_armor = |client: &mut Client, wait| {
        let world = guns_in_world(&client.recv_for(Duration::from_millis(wait)));
        world.iter().find(|a| a.0 == bob_id).expect("Bob").2
    };

    // Bay 0 has no gun; bay 1 fires three times in 0.1 s and only the
    // first gets through its 0.55 s reload; bay 2 reloads on its own.
    fire(&mut alice, 0);
    for _ in 0..3 {
        fire(&mut alice, 1);
        std::thread::sleep(Duration::from_millis(50));
    }
    fire(&mut alice, 2);
    let shots = bob.recv_for(Duration::from_millis(1500));
    let relayed: Vec<_> = shots
        .iter()
        .filter_map(|m| match m {
            ServerMessage::Shot { bay, .. } => Some(*bay),
            _ => None,
        })
        .collect();
    assert_eq!(relayed, vec![1, 2], "one shot a gun and a reload");
    let after = guns_in_world(&shots)
        .iter()
        .find(|a| a.0 == bob_id)
        .expect("Bob")
        .2;
    let (light, heavy) = (bay_damage(1), bay_damage(2));
    assert_eq!((light, heavy), (1250, 2500), "Ghorbs' FirstPower");
    assert_eq!(
        after,
        b_max - light - heavy,
        "each gun takes its own damage"
    );

    // Reloaded: the light Ghorb hits again.
    fire(&mut alice, 1);
    assert_eq!(bob_armor(&mut bob, 1500), b_max - 2 * light - heavy);
}

fn join_as(client: &mut Client, name: &str, color: u8) -> u32 {
    client.send(&ClientMessage::Join {
        player_name: name.into(),
        car_name: "TestCar".into(),
        color,
        protocol: vangers_net::PROTOCOL_VERSION,
        guns: guns(),
    });
    // Back as soon as the Welcome is in, so other clients stay talking.
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        for msg in client.recv_for(Duration::from_millis(50)) {
            if let ServerMessage::Welcome { player_id, .. } = msg {
                return player_id;
            }
        }
    }
    panic!("{name}: no Welcome within 3 s");
}

/// Keep `client` talking (an idle `Input` every 100 ms) for `duration`,
/// collecting what it hears.
fn keep_talking(client: &mut Client, duration: Duration) -> Vec<ServerMessage> {
    let deadline = Instant::now() + duration;
    let mut msgs = Vec::new();
    while Instant::now() < deadline {
        client.input(1, vangers_net::NetControl::default());
        msgs.extend(client.recv_for(Duration::from_millis(100)));
    }
    msgs
}

/// Whether the server closes `client`'s connection within `duration`.
fn closed_within(client: &mut Client, duration: Duration) -> bool {
    let deadline = Instant::now() + duration;
    let mut tmp = [0u8; 4096];
    while Instant::now() < deadline {
        match client.stream.read(&mut tmp) {
            Ok(0) => return true,
            Ok(_) => {}
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => return true,
        }
    }
    false
}

fn left(msgs: &[ServerMessage]) -> Vec<u32> {
    msgs.iter()
        .filter_map(|m| match m {
            ServerMessage::PlayerLeft { player_id } => Some(*player_id),
            _ => None,
        })
        .collect()
}

#[test]
fn test_a_silent_client_times_out_but_not_in_an_escave() {
    // Bob (the second joiner) stands within reach of this escave.
    let server = ServerProcess::start_with_args(
        "test",
        &[],
        &["--escave", "130,90", "--silent-timeout-secs", "1.5"],
    );
    let mut alice = server.connect();
    let mut bob = server.connect();
    let mut carol = server.connect();
    let alice_id = join_as(&mut alice, "Alice", 21);
    let bob_id = join_as(&mut bob, "Bob", 7);
    bob.send(&ClientMessage::Escave { inside: true });
    let carol_id = join_as(&mut carol, "Carol", 3);

    // Alice keeps sending input; Bob (in the escave) and Carol go quiet.
    let msgs = keep_talking(&mut alice, Duration::from_secs(4));
    assert_eq!(left(&msgs), vec![carol_id], "only silent Carol is dropped");
    let (_, cars) = cars_in_world(&msgs).expect("a WorldState");
    let ids: Vec<u32> = cars.iter().map(|c| c.0).collect();
    assert!(ids.contains(&alice_id), "Alice talks: {ids:?}");
    assert!(
        cars.iter().any(|c| c.0 == bob_id && c.3),
        "Bob stays, in the escave: {ids:?}"
    );
    assert!(!ids.contains(&carol_id), "Carol is gone: {ids:?}");
    assert!(
        closed_within(&mut carol, Duration::from_secs(1)),
        "the server closes Carol's connection"
    );

    // Carol's client comes back under the same name: her id, not a new one.
    let mut carol2 = server.connect();
    assert_eq!(join_as(&mut carol2, "Carol", 3), carol_id);
    let msgs = keep_talking(&mut carol2, Duration::from_millis(500));
    let (_, cars) = cars_in_world(&msgs).expect("a WorldState");
    assert_eq!(
        cars.iter().filter(|c| c.0 == carol_id).count(),
        1,
        "one Carol"
    );
}

#[test]
fn test_a_same_name_rejoin_replaces_the_live_session() {
    let server = ServerProcess::start();
    let mut alice = server.connect();
    let mut bob = server.connect();
    let alice_id = join_as(&mut alice, "Alice", 21);
    let bob_id = join_as(&mut bob, "Bob", 7);
    bob.recv_for(Duration::from_millis(300));

    // Alice's client restarts while the server still holds her old
    // connection open: the new one takes her id, and there is one Alice.
    let mut alice2 = server.connect();
    assert_eq!(
        join_as(&mut alice2, "Alice", 21),
        alice_id,
        "the rejoin takes over Alice's id"
    );
    let msgs = keep_talking(&mut bob, Duration::from_millis(800));
    assert_eq!(left(&msgs), vec![alice_id], "the stale session leaves once");
    let (_, cars) = cars_in_world(&msgs).expect("a WorldState");
    let mut ids: Vec<u32> = cars.iter().map(|c| c.0).collect();
    ids.sort();
    let mut want = vec![alice_id, bob_id];
    want.sort();
    assert_eq!(ids, want, "no duplicate Alice");
    assert!(
        closed_within(&mut alice, Duration::from_secs(1)),
        "the server closes the stale connection"
    );

    // The old connection's late input and its close must not touch the
    // new session, which now owns the same id.
    alice.send(&ClientMessage::Leave);
    drop(alice);
    let msgs = keep_talking(&mut bob, Duration::from_millis(800));
    alice2.recv_for(Duration::from_millis(10));
    assert!(
        left(&msgs).is_empty(),
        "the new Alice stays: {:?}",
        left(&msgs)
    );
    let (_, cars) = cars_in_world(&msgs).expect("a WorldState");
    assert!(
        cars.iter().any(|c| c.0 == alice_id),
        "Alice is still on the road"
    );
}
