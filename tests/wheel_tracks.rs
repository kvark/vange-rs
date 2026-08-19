//! Headless integration test: drive the real physics over a real level and
//! check that the wheels leave the ground the way they should.
//!
//! The unit tests in `level::terraform` pin the tread pattern down given a
//! track; this one is about the other half - that driving produces tracks at
//! all, that they follow the car, and that they stop when it leaves the
//! ground.

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
    config: &terraform::Config,
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
        for track in car.tracks.drain() {
            terraform::apply(level, config, &track, &mut regions);
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
fn driving_marks_the_ground_it_covers() {
    let mut level = drivable_level();
    let mut car = spawn(&level);
    let before = altitudes(&level);

    let regions = drive(&mut car, &mut level, &terraform::Config::default(), 200);
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

    drive(&mut car, &mut level, &terraform::Config::default(), 200);
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

    let regions = drive(&mut car, &mut level, &terraform::Config::default(), 20);

    assert_eq!(changed(&before, &altitudes(&level)), 0);
    assert!(regions.is_empty());
}

#[test]
fn turning_it_off_leaves_the_level_untouched() {
    let mut level = drivable_level();
    let mut car = spawn(&level);
    let before = altitudes(&level);

    let config = terraform::Config {
        enabled: false,
        ..terraform::Config::default()
    };
    let regions = drive(&mut car, &mut level, &config, 200);

    assert_eq!(changed(&before, &altitudes(&level)), 0);
    assert!(regions.is_empty());
}
