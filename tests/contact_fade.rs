//! A hard shove with turbo under latency, simulated end to end in-process:
//! the local car rams a parked second car with turbo, and that car's driver
//! fights back with turbo the moment it is hit, flipping between reverse
//! and forward. The client only learns of each flip a round trip later
//! (its ghost of the other car replays the last control it heard of), so
//! the contact plays out differently on the server than in the prediction
//! and a reconcile can move the car over 100 units. The client must fade
//! such a contact correction out over a few frames rather than show it at
//! once, as the old 50-unit teleport rule did.
//!
//! Server: 20 Hz ticks, `QuantClock` quanta, the local car's numbered
//! inputs through an `InputQueue`, the other car on its own control,
//! contact after every quantum. Client: `physics::client::ClientSync` and
//! `RemoteCar` (as the native game and the web build run them), 60 fps
//! frames (and a few slow-frame cases), a fixed latency each way. The test
//! level's lumpy terrain makes the fight chaotic, but the run is
//! deterministic.

use glam::{Quat, Vec2, Vec3};
use std::collections::{HashMap, VecDeque};
use vangers::{
    config::{self, settings},
    level,
    physics::{
        self, CarPhysicsData, Dynamo, ServerCar,
        client::{ClientSync, LocalCar, LocalSnapshot, RemoteCar, Sim, SyncEvent},
        prediction::{InputQueue, QuantInput, Reconciled, RenderOffset, fold_home},
    },
    space,
};

const TICK: f32 = 0.05;
const FRAME: f32 = 1.0 / 60.0;
const SECONDS: f32 = 5.0;
/// How long the parked car's driver fights back after the first hit.
const SHOVE_FOR: f32 = 2.5;
/// How far ahead of the rammer the other car is parked, one run per
/// distance: the fight is chaotic, and floating point differs a little
/// between platforms, so the test asks for the fade to hold in every run
/// and for enough of them to be hard shoves, not for one exact shove.
const PARK_AHEAD: [f32; 12] = [
    46.0, 50.0, 54.0, 58.0, 60.0, 64.0, 68.0, 72.0, 76.0, 80.0, 84.0, 88.0,
];
/// Below this the lumpy test level has let a car through; that run is
/// over (it falls forever) and does not count.
const FELL_THROUGH_Z: f32 = -20.0;

struct World {
    level: level::Level,
    common: config::common::Common,
    car: CarPhysicsData,
    max_quant: f32,
}

impl World {
    fn new() -> Self {
        let level = level::load(
            &level::LevelConfig::new_test(),
            &settings::Geometry::default(),
        );
        let mut common = config::common::Common::test_default();
        common.nature.time_delta0 = 0.1286;
        common.nature.num_calls_analysis = 5;
        World {
            level,
            common,
            car: CarPhysicsData::test_default(),
            max_quant: 0.1,
        }
    }

    fn physics_per_sec(&self) -> f32 {
        let n = &self.common.nature;
        self.common.speed.standard_frame_rate as f32 * n.time_delta0 * n.num_calls_analysis as f32
    }

    fn sim(&self) -> Sim<'_> {
        Sim {
            level: &self.level,
            common: &self.common,
            max_quant: self.max_quant,
        }
    }

    fn step(&self, car: &mut Car, input: &QuantInput) {
        physics::prediction::step_input(
            &mut car.dynamo,
            &mut car.transform,
            input,
            self.max_quant,
            physics::QuantClock::input_factor(self.max_quant, &self.common),
            &self.car,
            &self.level,
            &self.common,
            None,
        );
        fold_home(&mut car.transform.disp, self.level.period());
    }

    /// Contact; returns whether the cars touched and whether `a` felt it.
    fn collide(&self, a: &mut Car, b: &mut Car) -> (bool, bool) {
        let wrap = (self.level.size.0 as f32, self.level.size.1 as f32);
        let felt = physics::collide_cars(
            physics::CarBody {
                dynamo: &mut a.dynamo,
                transform: &mut a.transform,
                car: &self.car,
            },
            physics::CarBody {
                dynamo: &mut b.dynamo,
                transform: &mut b.transform,
                car: &self.car,
            },
            Some(wrap),
            &self.common,
        )
        .map(|c| c.delta_v[0] >= physics::FELT_BUMP_DV);
        fold_home(&mut a.transform.disp, self.level.period());
        fold_home(&mut b.transform.disp, self.level.period());
        (felt.is_some(), felt == Some(true))
    }

    fn settle(&self, spawn: space::Transform) -> Car {
        let mut car = Car {
            transform: spawn,
            dynamo: Dynamo::default(),
        };
        let quanta = (2.0 * self.physics_per_sec() / self.max_quant) as u32;
        for _ in 0..quanta {
            self.step(&mut car, &QuantInput::default());
        }
        car.dynamo = Dynamo::default();
        car
    }
}

#[derive(Clone, Copy)]
struct Car {
    transform: space::Transform,
    dynamo: Dynamo,
}

impl Car {
    fn server_car(&self) -> ServerCar {
        ServerCar {
            transform: self.transform,
            linear_velocity: self.dynamo.linear_velocity,
            angular_velocity: self.dynamo.angular_velocity,
            traction: self.dynamo.traction,
            rudder: self.dynamo.rudder,
            bumped: false,
        }
    }
}

/// The rammer: full motor and turbo, then brakes.
fn rammer_input(t: f32) -> QuantInput {
    QuantInput {
        motor: if t < 3.0 { 1.0 } else { 0.0 },
        turbo: t < 3.0,
        brake: t >= 3.0,
        ..QuantInput::default()
    }
}

/// The parked car: still until the rammer hits it, then its driver fights
/// back with turbo for [`SHOVE_FOR`] seconds, flipping between reverse and
/// forward and wrenching the wheel every few frames, which the rammer's
/// client only learns a round trip late.
fn shover_input(since_hit: Option<f32>) -> QuantInput {
    let Some(t) = since_hit.filter(|&t| t < SHOVE_FOR) else {
        return QuantInput::default();
    };
    // A fixed, irregular pattern rather than an RNG, so runs are repeatable.
    const PATTERN: [(f32, f32); 7] = [
        (-1.0, 1.0),
        (1.0, -1.0),
        (-1.0, -1.0),
        (-1.0, 0.0),
        (1.0, 1.0),
        (-1.0, 1.0),
        (1.0, 0.0),
    ];
    let (motor, rudder) = PATTERN[(t / 0.15) as usize % PATTERN.len()];
    QuantInput {
        motor,
        rudder,
        turbo: true,
        ..QuantInput::default()
    }
}

struct Snapshot {
    a: Car,
    ack: u32,
    bumps: u32,
    b: Car,
    b_control: QuantInput,
}

struct Pipe<T> {
    queue: VecDeque<(f32, T)>,
    latency: f32,
}

impl<T> Pipe<T> {
    fn send(&mut self, now: f32, item: T) {
        self.queue.push_back((now + self.latency, item));
    }

    fn recv(&mut self, now: f32) -> Vec<T> {
        let mut out = Vec::new();
        while self.queue.front().is_some_and(|&(at, _)| at <= now) {
            out.push(self.queue.pop_front().unwrap().1);
        }
        out
    }
}

/// `d` with whole level periods taken out of its XY.
fn unfold(d: Vec3, period: Option<Vec2>) -> Vec3 {
    let p = period.unwrap_or(Vec2::ZERO);
    let fold = |d: f32, p: f32| if p > 0.0 { d - (d / p).round() * p } else { d };
    Vec3::new(fold(d.x, p.x), fold(d.y, p.y), d.z)
}

#[derive(Debug, Default)]
struct Outcome {
    /// Largest physics move of one reconcile.
    max_moved: f32,
    /// Largest move of a reconcile flagged as contact.
    max_contact_moved: f32,
    /// Largest visible per-frame correction of the drawn car.
    max_visible: f32,
    /// Corrections shown at once.
    snaps: u32,
    bumps: u32,
    /// Car contacts on the server, felt or not.
    contacts: u32,
    /// Physics move of the last reconcile (the prediction settles).
    last_moved: f32,
    /// Contact corrections under [`RenderOffset::CONTACT_TELEPORT`] shown
    /// at once anyway.
    contact_snaps: u32,
    /// A car fell through the test level; the run stopped there.
    fell_through: bool,
}

fn run(world: &World, park: f32, latency_ticks: u32, frame: f32, fade: bool) -> Outcome {
    let (x, y) = (world.level.size.0 / 4, world.level.size.1 / 4);
    let spawn = space::Transform {
        disp: Vec3::new(x as f32, y as f32, world.level.get((x, y)).high() + 5.0),
        rot: Quat::IDENTITY,
        scale: 1.0,
    };
    let start = world.settle(spawn);
    // Park the other car straight ahead.
    let mut parked = start.transform;
    parked.disp.y += park;
    let (px, py) = (parked.disp.x as i32, parked.disp.y as i32);
    parked.disp.z = world.level.get((px, py)).high() + 5.0;
    let b_start = world.settle(parked);

    let mut a = start;
    let mut b = b_start;
    let mut queue = InputQueue::default();
    let mut server_clock = physics::QuantClock::default();
    let mut bumps = 0u32;
    let mut contacts = 0u32;
    let mut hit_at = None;

    let mut sync = ClientSync {
        synced: true,
        server_bumps: Some(0),
        no_contact_fade: !fade,
        ..ClientSync::default()
    };
    let mut car = start;
    let mut remotes: HashMap<u32, RemoteCar> = HashMap::new();
    let latency = latency_ticks as f32 * TICK;
    let mut up = Pipe::<(u32, u32, QuantInput)> {
        queue: VecDeque::new(),
        latency,
    };
    let mut down = Pipe::<Snapshot> {
        queue: VecDeque::new(),
        latency,
    };
    let period = Some(world.level.period());
    let sim = world.sim();
    let mut out = Outcome::default();
    let mut now = 0.0f32;
    let mut next_tick = TICK;
    let mut drawn_prev = car.transform.disp;
    while now < SECONDS + 2.0 * latency + 1.0 {
        let frame_end = now + frame;
        while next_tick <= frame_end {
            for (first, count, input) in up.recv(next_tick) {
                queue.push(first, count, input);
            }
            let quanta = server_clock.advance(TICK * world.physics_per_sec(), world.max_quant);
            let a_quanta = queue.budget(quanta).total();
            let b_control = shover_input(hit_at.map(|at| next_tick - at));
            for i in 0..quanta.max(a_quanta) {
                if i < a_quanta {
                    let input = queue.pop();
                    world.step(&mut a, &input);
                }
                if i < quanta {
                    world.step(&mut b, &b_control);
                }
                let (touched, felt) = world.collide(&mut a, &mut b);
                let fell = |c: &Car| {
                    !(c.transform.disp.is_finite() && c.transform.disp.z > FELL_THROUGH_Z)
                };
                if fell(&a) || fell(&b) {
                    out.fell_through = true;
                    return out;
                }
                contacts += u32::from(touched);
                if touched && hit_at.is_none() {
                    hit_at = Some(next_tick);
                }
                bumps += u32::from(felt);
            }
            down.send(
                next_tick,
                Snapshot {
                    a,
                    ack: queue.ack(),
                    bumps,
                    b,
                    b_control,
                },
            );
            next_tick += TICK;
        }
        now = frame_end;

        // Client frame: the newest snapshot (remote car first), then the
        // frame's quanta, then the drawn pose.
        if let Some(snap) = down.recv(now).pop() {
            let remote = remotes
                .entry(2)
                .or_insert_with(|| RemoteCar::new(world.car.clone()));
            remote.control = snap.b_control;
            remote.on_snapshot(&snap.b.server_car(), period);
            let (event, _) = sync.on_snapshot(
                &LocalSnapshot {
                    car: snap.a.server_car(),
                    ack: snap.ack,
                    bumps: snap.bumps,
                },
                LocalCar {
                    transform: &mut car.transform,
                    dynamo: &mut car.dynamo,
                    phys: &world.car,
                },
                &remotes,
                sim,
                false,
            );
            if let SyncEvent::Reconciled {
                outcome: Reconciled::Replayed(_) | Reconciled::Ahead,
                moved,
                shown_at_once,
                contact,
            } = event
            {
                out.max_moved = out.max_moved.max(moved);
                out.last_moved = moved;
                if contact {
                    out.max_contact_moved = out.max_contact_moved.max(moved);
                }
                if shown_at_once {
                    out.snaps += 1;
                    if contact && moved <= RenderOffset::CONTACT_TELEPORT {
                        out.contact_snaps += 1;
                    }
                }
            }
        }
        let after_reconcile = car.transform.disp;
        let held = rammer_input(now);
        let quanta = sync.due_quanta(frame * world.physics_per_sec(), world.max_quant);
        let first = sync.prediction.next();
        for i in 0..quanta {
            let input = ClientSync::frame_input(&held, i, quanta);
            sync.step_quantum(
                LocalCar {
                    transform: &mut car.transform,
                    dynamo: &mut car.dynamo,
                    phys: &world.car,
                },
                &input,
                &remotes,
                sim,
                None,
            );
        }
        if quanta > 0 {
            up.send(now, (first, quanta, held));
        }
        sync.decay(frame);
        let drawn = sync.drawn(&car.transform).disp;
        // What the drawn car moved for any reason but this frame's own
        // physics steps; folds move it by whole periods, which don't show.
        let visible = unfold(
            (drawn - drawn_prev) - (car.transform.disp - after_reconcile),
            period,
        );
        out.max_visible = out.max_visible.max(visible.length());
        drawn_prev = drawn;
        for r in remotes.values_mut() {
            r.interpolate(frame);
        }
    }
    out.bumps = bumps;
    out.contacts = contacts;
    out
}

#[test]
fn hard_shove_with_turbo_fades_instead_of_snapping() {
    let world = World::new();
    let (mut runs, mut big) = (0, 0);
    for park in PARK_AHEAD {
        for (latency, frame) in [
            (2, FRAME),
            (4, FRAME),
            (6, FRAME),
            (8, FRAME),
            (2, 0.25),
            (4, 0.5),
        ] {
            let old = run(&world, park, latency, frame, false);
            let new = run(&world, park, latency, frame, true);
            println!(
                "park {park}, latency {latency} ticks, frame {frame}:\n  no fade {old:?}\n  fade    {new:?}"
            );
            if new.fell_through {
                continue;
            }
            runs += 1;
            assert_eq!(
                new.contact_snaps, 0,
                "a contact correction shown at once: {new:?}"
            );
            // The fade changes only what is drawn, never the physics.
            assert_eq!(new.max_moved, old.max_moved);
            if old.contact_snaps > 0 {
                // The old rule showed it at once; the fade spreads it out over
                // the frames of `CONTACT_FADE` (at 4 fps that is about one).
                // Shoves piling up past `CONTACT_TELEPORT` still show the
                // excess, so only some runs cut the snap by 4x.
                assert!(new.max_visible <= old.max_visible, "{new:?} vs {old:?}");
                if frame * 8.0 <= RenderOffset::CONTACT_FADE
                    && new.max_visible < 0.25 * old.max_visible
                {
                    big += 1;
                }
            }
        }
    }
    println!("{runs} runs on the level, {big} hard shoves faded");
    assert!(runs >= 36, "most runs fell through the level: {runs}");
    assert!(
        big >= 1,
        "too few shoves hard enough to snap the old way: {big}"
    );
}
