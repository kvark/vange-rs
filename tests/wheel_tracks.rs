//! Headless integration test: drive the real physics over a real level and
//! check that the car reshapes the ground the way it should.
//!
//! The unit tests in `level::terraform` pin the tread pattern and the
//! blade down given a track or a sweep; this one is about the other half -
//! that driving produces them at all, that they follow the car, and that
//! they stop when it leaves the ground.

use glam::{Quat, Vec3};
use vangers::{
    config::{self, settings},
    level::{self, terraform},
    physics::{self, CarPhysicsData, Dynamo},
    space,
};

struct Car {
    transform: space::Transform,
    dynamo: Dynamo,
    tracks: terraform::Tracks,
    data: CarPhysicsData,
}

/// The stock test level with its terrain flattened to the one type a wheel
/// is allowed to disturb - otherwise the type varies with the altitude and
/// a track would be cut into stripes.
fn drivable_level() -> level::Level {
    let mut level = level::load(
        &level::LevelConfig::new_test(),
        &settings::Geometry::default(),
    );
    let bits = level.terrain_bits();
    let main = bits.write(terraform::MAIN_TERRAIN);
    for meta in level.meta.iter_mut() {
        if *meta & level::DOUBLE_LEVEL == 0 {
            *meta = main;
        }
    }
    level
}

fn spawn(level: &level::Level) -> Car {
    let (x, y) = (level.size.0 / 4, level.size.1 / 4);
    Car {
        transform: space::Transform {
            disp: Vec3::new(x as f32, y as f32, level.get((x, y)).high() + 5.0),
            rot: Quat::from_rotation_z(std::f32::consts::PI),
            scale: 1.0,
        },
        dynamo: Dynamo::default(),
        tracks: terraform::Tracks::default(),
        data: CarPhysicsData::test_default(),
    }
}

/// Runs `frames` of the game loop, cutting each frame's tracks into the
/// level. Returns the rectangles that were touched.
fn drive(
    car: &mut Car,
    level: &mut level::Level,
    config: &terraform::Tread,
    frames: usize,
) -> Vec<level::Region> {
    drive_with(car, level, config, &terraform::Grader::default(), frames)
}

fn drive_with(
    car: &mut Car,
    level: &mut level::Level,
    tread: &terraform::Tread,
    grader: &terraform::Grader,
    frames: usize,
) -> Vec<level::Region> {
    drive_full(car, level, tread, grader, &press_off(), frames)
}

fn press_off() -> terraform::Press {
    terraform::Press {
        enabled: false,
        ..terraform::Press::default()
    }
}

fn drive_full(
    car: &mut Car,
    level: &mut level::Level,
    tread: &terraform::Tread,
    grader: &terraform::Grader,
    press: &terraform::Press,
    frames: usize,
) -> Vec<level::Region> {
    let common = config::common::Common::test_default();
    let mut regions = Vec::new();
    for _ in 0..frames {
        car.dynamo.change_traction(0.5);
        physics::step(
            &mut car.dynamo,
            &mut car.transform,
            0.02,
            &car.data,
            level,
            &common,
            1.0,
            0.0,
            None,
            0.0,
            None,
            Some(&mut car.tracks),
        );
        if let Some(hull) = car.tracks.take_hull() {
            terraform::apply_press(level, press, &hull, &mut regions);
        }
        for sweep in car.tracks.drain_sweeps() {
            terraform::apply_grader(level, grader, &sweep, &mut regions);
        }
        for track in car.tracks.drain() {
            terraform::apply_tread(level, tread, &track, &mut regions);
        }
    }
    regions
}

fn altitudes(level: &level::Level) -> Vec<u8> {
    level.height.to_vec()
}

fn changed(before: &[u8], after: &[u8]) -> usize {
    before.iter().zip(after).filter(|(a, b)| a != b).count()
}

#[test]
fn tread_endpoints_use_the_post_step_wheel_positions() {
    let mut level = drivable_level();
    let mut car = spawn(&level);
    // Settle and get rolling through the same path as the game first.
    drive(&mut car, &mut level, &terraform::Tread::default(), 80);
    car.tracks.reset();

    let common = config::common::Common::test_default();
    let step = |car: &mut Car| {
        car.dynamo.change_traction(0.5);
        physics::step(
            &mut car.dynamo,
            &mut car.transform,
            0.02,
            &car.data,
            &level,
            &common,
            1.0,
            0.0,
            None,
            0.0,
            None,
            Some(&mut car.tracks),
        );
    };
    for _ in 0..100 {
        step(&mut car);
        let tracks = car.tracks.drain().collect::<Vec<_>>();
        if tracks.is_empty() {
            continue;
        }
        let expected = car
            .data
            .wheels
            .iter()
            .map(|wheel| {
                let p = car.transform.transform_point(Vec3::from(wheel.pos));
                (p.x.round() as i32, p.y.round() as i32)
            })
            .collect::<Vec<_>>();
        for track in tracks {
            assert!(
                expected.contains(&track.to),
                "endpoint {:?} is not under any post-step wheel {:?}",
                track.to,
                expected
            );
        }
        return;
    }
    panic!("the rolling wheels produced no stretches");
}

#[test]
fn one_suspended_wheel_does_not_leave_a_track() {
    let mut level = drivable_level();
    let mut car = spawn(&level);
    drive(&mut car, &mut level, &terraform::Tread::default(), 80);
    car.tracks.reset();
    car.data.wheels[0].pos[2] += 100.0;

    let common = config::common::Common::test_default();
    for _ in 0..100 {
        car.dynamo.change_traction(0.5);
        physics::step(
            &mut car.dynamo,
            &mut car.transform,
            0.02,
            &car.data,
            &level,
            &common,
            1.0,
            0.0,
            None,
            0.0,
            None,
            Some(&mut car.tracks),
        );
        let tracks = car.tracks.drain().collect::<Vec<_>>();
        if !tracks.is_empty() {
            let wheel = car
                .transform
                .transform_point(Vec3::from(car.data.wheels[0].pos));
            let suspended = (wheel.x.round() as i32, wheel.y.round() as i32);
            assert!(
                tracks.iter().all(|track| track.to != suspended),
                "the suspended wheel left a track at {suspended:?}: {tracks:?}"
            );
            return;
        }
    }
    panic!("the grounded wheels produced no tracks");
}

#[test]
fn driving_marks_the_ground_it_covers() {
    let mut level = drivable_level();
    let mut car = spawn(&level);
    let before = altitudes(&level);

    let regions = drive(&mut car, &mut level, &terraform::Tread::default(), 200);
    let after = altitudes(&level);

    assert!(
        changed(&before, &after) > 0,
        "the wheels left nothing behind"
    );
    assert!(!regions.is_empty(), "and reported nothing to re-upload");

    // The tread cuts two texels for every one it raises, so the ground the
    // car covered has to end up lower than it started. This is the whole
    // point: driving somewhere digs it out.
    let displaced: i32 = before
        .iter()
        .zip(&after)
        .map(|(a, b)| *b as i32 - *a as i32)
        .sum();
    assert!(
        displaced < 0,
        "the track should sink on balance, not rise: {}",
        displaced
    );

    // Every changed texel has to fall inside something the renderer was told
    // about, or the screen and the level would drift apart.
    for (i, (a, b)) in before.iter().zip(&after).enumerate() {
        if a == b {
            continue;
        }
        let (x, y) = (i as i32 % level.size.0, i as i32 / level.size.0);
        assert!(
            regions
                .iter()
                .any(|r| { x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h }),
            "texel ({}, {}) changed outside every reported region",
            x,
            y
        );
    }
}

#[test]
fn the_marks_follow_the_car() {
    let mut level = drivable_level();
    let mut car = spawn(&level);
    let before = altitudes(&level);
    let start = car.transform.disp;

    drive(&mut car, &mut level, &terraform::Tread::default(), 200);
    let after = altitudes(&level);

    let size = glam::Vec2::new(level.size.0 as f32, level.size.1 as f32);
    // The level wraps, and the test car drives straight off the near edge.
    let span = |a: glam::Vec2, b: glam::Vec2| {
        let d = (a - b).abs();
        glam::Vec2::new(d.x.min(size.x - d.x), d.y.min(size.y - d.y)).length()
    };
    let travelled = span(car.transform.disp.truncate(), start.truncate());
    assert!(travelled > 1.0, "the car never moved: {:?}", travelled);

    // Nothing should have been touched further from the path than the car is
    // wide, plus the reach of one tread bar.
    let reach = car.data.bbox.max[0].abs().max(car.data.bbox.min[0].abs()) + 8.0;
    for (i, (a, b)) in before.iter().zip(&after).enumerate() {
        if a == b {
            continue;
        }
        let pos = glam::Vec2::new(
            (i as i32 % level.size.0) as f32,
            (i as i32 / level.size.0) as f32,
        );
        let from_start = span(pos, start.truncate());
        let from_end = span(pos, car.transform.disp.truncate());
        assert!(
            from_start.min(from_end) < travelled + reach,
            "a mark at {:?} is nowhere near the {:?} the car drove",
            pos,
            travelled
        );
    }
}

#[test]
fn a_car_off_the_ground_marks_nothing() {
    let mut level = drivable_level();
    let mut car = spawn(&level);
    // Well clear of anything, and moving.
    car.transform.disp.z += 400.0;
    car.dynamo.linear_velocity = Vec3::new(0.0, 40.0, 0.0);
    let before = altitudes(&level);

    let regions = drive(&mut car, &mut level, &terraform::Tread::default(), 20);

    assert_eq!(changed(&before, &altitudes(&level)), 0);
    assert!(regions.is_empty());
}

#[test]
fn turning_it_off_leaves_the_level_untouched() {
    let mut level = drivable_level();
    let mut car = spawn(&level);
    let before = altitudes(&level);

    let config = terraform::Tread {
        enabled: false,
        ..terraform::Tread::default()
    };
    let regions = drive(&mut car, &mut level, &config, 200);

    assert_eq!(changed(&before, &altitudes(&level)), 0);
    assert!(regions.is_empty());
}

#[test]
fn the_blade_carves_where_the_car_drives() {
    let mut level = drivable_level();
    let mut car = spawn(&level);
    let start = car.transform.disp;
    let before = altitudes(&level);

    let grader = terraform::Grader {
        enabled: true,
        ..terraform::Grader::default()
    };
    // Tread off, so anything that moves is the blade's doing.
    let tread = terraform::Tread {
        enabled: false,
        ..terraform::Tread::default()
    };
    let regions = drive_with(&mut car, &mut level, &tread, &grader, 200);
    let after = altitudes(&level);

    assert!(changed(&before, &after) > 0, "the blade cut nothing");
    assert!(!regions.is_empty());

    // The blade rides under the car, so it has to have lowered ground as
    // well as raised it - a pure heap would mean it never bit.
    assert!(
        before.iter().zip(&after).any(|(a, b)| b < a),
        "the blade only piled soil up, it never cut any"
    );
    assert!(
        before.iter().zip(&after).any(|(a, b)| b > a),
        "the blade cut soil but never put any back"
    );

    let size = glam::Vec2::new(level.size.0 as f32, level.size.1 as f32);
    let span = |a: glam::Vec2, b: glam::Vec2| {
        let d = (a - b).abs();
        glam::Vec2::new(d.x.min(size.x - d.x), d.y.min(size.y - d.y)).length()
    };
    let travelled = span(car.transform.disp.truncate(), start.truncate());
    let reach = car.data.bbox.max[1].abs().max(car.data.bbox.min[1].abs()) + 64.0;
    for (i, (a, b)) in before.iter().zip(&after).enumerate() {
        if a == b {
            continue;
        }
        let pos = glam::Vec2::new(
            (i as i32 % level.size.0) as f32,
            (i as i32 / level.size.0) as f32,
        );
        let from_start = span(pos, start.truncate());
        let from_end = span(pos, car.transform.disp.truncate());
        assert!(
            from_start.min(from_end) < travelled + reach,
            "the blade reshaped {:?}, nowhere near the {:?} the car drove",
            pos,
            travelled
        );
    }
}

#[test]
fn a_car_off_the_ground_grades_nothing() {
    let mut level = drivable_level();
    let mut car = spawn(&level);
    car.transform.disp.z += 400.0;
    car.dynamo.linear_velocity = Vec3::new(0.0, 40.0, 0.0);
    let before = altitudes(&level);

    let grader = terraform::Grader {
        enabled: true,
        ..terraform::Grader::default()
    };
    let tread = terraform::Tread {
        enabled: false,
        ..terraform::Tread::default()
    };
    let regions = drive_with(&mut car, &mut level, &tread, &grader, 20);

    assert_eq!(changed(&before, &altitudes(&level)), 0);
    assert!(regions.is_empty());
}

#[test]
fn a_car_presses_its_own_hollow_into_soft_ground() {
    let mut level = drivable_level();
    let mut car = spawn(&level);
    // Park it on a mound, so there is something standing proud of the hull.
    let (cx, cy) = (car.transform.disp.x as i32, car.transform.disp.y as i32);
    for dy in -12..12 {
        for dx in -12..12 {
            let i = level.wrap((cx + dx, cy + dy));
            level.height[i] = level.height[i].saturating_add(60);
        }
    }
    let before = altitudes(&level);

    let off = terraform::Tread {
        enabled: false,
        ..terraform::Tread::default()
    };
    let grader = terraform::Grader::default();
    let regions = drive_full(
        &mut car,
        &mut level,
        &off,
        &grader,
        &terraform::Press {
            enabled: true,
            ..terraform::Press::default()
        },
        60,
    );
    let after = altitudes(&level);

    assert!(!regions.is_empty(), "the hull pressed nothing");
    // A hull only ever pushes down; nothing it touches may come out higher.
    assert!(
        before.iter().zip(&after).all(|(a, b)| b <= a),
        "the hull raised ground somewhere"
    );
    assert!(
        before.iter().zip(&after).any(|(a, b)| b < a),
        "and it lowered none"
    );
}

#[test]
fn a_car_in_the_air_presses_nothing() {
    let mut level = drivable_level();
    let mut car = spawn(&level);
    car.transform.disp.z += 400.0;
    let before = altitudes(&level);

    let off = terraform::Tread {
        enabled: false,
        ..terraform::Tread::default()
    };
    let regions = drive_full(
        &mut car,
        &mut level,
        &off,
        &terraform::Grader::default(),
        &terraform::Press {
            enabled: true,
            ..terraform::Press::default()
        },
        15,
    );
    assert_eq!(changed(&before, &altitudes(&level)), 0);
    assert!(regions.is_empty());
}

/// How deep the car sits relative to the ground it is over.
fn depth(car: &Car, level: &level::Level) -> f32 {
    let coord = (car.transform.disp.x as i32, car.transform.disp.y as i32);
    terraform::surface_height(level, coord) - car.transform.disp.z
}

#[test]
fn the_mole_pulls_a_car_under_and_lets_it_back_out() {
    let mut level = drivable_level();
    let mut car = spawn(&level);
    let common = config::common::Common::test_default();
    let tread = terraform::Tread {
        enabled: false,
        ..terraform::Tread::default()
    };

    let step = |car: &mut Car, level: &mut level::Level, frames: usize| {
        for _ in 0..frames {
            car.dynamo.change_traction(0.5);
            physics::step(
                &mut car.dynamo,
                &mut car.transform,
                0.02,
                &car.data,
                level,
                &common,
                1.0,
                0.0,
                None,
                0.0,
                None,
                Some(&mut car.tracks),
            );
            let _ = car.tracks.drain_burrows().count();
            let _ = car.tracks.take_hull();
            let _ = car.tracks.drain_sweeps().count();
            let _ = car.tracks.drain().count();
        }
    };

    // Settle on the surface first.
    step(&mut car, &mut level, 40);
    let resting = depth(&car, &level);

    car.dynamo.mole = physics::Mole::Under;
    step(&mut car, &mut level, 120);
    let buried = depth(&car, &level);
    assert_eq!(
        car.dynamo.mole,
        physics::Mole::Under,
        "it surfaced by itself"
    );
    assert!(
        buried > resting + 1.0,
        "the mole did not pull the car under: {} -> {}",
        resting,
        buried
    );

    car.dynamo.mole = physics::Mole::Surfacing;
    step(&mut car, &mut level, 400);
    assert_eq!(
        car.dynamo.mole,
        physics::Mole::Off,
        "the mole never finished surfacing, still {:?} deep",
        depth(&car, &level)
    );
    assert!(
        depth(&car, &level) < buried,
        "and it came back up no higher than it went"
    );
    let _ = tread;
}

#[test]
fn a_burrowing_car_leaves_mounds_and_a_settled_one_does_not() {
    let mut level = drivable_level();
    // Low ground, so the mounds come through at full height.
    level.height.iter_mut().for_each(|h| *h = 20);
    let mut car = spawn(&level);
    car.transform.disp.z = 25.0;
    let common = config::common::Common::test_default();

    let run = |car: &mut Car, level: &mut level::Level, frames: usize| {
        let mut regions = Vec::new();
        for _ in 0..frames {
            car.dynamo.change_traction(1.0);
            physics::step(
                &mut car.dynamo,
                &mut car.transform,
                0.02,
                &car.data,
                level,
                &common,
                1.0,
                0.0,
                None,
                0.0,
                None,
                Some(&mut car.tracks),
            );
            for b in car.tracks.drain_burrows() {
                terraform::apply_burrow(
                    level,
                    &terraform::Molehills {
                        enabled: true,
                        ..terraform::Molehills::default()
                    },
                    &b,
                    &mut regions,
                );
            }
            let _ = car.tracks.take_hull();
            let _ = car.tracks.drain_sweeps().count();
            let _ = car.tracks.drain().count();
        }
        regions
    };

    let before = altitudes(&level);
    let idle = run(&mut car, &mut level, 60);
    assert!(idle.is_empty(), "a car on the surface threw up mounds");
    assert_eq!(changed(&before, &altitudes(&level)), 0);

    car.dynamo.mole = physics::Mole::Under;
    let dug = run(&mut car, &mut level, 200);
    assert!(!dug.is_empty(), "a burrowing car threw up nothing");
    let after = altitudes(&level);
    assert!(
        before.iter().zip(&after).all(|(a, b)| b >= a),
        "a burrow lowered the surface somewhere"
    );
}
