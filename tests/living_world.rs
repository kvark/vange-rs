//! Headless integration: a car that actually rolls produces dust from the
//! same `from_track` path the road loop uses.

use glam::{Quat, Vec3};
use vangers::{
    config::{self, settings},
    creature::{Insect, Swarm, TIER_PRICES},
    level::{self, terraform},
    particle::{Kind, System as Particles},
    physics::{self, CarPhysicsData, Dynamo},
    space,
};

struct Car {
    transform: space::Transform,
    dynamo: Dynamo,
    tracks: terraform::Tracks,
    data: CarPhysicsData,
}

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

fn step(car: &mut Car, level: &level::Level) {
    let common = config::common::Common::test_default();
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
}

#[test]
fn driving_on_drivable_ground_emits_dust_from_the_real_tracks() {
    let level = drivable_level();
    let mut car = spawn(&level);
    for _ in 0..40 {
        step(&mut car, &level);
        car.tracks.drain();
    }

    let mut particles = Particles::new();
    let mut saw_track = false;
    for _ in 0..40 {
        step(&mut car, &level);
        for track in car.tracks.drain() {
            saw_track = true;
            particles.from_track(&track, &level);
        }
    }
    assert!(saw_track, "the car never laid a track");
    assert!(
        particles.particles().iter().any(|p| p.kind == Kind::Dust),
        "rolling on main terrain produced no dust"
    );

    let before: Vec<_> = particles.particles().iter().map(|p| p.pos).collect();
    particles.quant();
    let after: Vec<_> = particles.particles().iter().map(|p| p.pos).collect();
    assert!(
        before.iter().zip(&after).any(|(a, b)| a != b),
        "dust did not move"
    );
    for _ in 0..32 {
        particles.quant();
    }
    assert!(
        particles
            .particles()
            .iter()
            .all(|p| p.kind != Kind::Dust || p.age < p.life)
    );
    assert!(
        particles.is_empty() || particles.particles().iter().all(|p| p.kind != Kind::Dust),
        "dust outlived its lifetime"
    );
}

#[test]
fn crushing_a_beeb_pays_and_a_burst_follows() {
    let mut swarm = Swarm::new((128, 128));
    swarm.push(Insect::at(Vec3::new(20.0, 20.0, 8.0), 1));
    let crush = swarm.crush(Vec3::new(20.0, 20.0, 8.0), 12.0);
    assert_eq!(crush.awarded, TIER_PRICES[1]);
    assert_ne!(swarm.insects()[0].pos, Vec3::new(20.0, 20.0, 8.0));

    let mut particles = Particles::new();
    for at in crush.at {
        particles.from_crush(at);
    }
    assert!(particles.particles().iter().any(|p| p.kind == Kind::Burst));
}
