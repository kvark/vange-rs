//! Headless integration test: simulate client + server physics loop
//! and verify the camera doesn't oscillate.
//!
//! The test runs both sides in-process with the same test level and
//! test physics constants. It feeds identical input, steps physics on
//! both sides, periodically "sends" a WorldState from server to client
//! (just copying the transform), and checks that the camera position
//! stays stable.

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
            dynamo, transform, max_quant, car, level, common, 1.0, 0.0, None, 0.0, None,
        );
        remaining -= max_quant;
    }
    physics::step(
        dynamo, transform, remaining, car, level, common, 1.0, 0.0, None, 0.0, None,
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
    let var = deltas.iter().map(|d| (d - mean).powi(2)).sum::<f32>() / deltas.len() as f32;
    var
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
