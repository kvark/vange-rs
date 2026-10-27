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

/// The server's 20 Hz tick: control, physics and contact per quantum.
fn ram(
    level: &level::Level,
    common: &config::common::Common,
    car: &CarPhysicsData,
    max_quant: f32,
    collide: bool,
) -> Run {
    let tick_dt = 0.05;
    let quant_input = physics::QuantClock::input_factor(max_quant, common);
    let mut clock = physics::QuantClock::default();
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
        // Whole quanta with contact after each, as `vangers-server` does.
        for _ in 0..clock.advance(physics_dt, max_quant) {
            if (20..40).contains(&tick) {
                run.a
                    .dynamo
                    .change_traction(quant_input * common.car.traction_incr);
            }
            for c in [&mut run.a, &mut run.b] {
                physics::step(
                    &mut c.dynamo,
                    &mut c.transform,
                    max_quant,
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
            }
            if !collide {
                continue;
            }
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

/// When car-car contact is resolved, relative to the whole `max_quant`
/// physics quanta both sides step (`physics::QuantClock`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Cadence {
    /// Once per call: per 20 Hz tick on the server, per render frame on
    /// the client (`predict_remote_contacts` before this fix).
    PerCall,
    /// After every quantum, on both sides.
    PerQuantum,
}

/// Remote car as the server had it after each quantum's `physics::step`,
/// before that quantum's contact: what an up-to-date ghost shows.
type GhostTrack = Vec<(space::Transform, Vec3, Vec3)>;

/// `a` holds W and jumps at `jump_at` into `b`, which is parked.
///
/// Server mode (`ghost: None`, `frame_dt` 0.05): both cars are simulated
/// and contact moves both. Client mode: only `a` is simulated, `b` is a
/// ghost replaying the server's track, and contact only moves `a`, as in
/// `predict_remote_contacts`. Returns `a`'s pose at every frame end (ms),
/// `b`'s track, and the number of impulses `a` took while airborne.
#[allow(clippy::too_many_arguments)]
fn jump_ram(
    start: (&Car, &Car),
    frame_dt: f32,
    seconds: f32,
    jump_at: f32,
    power: f32,
    cadence: Cadence,
    ghost: Option<&GhostTrack>,
    max_quant: f32,
    car: &CarPhysicsData,
    level: &level::Level,
    common: &config::common::Common,
) -> (Vec<(u32, Vec3)>, GhostTrack, usize) {
    let physics_dt = frame_dt * {
        let n = &common.nature;
        common.speed.standard_frame_rate as f32 * n.time_delta0 * n.num_calls_analysis as f32
    };
    let quant_input = physics::QuantClock::input_factor(max_quant, common);
    let wrap = (level.size.0 as f32, level.size.1 as f32);
    let frames = (seconds / frame_dt).round() as u32;
    let jump_frame = (jump_at / frame_dt).round() as u32 - 1;
    let copy = |c: &Car| Car {
        dynamo: Dynamo {
            linear_velocity: c.dynamo.linear_velocity,
            angular_velocity: c.dynamo.angular_velocity,
            ..Dynamo::default()
        },
        transform: c.transform,
    };
    let (mut a, mut b) = (copy(start.0), copy(start.1));
    let mut clock = physics::QuantClock::default();
    let mut quantum = 0usize;
    let mut track = GhostTrack::new();
    let mut out = Vec::new();
    let mut airborne_hits = 0;
    // Above its parked height: in the air, not driving into the side.
    let airborne_z = start.0.transform.disp.z + 8.0;
    let step = |c: &mut Car, jump| {
        physics::step(
            &mut c.dynamo,
            &mut c.transform,
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
    };
    let collide = |a: &mut Car, b: &mut Car, quantum: usize, hits: &mut usize| {
        let mut ghost_b;
        let b = match ghost {
            Some(track) => {
                let (transform, linear_velocity, angular_velocity) = track[quantum];
                ghost_b = Car {
                    dynamo: Dynamo {
                        linear_velocity,
                        angular_velocity,
                        ..Dynamo::default()
                    },
                    transform,
                };
                &mut ghost_b
            }
            None => b,
        };
        let contact = physics::collide_cars(
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
            Some(wrap),
            common,
        );
        if contact.is_some_and(|c| c.impulse > 0.0) && a.transform.disp.z > airborne_z {
            *hits += 1;
        }
    };
    for frame in 0..frames {
        let jump = (frame == jump_frame).then_some(power);
        let quanta = clock.advance(physics_dt, max_quant);
        assert!(quanta > 0, "test frames are longer than a quantum");
        for i in 0..quanta {
            let last = i + 1 == quanta;
            a.dynamo
                .change_traction(quant_input * common.car.traction_incr);
            step(&mut a, if last { jump } else { None });
            if ghost.is_none() {
                step(&mut b, None);
                track.push((
                    b.transform,
                    b.dynamo.linear_velocity,
                    b.dynamo.angular_velocity,
                ));
            }
            if cadence == Cadence::PerQuantum || last {
                collide(&mut a, &mut b, quantum, &mut airborne_hits);
            }
            quantum += 1;
        }
        let end_ms = ((frame + 1) as f32 * frame_dt * 1000.0).round() as u32;
        out.push((end_ms, a.transform.disp));
    }
    (out, track, airborne_hits)
}

/// Frame times whose frame ends line up with 20 Hz tick ends at 1 s.
const FRAME_DTS: [f32; 5] = [1.0 / 60.0, 1.0 / 40.0, 0.1, 0.125, 0.2];

fn worst_gap(server: &[(u32, Vec3)], client: &[(u32, Vec3)]) -> f32 {
    client
        .iter()
        .filter_map(|&(ms, pos)| {
            server
                .iter()
                .find(|&&(t, _)| t == ms)
                .map(|&(_, srv)| pos.distance(srv))
        })
        .fold(0.0, f32::max)
}

/// MP playtest: Alice jumped onto Bob and her prediction then sat ~12u off
/// her server pose. Same pose, same input, same jump, an up-to-date ghost:
/// the client predicted the contact once per render frame and the server
/// once per tick, and the half-overlap push-apart made that a different
/// path at every frame rate. Resolving contact after every quantum on both
/// sides makes the prediction exact.
#[test]
fn airborne_contact_prediction_matches_the_server() {
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
    // Centres 50 apart: a power-10 jump comes down on Bob's side.
    let gap_y = 50;
    let settle = |mut c: Car| {
        for _ in 0..400 {
            physics::step(
                &mut c.dynamo,
                &mut c.transform,
                max_quant,
                car,
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
        c
    };
    // Alice faces +Y (north) towards Bob, parked side-on.
    let a = settle(spawn(&level, 830, 3983, 0.0, car));
    let b = settle(spawn(&level, 830, 3983 + gap_y, FRAC_PI_2, car));
    let power = 10.0;
    let run = |frame_dt: f32, cadence: Cadence, ghost: Option<&GhostTrack>| {
        jump_ram(
            (&a, &b),
            frame_dt,
            3.0,
            1.0,
            power,
            cadence,
            ghost,
            max_quant,
            car,
            &level,
            &common,
        )
    };
    let mut worst = [0.0f32; 2];
    for (k, cadence) in [Cadence::PerCall, Cadence::PerQuantum]
        .into_iter()
        .enumerate()
    {
        let (server, track, hits) = run(0.05, cadence, None);
        assert!(hits > 0, "{cadence:?}: Alice should hit Bob in the air");
        for frame_dt in FRAME_DTS {
            let (client, _, _) = run(frame_dt, cadence, Some(&track));
            let gap = worst_gap(&server, &client);
            println!(
                "{cadence:?}, frame {:.0} ms: {hits} airborne impulses on the server, client gap {gap:.3}",
                frame_dt * 1000.0
            );
            worst[k] = worst[k].max(gap);
        }
    }
    assert!(
        worst[0] > 1.0,
        "per-call contact was expected to disagree, worst {}",
        worst[0]
    );
    assert!(worst[1] < 0.01, "per-quantum contact strayed {}", worst[1]);
}
