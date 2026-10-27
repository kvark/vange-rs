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
    (c.transform.rot * c.dynamo.linear_velocity)
        .truncate()
        .length()
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
    // Centres 52 apart: a power-10 jump comes down on Bob's side. The
    // airborne hit is a graze, so the last bit of the jump decides it: 50
    // missed once the physics math went through `libm` and scalar glam.
    let gap_y = 52;
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

/// One car in a `WorldState`, as the rammed client receives it.
#[derive(Clone, Copy)]
struct Snapshot {
    transform: space::Transform,
    linear_velocity: Vec3,
    angular_velocity: Vec3,
    traction: f32,
    rudder: f32,
    bumps: u32,
}

fn snapshot(c: &Car, bumps: u32) -> Snapshot {
    Snapshot {
        transform: c.transform,
        linear_velocity: c.dynamo.linear_velocity,
        angular_velocity: c.dynamo.angular_velocity,
        traction: c.dynamo.traction,
        rudder: c.dynamo.rudder,
        bumps,
    }
}

/// Server side of the duel ram: `a` holds W into parked `b`, quantum by
/// quantum with contact after each, counting `b`'s felt bumps like
/// `vangers-server`. Returns per-tick snapshots of both.
fn ram_snapshots(
    level: &level::Level,
    common: &config::common::Common,
    car: &CarPhysicsData,
    max_quant: f32,
    ticks: u32,
) -> Vec<(Snapshot, Snapshot)> {
    let physics_dt = 0.05 * {
        let n = &common.nature;
        common.speed.standard_frame_rate as f32 * n.time_delta0 * n.num_calls_analysis as f32
    };
    let quant_input = physics::QuantClock::input_factor(max_quant, common);
    let (cx, cy) = (590, 4151);
    let mut a = spawn(level, cx - 35, cy, -FRAC_PI_2, car);
    let mut b = spawn(level, cx + 35, cy, FRAC_PI_2, car);
    let mut clock = physics::QuantClock::default();
    let wrap = (level.size.0 as f32, level.size.1 as f32);
    let mut bumps = 0u32;
    let mut out = Vec::new();
    for tick in 0..ticks {
        for _ in 0..clock.advance(physics_dt, max_quant) {
            // Settle 1 s, then drive.
            if tick >= 20 {
                a.dynamo
                    .change_traction(quant_input * common.car.traction_incr);
            }
            for c in [&mut a, &mut b] {
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
            if contact.is_some_and(|c| c.delta_v[1] >= physics::FELT_BUMP_DV) {
                bumps += 1;
            }
        }
        out.push((snapshot(&a, 0), snapshot(&b, bumps)));
    }
    out
}

/// The rammed (parked) car's own client at `frame_dt`: predicts its car in
/// quanta with contact against the rammer's ghost (latest `WorldState`),
/// receives each tick's snapshot `latency_ticks` late and reconciles
/// against it. Returns its world velocity at the tick ends its frames end
/// on, keyed by tick, and the hard-snap count.
fn rammed_client(
    server: &[(Snapshot, Snapshot)],
    frame_dt: f32,
    latency_ticks: usize,
    adopt_bumps: bool,
    level: &level::Level,
    common: &config::common::Common,
    car: &CarPhysicsData,
    max_quant: f32,
) -> (Vec<(usize, Vec3)>, usize) {
    let physics_dt = frame_dt * {
        let n = &common.nature;
        common.speed.standard_frame_rate as f32 * n.time_delta0 * n.num_calls_analysis as f32
    };
    let wrap = (level.size.0 as f32, level.size.1 as f32);
    let first = server[0].1;
    let mut me = Car {
        dynamo: Dynamo {
            linear_velocity: first.linear_velocity,
            angular_velocity: first.angular_velocity,
            ..Dynamo::default()
        },
        transform: first.transform,
    };
    let mut clock = physics::QuantClock::default();
    let mut seen_bumps = first.bumps;
    let mut reconciled = 0usize;
    let mut snaps = 0;
    let mut out = Vec::new();
    let frames = (server.len() as f32 * 0.05 / frame_dt).round() as u32;
    for frame in 0..frames {
        let now_ms = ((frame + 1) as f32 * frame_dt * 1000.0).round() as usize;
        // Tick `t` is sent at its end, (t+1)*50 ms, and arrives
        // `latency_ticks` later.
        let delivered = (now_ms / 50)
            .saturating_sub(latency_ticks)
            .min(server.len());
        let ghost = server[delivered.saturating_sub(1)].0;
        for _ in 0..clock.advance(physics_dt, max_quant) {
            physics::step(
                &mut me.dynamo,
                &mut me.transform,
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
            let mut ghost_transform = ghost.transform;
            let mut ghost_dynamo = Dynamo {
                linear_velocity: ghost.linear_velocity,
                angular_velocity: ghost.angular_velocity,
                ..Dynamo::default()
            };
            physics::collide_cars(
                physics::CarBody {
                    dynamo: &mut me.dynamo,
                    transform: &mut me.transform,
                    car,
                },
                physics::CarBody {
                    dynamo: &mut ghost_dynamo,
                    transform: &mut ghost_transform,
                    car,
                },
                Some(wrap),
                common,
            );
        }
        // Newest snapshot only, as the client does.
        if delivered > reconciled {
            reconciled = delivered;
            let s = server[delivered - 1].1;
            let bumped = adopt_bumps && s.bumps != seen_bumps;
            seen_bumps = s.bumps;
            let kind = physics::reconcile_local_car(
                &mut me.transform,
                &mut me.dynamo,
                &physics::ServerCar {
                    transform: s.transform,
                    linear_velocity: s.linear_velocity,
                    angular_velocity: s.angular_velocity,
                    traction: s.traction,
                    rudder: s.rudder,
                    bumped,
                },
            );
            if kind == physics::Reconcile::Snap {
                snaps += 1;
            }
        }
        if now_ms % 50 == 0 {
            out.push((
                now_ms / 50 - 1,
                me.transform.rot * me.dynamo.linear_velocity,
            ));
        }
    }
    (out, snaps)
}

/// A parked car rammed by another player. Its owner only predicts the hit
/// against the rammer's ghost, which arrives after the server already
/// slowed the rammer, so the local bump is a fraction of the real one
/// (1.35 against 5.19 here) and the blend-only reconcile never copies the
/// velocity. With the server's bump counter the client adopts the server
/// velocity on the first snapshot after the bump.
#[test]
fn rammed_client_takes_the_server_bump_within_two_ticks() {
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
    let server = ram_snapshots(&level, &common, car, max_quant, 120);
    let bump_tick = server
        .iter()
        .position(|(_, b)| b.bumps > 0)
        .expect("the rammer never hit the parked car");
    let hit = (server[bump_tick].1.transform.rot * server[bump_tick].1.linear_velocity).length();
    println!("server: first felt bump at tick {bump_tick}, parked car speed {hit:.2}");
    assert!(hit > physics::FELT_BUMP_DV);
    // The client cannot know of the bump before its snapshot arrives, so
    // compare it with the server `latency` ticks earlier, at the client's
    // first tick-end sample once the bump has arrived. Later bumps of the
    // slow ram the client predicts itself, ahead of that lagged copy.
    let first_error = |client: &[(usize, Vec3)], latency: usize| {
        let &(t, v) = client
            .iter()
            .find(|(t, _)| *t >= bump_tick + latency)
            .expect("the bump never arrived");
        let s = server[t - latency].1;
        v.distance(s.transform.rot * s.linear_velocity)
    };
    let mut worst_old = 0.0f32;
    for latency in [1, 2, 3] {
        for frame_dt in FRAME_DTS {
            let run = |adopt| {
                rammed_client(
                    &server, frame_dt, latency, adopt, &level, &common, car, max_quant,
                )
            };
            let (old, old_snaps) = run(false);
            let (new, new_snaps) = run(true);
            let (old_err, new_err) = (first_error(&old, latency), first_error(&new, latency));
            println!(
                "latency {latency} ticks, frame {:.0} ms: velocity error once the bump arrives {old_err:.2} without adopting ({old_snaps} snaps), {new_err:.2} with ({new_snaps} snaps)",
                frame_dt * 1000.0
            );
            assert!(new_err < 0.1 * hit, "adopted bump off by {new_err}");
            worst_old = worst_old.max(old_err);
            assert!(
                new_snaps <= old_snaps,
                "adopting the bump must not add snaps"
            );
        }
    }
    assert!(
        worst_old > 0.5 * hit,
        "the blend-only client was expected to miss most of the hit"
    );
}
