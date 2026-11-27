//! Car-car contact must not create energy: the impulse is elastic at
//! `RESTITUTION` 1, so the two cars' kinetic energy (translation plus
//! rotation) after a bump is at most what it was before.

use glam::{Mat3, Quat, Vec3};
use rand::{Rng, SeedableRng};
use std::f32::consts::FRAC_PI_2;
use vangers::{
    config, level,
    physics::{self, CarPhysicsData, Dynamo},
    space,
};

struct Car {
    dynamo: Dynamo,
    transform: space::Transform,
}

/// Kinetic energy as `collide_cars` sees the body: mass from the density
/// and volume, inertia per unit mass from the jacobi.
fn energy(car: &CarPhysicsData, common: &config::common::Common, c: &Car) -> f32 {
    let s = c.transform.scale;
    let mass = common.nature.density * car.body_physics.volume * s * s;
    let inertia =
        Mat3::from_cols_array_2d(&car.body_physics.jacobi) * (s * s / car.body_physics.volume);
    let w = c.dynamo.angular_velocity;
    0.5 * mass * (c.dynamo.linear_velocity.length_squared() + w.dot(inertia * w))
}

fn collide(
    car: &CarPhysicsData,
    common: &config::common::Common,
    a: &mut Car,
    b: &mut Car,
    wrap: Option<(f32, f32)>,
) -> Option<physics::CarContact> {
    physics::collide_cars(
        physics::CarBody {
            dynamo: &mut a.dynamo,
            transform: &mut a.transform,
            car,
        },
        physics::CarBody {
            dynamo: &mut b.dynamo,
            transform: &mut b.transform,
            car,
        },
        wrap,
        common,
    )
}

/// Overlapping cars at random orientations, closing at random speeds and
/// spins. The original's `-2 K⁻¹ u0` gained up to 11% here.
#[test]
fn bumps_never_add_energy() {
    let car = CarPhysicsData::test_default();
    let common = config::common::Common::test_default();
    let reach = car.bbox.radius * car.scale;
    let mut rng = rand::rngs::StdRng::seed_from_u64(7);
    let mut v3 = move |s: f32| {
        Vec3::new(
            rng.gen_range(-s..s),
            rng.gen_range(-s..s),
            rng.gen_range(-s..s),
        )
    };
    let spawn = |at: Vec3, v3: &mut dyn FnMut(f32) -> Vec3| {
        let dynamo = Dynamo {
            linear_velocity: v3(10.0),
            angular_velocity: v3(0.5),
            ..Default::default()
        };
        let axis = v3(1.0).normalize_or(Vec3::Z);
        let angle = v3(3.0).x;
        Car {
            dynamo,
            transform: space::Transform {
                disp: at,
                rot: Quat::from_axis_angle(axis, angle),
                scale: car.scale,
            },
        }
    };
    let mut bumps = 0;
    let mut worst = f32::MIN;
    for _ in 0..20_000 {
        let mut a = spawn(Vec3::ZERO, &mut v3);
        let at = v3(reach);
        let mut b = spawn(at, &mut v3);
        let before = energy(&car, &common, &a) + energy(&car, &common, &b);
        if collide(&car, &common, &mut a, &mut b, None).is_some_and(|c| c.impulse > 0.0) {
            bumps += 1;
            let after = energy(&car, &common, &a) + energy(&car, &common, &b);
            worst = worst.max((after - before) / before);
        }
    }
    println!("{bumps} bumps, worst energy change {:+.3}%", worst * 100.0);
    assert!(bumps > 1000, "too few bumps to tell: {bumps}");
    assert!(worst < 1e-3, "a bump added {:.2}% energy", worst * 100.0);
}

/// Holding W into a parked car on Khox's flat duel ground (needs game
/// data, skipped without it): after the first few bounces the rammer
/// pushes the other car along in steady contact, and no bump on the way
/// adds energy.
#[test]
fn held_throttle_pushes() {
    let Some(settings) = config::Settings::try_load("config/settings.ron") else {
        eprintln!("skipping: no config/settings.ron with game data");
        return;
    };
    let ini = settings.data_path.join("thechain/khox/world.ini");
    if !ini.is_file() {
        eprintln!("skipping: no Khox data");
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
    let input_factor = physics::QuantClock::input_factor(max_quant, &common);
    let n = &common.nature;
    let physics_dt = 0.05
        * common.speed.standard_frame_rate as f32
        * n.time_delta0
        * n.num_calls_analysis as f32;
    let wrap = (level.size.0 as f32, level.size.1 as f32);
    // `vangers-server --duel-spawn --duel-center 410,195`.
    let spawn = |x: i32, yaw: f32| Car {
        dynamo: Dynamo::default(),
        transform: space::Transform {
            disp: Vec3::new(x as f32, 195.0, level.get((x, 195)).high() + 5.0),
            rot: Quat::from_rotation_z(yaw),
            scale: car.scale,
        },
    };
    let mut a = spawn(375, -FRAC_PI_2);
    let mut b = spawn(445, FRAC_PI_2);
    let w = physics::prediction::QuantInput {
        motor: 1.0,
        ..Default::default()
    };
    let idle = physics::prediction::QuantInput::default();
    let mut clock = physics::QuantClock::default();
    let (mut q, mut last_bump, mut worst) = (0u32, None, f32::MIN);
    let mut gaps = Vec::new();
    // 1 s settle, then W for 4 s: the push crosses the flat sand.
    for tick in 0..100 {
        for _ in 0..clock.advance(physics_dt, max_quant) {
            q += 1;
            let a_input = if tick >= 20 { &w } else { &idle };
            for (c, input) in [(&mut a, a_input), (&mut b, &idle)] {
                physics::prediction::step_input(
                    &mut c.dynamo,
                    &mut c.transform,
                    input,
                    max_quant,
                    input_factor,
                    car,
                    &level,
                    &common,
                    None,
                );
            }
            let before = energy(car, &common, &a) + energy(car, &common, &b);
            let contact = collide(car, &common, &mut a, &mut b, Some(wrap));
            if contact.is_some_and(|c| c.impulse > 0.0) {
                let after = energy(car, &common, &a) + energy(car, &common, &b);
                worst = worst.max((after - before) / before);
                if let Some(last) = last_bump {
                    gaps.push(q - last);
                }
                last_bump = Some(q);
            }
        }
    }
    // The last 100 bumps: a tenth of a second apart at most (~9 quanta),
    // where bounces apart took 0.3 s and more.
    let late = &gaps[gaps.len().saturating_sub(100)..];
    println!(
        "{} bumps, worst energy change {:+.3}%, gaps {:?}, a x {:.1}, b x {:.1}",
        gaps.len() + 1,
        worst * 100.0,
        &gaps[..gaps.len().min(12)],
        a.transform.disp.x,
        b.transform.disp.x,
    );
    assert!(worst < 1e-3, "a bump added {:.2}% energy", worst * 100.0);
    assert!(gaps.len() >= 100, "the rammer should keep pushing");
    assert!(
        late.iter().all(|&g| g <= 10),
        "the push should be steady contact, not bounces: {late:?}"
    );
    assert!(
        b.transform.disp.x > 520.0,
        "the parked car should be pushed east"
    );
}
