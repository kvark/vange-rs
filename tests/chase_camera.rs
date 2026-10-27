//! The chase camera through a max jump on Fostral (needs game data;
//! skipped without it). Replays the car with the client's quantum
//! stepping at 60 fps and drives the camera exactly as `road` does
//! (`Camera::chase`), checking how far the
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
        under_slab: 0.0,
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
        cam.chase(level, &transform, frame_dt, follow, CLEARANCE);
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

struct Fostral {
    level: level::Level,
    common: config::common::Common,
    car: CarPhysicsData,
    max_quant: f32,
}

/// Fostral and the OxidizeMonk, or `None` (and a note) without game data.
fn fostral() -> Option<Fostral> {
    let Some(settings) = config::Settings::try_load("config/settings.ron") else {
        eprintln!("skipping: no config/settings.ron with game data");
        return None;
    };
    let ini = settings.data_path.join("thechain/fostral/world.ini");
    if !ini.is_file() {
        eprintln!("skipping: no Fostral data");
        return None;
    }
    let reg = config::game::Registry::load(&settings);
    let level = level::load(&level::LevelConfig::load(&ini), &settings.game.geometry);
    let common = config::common::load(settings.open_relative("common.prm"));
    let mut cars = config::car::load_physics_registry(
        &settings.data_path,
        &reg,
        settings.game.physics.shape_sampling,
    );
    let Some(car) = cars.remove("OxidizeMonk") else {
        eprintln!("skipping: no OxidizeMonk");
        return None;
    };
    Some(Fostral {
        level,
        common,
        car,
        max_quant: settings.game.physics.max_quant,
    })
}

#[test]
fn chase_camera_rides_out_a_max_jump() {
    let Some(Fostral {
        level,
        common,
        car,
        max_quant,
    }) = fostral()
    else {
        return;
    };
    let car = &car;
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

/// The settings.ron overhead chase and the web build's close chase.
fn chase_follows() -> [space::Follow; 2] {
    [
        space::Follow {
            angle_x: 60f32.to_radians() - std::f32::consts::FRAC_PI_2,
            offset: Vec3::new(0.0, 0.0, 240.0),
            speed: 1.0,
            look_ahead: 0.0,
        },
        space::Follow {
            angle_x: 12f32.to_radians() - std::f32::consts::FRAC_PI_2,
            offset: Vec3::new(0.0, 24.0, 9.0),
            speed: 8.0,
            look_ahead: 10.0,
        },
    ]
}

fn camera_at(loc: Vec3) -> space::Camera {
    space::Camera {
        loc,
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
        under_slab: 0.0,
    }
}

/// `p` is open air: above the floor below it and, in a two-level column,
/// not inside the slab. `margin` is how close to either surface counts.
fn in_air(level: &level::Level, p: Vec3, margin: f32) -> bool {
    match level.get((p.x as i32, p.y as i32)) {
        level::Texel::Single(h) => p.z >= h.0 + margin,
        level::Texel::Dual { low, mid, high } => {
            (p.z >= low.0 + margin && p.z <= mid - margin) || p.z >= high.0 + margin
        }
    }
}

/// Every sample of the camera→car segment (short of the car body) is air.
fn clear_sight(level: &level::Level, cam: Vec3, car: Vec3) -> bool {
    (1..32).all(|i| in_air(level, cam.lerp(car, 0.9 * i as f32 / 32.0), 0.0))
}

/// The car's resting height at `(x, y)`: on the floor under the slab,
/// riding over the odd one-texel dip as a hull does.
fn car_on_floor(level: &level::Level, x: f32, y: f32) -> Vec3 {
    let floor = (-2..=2)
        .flat_map(|j| (-2..=2).map(move |i| (i, j)))
        .map(|(i, j)| {
            level
                .get(((x + 4.0 * i as f32) as i32, (y + 4.0 * j as f32) as i32))
                .low()
        })
        .fold(f32::MIN, f32::max);
    Vec3::new(x, y, floor + 3.6)
}

/// Underpass spots: where the live Q-roll jump came to rest under the
/// slab (710.4, 4053.7), single-level hole texels in the same roof
/// (692, 4054) and (716, 4054), and three more across the underpass.
const UNDERPASS: [(f32, f32); 6] = [
    (710.4, 4053.7),
    (692.0, 4054.0),
    (716.0, 4054.0),
    (700.0, 4060.0),
    (740.0, 4100.0),
    (660.0, 4150.0),
];

/// Parked under the slab, the chase camera sits in the gap between the
/// floor and the slab underside, a sane distance off, seeing the car.
/// Before the fix the 240u chase parked a hundred units straight up
/// against the ceiling, and on a hole texel dived onto the roof.
#[test]
fn chase_camera_settles_in_the_underpass_gap() {
    let Some(f) = fostral() else { return };
    let level = &f.level;
    for (fi, follow) in chase_follows().iter().enumerate() {
        for &(x, y) in &UNDERPASS {
            let car = car_on_floor(level, x, y);
            let ceiling = space::Camera::under_slab_at(level, car)
                .unwrap_or_else(|| panic!("({x}, {y}) should be under the slab"));
            for k in 0..6 {
                let yaw = k as f32 * std::f32::consts::FRAC_PI_3;
                let target = space::Transform {
                    disp: car,
                    rot: Quat::from_rotation_z(yaw),
                    scale: 1.0,
                };
                // As if a jump just dropped the car in from above.
                let mut cam = camera_at(car + Vec3::new(0.0, 0.0, 240.0));
                let mut dist = 0.0;
                for frame in 0..180 {
                    cam.chase(level, &target, 1.0 / 60.0, follow, CLEARANCE);
                    let at = format!(
                        "follow {fi} at ({x}, {y}) yaw {k}x60° frame {frame}: camera {:?}",
                        cam.loc
                    );
                    assert!(
                        in_air(level, cam.loc, CLEARANCE - 0.01),
                        "{at} is not in open air"
                    );
                    // Not over, or up through, the roof the car is under.
                    let roof = Vec3::new(cam.loc.x, cam.loc.y, car.z);
                    if let Some(c) = space::Camera::under_slab_at(level, roof) {
                        assert!(cam.loc.z < c, "{at} is above the slab underside {c}");
                    }
                    if frame >= 90 {
                        dist = (cam.loc - car).length();
                        assert!(
                            (20.0..=60.0).contains(&dist),
                            "{at} is {dist:.1} from the car"
                        );
                        assert!(clear_sight(level, cam.loc, car), "{at} cannot see the car");
                        assert!(
                            !space::Camera::terrain_blocks_view(level, cam.loc, car, CLEARANCE),
                            "{at}: the veil would kick in"
                        );
                    }
                }
                if k == 0 {
                    println!(
                        "follow {fi} at ({x}, {y}): settled {dist:.1} from the car at {:?}, {:.1} under the {ceiling} underside",
                        cam.loc,
                        ceiling - cam.loc.z
                    );
                }
            }
        }
    }
}

/// Creeping across the hole-speckled roof: the car's own texel flips
/// between two-level and single-level every few units. The camera must
/// not dive onto the roof (old: 6.8u above the car) or jump about.
#[test]
fn chase_camera_ignores_holes_in_the_slab() {
    let Some(f) = fostral() else { return };
    let level = &f.level;
    for (fi, follow) in chase_follows().iter().enumerate() {
        let mut cam = camera_at(car_on_floor(level, 690.0, 4054.0) + Vec3::new(0.0, 0.0, 240.0));
        let (mut near, mut lurch) = (f32::INFINITY, 0.0f32);
        let (mut last_cam, mut last_car) = (Vec3::ZERO, Vec3::ZERO);
        for frame in 0..360 {
            let car = Vec3::new(690.0 + frame as f32 * 10.0 / 60.0, 4054.0, 13.6);
            let target = space::Transform {
                disp: car,
                rot: Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2),
                scale: 1.0,
            };
            cam.chase(level, &target, 1.0 / 60.0, follow, CLEARANCE);
            if frame >= 90 {
                near = near.min((cam.loc - car).length());
                lurch = lurch.max(((cam.loc - last_cam) - (car - last_car)).length());
            }
            (last_cam, last_car) = (cam.loc, car);
        }
        println!("follow {fi}: nearest {near:.1}, worst lurch {lurch:.2}u/frame");
        assert!(near > 20.0, "follow {fi} came within {near:.1} of the car");
        assert!(lurch < 1.0, "follow {fi} lurched {lurch:.2}u in a frame");
    }
}

/// Drive into the underpass from the open sand to the west and on
/// through it, at full throttle, checking every frame that the camera
/// is in open air (never in rock, never over the slab while the car is
/// under it) and that it neither lurches nor swings.
#[test]
fn chase_camera_drives_through_the_underpass() {
    let Some(Fostral {
        level,
        common,
        car,
        max_quant,
    }) = fostral()
    else {
        return;
    };
    let frame_dt = 1.0 / 60.0;
    let physics_dt = frame_dt * {
        let n = &common.nature;
        common.speed.standard_frame_rate as f32 * n.time_delta0 * n.num_calls_analysis as f32
    };
    let quant_input = physics::QuantClock::input_factor(max_quant, &common);
    for (fi, follow) in chase_follows().iter().enumerate() {
        let mut clock = physics::QuantClock::default();
        let mut dynamo = Dynamo::default();
        let start = car_on_floor(&level, 540.0, 4060.0);
        let mut transform = space::Transform {
            disp: start + Vec3::new(0.0, 0.0, 2.0),
            rot: Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2),
            scale: car.scale,
        };
        let mut cam = camera_at(transform.disp + Vec3::new(0.0, 0.0, 240.0));
        let (mut lurch, mut turn, mut under, mut deep, mut max_x) = (0.0f32, 0.0f32, 0, 0, 0.0f32);
        let (mut last_cam, mut last_car, mut last_dir) = (cam.loc, transform.disp, cam.dir());
        for frame in 0..60 * 9 {
            for _ in 0..clock.advance(physics_dt, max_quant) {
                if frame >= 60 {
                    dynamo.change_traction(quant_input * common.car.traction_incr);
                }
                physics::step(
                    &mut dynamo,
                    &mut transform,
                    max_quant,
                    &car,
                    &level,
                    &common,
                    1.0,
                    0.0,
                    None,
                    0.0,
                    None,
                    None,
                );
            }
            cam.chase(&level, &transform, frame_dt, follow, CLEARANCE);
            let at = format!(
                "follow {fi} frame {frame}: car {:?}, camera {:?}",
                transform.disp, cam.loc
            );
            assert!(
                in_air(&level, cam.loc, CLEARANCE - 0.01),
                "{at}: camera in rock"
            );
            if space::Camera::under_slab_at(&level, transform.disp).is_some() {
                under += 1;
                // Not over (or up through) the roof the car is under...
                let roof = Vec3::new(cam.loc.x, cam.loc.y, transform.disp.z);
                if let Some(ceiling) = space::Camera::under_slab_at(&level, roof) {
                    assert!(
                        cam.loc.z < ceiling,
                        "{at}: camera over the slab, car under it"
                    );
                }
                // ...and seeing the car.
                assert!(
                    clear_sight(&level, cam.loc, transform.disp),
                    "{at}: no sight of the car"
                );
                if cam.under_slab > 0.99 {
                    deep += 1;
                }
            }
            if frame > 60 {
                lurch = lurch.max(((cam.loc - last_cam) - (transform.disp - last_car)).length());
                turn = turn.max(cam.dir().angle_between(last_dir));
            }
            max_x = max_x.max(transform.disp.x);
            (last_cam, last_car, last_dir) = (cam.loc, transform.disp, cam.dir());
        }
        println!(
            "follow {fi}: {under} frames under the slab ({deep} fully in the under-slab pose), reached x {max_x:.0}; worst per-frame lurch {lurch:.1}u, view turn {:.2}°",
            turn.to_degrees()
        );
        assert!(under > 120, "the car was meant to drive under the slab");
        assert!(lurch < 12.0, "camera lurched {lurch:.1}u in one frame");
        assert!(
            turn < 6f32.to_radians(),
            "view turned {:.1}° in one frame",
            turn.to_degrees()
        );
    }
}

/// Where the live #393 test backed (S) out of the underpass toward its
/// south wall: (739.5, 3955.1) after an S tap from (714.8, 4017.4), and
/// (767, 3964) by the ramp to the east, plus a step closer to each wall.
/// Still under the 130 slab, but with the cliff 15u behind the car.
const BACKED_TO_THE_WALL: [(f32, f32); 4] = [
    (739.48, 3955.08),
    (767.0, 3964.0),
    (739.48, 3948.0),
    (767.0, 3958.0),
];

/// Parked under the slab with the cave wall (or the rising floor of the
/// ramp) right behind the car: the low pose behind it is inside rock.
/// Pulling that arm in along the ray left the camera 6-17u from the car,
/// which filled the screen behind the see-through veil. It must swing
/// up over the car instead, at full arm, in the open gap.
#[test]
fn chase_camera_backed_to_a_wall_under_the_slab() {
    let Some(f) = fostral() else { return };
    let level = &f.level;
    // The S-tap heading (car nose toward (714.8, 4017.4)), then the axes.
    let yaws = [
        0.377f32,
        0.0,
        std::f32::consts::PI,
        std::f32::consts::FRAC_PI_2,
        -std::f32::consts::FRAC_PI_2,
    ];
    for (fi, follow) in chase_follows().iter().enumerate() {
        for &(x, y) in &BACKED_TO_THE_WALL {
            let car = Vec3::new(x, y, level.get((x as i32, y as i32)).low() + 3.6);
            let ceiling = space::Camera::under_slab_at(level, car)
                .unwrap_or_else(|| panic!("({x}, {y}) should be under the slab"));
            let mut nearest = f32::INFINITY;
            for &yaw in &yaws {
                let target = space::Transform {
                    disp: car,
                    rot: Quat::from_rotation_z(yaw),
                    scale: 1.0,
                };
                let mut cam = camera_at(car + Vec3::new(0.0, 0.0, 240.0));
                for frame in 0..240 {
                    cam.chase(level, &target, 1.0 / 60.0, follow, CLEARANCE);
                    let at = format!(
                        "follow {fi} at ({x}, {y}) yaw {yaw:.2} frame {frame}: camera {:?}",
                        cam.loc
                    );
                    assert!(
                        in_air(level, cam.loc, CLEARANCE - 0.01),
                        "{at} is not in open air"
                    );
                    if frame >= 120 {
                        let dist = (cam.loc - car).length();
                        nearest = nearest.min(dist);
                        assert!(dist >= 20.0, "{at} collapsed to {dist:.1} from the car");
                        assert!(cam.loc.z < ceiling, "{at} is above the {ceiling} underside");
                        // Sight of the car's roof: a one-texel bump in the
                        // floor between can hide its belly.
                        let roof = car + Vec3::new(0.0, 0.0, CLEARANCE);
                        assert!(clear_sight(level, cam.loc, roof), "{at} cannot see the car");
                        assert!(
                            !space::Camera::terrain_blocks_view(level, cam.loc, car, CLEARANCE),
                            "{at}: the veil would kick in"
                        );
                    }
                }
            }
            println!("follow {fi} at ({x}, {y}): nearest {nearest:.1}");
        }
    }
}

/// On the roof by the same wall, next to a taller overhang whose cave
/// floor is the underpass far below: not "under a slab". The chase stays
/// the authored one (it used to blend toward the under-slab pose here).
#[test]
fn chase_camera_on_the_roof_beside_a_taller_overhang() {
    let Some(f) = fostral() else { return };
    let level = &f.level;
    for &(x, y) in &[(739.48f32, 3948.0f32), (743.5, 3958.9)] {
        let car = Vec3::new(x, y, level.get((x as i32, y as i32)).high() + 3.6);
        assert!(space::Camera::under_slab_at(level, car).is_none());
        for follow in chase_follows().iter() {
            let target = space::Transform {
                disp: car,
                rot: Quat::from_rotation_z(std::f32::consts::PI),
                scale: 1.0,
            };
            let mut cam = camera_at(car + Vec3::new(0.0, 0.0, 240.0));
            for _ in 0..120 {
                cam.chase(level, &target, 1.0 / 60.0, follow, CLEARANCE);
                assert_eq!(
                    cam.under_slab, 0.0,
                    "({x}, {y}) roof counted as under a slab"
                );
            }
        }
    }
}

/// Back up (S) from under the slab toward its south wall, as the live
/// test did: the camera never collapses onto the car or lurches.
#[test]
fn chase_camera_reverses_toward_the_wall() {
    let Some(Fostral {
        level,
        common,
        car,
        max_quant,
    }) = fostral()
    else {
        return;
    };
    let frame_dt = 1.0 / 60.0;
    let physics_dt = frame_dt * {
        let n = &common.nature;
        common.speed.standard_frame_rate as f32 * n.time_delta0 * n.num_calls_analysis as f32
    };
    let quant_input = physics::QuantClock::input_factor(max_quant, &common);
    for (fi, follow) in chase_follows().iter().enumerate() {
        let mut clock = physics::QuantClock::default();
        let mut dynamo = Dynamo::default();
        let mut transform = space::Transform {
            disp: Vec3::new(714.8, 4017.4, level.get((714, 4017)).low() + 5.6),
            rot: Quat::from_rotation_z(0.377),
            scale: car.scale,
        };
        let mut cam = camera_at(transform.disp + Vec3::new(0.0, 0.0, 240.0));
        let (mut near, mut lurch, mut min_y) = (f32::INFINITY, 0.0f32, f32::INFINITY);
        let (mut close, mut longest_close) = (0, 0);
        let (mut last_cam, mut last_car) = (cam.loc, transform.disp);
        for frame in 0..60 * 8 {
            for _ in 0..clock.advance(physics_dt, max_quant) {
                if (60..150).contains(&frame) {
                    dynamo.change_traction(-quant_input * common.car.traction_incr);
                }
                physics::step(
                    &mut dynamo,
                    &mut transform,
                    max_quant,
                    &car,
                    &level,
                    &common,
                    1.0,
                    0.0,
                    None,
                    0.0,
                    None,
                    None,
                );
            }
            cam.chase(&level, &transform, frame_dt, follow, CLEARANCE);
            let at = format!(
                "follow {fi} frame {frame}: car {:?}, camera {:?}",
                transform.disp, cam.loc
            );
            assert!(
                in_air(&level, cam.loc, CLEARANCE - 0.01),
                "{at}: camera in rock"
            );
            if frame > 120 {
                let d = (cam.loc - transform.disp).length();
                near = near.min(d);
                close = if d < 12.0 { close + 1 } else { 0 };
                longest_close = longest_close.max(close);
                lurch = lurch.max(((cam.loc - last_cam) - (transform.disp - last_car)).length());
            }
            min_y = min_y.min(transform.disp.y);
            (last_cam, last_car) = (cam.loc, transform.disp);
        }
        println!(
            "follow {fi}: car reached y {min_y:.0}, ended at {:?}; nearest {near:.1}, closer than 12 for {longest_close} frames at most, worst lurch {lurch:.1}u",
            transform.disp
        );
        assert!(min_y < 3975.0, "the car was meant to back up to the wall");
        // Reversing into the camera, and the spin on the slope by the
        // wall (the follow's position lerp cuts the swing's chord), bring
        // it in for a moment. A camera folded onto the car stays there
        // (live: ~3 s at 6-10u).
        assert!(near > 8.0, "camera came within {near:.1} of the car");
        assert!(
            longest_close < 15,
            "camera stayed within 12u of the car for {longest_close} frames"
        );
        assert!(lurch < 12.0, "camera lurched {lurch:.1}u in one frame");
    }
}

/// Where the underpass floor (10) is pitted with single texels down at
/// 0-4, or ends in the 10u drop of its west mouth: car centres the
/// OxidizeMonk's hull came to rest over near (706.2, 4147.6), and pits
/// across the floor further north. Resting heights are the replayed ones.
const OVER_A_FLOOR_PIT: [(f32, f32, f32); 6] = [
    (683.3, 4152.2, 10.2),
    (682.9, 4149.0, 9.9),
    (681.1, 4153.2, 10.2),
    (780.0, 3998.0, 11.6),
    (668.0, 4002.0, 13.6),
    (764.0, 4010.0, 13.6),
];

/// `road`'s ground-anchored chase (the settings.ron 60° 240u follow,
/// target dropped to the ground under the car). Anchored to the pit
/// under the hull's centre, the arm started below the surrounding floor,
/// its first sample hit that floor, and the camera sat 3-10u from the
/// car with its near plane in the rock, for as long as the car stayed
/// there.
#[test]
fn chase_camera_rides_over_a_floor_pit() {
    let Some(f) = fostral() else { return };
    let level = &f.level;
    let follow = chase_follows()[0];
    let yaws = [
        0.0f32,
        std::f32::consts::FRAC_PI_4,
        std::f32::consts::FRAC_PI_2,
        std::f32::consts::PI,
        -std::f32::consts::FRAC_PI_2,
    ];
    for &(x, y, z) in &OVER_A_FLOOR_PIT {
        let car = Vec3::new(x, y, z);
        let ground = space::Camera::ground_under(level, car);
        assert!(
            ground >= level.floor_below(car) + 4.0,
            "({x}, {y}) should be over a pit: ground {ground}"
        );
        let mut nearest = f32::INFINITY;
        for &yaw in &yaws {
            let mut target = space::Transform {
                disp: car,
                rot: Quat::from_rotation_z(yaw),
                scale: 1.0,
            };
            target.disp.z = ground;
            let mut cam = camera_at(car + Vec3::new(0.0, 0.0, 240.0));
            for frame in 0..180 {
                cam.chase(level, &target, 1.0 / 60.0, &follow, CLEARANCE);
                if frame < 120 {
                    continue;
                }
                let at = format!(
                    "({x}, {y}) yaw {yaw:.2} frame {frame}: camera {:?}",
                    cam.loc
                );
                let dist = (cam.loc - car).length();
                nearest = nearest.min(dist);
                assert!(dist >= 25.0, "{at} collapsed to {dist:.1} from the car");
                assert!(in_air(level, cam.loc, CLEARANCE - 0.01), "{at} is in rock");
                assert!(cam.loc.z > car.z, "{at} is below the car");
                let roof = car + Vec3::new(0.0, 0.0, CLEARANCE);
                assert!(clear_sight(level, cam.loc, roof), "{at} cannot see the car");
            }
        }
        println!("over the pit at ({x}, {y}): nearest {nearest:.1}");
    }
}
