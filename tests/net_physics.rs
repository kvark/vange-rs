//! Headless integration test: simulate client + server physics loop
//! and verify the camera doesn't oscillate.
//!
//! The test runs both sides in-process with the same test level and
//! test physics constants. It feeds identical input, steps physics on
//! both sides, periodically "sends" a WorldState from server to client
//! (just copying the transform), and checks that the camera position
//! stays stable.
//!
//! ## Extending for story-cycle sync
//!
//! `WorldState` now carries an optional `CycleState` (banks, light, fade,
//! per-player cirtainer). A follow-up test can:
//! 1. Build a tiny `level::cycle::Bunch` (see `src/level/cycle.rs` tests).
//! 2. Step gather/deliver/quant on the server side each 20 Hz tick.
//! 3. Apply `Bunch::sync_authority` on the client from the snapshot.
//! 4. Assert matching `current`, `banked`, and `light` after a delivery.

use glam::{Quat, Vec3};
use vangers::{
    config::{self, settings},
    level,
    physics::{self, CarPhysicsData, Dynamo},
    space,
};

/// Minimal client-side state: transform, dynamo, camera.
struct ClientState {
    transform: space::Transform,
    dynamo: Dynamo,
    cam: space::Camera,
    follow: space::Follow,
}

/// Minimal server-side state: transform, dynamo.
struct ServerState {
    transform: space::Transform,
    dynamo: Dynamo,
}

fn make_follow() -> space::Follow {
    // Matches the default Follow camera from settings
    space::Follow {
        angle_x: (60.0f32).to_radians() - std::f32::consts::FRAC_PI_2,
        offset: Vec3::new(0.0, -100.0, 60.0),
        speed: 4.0,
        look_ahead: 0.0,
    }
}

fn make_camera(transform: &space::Transform) -> space::Camera {
    let mut cam = space::Camera {
        loc: transform.disp + Vec3::new(0.0, 0.0, 200.0),
        rot: Quat::IDENTITY,
        scale: Vec3::new(1.0, -1.0, 1.0),
        proj: space::Projection::Perspective(space::PerspectiveParams {
            fovy: 45.0f32.to_radians(),
            aspect: 4.0 / 3.0,
            near: 10.0,
            far: 2000.0,
            focal_px: None,
        }),
    };
    // Warm up the camera follow so it starts at rest
    let follow = make_follow();
    for _ in 0..100 {
        cam.follow(transform, 1.0 / 60.0, &follow);
    }
    cam
}

fn spawn_transform(level: &level::Level) -> space::Transform {
    let x = level.size.0 / 4;
    let y = level.size.1 / 4;
    let height = level.get((x, y)).high() + 5.0;
    space::Transform {
        disp: Vec3::new(x as f32, y as f32, height),
        rot: Quat::from_rotation_z(std::f32::consts::PI),
        scale: 1.0,
    }
}

fn step_physics(
    dynamo: &mut Dynamo,
    transform: &mut space::Transform,
    physics_dt: f32,
    max_quant: f32,
    car: &CarPhysicsData,
    level: &level::Level,
    common: &config::common::Common,
) {
    let mut remaining = physics_dt;
    while remaining > max_quant {
        physics::step(
            dynamo, transform, max_quant, car, level, common, 1.0, 0.0, None, 0.0, None, None,
        );
        remaining -= max_quant;
    }
    physics::step(
        dynamo, transform, remaining, car, level, common, 1.0, 0.0, None, 0.0, None, None,
    );
}

/// Run the simulation and return per-frame camera positions and
/// per-frame divergence from the server.
/// `camera_after_snap` controls when the camera reads the transform.
/// `apply_snap` controls whether server WorldState corrections are applied.
fn simulate_full(camera_after_snap: bool, apply_snap: bool) -> (Vec<Vec3>, Vec<f32>) {
    let level_config = level::LevelConfig::new_test();
    let geometry = settings::Geometry::default();
    let level = level::load(&level_config, &geometry);
    let common = config::common::Common::test_default();
    let car = CarPhysicsData::test_default();
    let max_quant = 0.02f32;

    let spawn = spawn_transform(&level);

    let mut server = ServerState {
        transform: spawn,
        dynamo: Dynamo::default(),
    };
    let mut client = ClientState {
        transform: spawn,
        dynamo: Dynamo::default(),
        cam: make_camera(&spawn),
        follow: make_follow(),
    };

    // Server ticks at 20 Hz, client at 60 Hz
    let server_dt = 1.0 / 20.0;
    let client_dt = 1.0 / 60.0;

    let server_physics_dt = server_dt * {
        let n = &common.nature;
        let fps = common.speed.standard_frame_rate as f32;
        fps * n.time_delta0 * n.num_calls_analysis as f32
    };
    let client_physics_dt = client_dt * {
        let n = &common.nature;
        let fps = common.speed.standard_frame_rate as f32;
        fps * n.time_delta0 * n.num_calls_analysis as f32
    };

    let mut cam_positions = Vec::new();
    let mut divergences = Vec::new();
    let mut server_tick_accum = 0.0f32;

    let input_factor_server = server_dt / config::common::MAIN_LOOP_TIME;
    let input_factor_client = client_dt / config::common::MAIN_LOOP_TIME;

    // Run for 3 seconds of game time
    let total_frames = (3.0 / client_dt) as usize;
    for _ in 0..total_frames {
        // --- Client applies control + physics each frame ---
        client
            .dynamo
            .change_traction(1.0 * input_factor_client * common.car.traction_incr);
        step_physics(
            &mut client.dynamo,
            &mut client.transform,
            client_physics_dt,
            max_quant,
            &car,
            &level,
            &common,
        );

        // --- Server tick (every 50ms) ---
        server_tick_accum += client_dt;
        let have_server_update = server_tick_accum >= server_dt;
        if have_server_update {
            server_tick_accum -= server_dt;

            // Server applies control + physics at its own rate
            server
                .dynamo
                .change_traction(1.0 * input_factor_server * common.car.traction_incr);
            step_physics(
                &mut server.dynamo,
                &mut server.transform,
                server_physics_dt,
                max_quant,
                &car,
                &level,
                &common,
            );
        }

        if camera_after_snap {
            // --- Network snap (before camera) ---
            if have_server_update && apply_snap {
                client.transform = server.transform;
                client.dynamo.linear_velocity = server.dynamo.linear_velocity;
                client.dynamo.angular_velocity = server.dynamo.angular_velocity;
                client.dynamo.traction = server.dynamo.traction;
                client.dynamo.rudder = server.dynamo.rudder;
            }

            // --- Camera follow ---
            client
                .cam
                .follow(&client.transform, client_dt, &client.follow);
        } else {
            // --- Camera follow (before snap — the old broken order) ---
            client
                .cam
                .follow(&client.transform, client_dt, &client.follow);

            // --- Network snap ---
            if have_server_update && apply_snap {
                client.transform = server.transform;
                client.dynamo.linear_velocity = server.dynamo.linear_velocity;
                client.dynamo.angular_velocity = server.dynamo.angular_velocity;
                client.dynamo.traction = server.dynamo.traction;
                client.dynamo.rudder = server.dynamo.rudder;
            }
        }

        divergences.push(client.transform.disp.distance(server.transform.disp));
        cam_positions.push(client.cam.loc);
    }

    (cam_positions, divergences)
}

fn simulate(camera_after_snap: bool) -> Vec<Vec3> {
    simulate_full(camera_after_snap, true).0
}

/// Compute the max frame-to-frame change in camera position.
fn max_frame_delta(positions: &[Vec3]) -> f32 {
    positions
        .windows(2)
        .map(|w| w[1].distance(w[0]))
        .fold(0.0f32, f32::max)
}

/// Compute the variance of frame-to-frame deltas (jitter metric).
fn delta_variance(positions: &[Vec3]) -> f32 {
    let deltas: Vec<f32> = positions.windows(2).map(|w| w[1].distance(w[0])).collect();
    if deltas.len() < 2 {
        return 0.0;
    }
    let mean = deltas.iter().sum::<f32>() / deltas.len() as f32;
    deltas.iter().map(|d| (d - mean).powi(2)).sum::<f32>() / deltas.len() as f32
}

#[test]
fn test_camera_stability_with_snap_before_camera() {
    let positions = simulate(true);
    let variance = delta_variance(&positions);
    let max_delta = max_frame_delta(&positions);

    eprintln!(
        "camera_after_snap=true: max_delta={:.4}, variance={:.6}, frames={}",
        max_delta,
        variance,
        positions.len()
    );

    // With correct ordering (snap before camera), variance should be very low
    assert!(
        variance < 1.0,
        "Camera jitter variance too high with correct ordering: {:.4}",
        variance
    );
}

#[test]
fn test_no_snap_baseline() {
    // No server corrections at all — pure local physics. Camera should be smooth.
    let (positions, _) = simulate_full(true, false);
    let variance = delta_variance(&positions);
    let max_delta = max_frame_delta(&positions);
    eprintln!(
        "No snap (baseline):  max_delta={:.4}, variance={:.6}",
        max_delta, variance
    );
}

#[test]
fn test_snap_oscillation_pattern() {
    // Print the actual frame deltas around server snap frames to see the pattern
    let (positions, divergences) = simulate_full(false, true); // broken order
    let deltas: Vec<f32> = positions.windows(2).map(|w| w[1].distance(w[0])).collect();

    // Find frames where divergence drops (snap happened) and print surrounding deltas
    eprintln!("\n=== Broken order: camera BEFORE snap ===");
    eprintln!("Frame | CamDelta  | Divergence");
    for i in 1..divergences.len().min(60) {
        let snap = divergences[i] < divergences[i - 1] * 0.5 && divergences[i - 1] > 0.001;
        eprintln!(
            "  {:3} | {:8.4}  | {:8.4} {}",
            i,
            deltas.get(i).copied().unwrap_or(0.0),
            divergences[i],
            if snap { " <-- SNAP" } else { "" }
        );
    }

    let (positions2, divergences2) = simulate_full(true, true); // fixed order
    let deltas2: Vec<f32> = positions2.windows(2).map(|w| w[1].distance(w[0])).collect();
    eprintln!("\n=== Fixed order: camera AFTER snap ===");
    eprintln!("Frame | CamDelta  | Divergence");
    for i in 1..divergences2.len().min(60) {
        let snap = divergences2[i] < divergences2[i - 1] * 0.5 && divergences2[i - 1] > 0.001;
        eprintln!(
            "  {:3} | {:8.4}  | {:8.4} {}",
            i,
            deltas2.get(i).copied().unwrap_or(0.0),
            divergences2[i],
            if snap { " <-- SNAP" } else { "" }
        );
    }
}

#[test]
fn test_camera_stability_comparison() {
    let positions_fixed = simulate(true);
    let positions_broken = simulate(false);

    let var_fixed = delta_variance(&positions_fixed);
    let var_broken = delta_variance(&positions_broken);
    let max_fixed = max_frame_delta(&positions_fixed);
    let max_broken = max_frame_delta(&positions_broken);

    eprintln!(
        "Fixed ordering:  max_delta={:.4}, variance={:.6}",
        max_fixed, var_fixed
    );
    eprintln!(
        "Broken ordering: max_delta={:.4}, variance={:.6}",
        max_broken, var_broken
    );

    // The fixed ordering should have less or equal jitter
    // (If physics is perfectly deterministic, both may be similar,
    //  but the broken ordering will show oscillation when they diverge)
}

#[test]
fn test_different_spawn_positions() {
    // Simulate what actually happens: client and server start at different positions.
    // This is the real game scenario — client spawns at escave coords,
    // server spawns at find_spawn_point coords.
    let level_config = level::LevelConfig::new_test();
    let geometry = settings::Geometry::default();
    let level = level::load(&level_config, &geometry);
    let common = config::common::Common::test_default();
    let car = CarPhysicsData::test_default();
    let max_quant = 0.02f32;

    // Server spawn (find_spawn_point style)
    let server_spawn = spawn_transform(&level);
    // Client spawn: different location entirely
    let client_spawn = {
        let x = level.size.0 / 2;
        let y = level.size.1 / 2;
        let height = level.get((x, y)).high() + 5.0;
        space::Transform {
            disp: Vec3::new(x as f32, y as f32, height),
            rot: Quat::from_rotation_z(std::f32::consts::PI),
            scale: 1.0,
        }
    };

    let mut server_transform = server_spawn;
    let mut server_dynamo = Dynamo::default();
    let mut client_transform = client_spawn;
    let mut client_dynamo = Dynamo::default();
    let mut cam = make_camera(&client_spawn);
    let follow = make_follow();

    let server_dt = 1.0 / 20.0;
    let client_dt = 1.0 / 60.0;

    let server_physics_dt = server_dt * {
        let n = &common.nature;
        let fps = common.speed.standard_frame_rate as f32;
        fps * n.time_delta0 * n.num_calls_analysis as f32
    };
    let client_physics_dt = client_dt * {
        let n = &common.nature;
        let fps = common.speed.standard_frame_rate as f32;
        fps * n.time_delta0 * n.num_calls_analysis as f32
    };

    eprintln!("\nServer spawn: {:?}", server_spawn.disp);
    eprintln!("Client spawn: {:?}", client_spawn.disp);
    eprintln!(
        "Initial distance: {:.1}",
        server_spawn.disp.distance(client_spawn.disp)
    );

    let mut server_tick_accum = 0.0f32;
    let input_factor_server = server_dt / config::common::MAIN_LOOP_TIME;
    let input_factor_client = client_dt / config::common::MAIN_LOOP_TIME;

    // Simulate Welcome arriving after ~200ms (4 server ticks)
    let welcome_frame = (0.2 / client_dt) as usize;

    eprintln!("\nFrame | CamPos.y    | PlayerPos.y | CamDelta");
    let mut prev_cam = cam.loc;

    for frame in 0..(3.0 / client_dt) as usize {
        // Client physics
        client_dynamo.change_traction(1.0 * input_factor_client * common.car.traction_incr);
        step_physics(
            &mut client_dynamo,
            &mut client_transform,
            client_physics_dt,
            max_quant,
            &car,
            &level,
            &common,
        );

        // Server physics
        server_tick_accum += client_dt;
        let have_server_update = server_tick_accum >= server_dt;
        if have_server_update {
            server_tick_accum -= server_dt;
            server_dynamo.change_traction(1.0 * input_factor_server * common.car.traction_incr);
            step_physics(
                &mut server_dynamo,
                &mut server_transform,
                server_physics_dt,
                max_quant,
                &car,
                &level,
                &common,
            );
        }

        // Snap on server ticks, but only after Welcome
        if have_server_update && frame >= welcome_frame {
            client_transform = server_transform;
            client_dynamo.linear_velocity = server_dynamo.linear_velocity;
            client_dynamo.angular_velocity = server_dynamo.angular_velocity;
            client_dynamo.traction = server_dynamo.traction;
            client_dynamo.rudder = server_dynamo.rudder;
        }

        cam.follow(&client_transform, client_dt, &follow);
        let cam_delta = cam.loc.distance(prev_cam);
        prev_cam = cam.loc;

        if frame < 30 || (frame >= welcome_frame - 2 && frame <= welcome_frame + 15) {
            eprintln!(
                "  {:3} | {:10.3}  | {:10.3}  | {:8.4}{}",
                frame,
                cam.loc.y,
                client_transform.disp.y,
                cam_delta,
                if frame == welcome_frame {
                    "  <-- FIRST SNAP"
                } else {
                    ""
                },
            );
        }
    }
}

#[test]
fn test_server_client_physics_divergence() {
    // Measure how much the server and client transforms diverge
    // when running identical physics with different tick rates.
    let level_config = level::LevelConfig::new_test();
    let geometry = settings::Geometry::default();
    let level = level::load(&level_config, &geometry);
    let common = config::common::Common::test_default();
    let car = CarPhysicsData::test_default();
    let max_quant = 0.02f32;

    let spawn = spawn_transform(&level);

    let mut server_transform = spawn;
    let mut server_dynamo = Dynamo::default();
    let mut client_transform = spawn;
    let mut client_dynamo = Dynamo::default();

    let server_dt = 1.0 / 20.0;
    let client_dt = 1.0 / 60.0;

    let server_physics_dt = server_dt * {
        let n = &common.nature;
        let fps = common.speed.standard_frame_rate as f32;
        fps * n.time_delta0 * n.num_calls_analysis as f32
    };
    let client_physics_dt = client_dt * {
        let n = &common.nature;
        let fps = common.speed.standard_frame_rate as f32;
        fps * n.time_delta0 * n.num_calls_analysis as f32
    };

    // Apply same traction
    server_dynamo.change_traction(1.0 * server_dt * common.car.traction_incr);
    client_dynamo.change_traction(1.0 * client_dt * common.car.traction_incr);

    let mut max_divergence = 0.0f32;
    let mut server_tick_accum = 0.0f32;

    // Run for 2 seconds
    for _ in 0..(2.0 / client_dt) as usize {
        // Client physics every frame
        step_physics(
            &mut client_dynamo,
            &mut client_transform,
            client_physics_dt,
            max_quant,
            &car,
            &level,
            &common,
        );

        // Server physics at 20 Hz
        server_tick_accum += client_dt;
        if server_tick_accum >= server_dt {
            server_tick_accum -= server_dt;
            step_physics(
                &mut server_dynamo,
                &mut server_transform,
                server_physics_dt,
                max_quant,
                &car,
                &level,
                &common,
            );

            let divergence = server_transform.disp.distance(client_transform.disp);
            max_divergence = max_divergence.max(divergence);
        }
    }

    eprintln!(
        "Max server/client position divergence: {:.4}",
        max_divergence
    );
    eprintln!("Server final pos: {:?}", server_transform.disp);
    eprintln!("Client final pos: {:?}", client_transform.disp);
}

/// Drive straight for `seconds` at the given frame rate, returning how far
/// the car travelled. `max_quant` of `f32::INFINITY` disables sub-stepping,
/// which is what the visibility gate in the game loop effectively does when
/// it decides an agent is off screen.
fn distance_travelled(frame_dt: f32, seconds: f32, max_quant: f32) -> f32 {
    let level_config = level::LevelConfig::new_test();
    let level = level::load(&level_config, &settings::Geometry::default());
    let common = config::common::Common::test_default();
    let car = CarPhysicsData::test_default();

    let spawn = spawn_transform(&level);
    let mut transform = spawn;
    let mut dynamo = Dynamo::default();

    let physics_dt = frame_dt * {
        let n = &common.nature;
        common.speed.standard_frame_rate as f32 * n.time_delta0 * n.num_calls_analysis as f32
    };
    let input_factor = frame_dt / config::common::MAIN_LOOP_TIME;

    for _ in 0..(seconds / frame_dt) as usize {
        dynamo.change_traction(input_factor * common.car.traction_incr);
        step_physics(
            &mut dynamo,
            &mut transform,
            physics_dt,
            max_quant,
            &car,
            &level,
            &common,
        );
    }
    transform.disp.truncate().distance(spawn.disp.truncate())
}

/// `physics::step` is not linear in `dt`: it scales impulses by
/// `dt / time_delta0` and applies drag and collision response once per
/// call, both of which saturate. Folding a frame into one large step
/// therefore travels a different distance than splitting it, and the gap
/// grows with the step size.
///
/// This matters because the step size is not a constant. `physics_dt` is
/// about nine times the frame time with the stock constants, so it tracks
/// the frame rate, and the game loop additionally skips the split entirely
/// for agents it decides is off screen. Both make how far the car goes depend
/// on something that is not the car.
#[test]
fn one_big_step_does_not_match_sub_stepping() {
    const QUANT: f32 = 0.01;
    let mut worst = 0.0f32;
    for &fps in &[60.0f32, 30.0, 20.0] {
        let split = distance_travelled(1.0 / fps, 3.0, QUANT);
        let single = distance_travelled(1.0 / fps, 3.0, f32::INFINITY);
        let rel = (split - single).abs() / split.max(1e-3);
        println!(
            "{fps:>4} fps: sub-stepped {split:8.2}, one step {single:8.2} ({:+.1}%)",
            100.0 * (single - split) / split.max(1e-3)
        );
        worst = worst.max(rel);
    }
    assert!(
        worst > 0.01,
        "expected the two integrations to diverge; if this ever stops being \
         true the sub-stepping is dead code and the gate can go"
    );
}

/// Run 20 Hz server ticks from `transform`, holding W and jumping once at
/// tick 5, stepping whole `max_quant` quanta with per-quantum controls and
/// the jump on the tick's last quantum like `vangers-server` does. Returns
/// how far the car got.
fn jump_and_drive(
    mut transform: space::Transform,
    max_quant: f32,
    car: &CarPhysicsData,
    level: &level::Level,
    common: &config::common::Common,
) -> f32 {
    let dt_fixed = 1.0 / 20.0;
    let physics_dt = dt_fixed * {
        let n = &common.nature;
        common.speed.standard_frame_rate as f32 * n.time_delta0 * n.num_calls_analysis as f32
    };
    let quant_input = physics::QuantClock::input_factor(max_quant, common);
    let mut clock = physics::QuantClock::default();
    let start = transform.disp;
    let mut dynamo = Dynamo::default();
    for tick in 0..60 {
        let quanta = clock.advance(physics_dt, max_quant);
        for i in 0..quanta {
            dynamo.change_traction(quant_input * common.car.traction_incr);
            let jump = (tick == 5 && i + 1 == quanta).then_some(1.0);
            physics::step(
                &mut dynamo,
                &mut transform,
                max_quant,
                car,
                level,
                common,
                1.0,
                0.0,
                jump,
                0.0,
                None,
                None,
            );
        }
    }
    transform.disp.distance(start)
}

/// MP freeze repro (needs game data; skipped without it). Alice's server
/// pose after she "ignored every key" on Fostral: tipped ~50° on its side
/// against a tree cluster (`(rot * X).z = 0.76`, past the 0.7 wheels
/// limit), at rest, so W/S/A/D have no traction. Jump + W is the way out
/// and works at the clients' `max_quant`, but the server's old hard-coded
/// 0.02 ran ~4x as many contact passes per tick and swallowed the jump;
/// the WorldState reconcile then pinned the client's car to that pose.
#[test]
fn wedged_car_escapes_at_client_max_quant() {
    let Some(settings) = config::Settings::try_load("config/settings.ron") else {
        eprintln!("skipping: no config/settings.ron with game data");
        return;
    };
    let ini = settings.data_path.join("thechain/fostral/world.ini");
    if !ini.is_file() {
        eprintln!("skipping: no Fostral data");
        return;
    }
    let reg = config::game::Registry::load(&settings);
    let level = level::load(&level::LevelConfig::load(&ini), &settings.game.geometry);
    let common = config::common::load(settings.open_relative("common.prm"));
    let cars = config::car::load_physics_registry(
        &settings.data_path,
        &reg,
        settings.game.physics.shape_sampling,
    );
    let Some(car) = cars.get("OxidizeMonk") else {
        eprintln!("skipping: no OxidizeMonk");
        return;
    };
    let wedged = space::Transform {
        disp: Vec3::new(432.90, 3925.99, 61.04),
        rot: Quat::from_xyzw(-0.227, -0.368, -0.620, 0.655).normalize(),
        scale: car.scale,
    };
    let client_quant = settings.game.physics.max_quant;
    let at_client = jump_and_drive(wedged, client_quant, car, &level, &common);
    let at_old_server = jump_and_drive(wedged, 0.02, car, &level, &common);
    println!(
        "jump+W from wedge: max_quant {client_quant} -> {at_client:.2}, 0.02 -> {at_old_server:.2}"
    );
    assert!(
        at_old_server < 5.0,
        "old server quant was expected to stay pinned"
    );
    assert!(
        at_client > 20.0,
        "car should jump free at the client max_quant"
    );
}

/// How a side cuts its frame's physics time into `physics::step` calls.
#[derive(Clone, Copy, PartialEq)]
enum Split {
    /// Pre-fix: `max_quant` steps plus the remainder, jump on the remainder.
    PerFrame,
    /// `physics::QuantClock`: whole quanta only, jump on the last quantum.
    Quanta,
}

/// Hold W for `seconds`, releasing a jump of `power` on
/// the frame that ends at `jump_at`. One frame is a client render frame or,
/// at `frame_dt = 0.05`, a 20 Hz server tick. Returns the pose at the end
/// of every frame, keyed by the frame's end time in milliseconds.
#[allow(clippy::too_many_arguments)]
fn jump_trajectory(
    start: space::Transform,
    frame_dt: f32,
    seconds: f32,
    jump_at: f32,
    power: f32,
    split: Split,
    max_quant: f32,
    car: &CarPhysicsData,
    level: &level::Level,
    common: &config::common::Common,
) -> Vec<(u32, Vec3)> {
    let physics_dt = frame_dt * {
        let n = &common.nature;
        common.speed.standard_frame_rate as f32 * n.time_delta0 * n.num_calls_analysis as f32
    };
    let input_factor = frame_dt / config::common::MAIN_LOOP_TIME;
    let quant_input = physics::QuantClock::input_factor(max_quant, common);
    let frames = (seconds / frame_dt).round() as u32;
    let jump_frame = (jump_at / frame_dt).round() as u32 - 1;
    let mut transform = start;
    let mut dynamo = Dynamo::default();
    let mut clock = physics::QuantClock::default();
    let mut out = Vec::new();
    for frame in 0..frames {
        let jump = (frame == jump_frame).then_some(power);
        let step = |dynamo: &mut Dynamo, transform: &mut space::Transform, dt, jump| {
            physics::step(
                dynamo, transform, dt, car, level, common, 1.0, 0.0, jump, 0.0, None, None,
            )
        };
        match split {
            Split::PerFrame => {
                dynamo.change_traction(input_factor * common.car.traction_incr);
                let mut remaining = physics_dt;
                while remaining > max_quant {
                    step(&mut dynamo, &mut transform, max_quant, None);
                    remaining -= max_quant;
                }
                step(&mut dynamo, &mut transform, remaining, jump);
            }
            Split::Quanta => {
                let quanta = clock.advance(physics_dt, max_quant);
                assert!(quanta > 0, "test frames are longer than a quantum");
                for i in 0..quanta {
                    let last = i + 1 == quanta;
                    dynamo.change_traction(quant_input * common.car.traction_incr);
                    step(
                        &mut dynamo,
                        &mut transform,
                        max_quant,
                        if last { jump } else { None },
                    );
                }
            }
        }
        let end_ms = ((frame + 1) as f32 * frame_dt * 1000.0).round() as u32;
        out.push((end_ms, transform.disp));
    }
    out
}

/// Largest gap between a client and the 20 Hz server at the instants both
/// have a pose for (frame ends that land on tick ends).
fn worst_gap(server: &[(u32, Vec3)], client: &[(u32, Vec3)]) -> f32 {
    let mut worst = 0.0f32;
    for &(ms, pos) in client {
        if let Some(&(_, srv)) = server.iter().find(|&&(t, _)| t == ms) {
            worst = worst.max(pos.distance(srv));
        }
    }
    worst
}

/// Frame times whose frame ends line up with 20 Hz tick ends at 1 s.
const JUMP_FRAME_DTS: [f32; 5] = [1.0 / 60.0, 1.0 / 40.0, 0.1, 0.125, 0.2];

/// With `QuantClock` a client at any frame rate integrates a moving jump
/// along exactly the server's path, so the reconcile has nothing to snap.
#[test]
fn quantized_jump_matches_server_at_any_frame_rate() {
    let level = level::load(
        &level::LevelConfig::new_test(),
        &settings::Geometry::default(),
    );
    let common = config::common::Common::test_default();
    let car = CarPhysicsData::test_default();
    let start = spawn_transform(&level);
    // The test constants make 0.56 physics time per second, not ~9. Keep
    // the quantum off a divisor of the tick so float rounding of the carry
    // cannot move a quantum boundary between the two sides.
    let max_quant = 0.0045;
    let run = |frame_dt, split| {
        jump_trajectory(
            start, frame_dt, 4.0, 1.0, 10.0, split, max_quant, &car, &level, &common,
        )
    };
    let server = run(0.05, Split::Quanta);
    let moved = server[0].1.distance(server.last().unwrap().1);
    assert!(moved > 10.0, "the car should drive and jump, moved {moved}");
    for frame_dt in JUMP_FRAME_DTS {
        let gap = worst_gap(&server, &run(frame_dt, Split::Quanta));
        println!(
            "test level, frame {:.0} ms: quantized gap {gap:.4}",
            frame_dt * 1000.0
        );
        assert!(
            gap < 0.01,
            "frame {frame_dt}: client strayed {gap} from the server"
        );
    }
}

/// MP jump snap repro on Fostral (needs game data; skipped without it).
/// Overseer's #388 playtest: Alice's moving jump landed ~127u from where
/// the server put her, and the `WorldState` reconcile hard-snapped her back.
/// Same pose, same input, same jump power: the only difference between the
/// client and the server was the frame time the physics was cut up by.
#[test]
fn fostral_moving_jump_lands_where_the_server_does() {
    let Some(settings) = config::Settings::try_load("config/settings.ron") else {
        eprintln!("skipping: no config/settings.ron with game data");
        return;
    };
    let ini = settings.data_path.join("thechain/fostral/world.ini");
    if !ini.is_file() {
        eprintln!("skipping: no Fostral data");
        return;
    }
    let reg = config::game::Registry::load(&settings);
    let level = level::load(&level::LevelConfig::load(&ini), &settings.game.geometry);
    let common = config::common::load(settings.open_relative("common.prm"));
    let cars = config::car::load_physics_registry(
        &settings.data_path,
        &reg,
        settings.game.physics.shape_sampling,
    );
    let Some(car) = cars.get("OxidizeMonk") else {
        eprintln!("skipping: no OxidizeMonk");
        return;
    };
    let max_quant = settings.game.physics.max_quant;
    // Settle at the spot of the playtest's standing jump 2 (830, 3983).
    let mut start = space::Transform {
        disp: Vec3::new(830.0, 3983.0, level.get((830, 3983)).high() + 5.0),
        rot: Quat::IDENTITY,
        scale: car.scale,
    };
    let mut rest = Dynamo::default();
    for _ in 0..100 {
        step_physics(&mut rest, &mut start, 0.45, max_quant, car, &level, &common);
    }
    let power = common.force.max_jump_power;
    let run = |frame_dt, split| {
        jump_trajectory(
            start, frame_dt, 4.0, 1.0, power, split, max_quant, car, &level, &common,
        )
    };
    let (mut worst_old, mut worst_new) = (0.0f32, 0.0f32);
    let old_server = run(0.05, Split::PerFrame);
    let server = run(0.05, Split::Quanta);
    for frame_dt in JUMP_FRAME_DTS {
        let old = worst_gap(&old_server, &run(frame_dt, Split::PerFrame));
        let new = worst_gap(&server, &run(frame_dt, Split::Quanta));
        println!(
            "Fostral, frame {:.0} ms: per-frame split gap {old:.2}, quantized gap {new:.4}",
            frame_dt * 1000.0
        );
        worst_old = worst_old.max(old);
        worst_new = worst_new.max(new);
    }
    assert!(
        worst_old > space::Transform::WORLD_STATE_SNAP_POS,
        "the per-frame split was expected to stray past the snap threshold"
    );
    assert!(worst_new < 0.01, "quantized client strayed {worst_new}");
}
