//! The chase camera through a max jump on Fostral (needs game data;
//! skipped without it). Replays the car with the client's quantum
//! stepping at 60 fps and drives the camera exactly as `road` does
//! (`follow`, `avoid_terrain`, look-ahead re-aim), checking how far the
//! view turns and the camera moves in any one frame.

use glam::{Quat, Vec3};
use vangers::{
    config, level,
    physics::{self, CarPhysicsData, Dynamo},
    space,
};

/// `CAMERA_CLEARANCE` of `road` and `web`.
const CLEARANCE: f32 = 4.0;

struct Worst {
    /// Largest per-frame turn of the view direction's yaw, radians.
    turn: f32,
    /// Largest per-frame turn of the chase heading, radians.
    heading: f32,
    /// The same for the raw flattened forward axis the camera used to take.
    raw_heading: f32,
    /// Largest per-frame camera move beyond the car's own move.
    lurch: f32,
    /// Lowest `up.z` of the car: below 0 it went over.
    min_up: f32,
}

fn wrap_angle(a: f32) -> f32 {
    let pi = std::f32::consts::PI;
    (a + pi).rem_euclid(2.0 * pi) - pi
}

fn view_yaw(cam: &space::Camera) -> f32 {
    let d = cam.dir();
    f32::atan2(-d.x, d.y)
}

/// Settle at `(x, y)` facing `yaw`, hold W for `drive` seconds, release a
/// max jump, and fly for 4 s more.
#[allow(clippy::too_many_arguments)]
fn jump_replay(
    level: &level::Level,
    common: &config::common::Common,
    car: &CarPhysicsData,
    max_quant: f32,
    (x, y, yaw): (f32, f32, f32),
    drive: f32,
    follow: &space::Follow,
) -> Worst {
    let frame_dt = 1.0 / 60.0;
    let physics_dt = frame_dt * {
        let n = &common.nature;
        common.speed.standard_frame_rate as f32 * n.time_delta0 * n.num_calls_analysis as f32
    };
    let quant_input = physics::QuantClock::input_factor(max_quant, common);
    let mut clock = physics::QuantClock::default();
    let mut dynamo = Dynamo::default();
    let mut transform = space::Transform {
        disp: Vec3::new(x, y, level.get((x as i32, y as i32)).high() + 5.0),
        rot: Quat::from_rotation_z(yaw),
        scale: car.scale,
    };
    let mut cam = space::Camera {
        loc: transform.disp + Vec3::new(0.0, 0.0, 200.0),
        rot: Quat::IDENTITY,
        scale: Vec3::new(-1.0, 1.0, 1.0),
        proj: space::Projection::Perspective(space::PerspectiveParams {
            fovy: 45f32.to_radians(),
            aspect: 1.6,
            near: 10.0,
            far: 2000.0,
            focal_px: None,
        }),
        chase_yaw: None,
    };
    let (settle, jump_frame) = (60u32, 60 + (drive * 60.0) as u32);
    let mut worst = Worst {
        turn: 0.0,
        heading: 0.0,
        raw_heading: 0.0,
        lurch: 0.0,
        min_up: 1.0,
    };
    let (mut last_yaw, mut last_loc, mut last_car) = (0.0, Vec3::ZERO, Vec3::ZERO);
    let (mut last_heading, mut last_raw) = (0.0, 0.0);
    let raw_yaw = |rot: Quat| {
        let f = rot * Vec3::Y;
        f32::atan2(-f.x, f.y)
    };
    for frame in 0..jump_frame + 240 {
        let quanta = clock.advance(physics_dt, max_quant);
        for i in 0..quanta {
            if (settle..jump_frame).contains(&frame) {
                dynamo.change_traction(quant_input * common.car.traction_incr);
            }
            let jump =
                (frame == jump_frame && i + 1 == quanta).then_some(common.force.max_jump_power);
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
        cam.follow(&transform, frame_dt, follow);
        cam.avoid_terrain(level, transform.disp, CLEARANCE);
        if follow.look_ahead.abs() > 1e-4 {
            let look_at =
                transform.disp + cam.chase_twist() * Vec3::new(0.0, follow.look_ahead, 0.0);
            cam.rot = space::Camera::look_rotation(look_at - cam.loc);
        }
        if frame >= jump_frame {
            worst.turn = worst.turn.max(wrap_angle(view_yaw(&cam) - last_yaw).abs());
            let heading = cam.chase_yaw.unwrap();
            worst.heading = worst.heading.max(wrap_angle(heading - last_heading).abs());
            let raw = raw_yaw(transform.rot);
            worst.raw_heading = worst.raw_heading.max(wrap_angle(raw - last_raw).abs());
            let lurch = (cam.loc - last_loc) - (transform.disp - last_car);
            worst.lurch = worst.lurch.max(lurch.length());
            worst.min_up = worst.min_up.min((transform.rot * Vec3::Z).z);
        }
        (last_yaw, last_loc, last_car) = (view_yaw(&cam), cam.loc, transform.disp);
        (last_heading, last_raw) = (cam.chase_yaw.unwrap(), raw_yaw(transform.rot));
    }
    worst
}

#[test]
fn chase_camera_rides_out_a_max_jump() {
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
    let follows = [
        // `config/settings.ron`: 240u overhead, tilted 30°, speed 1.
        space::Follow {
            angle_x: 60f32.to_radians() - std::f32::consts::FRAC_PI_2,
            offset: Vec3::new(0.0, 0.0, 240.0),
            speed: 1.0,
            look_ahead: 0.0,
        },
        // The web build's close look-ahead chase.
        space::Follow {
            angle_x: 12f32.to_radians() - std::f32::consts::FRAC_PI_2,
            offset: Vec3::new(0.0, 24.0, 9.0),
            speed: 8.0,
            look_ahead: 10.0,
        },
    ];
    // Moving max jumps: across the duel sand, off the ~50° slope at
    // (884, 3960) the standing-jump playtest slid down, and two that
    // land the car on its roof after an end-over-end tumble.
    let starts = [
        ((555.0, 4151.0, -std::f32::consts::FRAC_PI_2), 1.5),
        ((884.5, 3960.3, 2.4), 0.5),
        ((900.0, 3900.0, std::f32::consts::PI), 1.5),
        ((600.0, 4300.0, std::f32::consts::FRAC_PI_4), 0.5),
    ];
    let mut worst_raw = 0.0f32;
    for (fi, follow) in follows.iter().enumerate() {
        for &(start, drive) in &starts {
            let w = jump_replay(&level, &common, car, max_quant, start, drive, follow);
            println!(
                "follow {fi} start {start:?} drive {drive}s: per frame worst heading turn {:.2}° (raw forward {:.2}°), view turn {:.2}°, camera lurch {:.1}u; car up.z down to {:.2}",
                w.heading.to_degrees(),
                w.raw_heading.to_degrees(),
                w.turn.to_degrees(),
                w.lurch,
                w.min_up
            );
            worst_raw = worst_raw.max(w.raw_heading);
            // `CHASE_YAW_RATE` is half a turn a second: 3° a 60 fps frame.
            assert!(
                w.heading <= 3.01f32.to_radians(),
                "heading turned {:.1}° in one frame",
                w.heading.to_degrees()
            );
            // The overhead view turns only with the heading; a close
            // look-ahead view also turns as the car crosses it.
            if follow.look_ahead == 0.0 {
                assert!(
                    w.turn < 6.5f32.to_radians(),
                    "view turned {:.1}°",
                    w.turn.to_degrees()
                );
            }
        }
    }
    assert!(
        worst_raw > 45f32.to_radians(),
        "the replays were meant to tumble the car"
    );
}
