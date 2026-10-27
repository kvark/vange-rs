//! Multiplayer lag compensation, simulated end to end in-process.
//!
//! A server (20 Hz ticks, `QuantClock` quanta, per-player input queue,
//! car contact after every quantum) and a client (render frames of
//! 17-200 ms, the same quanta, prediction history, render offset) talk over
//! a simulated link with 1-5 ticks of latency each way plus jitter. The
//! "old" mode is the pre-prediction code: the server drives the car with
//! the latest control it has and the client reconciles its *present* pose
//! against the newest snapshot (`physics::reconcile_local_car`).
//!
//! The metric is the visible per-frame correction: how far the drawn car
//! moves in a frame for any reason but its own physics step (the reconcile
//! blend or snap in old mode; the render-offset decay in new mode).
//! Scenarios: high-speed driving, a jump, a ram into a second car, on the
//! built-in test level (CI) and on Fostral when the game data is there.

use glam::{Quat, Vec3};
use std::collections::VecDeque;
use vangers::{
    config::{self, settings},
    level,
    physics::{
        self, CarPhysicsData, Dynamo, ServerCar,
        prediction::{
            Ghost, InputQueue, Prediction, QuantInput, Reconciled, RenderOffset, ServerSnapshot,
            fold_home,
        },
    },
    space,
};

const TICK: f32 = 0.05;

struct World {
    name: &'static str,
    level: level::Level,
    common: config::common::Common,
    car: CarPhysicsData,
    max_quant: f32,
    jump_power: f32,
    spawn: space::Transform,
    /// Ram: park the other car where the rammer alone is at this time
    /// (the test level's hills and bridges make "straight ahead" miss);
    /// `None`: 70u straight ahead.
    ram_at: Option<f32>,
}

impl World {
    fn physics_per_sec(&self) -> f32 {
        let n = &self.common.nature;
        self.common.speed.standard_frame_rate as f32 * n.time_delta0 * n.num_calls_analysis as f32
    }

    fn input_factor(&self) -> f32 {
        physics::QuantClock::input_factor(self.max_quant, &self.common)
    }

    fn step(&self, car: &mut Car, input: &QuantInput) {
        physics::prediction::step_input(
            &mut car.dynamo,
            &mut car.transform,
            input,
            self.max_quant,
            self.input_factor(),
            &self.car,
            &self.level,
            &self.common,
            None,
        );
    }

    fn wrap(&self) -> (f32, f32) {
        (self.level.size.0 as f32, self.level.size.1 as f32)
    }

    /// Car-car contact; returns whether `a` felt a bump.
    fn collide(&self, a: &mut Car, b: &mut Car) -> bool {
        physics::collide_cars(
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
            Some(self.wrap()),
            &self.common,
        )
        .is_some_and(|c| c.delta_v[0] >= physics::FELT_BUMP_DV)
    }

    /// Settle a car at rest at `spawn`.
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

/// The procedural test level with the game's time scale (9 physics time a
/// second, 0.1 quanta) so cars reach real speeds.
fn test_world() -> World {
    let level = level::load(
        &level::LevelConfig::new_test(),
        &settings::Geometry::default(),
    );
    let mut common = config::common::Common::test_default();
    common.nature.time_delta0 = 0.1286;
    common.nature.num_calls_analysis = 5;
    let (x, y) = (level.size.0 / 4, level.size.1 / 4);
    let spawn = space::Transform {
        disp: Vec3::new(x as f32, y as f32, level.get((x, y)).high() + 5.0),
        rot: Quat::IDENTITY,
        scale: 1.0,
    };
    World {
        name: "test",
        level,
        jump_power: common.force.max_jump_power,
        common,
        car: CarPhysicsData::test_default(),
        max_quant: 0.1,
        spawn,
        ram_at: Some(1.6),
    }
}

fn fostral_world() -> Option<World> {
    let settings = config::Settings::try_load("config/settings.ron")?;
    let ini = settings.data_path.join("thechain/fostral/world.ini");
    if !ini.is_file() {
        return None;
    }
    let reg = config::game::Registry::load(&settings);
    let level = level::load(&level::LevelConfig::load(&ini), &settings.game.geometry);
    let common = config::common::load(settings.open_relative("common.prm"));
    let cars = config::car::load_physics_registry(
        &settings.data_path,
        &reg,
        settings.game.physics.shape_sampling,
    );
    let car = cars.get("OxidizeMonk")?.clone();
    // The open ground of the #388 jump playtest.
    let spawn = space::Transform {
        disp: Vec3::new(830.0, 3983.0, level.get((830, 3983)).high() + 5.0),
        rot: Quat::IDENTITY,
        scale: car.scale,
    };
    Some(World {
        name: "Fostral",
        level,
        jump_power: common.force.max_jump_power,
        common,
        car,
        max_quant: settings.game.physics.max_quant,
        spawn,
        ram_at: None,
    })
}

#[derive(Clone, Copy)]
struct Car {
    transform: space::Transform,
    dynamo: Dynamo,
}

impl Car {
    fn server_car(&self, bumped: bool) -> ServerCar {
        ServerCar {
            transform: self.transform,
            linear_velocity: self.dynamo.linear_velocity,
            angular_velocity: self.dynamo.angular_velocity,
            traction: self.dynamo.traction,
            rudder: self.dynamo.rudder,
            bumped,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    /// Before this change: latest control on the server, present-pose
    /// reconcile on the client.
    Old,
    /// Numbered inputs, ack, rewind and replay, render offset.
    New,
}

#[derive(Clone, Copy, Debug)]
enum Scenario {
    /// Full throttle and turbo with steering sweeps.
    Drive,
    /// Full throttle, a full-power jump at 1.5 s.
    Jump,
    /// Full throttle into a parked second car 70u ahead.
    Ram,
}

impl Scenario {
    const SECONDS: f32 = 6.0;
    /// Input stops here; the rest is for settling and convergence.
    const DRIVE_UNTIL: f32 = 4.5;

    fn input(self, t: f32) -> QuantInput {
        if t >= Self::DRIVE_UNTIL {
            return QuantInput {
                brake: true,
                ..QuantInput::default()
            };
        }
        match self {
            Scenario::Drive => QuantInput {
                motor: 1.0,
                turbo: true,
                rudder: if (t * 0.8).fract() < 0.5 { 1.0 } else { -1.0 },
                ..QuantInput::default()
            },
            Scenario::Jump | Scenario::Ram => QuantInput {
                motor: 1.0,
                ..QuantInput::default()
            },
        }
    }

    fn jump_at(self) -> Option<f32> {
        matches!(self, Scenario::Jump).then_some(1.5)
    }
}

/// Deterministic xorshift.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * (self.next() % 10_000) as f32 / 10_000.0
    }
}

#[derive(Clone, Copy, Debug)]
enum Frames {
    Fixed(f32),
    /// Uniform 17-200 ms.
    Random,
}

#[derive(Clone, Copy, Debug)]
struct Link {
    /// One-way latency in server ticks.
    ticks: u32,
    /// Up to this much extra per message, random, order kept.
    jitter: f32,
}

/// An ordered, delayed one-way channel.
struct Pipe<T> {
    queue: VecDeque<(f32, T)>,
    last: f32,
    link: Link,
    rng: Rng,
}

impl<T> Pipe<T> {
    fn new(link: Link, seed: u32) -> Self {
        Pipe {
            queue: VecDeque::new(),
            last: 0.0,
            link,
            rng: Rng(seed | 1),
        }
    }

    fn send(&mut self, now: f32, item: T) {
        let at = now + self.link.ticks as f32 * TICK + self.rng.range(0.0, self.link.jitter);
        self.last = self.last.max(at);
        self.queue.push_back((self.last, item));
    }

    fn recv(&mut self, now: f32) -> Vec<T> {
        let mut out = Vec::new();
        while self.queue.front().is_some_and(|&(at, _)| at <= now) {
            out.push(self.queue.pop_front().unwrap().1);
        }
        out
    }
}

/// `Input` on the wire.
struct InputMsg {
    first: u32,
    count: u32,
    input: QuantInput,
}

/// The local car's part of a `WorldState`, plus the other car's pose.
#[derive(Clone, Copy)]
struct Snapshot {
    car: Car,
    ack: u32,
    bumps: u32,
    other: Option<Car>,
}

struct Server {
    a: Car,
    queue: InputQueue,
    /// Old mode: latest control, jump kept until a quantum takes it.
    control: QuantInput,
    b: Option<Car>,
    bumps: u32,
    clock: physics::QuantClock,
}

impl Server {
    fn receive(&mut self, mode: Mode, msg: InputMsg) {
        match mode {
            Mode::New => self.queue.push(msg.first, msg.count, msg.input),
            Mode::Old => {
                let jump = self.control.jump.take();
                self.control = msg.input;
                self.control.jump = self.control.jump.or(jump);
            }
        }
    }

    /// One server tick. `drain`: only run the slots the local car has
    /// inputs for, so the final pose compares like for like (the other car
    /// is not run on past the client's last quantum).
    fn tick(&mut self, mode: Mode, world: &World, drain: bool) -> Snapshot {
        let quanta = self
            .clock
            .advance(TICK * world.physics_per_sec(), world.max_quant);
        let a_quanta = match mode {
            Mode::New => self.queue.budget(quanta).total(),
            Mode::Old => quanta,
        };
        let b_quanta = match self.b {
            Some(_) if drain => a_quanta,
            Some(_) => quanta,
            None => 0,
        };
        let slots = a_quanta.max(b_quanta);
        let wrap = world.wrap();
        let fold = |car: &mut Car| {
            car.transform.disp.x = car.transform.disp.x.rem_euclid(wrap.0);
            car.transform.disp.y = car.transform.disp.y.rem_euclid(wrap.1);
        };
        for i in 0..slots {
            if i < a_quanta {
                let input = match mode {
                    Mode::New => self.queue.pop(),
                    Mode::Old if i + 1 == quanta => {
                        let mut input = self.control;
                        input.jump = self.control.jump.take();
                        input
                    }
                    Mode::Old => self.control.held(),
                };
                world.step(&mut self.a, &input);
                fold(&mut self.a);
            }
            if let Some(ref mut b) = self.b {
                if i < b_quanta {
                    world.step(b, &QuantInput::default());
                    fold(b);
                }
                if world.collide(&mut self.a, b) {
                    self.bumps += 1;
                }
                fold(&mut self.a);
                fold(b);
            }
        }
        Snapshot {
            car: self.a,
            ack: self.queue.ack(),
            bumps: self.bumps,
            other: self.b,
        }
    }
}

struct Client {
    car: Car,
    prediction: Prediction,
    offset: RenderOffset,
    clock: physics::QuantClock,
    /// The other car's latest server pose.
    ghost: Option<Car>,
    /// New: the other car carried along with ours (at our next quantum).
    pred_ghost: Option<Ghost>,
    bumps: u32,
    pending_jump: Option<f32>,
}

impl Client {
    /// Old: contact against the other car's latest server pose, which
    /// keeps following the snapshots (it never gives way).
    fn snapshot_contact(world: &World, car: &mut Car, ghost: Option<Car>) {
        if let Some(mut g) = ghost {
            world.collide(car, &mut g);
        }
    }

    /// New: one quantum of the predicted ghost, and contact against it.
    fn ghost_contact(world: &World, car: &mut Car, ghost: &mut Option<Ghost>) {
        if let Some(g) = ghost {
            g.advance(world.max_quant, &world.car, &world.level, &world.common);
            fold_home(&mut g.transform.disp, world.level.period());
            let mut other = Car {
                transform: g.transform,
                dynamo: g.dynamo,
            };
            world.collide(car, &mut other);
            g.transform = other.transform;
            g.dynamo = other.dynamo;
            fold_home(&mut g.transform.disp, world.level.period());
        }
    }

    /// The pose the player sees.
    fn drawn(&self) -> Vec3 {
        self.offset.apply(&self.car.transform).disp
    }
}

#[derive(Debug, Default)]
struct Outcome {
    /// Largest visible per-frame correction.
    max_correction: f32,
    /// Corrections shown at once: old hard snaps (> 8u), new teleports.
    snaps: u32,
    /// Replays that moved the car over 0.01u (new), corrections over
    /// 0.1u (old).
    corrections: u32,
    /// Client pose vs the server's at the end (inputs stopped, all
    /// messages delivered).
    final_gap: f32,
    /// What is still in the render offset at the end.
    final_offset: f32,
    /// The car's path length, to show it went fast.
    travelled: f32,
    bumps: u32,
    /// Old: the physics went non-finite and the run stopped there.
    blew_up: bool,
}

fn wrapped_distance(world: &World, a: Vec3, b: Vec3) -> f32 {
    let p = world.level.period();
    let d = a - b;
    let fold = |d: f32, p: f32| d - p * (d / p).round();
    Vec3::new(fold(d.x, p.x), fold(d.y, p.y), d.z).length()
}

fn run(
    world: &World,
    mode: Mode,
    scenario: Scenario,
    frames: Frames,
    link: Link,
    seed: u32,
) -> Outcome {
    let start = world.settle(world.spawn);
    let b = matches!(scenario, Scenario::Ram).then(|| match world.ram_at {
        Some(at) => {
            // The rammer's own path, run alone.
            let mut solo = start;
            let quanta = (at * world.physics_per_sec() / world.max_quant) as u32;
            for i in 0..quanta {
                let t = i as f32 * world.max_quant / world.physics_per_sec();
                world.step(&mut solo, &scenario.input(t));
                fold_home(&mut solo.transform.disp, world.level.period());
            }
            let mut spawn = solo.transform;
            spawn.disp.z += 1.0;
            world.settle(spawn)
        }
        None => {
            let ahead = start.transform.rot * Vec3::Y * 70.0;
            let mut spawn = start.transform;
            spawn.disp += ahead;
            let (x, y) = (spawn.disp.x as i32, spawn.disp.y as i32);
            spawn.disp.z = world.level.get((x, y)).high() + 5.0;
            world.settle(spawn)
        }
    });
    let mut server = Server {
        a: start,
        queue: InputQueue::default(),
        control: QuantInput::default(),
        b,
        bumps: 0,
        clock: Default::default(),
    };
    let mut client = Client {
        car: start,
        prediction: Prediction::default(),
        offset: RenderOffset::default(),
        clock: Default::default(),
        ghost: b,
        pred_ghost: b.map(|b| Ghost::new(b.transform, b.dynamo)),
        bumps: 0,
        pending_jump: None,
    };
    let mut up = Pipe::<InputMsg>::new(link, seed.wrapping_mul(7919));
    let mut down = Pipe::<Snapshot>::new(link, seed.wrapping_mul(104_729));
    let mut rng = Rng(seed.wrapping_mul(2_654_435_761) | 1);
    let mut out = Outcome::default();
    let mut now = 0.0f32;
    let mut next_tick = TICK;
    let mut jumped = false;
    let end = Scenario::SECONDS + 2.0 * (link.ticks as f32 * TICK + link.jitter) + 3.0;
    let period = Some(world.level.period());
    let input_factor = world.input_factor();
    while now < end {
        let dt = match frames {
            Frames::Fixed(dt) => dt,
            Frames::Random => rng.range(0.017, 0.2),
        };
        let frame_end = now + dt;
        // Server ticks due before the frame ends.
        while next_tick <= frame_end {
            for msg in up.recv(next_tick) {
                server.receive(mode, msg);
            }
            let snap = server.tick(mode, world, false);
            down.send(next_tick, snap);
            next_tick += TICK;
        }
        now = frame_end;

        // Client frame: input, quanta, send, reconcile, draw.
        let held = scenario.input(now);
        if let Some(at) = scenario.jump_at()
            && !jumped
            && now >= at
        {
            jumped = true;
            client.pending_jump = Some(world.jump_power);
        }
        let drawn_before = client.drawn();
        let quanta = client
            .clock
            .advance(dt * world.physics_per_sec(), world.max_quant);
        let first = client.prediction.next();
        let mut last_input = held;
        for i in 0..quanta {
            let last = i + 1 == quanta;
            let mut input = held;
            input.jump = if last {
                client.pending_jump.take()
            } else {
                None
            };
            last_input = input;
            // Folded onto the home tile like the server: after the step
            // and after the contacts.
            world.step(&mut client.car, &input);
            fold_home(&mut client.car.transform.disp, world.level.period());
            match mode {
                Mode::Old => Client::snapshot_contact(world, &mut client.car, client.ghost),
                Mode::New => Client::ghost_contact(world, &mut client.car, &mut client.pred_ghost),
            }
            fold_home(&mut client.car.transform.disp, world.level.period());
            client
                .prediction
                .push(input, &client.car.transform, &client.car.dynamo);
        }
        let stepped = client.drawn();
        if quanta > 0 {
            up.send(
                now,
                InputMsg {
                    first,
                    count: quanta,
                    input: last_input,
                },
            );
        }
        out.travelled += wrapped_distance(world, stepped, drawn_before);

        let mut correction = 0.0f32;
        if let Some(snap) = down.recv(now).pop() {
            client.ghost = snap.other;
            let bumped = snap.bumps != client.bumps;
            client.bumps = snap.bumps;
            let before = client.car.transform;
            match mode {
                Mode::Old => {
                    // The client keeps a nearby replica of the wrapped world.
                    let mut server_car = snap.car.server_car(bumped);
                    let p = world.level.period();
                    let d = server_car.transform.disp - before.disp;
                    server_car.transform.disp.x -= p.x * (d.x / p.x).round();
                    server_car.transform.disp.y -= p.y * (d.y / p.y).round();
                    let kind = physics::reconcile_local_car(
                        &mut client.car.transform,
                        &mut client.car.dynamo,
                        &server_car,
                    );
                    let moved = wrapped_distance(world, client.car.transform.disp, before.disp);
                    correction += moved;
                    if kind == physics::Reconcile::Snap {
                        out.snaps += 1;
                    }
                    if moved > 0.1 {
                        out.corrections += 1;
                    }
                }
                Mode::New => {
                    let fresh = snap.other.map(|b| Ghost::new(b.transform, b.dynamo));
                    let force = fresh.is_some_and(|g| {
                        physics::prediction::wrapped_distance(
                            g.transform.disp,
                            client.car.transform.disp,
                            period,
                        ) < physics::prediction::GHOST_REPLAY_RANGE
                    });
                    let mut ghost = fresh;
                    let outcome = client.prediction.reconcile(
                        &ServerSnapshot {
                            car: snap.car.server_car(bumped),
                            ack: snap.ack,
                        },
                        force,
                        period,
                        &mut client.car.transform,
                        &mut client.car.dynamo,
                        |t, d, input, _| {
                            let mut car = Car {
                                transform: *t,
                                dynamo: *d,
                            };
                            physics::prediction::step_input(
                                &mut car.dynamo,
                                &mut car.transform,
                                input,
                                world.max_quant,
                                input_factor,
                                &world.car,
                                &world.level,
                                &world.common,
                                None,
                            );
                            fold_home(&mut car.transform.disp, world.level.period());
                            Client::ghost_contact(world, &mut car, &mut ghost);
                            fold_home(&mut car.transform.disp, world.level.period());
                            *t = car.transform;
                            *d = car.dynamo;
                        },
                    );
                    client.pred_ghost = match outcome {
                        Reconciled::Replayed(_) => ghost,
                        _ => fresh.map(|mut g| {
                            for _ in 0..client.prediction.next().wrapping_sub(snap.ack) {
                                g.advance(world.max_quant, &world.car, &world.level, &world.common);
                            }
                            g
                        }),
                    };
                    let moved = wrapped_distance(world, client.car.transform.disp, before.disp);
                    if matches!(outcome, Reconciled::Replayed(_) | Reconciled::Ahead) {
                        if moved > 0.01 {
                            out.corrections += 1;
                        }
                        if !client.offset.absorb(&before, &client.car.transform) {
                            // Teleport: the offset is dropped and the car
                            // shown where it is.
                            out.snaps += 1;
                            correction += wrapped_distance(world, client.drawn(), stepped);
                        }
                    }
                }
            }
        }
        if mode == Mode::New {
            correction += client.offset.decay(dt);
        }
        // The drawn car moved by its step plus `correction`; double-check
        // the bookkeeping against the drawn poses.
        let drawn_after = client.drawn();
        let seen = wrapped_distance(world, drawn_after, stepped);
        if mode == Mode::Old && (!seen.is_finite() || seen > 1e4) {
            // The old reconcile can pull the car into a state the physics
            // blows up on (on the steep test hills): flag it and stop.
            out.blew_up = true;
            break;
        }
        assert!(
            seen <= correction + 1e-3,
            "unaccounted visible jump {seen} > {correction}"
        );
        out.max_correction = out.max_correction.max(correction);
    }
    // Let the server catch up with every input sent (the client is ahead
    // of it by the latency), then compare like for like.
    if mode == Mode::New {
        let mut guard = 0;
        while server.queue.ack() != client.prediction.next() && guard < 400 {
            for msg in up.recv(next_tick) {
                server.receive(mode, msg);
            }
            server.tick(mode, world, true);
            next_tick += TICK;
            guard += 1;
        }
    }
    out.final_gap = wrapped_distance(world, client.car.transform.disp, server.a.transform.disp);
    out.final_offset = client.offset.disp.length();
    out.bumps = server.bumps;
    out
}

const SCENARIOS: [Scenario; 3] = [Scenario::Drive, Scenario::Jump, Scenario::Ram];
const LINKS: [Link; 3] = [
    Link {
        ticks: 1,
        jitter: 0.01,
    },
    Link {
        ticks: 3,
        jitter: 0.03,
    },
    Link {
        ticks: 5,
        jitter: 0.05,
    },
];
const FRAMES: [Frames; 3] = [Frames::Fixed(0.017), Frames::Fixed(0.2), Frames::Random];

/// Worst visible correction over the latency x frame-time matrix.
fn sweep(world: &World, mode: Mode, scenario: Scenario) -> Outcome {
    let mut worst = Outcome::default();
    for (li, link) in LINKS.iter().enumerate() {
        for (fi, frames) in FRAMES.iter().enumerate() {
            let seed = 1 + (li * 10 + fi) as u32;
            let o = run(world, mode, scenario, *frames, *link, seed);
            println!(
                "{} {:?} {:?} latency {} ticks frames {:?}: max correction {:.2}u, snaps {}, corrections {}, final gap {:.3}u, offset left {:.3}u, travelled {:.0}u, bumps {}{}",
                world.name,
                mode,
                scenario,
                link.ticks,
                frames,
                o.max_correction,
                o.snaps,
                o.corrections,
                o.final_gap,
                o.final_offset,
                o.travelled,
                o.bumps,
                if o.blew_up { " (physics blew up)" } else { "" }
            );
            worst.max_correction = worst.max_correction.max(o.max_correction);
            worst.snaps += o.snaps;
            worst.corrections += o.corrections;
            worst.final_gap = worst.final_gap.max(o.final_gap);
            worst.final_offset = worst.final_offset.max(o.final_offset);
            worst.travelled = worst.travelled.max(o.travelled);
            worst.bumps = worst.bumps.max(o.bumps);
            worst.blew_up |= o.blew_up;
        }
    }
    worst
}

/// Max visible per-frame correction allowed with prediction.
const MAX_VISIBLE: f32 = 2.0;

fn check_world(world: &World) -> (f32, f32) {
    let (mut old_worst, mut new_worst) = (0.0f32, 0.0f32);
    for scenario in SCENARIOS {
        let new = sweep(world, Mode::New, scenario);
        let old = sweep(world, Mode::Old, scenario);
        println!(
            "== {} {:?}: old max {:.2}u ({} snaps), new max {:.2}u ({} snaps, {} replays)",
            world.name,
            scenario,
            old.max_correction,
            old.snaps,
            new.max_correction,
            new.snaps,
            new.corrections
        );
        assert!(
            new.travelled > 100.0,
            "{scenario:?}: the car should go fast, went {:.0}u",
            new.travelled
        );
        if matches!(scenario, Scenario::Ram) {
            assert!(new.bumps > 0, "the ram never hit the other car");
        }
        assert!(
            new.max_correction < MAX_VISIBLE,
            "{scenario:?}: visible correction {:.2}u",
            new.max_correction
        );
        assert_eq!(new.snaps, 0, "{scenario:?}: correction shown at once");
        assert!(
            new.final_gap < 0.05,
            "{scenario:?}: client did not converge to the server: {:.3}u",
            new.final_gap
        );
        assert!(new.final_offset < 0.05, "{scenario:?}: offset left over");
        assert!(
            old.max_correction > new.max_correction,
            "{scenario:?}: old code was expected to correct more"
        );
        old_worst = old_worst.max(old.max_correction);
        new_worst = new_worst.max(new.max_correction);
    }
    (old_worst, new_worst)
}

#[test]
fn prediction_hides_latency_on_test_level() {
    let (old, new) = check_world(&test_world());
    println!("test level: old worst {old:.2}u, new worst {new:.2}u");
}

#[test]
fn prediction_hides_latency_on_fostral() {
    let Some(world) = fostral_world() else {
        eprintln!("skipping: no Fostral data");
        return;
    };
    let (old, new) = check_world(&world);
    println!("Fostral: old worst {old:.2}u, new worst {new:.2}u");
    // The 8-15u corrections seen live, and past the hard-snap threshold.
    assert!(
        old > space::Transform::WORLD_STATE_SNAP_POS,
        "old code was expected to snap: worst {old:.2}u"
    );
}

/// Even at one tick of latency (localhost-like) the old reconcile pulls
/// the car by several units a frame at speed; the new one is silent.
#[test]
fn old_reconcile_corrects_at_one_tick() {
    let world = test_world();
    let link = Link {
        ticks: 1,
        jitter: 0.0,
    };
    let old = run(
        &world,
        Mode::Old,
        Scenario::Drive,
        Frames::Fixed(0.017),
        link,
        3,
    );
    let new = run(
        &world,
        Mode::New,
        Scenario::Drive,
        Frames::Fixed(0.017),
        link,
        3,
    );
    println!(
        "1 tick, 60 fps drive: old {:.2}u, new {:.2}u",
        old.max_correction, new.max_correction
    );
    assert_eq!(new.corrections, 0, "nothing to correct without contact");
    assert!(old.max_correction > new.max_correction);
}
