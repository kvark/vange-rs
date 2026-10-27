//! Car-vs-car contact on the multiplayer server's tick (needs game data;
//! skipped without it). Mirrors the `--duel-spawn` ram test on Fostral:
//! two mechos on flat sand, facing each other with a ~30 unit gap between
//! their bumpers (an OxidizeMonk contact box is ~41 units long, so centres
//! are 70 apart), the west one holds W for a second and coasts.

use glam::{Quat, Vec3};
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

struct Run {
    a: Car,
    b: Car,
    /// Deepest box overlap seen after contact resolution.
    max_depth: f32,
    contacts: usize,
    /// Rammer's world speed right before the first impulse and right after.
    speed_at_contact: Option<(f32, f32)>,
}

fn spawn(level: &level::Level, x: i32, y: i32, yaw: f32, car: &CarPhysicsData) -> Car {
    Car {
        dynamo: Dynamo::default(),
        transform: space::Transform {
            disp: Vec3::new(x as f32, y as f32, level.get((x, y)).high() + 5.0),
            rot: Quat::from_rotation_z(yaw),
            scale: car.scale,
        },
    }
}

fn world_speed(c: &Car) -> f32 {
    (c.transform.rot * c.dynamo.linear_velocity).truncate().length()
}

/// The server's 20 Hz tick: control, sub-stepped physics, then contact.
fn ram(
    level: &level::Level,
    common: &config::common::Common,
    car: &CarPhysicsData,
    max_quant: f32,
    collide: bool,
) -> Run {
    let tick_dt = 0.05;
    let input_factor = tick_dt / config::common::MAIN_LOOP_TIME;
    let physics_dt = tick_dt * {
        let n = &common.nature;
        common.speed.standard_frame_rate as f32 * n.time_delta0 * n.num_calls_analysis as f32
    };
    let (cx, cy) = (590, 4151);
    let mut run = Run {
        a: spawn(level, cx - 35, cy, -FRAC_PI_2, car),
        b: spawn(level, cx + 35, cy, FRAC_PI_2, car),
        max_depth: 0.0,
        contacts: 0,
        speed_at_contact: None,
    };
    let wrap = (level.size.0 as f32, level.size.1 as f32);
    // 1 s settle, 1 s of W, 8 s coast.
    for tick in 0..200 {
        if (20..40).contains(&tick) {
            run.a
                .dynamo
                .change_traction(input_factor * common.car.traction_incr);
        }
        for c in [&mut run.a, &mut run.b] {
            let mut remaining = physics_dt;
            while remaining > 0.0 {
                let dt = remaining.min(max_quant);
                physics::step(
                    &mut c.dynamo,
                    &mut c.transform,
                    dt,
                    car,
                    level,
                    common,
                    1.0,
                    0.0,
                    None,
                    0.0,
                    None,
                    None,
                );
                remaining -= dt;
            }
        }
        if collide {
            let before = world_speed(&run.a);
            let contact = physics::collide_cars(
                physics::CarBody {
                    dynamo: &mut run.a.dynamo,
                    transform: &mut run.a.transform,
                    car,
                },
                physics::CarBody {
                    dynamo: &mut run.b.dynamo,
                    transform: &mut run.b.transform,
                    car,
                },
                Some(wrap),
                common,
            );
            if std::env::var_os("CAR_CONTACT_TRACE").is_some() {
                println!(
                    "t{tick} a ({:.1},{:.1},{:.1}) v{:.2} b ({:.1},{:.1}) v{:.2} {:?}",
                    run.a.transform.disp.x,
                    run.a.transform.disp.y,
                    run.a.transform.disp.z,
                    world_speed(&run.a),
                    run.b.transform.disp.x,
                    run.b.transform.disp.y,
                    world_speed(&run.b),
                    contact.map(|c| (c.impulse, c.depth)),
                );
            }
            if let Some(c) = contact {
                if c.impulse > 0.0 {
                    run.contacts += 1;
                    run.speed_at_contact
                        .get_or_insert((before, world_speed(&run.a)));
                }
                if run.contacts > 0 {
                    run.max_depth = run.max_depth.max(c.depth);
                }
            }
        }
    }
    run
}

#[test]
fn duel_ram_bumps_the_parked_car() {
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

    let ghost = ram(&level, &common, car, max_quant, false);
    println!(
        "no contact: a x {:.1}, b x {:.1}",
        ghost.a.transform.disp.x, ghost.b.transform.disp.x
    );
    assert!(
        ghost.a.transform.disp.x > ghost.b.transform.disp.x + 20.0,
        "the repro should drive straight through"
    );

    let b_start = ram(&level, &common, car, max_quant, false).b.transform.disp;
    let run = ram(&level, &common, car, max_quant, true);
    let (before, after) = run.speed_at_contact.expect("the cars never touched");
    println!(
        "contact: {} impulses, rammer {:.2} -> {:.2}, a x {:.1}, b x {:.1} (moved {:.1}), max depth {:.2}",
        run.contacts,
        before,
        after,
        run.a.transform.disp.x,
        run.b.transform.disp.x,
        run.b.transform.disp.x - b_start.x,
        run.max_depth,
    );
    assert!(after < before * 0.5, "rammer should slow at the bump");
    assert!(
        run.b.transform.disp.x - b_start.x > 5.0,
        "parked car should be shoved east"
    );
    assert!(
        run.a.transform.disp.x < run.b.transform.disp.x,
        "rammer must not pass through"
    );
    assert!(run.max_depth < 3.0, "cars interpenetrate");
}
