//! Multiplayer client: the parts of the predicting client that the native
//! game (`bin/road`) and the web build (`bin/web`) share.
//!
//! - [`RemoteCar`]: another player's car as the client sees it: the latest
//!   server state (the source of its contact [`Ghost`]) and the pose drawn
//!   from a small jitter buffer of its snapshots, dead reckoned when the
//!   buffer runs dry.
//! - [`ClientSync`]: the local car's side of [`super::prediction`]: the
//!   quantum clock, the numbered prediction history, the remote ghosts and
//!   the render offset, plus the per-quantum step and the snapshot handling
//!   (ghost lead, first sync, bump detection, rewind/replay with the ghosts,
//!   the offset absorbing what the replay moved).
//!
//! Both clients run exactly this code, so a car predicted in the browser
//! takes the same path, quantum for quantum, as one predicted natively.

use super::prediction::{
    GHOST_LEAD_MAX, GHOST_REPLAY_RANGE, Ghost, Prediction, QuantInput, Reconciled, RenderOffset,
    ServerSnapshot, fold_home, near, step_input, wrapped_distance,
};
use super::{CarBody, CarPhysicsData, Dynamo, QuantClock, ServerCar, collide_cars};
use crate::{config, level, space};

use glam::{Quat, Vec2, Vec3};
use std::{borrow::Borrow, collections::HashMap};
use vangers_net::PlayerId;

/// What a client steps the cars against.
#[derive(Clone, Copy)]
pub struct Sim<'a> {
    pub level: &'a level::Level,
    pub common: &'a config::common::Common,
    /// The physics quantum, the same as the server's.
    pub max_quant: f32,
}

impl Sim<'_> {
    fn period(&self) -> Vec2 {
        self.level.period()
    }

    fn wrap(&self) -> (f32, f32) {
        (self.level.size.0 as f32, self.level.size.1 as f32)
    }
}

/// The transform of a `WorldState` agent.
pub fn net_transform(t: &vangers_net::NetTransform) -> space::Transform {
    space::Transform {
        disp: Vec3::from(t.position),
        rot: Quat::from_xyzw(t.rotation[0], t.rotation[1], t.rotation[2], t.rotation[3]),
        scale: t.scale,
    }
}

/// A `WorldState` agent as [`ServerCar`] (`bumped` is left to
/// [`ClientSync::on_snapshot`]).
pub fn server_car(agent: &vangers_net::AgentState) -> ServerCar {
    ServerCar {
        transform: net_transform(&agent.transform),
        linear_velocity: Vec3::from(agent.dynamo.linear_velocity),
        angular_velocity: Vec3::from(agent.dynamo.angular_velocity),
        traction: agent.dynamo.traction,
        rudder: agent.dynamo.rudder,
        bumped: false,
    }
}

/// The held control of a `WorldState` agent, as its contact ghost is
/// driven on with.
pub fn agent_control(agent: &vangers_net::AgentState) -> QuantInput {
    let c = &agent.control;
    QuantInput {
        motor: c.motor,
        rudder: c.rudder,
        brake: c.brake,
        turbo: c.turbo,
        jump: None,
        roll: 0.0,
    }
}

/// The wire form of a frame's input batch: held controls plus the jump and
/// roll of its last quantum.
pub fn net_control(input: &QuantInput) -> vangers_net::NetControl {
    vangers_net::NetControl {
        motor: input.motor,
        rudder: input.rudder,
        roll: input.roll,
        brake: input.brake,
        turbo: input.turbo,
        jump: input.jump,
    }
}

/// Displacement over `t` seconds at velocity `v` turning at spin `w`
/// (scaled axis): along an arc, so a car reckoned through a turn stays on
/// its circle instead of running off on the tangent.
fn arc(v: Vec3, w: Vec3, t: f32) -> Vec3 {
    let rate = w.length();
    if rate * t < 1e-4 {
        return v * t;
    }
    let n = w / rate;
    let along = n * n.dot(v);
    let across = v - along;
    let angle = rate * t;
    along * t + (across * angle.sin() + n.cross(across) * (1.0 - angle.cos())) / rate
}

/// Another player's car.
///
/// Drawn from a small jitter buffer: the car's snapshots, each at its
/// sender time, played back a little behind the newest one, by
/// [`Self::target_delay`]: about one to two snapshot intervals, more when
/// snapshots arrive unevenly (an EWMA of how late the newest one of each
/// frame is, against the earliest seen). The playback clock runs up to
/// [`Self::SKEW_MAX`] fast or slow to follow a changing delay, never
/// jumping. When the buffer runs dry (a stall longer than the delay) the
/// car is dead reckoned on from its last snapshot, along the arc of its
/// last two, for up to [`Self::DEAD_RECKON_MAX`], then holds. Whatever a
/// late snapshot changes about the pose drawn now (the reckoning was off)
/// becomes an offset that decays over a few frames.
///
/// The contact ghost uses the latest server state, not the drawn pose.
#[derive(Clone)]
pub struct RemoteCar {
    /// Contact shape for the ghost.
    pub phys_data: CarPhysicsData,
    /// Latest snapshot pose (ghost start).
    pub target_transform: space::Transform,
    /// The pose drawn this frame.
    pub render_transform: space::Transform,
    /// False until the first snapshot pose: nothing to draw yet.
    pub pose_ready: bool,
    /// Car-local velocities, traction and rudder of the latest snapshot.
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub traction: f32,
    pub rudder: f32,
    /// The held control the server last ran the car with: its ghost is
    /// driven on with it.
    pub control: QuantInput,
    /// World motion between the last two snapshots, per second: the dead
    /// reckoning velocity, and spin as a scaled axis (radians per second).
    /// Taken from the poses rather than `linear_velocity`, which is in
    /// physics time, so it carries on exactly what the snapshots showed.
    pub reckon_velocity: Vec3,
    pub reckon_spin: Vec3,
    /// Seconds the drawn pose is behind the newest snapshot's (estimated)
    /// sender time now: the visual latency the buffer adds.
    pub delay: f32,
    /// The delay the playback clock is steering to.
    pub target_delay: f32,
    /// EWMA of how late the newest snapshot of a frame arrives, seconds.
    pub jitter: f32,
    /// The drawn pose ran past the newest snapshot: dead reckoned.
    pub dry: bool,
    /// Snapshots by sender time, oldest first.
    samples: std::collections::VecDeque<(f64, space::Transform)>,
    /// Local seconds, advanced by `interpolate`.
    clock: f64,
    /// Sender time minus local time of the earliest arrivals over the
    /// last [`Self::OFFSET_WINDOW`]: its maximum.
    offset: f64,
    /// Each frame's newest arrival offset `(local time, offset)` in that
    /// window, kept only while it could still be the maximum.
    offsets: std::collections::VecDeque<(f64, f64)>,
    /// Sender time the drawn pose is at.
    playback: f64,
    /// Sender time of the next `on_snapshot` (one per server tick).
    next_time: f64,
    /// Where `on_agent` pinned the car's input timeline: an ack and its
    /// sender time.
    ack_base: Option<(u32, f64)>,
    /// Server tick and sender time of the latest `on_agent`.
    last_tick: Option<(u32, f64)>,
    /// Newest sender time that arrived since the last frame.
    arrived: Option<f64>,
    /// Decaying correction added to the played-back pose.
    error_disp: Vec3,
    error_rot: Quat,
}

impl RemoteCar {
    /// Snapshots a second (the server tick).
    pub const SNAPSHOT_RATE: f32 = 20.0;
    /// Playback delay with snapshots arriving evenly: a small floor, the
    /// ack timeline and dead reckoning carry the pose between snapshots.
    pub const DELAY_BASE: f32 = 0.03;
    /// Delay added per second of jitter.
    pub const DELAY_PER_JITTER: f32 = 1.5;
    /// Bounds of the playback delay.
    pub const DELAY_MIN: f32 = 0.03;
    pub const DELAY_MAX: f32 = 0.25;
    /// Weight of a frame's lateness in the jitter EWMA.
    pub const JITTER_WEIGHT: f32 = 0.1;
    /// Seconds the earliest arrival counts for: an early burst (a
    /// connect, a route change) is forgotten after it.
    pub const OFFSET_WINDOW: f64 = 3.0;
    /// Fastest the playback clock runs fast or slow to reach its delay.
    pub const SKEW_MAX: f32 = 0.1;
    /// Playback rate change per second of delay error.
    pub const SKEW_GAIN: f32 = 1.0;
    /// A delay this far off is resynced at once (the first snapshot).
    pub const RESYNC: f32 = 1.0;
    /// Seconds a correction takes to decay to 1/e.
    pub const ERROR_DECAY: f32 = 0.1;
    /// Frames a correction takes at least: at a low frame rate a whole
    /// decay would fit in one frame and show it as a jump.
    pub const BLEND_FRAMES_MIN: f32 = 3.0;
    /// Longest the drawn car is carried on past its newest snapshot
    /// before it holds: covers a retransmit stall at a 250 ms RTT (~0.45
    /// s) plus the interval, not a dropped connection.
    pub const DEAD_RECKON_MAX: f32 = 0.7;
    /// A snapshot this far from the last one, or from the drawn car, is a
    /// respawn or teleport: drawn there at once, not slid to.
    pub const TELEPORT: f32 = 64.0;
    /// Farthest the ack timeline may move from the tick one between two
    /// snapshots before it is pinned again, seconds.
    pub const ACK_SLIP_MAX: f64 = 1.0;
    /// Snapshots kept behind the drawn pose, seconds.
    const KEEP: f64 = 1.0;

    pub fn new(phys_data: CarPhysicsData) -> Self {
        RemoteCar {
            phys_data,
            target_transform: space::Transform::IDENTITY,
            render_transform: space::Transform::IDENTITY,
            pose_ready: false,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            traction: 0.0,
            rudder: 0.0,
            control: QuantInput::default(),
            reckon_velocity: Vec3::ZERO,
            reckon_spin: Vec3::ZERO,
            delay: 0.0,
            target_delay: Self::DELAY_BASE,
            jitter: 0.0,
            dry: false,
            samples: Default::default(),
            clock: 0.0,
            offset: 0.0,
            offsets: Default::default(),
            playback: 0.0,
            next_time: 0.0,
            ack_base: None,
            last_tick: None,
            arrived: None,
            error_disp: Vec3::ZERO,
            error_rot: Quat::IDENTITY,
        }
    }

    /// Take this car's state from the `WorldState` of server `tick`.
    ///
    /// Timed by the car's own input timeline, its `ack` at
    /// `quanta_per_sec`, rather than by the tick: the server runs a car
    /// only as its owner's input comes in, so after an upstream stall it
    /// repeats the car's pose for a few ticks and then catches up two
    /// ticks' worth at a time. By the ack those poses are evenly spaced
    /// (a repeated one is the same moment again), and the car is drawn
    /// moving as it was driven.
    pub fn on_agent(
        &mut self,
        agent: &vangers_net::AgentState,
        tick: u32,
        quanta_per_sec: f32,
        period: Option<Vec2>,
    ) {
        self.control = agent_control(agent);
        let time = self.agent_time(agent.ack, tick, quanta_per_sec);
        self.on_snapshot_at(&server_car(agent), period, time);
    }

    /// Sender time of a snapshot of server `tick` with input `ack`.
    fn agent_time(&mut self, ack: u32, tick: u32, quanta_per_sec: f32) -> f64 {
        let rate = Self::SNAPSHOT_RATE as f64;
        // Where the tick alone puts it, next to the last one.
        let by_tick = match self.last_tick {
            Some((t, time)) => time + tick.wrapping_sub(t) as i32 as f64 / rate,
            None => tick as f64 / rate,
        };
        let by_ack = match self.ack_base {
            Some((a, base)) if quanta_per_sec > 0.0 => {
                Some(base + ack.wrapping_sub(a) as i32 as f64 / quanta_per_sec as f64)
            }
            _ => None,
        };
        let time = match by_ack {
            Some(time) if (time - by_tick).abs() <= Self::ACK_SLIP_MAX => time,
            // The first snapshot, or the ack moved on its own (a respawn
            // or a pose set by the server): pinned again where the tick
            // says.
            _ => {
                self.ack_base = Some((ack, by_tick));
                by_tick
            }
        };
        self.last_tick = Some((tick, time));
        time
    }

    /// Take the next snapshot of this car, one server tick after the last.
    pub fn on_snapshot(&mut self, car: &ServerCar, period: Option<Vec2>) {
        let time = self.next_time;
        self.on_snapshot_at(car, period, time);
    }

    /// Take a snapshot of this car at sender time `time` (seconds). The
    /// first one (and a teleport) is shown at once; later ones go in the
    /// buffer, on the replica nearest the last one (the server folds every
    /// car onto the home tile, so a car crossing the seam would otherwise
    /// be drawn sweeping across the whole world).
    pub fn on_snapshot_at(&mut self, car: &ServerCar, period: Option<Vec2>, time: f64) {
        self.linear_velocity = car.linear_velocity;
        self.angular_velocity = car.angular_velocity;
        self.traction = car.traction;
        self.rudder = car.rudder;
        self.next_time = time + 1.0 / Self::SNAPSHOT_RATE as f64;
        let mut target = car.transform;
        if self.pose_ready
            && let Some(p) = period
        {
            let d = target.disp.truncate() - self.target_transform.disp.truncate();
            let fold = |d: f32, p: f32| if p > 0.0 { (d / p).round() * p } else { 0.0 };
            target.disp.x -= fold(d.x, p.x);
            target.disp.y -= fold(d.y, p.y);
        }
        let step = (target.disp - self.target_transform.disp).length();
        let latest = self.samples.back().map(|&(t, _)| t);
        self.target_transform = target;
        if !self.pose_ready
            || step > Self::TELEPORT
            || (target.disp - self.render_transform.disp).length() > Self::TELEPORT
        {
            self.reset(target, time);
            return;
        }
        let before = self.played(self.playback);
        match latest {
            // Out of order: the ghost has it, the buffer does not.
            Some(t) if time < t => return,
            Some(t) if time == t => {
                self.samples.back_mut().unwrap().1 = target;
            }
            _ => self.samples.push_back((time, target)),
        }
        self.arrived = Some(self.arrived.map_or(time, |a| a.max(time)));
        self.update_reckoning();
        let after = self.played(self.playback);
        // The drawn pose stays where it is; the difference decays.
        self.error_disp += before.disp - after.disp;
        self.error_rot = (self.error_rot * before.rot * after.rot.inverse()).normalize();
        if self.error_disp.length() > Self::TELEPORT {
            self.reset(target, time);
        }
    }

    /// Show `target` at once, with nothing buffered before it.
    fn reset(&mut self, target: space::Transform, time: f64) {
        self.samples.clear();
        self.samples.push_back((time, target));
        self.render_transform = target;
        self.reckon_velocity = Vec3::ZERO;
        self.reckon_spin = Vec3::ZERO;
        self.error_disp = Vec3::ZERO;
        self.error_rot = Quat::IDENTITY;
        self.offset = time - self.clock;
        self.offsets.clear();
        self.offsets.push_back((self.clock, self.offset));
        // Held there for the delay, then played on as snapshots come.
        self.playback = time - self.target_delay as f64;
        self.arrived = None;
        self.dry = false;
        self.pose_ready = true;
    }

    /// Dead reckoning motion from the last two buffered snapshots.
    fn update_reckoning(&mut self) {
        let n = self.samples.len();
        if n < 2 {
            return;
        }
        let (t0, a) = self.samples[n - 2];
        let (t1, b) = self.samples[n - 1];
        let dt = (t1 - t0) as f32;
        if dt <= 0.0 {
            return;
        }
        let mut turn = b.rot * a.rot.inverse();
        if turn.w < 0.0 {
            turn = -turn;
        }
        self.reckon_spin = turn.to_scaled_axis() / dt;
        // The step is a chord: its direction is the heading half the
        // interval back. Turned on to the snapshot's heading.
        let half = Quat::from_scaled_axis(self.reckon_spin * (0.5 * dt));
        self.reckon_velocity = half * (b.disp - a.disp) / dt;
    }

    /// The buffered pose at sender time `time`: between the snapshots
    /// around it, or reckoned on from the newest.
    fn played(&self, time: f64) -> space::Transform {
        let Some(&(last_t, last)) = self.samples.back() else {
            return self.render_transform;
        };
        if time >= last_t {
            let ahead = ((time - last_t) as f32).min(Self::DEAD_RECKON_MAX);
            return space::Transform {
                disp: last.disp + arc(self.reckon_velocity, self.reckon_spin, ahead),
                rot: (Quat::from_scaled_axis(self.reckon_spin * ahead) * last.rot).normalize(),
                scale: last.scale,
            };
        }
        let next = self.samples.partition_point(|&(t, _)| t <= time);
        if next == 0 {
            return self.samples[0].1;
        }
        let (t0, a) = self.samples[next - 1];
        let (t1, b) = self.samples[next];
        let f = ((time - t0) / (t1 - t0)) as f32;
        space::Transform {
            disp: a.disp.lerp(b.disp, f),
            rot: a.rot.slerp(b.rot, f),
            scale: a.scale + (b.scale - a.scale) * f,
        }
    }

    /// Move the drawn pose `delta` seconds on.
    pub fn interpolate(&mut self, delta: f32) {
        if !self.pose_ready {
            return;
        }
        if let Some(time) = self.arrived.take() {
            // How late the newest snapshot of this frame came, against
            // the earliest arrivals.
            let offset = time - self.clock;
            while self.offsets.back().is_some_and(|&(_, o)| o <= offset) {
                self.offsets.pop_back();
            }
            self.offsets.push_back((self.clock, offset));
            while self.offsets[0].0 < self.clock - Self::OFFSET_WINDOW {
                self.offsets.pop_front();
            }
            self.offset = self.offsets[0].1;
            let late = (self.offset - offset) as f32;
            self.jitter += (late - self.jitter) * Self::JITTER_WEIGHT;
            self.target_delay = (Self::DELAY_BASE + Self::DELAY_PER_JITTER * self.jitter)
                .clamp(Self::DELAY_MIN, Self::DELAY_MAX);
        }
        let error = (self.clock + self.offset - self.playback) as f32 - self.target_delay;
        self.clock += delta as f64;
        let now = self.clock + self.offset;
        if error.abs() > Self::RESYNC {
            self.playback = now - self.target_delay as f64;
        } else {
            let skew = (error * Self::SKEW_GAIN).clamp(-Self::SKEW_MAX, Self::SKEW_MAX);
            self.playback += (delta * (1.0 + skew)) as f64;
        }
        self.delay = (now - self.playback) as f32;
        while self.samples.len() > 2 && self.samples[1].0 < self.playback - Self::KEEP {
            self.samples.pop_front();
        }
        self.dry = self.samples.back().is_some_and(|&(t, _)| self.playback > t);

        let keep = (-delta / Self::ERROR_DECAY)
            .exp()
            .max(1.0 - 1.0 / Self::BLEND_FRAMES_MIN);
        self.error_disp *= keep;
        self.error_rot = Quat::IDENTITY.slerp(self.error_rot, keep);
        let pose = self.played(self.playback);
        self.render_transform = space::Transform {
            disp: pose.disp + self.error_disp,
            rot: (self.error_rot * pose.rot).normalize(),
            scale: pose.scale,
        };
    }

    /// The contact ghost at the latest snapshot, once there is one: the
    /// server's pose and velocities, driven by the car's held control.
    pub fn ghost(&self) -> Option<Ghost> {
        self.pose_ready.then(|| {
            Ghost::driven(
                self.target_transform,
                Dynamo {
                    linear_velocity: self.linear_velocity,
                    angular_velocity: self.angular_velocity,
                    traction: self.traction,
                    rudder: self.rudder,
                    ..Default::default()
                },
                self.control,
            )
        })
    }

    /// The client moved its coordinates by `-by` (a torus rebase).
    pub fn shift(&mut self, by: Vec3) {
        self.target_transform.disp -= by;
        self.render_transform.disp -= by;
        for &mut (_, ref mut pose) in self.samples.iter_mut() {
            pose.disp -= by;
        }
    }
}

/// Fresh ghosts of the remote cars at their latest server pose.
pub fn snapshot_ghosts<R: Borrow<RemoteCar>>(
    remotes: &HashMap<PlayerId, R>,
) -> HashMap<PlayerId, Ghost> {
    remotes
        .iter()
        .filter_map(|(&id, r)| r.borrow().ghost().map(|g| (id, g)))
        .collect()
}

/// The local car against every remote ghost, one quantum: each ghost
/// moves on, then one `collide_cars`, and both keep the result (the
/// server's remote car gives way too). Folded onto the home tile after the
/// step and the contact, like the server's copy. Returns whether the car
/// touched any ghost.
pub fn ghost_contacts<R: Borrow<RemoteCar>>(
    transform: &mut space::Transform,
    dynamo: &mut Dynamo,
    car: &CarPhysicsData,
    ghosts: &mut HashMap<PlayerId, Ghost>,
    remotes: &HashMap<PlayerId, R>,
    sim: Sim<'_>,
) -> bool {
    let mut touched = false;
    for (id, ghost) in ghosts.iter_mut() {
        let Some(remote) = remotes.get(id) else {
            continue;
        };
        let phys = &remote.borrow().phys_data;
        ghost.advance(sim.max_quant, phys, sim.level, sim.common);
        fold_home(&mut ghost.transform.disp, sim.period());
        touched |= collide_cars(
            CarBody {
                dynamo,
                transform,
                car,
            },
            CarBody {
                dynamo: &mut ghost.dynamo,
                transform: &mut ghost.transform,
                car: phys,
            },
            Some(sim.wrap()),
            sim.common,
        )
        .is_some();
        fold_home(&mut ghost.transform.disp, sim.period());
    }
    touched
}

/// The local car, as [`ClientSync`] steps and corrects it.
pub struct LocalCar<'a> {
    pub transform: &'a mut space::Transform,
    pub dynamo: &'a mut Dynamo,
    pub phys: &'a CarPhysicsData,
}

/// The local car's entry of a `WorldState`.
#[derive(Clone, Copy, Debug)]
pub struct LocalSnapshot {
    pub car: ServerCar,
    /// Quanta of our input in `car`.
    pub ack: u32,
    /// The server's felt-bump counter of the car.
    pub bumps: u32,
}

impl LocalSnapshot {
    pub fn from_agent(agent: &vangers_net::AgentState) -> Self {
        LocalSnapshot {
            car: server_car(agent),
            ack: agent.ack,
            bumps: agent.bumps,
        }
    }
}

/// What [`ClientSync::on_snapshot`] did with the local car.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SyncEvent {
    /// The first snapshot: the car was put at the server state, history
    /// and offset dropped. The caller snaps its camera.
    FirstSync,
    /// The car is held (train ride, pad lock, a debug pose edit): left
    /// alone.
    Held,
    /// The prediction was checked at the ack.
    Reconciled {
        outcome: Reconciled,
        /// How far the physics pose moved.
        moved: f32,
        /// The move was too big for the render offset: shown at once.
        shown_at_once: bool,
        /// The correction came with car contact (a server bump, a
        /// predicted contact since shortly before the ack, or a remote car
        /// touching the server's pose). Faded like any other; for the logs.
        contact: bool,
    },
}

/// The local car's side of client prediction, shared by both clients.
#[derive(Default)]
pub struct ClientSync {
    pub prediction: Prediction,
    pub render_offset: RenderOffset,
    /// The remote cars as the prediction bumps into them, carried along to
    /// our next quantum.
    pub ghosts: HashMap<PlayerId, Ghost>,
    pub quant_clock: QuantClock,
    /// The local car's `AgentState::bumps` at the last snapshot.
    pub server_bumps: Option<u32>,
    /// Set by the first snapshot of the local car.
    pub synced: bool,
    /// The quantum of the latest predicted contact with a ghost (live or
    /// in a replay).
    pub contact_at: Option<u32>,
    /// Show corrections past `RenderOffset::FADE_ABOVE` at once and decay
    /// the rest at the usual rate: the behaviour before fading, for
    /// comparisons.
    pub no_fade: bool,
}

impl ClientSync {
    /// A predicted contact this many quanta before the ack still makes a
    /// correction a contact correction (the server's contact may come a
    /// little earlier or later than the prediction's).
    pub const CONTACT_RECENT: u32 = 32;
    /// A faded correction larger than this is worth a log line
    /// (`Server correction ...u: faded`).
    pub const FADE_LOG: f32 = 10.0;

    /// Whole quanta due for `physics_dt` of physics time.
    pub fn due_quanta(&mut self, physics_dt: f32, max_quant: f32) -> u32 {
        self.quant_clock.advance(physics_dt, max_quant)
    }

    /// The input for quantum `i` of a frame of `quanta` holding `held`:
    /// the jump and roll go on the last one only, as the server queues
    /// them.
    pub fn frame_input(held: &QuantInput, i: u32, quanta: u32) -> QuantInput {
        if i + 1 == quanta { *held } else { held.held() }
    }

    /// Contact of the local car against the ghosts for one quantum (after
    /// its own step), once synced.
    pub fn contacts<R: Borrow<RemoteCar>>(
        &mut self,
        car: LocalCar<'_>,
        remotes: &HashMap<PlayerId, R>,
        sim: Sim<'_>,
    ) {
        if !self.synced || remotes.is_empty() {
            return;
        }
        if ghost_contacts(
            car.transform,
            car.dynamo,
            car.phys,
            &mut self.ghosts,
            remotes,
            sim,
        ) {
            self.contact_at = Some(self.prediction.next());
        }
    }

    /// One whole quantum of the local car the way the server steps it:
    /// control, `physics::step`, fold onto the home tile, contact against
    /// the ghosts, fold again; then into the history. Returns how far the
    /// folds moved the car (whole periods), for a caller that keeps its
    /// camera next to the car.
    pub fn step_quantum<R: Borrow<RemoteCar>>(
        &mut self,
        car: LocalCar<'_>,
        input: &QuantInput,
        remotes: &HashMap<PlayerId, R>,
        sim: Sim<'_>,
        tracks: Option<&mut level::terraform::Tracks>,
    ) -> Vec3 {
        let input_factor = QuantClock::input_factor(sim.max_quant, sim.common);
        step_input(
            car.dynamo,
            car.transform,
            input,
            sim.max_quant,
            input_factor,
            car.phys,
            sim.level,
            sim.common,
            tracks,
        );
        let stepped = car.transform.disp;
        fold_home(&mut car.transform.disp, sim.period());
        let mut shift = car.transform.disp - stepped;
        self.contacts(
            LocalCar {
                transform: car.transform,
                dynamo: car.dynamo,
                phys: car.phys,
            },
            remotes,
            sim,
        );
        let contacted = car.transform.disp;
        fold_home(&mut car.transform.disp, sim.period());
        shift += car.transform.disp - contacted;
        self.prediction.push(*input, car.transform, car.dynamo);
        shift
    }

    /// `count` quanta pass with the car held: no history, the numbering
    /// moves on. Returns the first of them.
    pub fn skip(&mut self, count: u32) -> u32 {
        self.prediction.skip(count)
    }

    /// The car was moved outside its inputs (a reset, a debug pose).
    pub fn reset(&mut self) {
        self.prediction.clear();
        self.render_offset = RenderOffset::default();
    }

    /// The connection is gone: start over on the next one.
    pub fn disconnect(&mut self) {
        *self = ClientSync::default();
    }

    /// The client moved its coordinates by `-by` (a torus rebase).
    pub fn shift(&mut self, by: Vec3) {
        for ghost in self.ghosts.values_mut() {
            ghost.transform.disp -= by;
        }
    }

    /// The server's state of the local car, with its ack and bump counter.
    /// The remote cars of the same snapshot must already be in `remotes`
    /// (a replay bumps against their ghosts). `held`: leave the car alone.
    pub fn on_snapshot<R: Borrow<RemoteCar>>(
        &mut self,
        snap: &LocalSnapshot,
        car: LocalCar<'_>,
        remotes: &HashMap<PlayerId, R>,
        sim: Sim<'_>,
        held: bool,
    ) -> (SyncEvent, bool) {
        // The ghosts restart from this snapshot; a replay carries them
        // along, else they are only led to our present.
        let (ack, bumps) = (snap.ack, snap.bumps);
        let fresh = snapshot_ghosts(remotes);
        let lead = self.prediction.next().wrapping_sub(ack);
        self.ghosts = fresh.clone();
        for (id, ghost) in self.ghosts.iter_mut() {
            let phys = &remotes[id].borrow().phys_data;
            for _ in 0..lead.min(GHOST_LEAD_MAX) {
                ghost.advance(sim.max_quant, phys, sim.level, sim.common);
            }
        }
        let bumped = self
            .server_bumps
            .replace(bumps)
            .is_some_and(|seen| seen != bumps);
        let server = ServerCar { bumped, ..snap.car };

        // The first snapshot is the server's car at its ack, and the
        // quanta we stepped since have reached the server too (they went
        // out after the join): the car goes to the snapshot and replays
        // them, so it stands where the server will have it at our next
        // quantum. Taking the snapshot pose as is put the car a round trip
        // behind (the landing from the spawn drop, the first metres of a
        // drive), and the first reconcile corrected all of that.
        let first = !self.synced;
        if first {
            self.synced = true;
        }
        let snap_to_server = |car: LocalCar<'_>| {
            *car.transform = server.transform;
            car.dynamo.linear_velocity = server.linear_velocity;
            car.dynamo.angular_velocity = server.angular_velocity;
            car.dynamo.traction = server.traction;
            car.dynamo.rudder = server.rudder;
        };
        if held {
            if first {
                // Nothing of ours to replay while the car is held.
                snap_to_server(car);
                self.reset();
                return (SyncEvent::FirstSync, bumped);
            }
            return (SyncEvent::Held, bumped);
        }

        let before = *car.transform;
        let period = Some(sim.period());
        let force = fresh.values().any(|g| {
            wrapped_distance(g.transform.disp, car.transform.disp, period) < GHOST_REPLAY_RANGE
        });
        // Car contact around the ack: the server bumped the car, a remote
        // car touches its server pose, or the prediction touched a ghost
        // shortly before the ack or since.
        let radius = |phys: &CarPhysicsData, t: &space::Transform| phys.bbox.radius * t.scale;
        let touching = remotes.iter().any(|(_, r)| {
            let r = r.borrow();
            r.pose_ready
                && wrapped_distance(r.target_transform.disp, server.transform.disp, period)
                    < radius(&r.phys_data, &r.target_transform)
                        + radius(car.phys, &server.transform)
        });
        let recent = self
            .contact_at
            .is_some_and(|q| (q.wrapping_sub(ack.wrapping_sub(Self::CONTACT_RECENT)) as i32) >= 0);
        let mut replay_contact = None;
        let mut ghosts = fresh;
        let input_factor = QuantClock::input_factor(sim.max_quant, sim.common);
        let phys = car.phys;
        let outcome = self.prediction.reconcile(
            &ServerSnapshot { car: server, ack },
            force || first,
            period,
            car.transform,
            car.dynamo,
            |t, d, input, quant| {
                step_input(
                    d,
                    t,
                    input,
                    sim.max_quant,
                    input_factor,
                    phys,
                    sim.level,
                    sim.common,
                    None,
                );
                fold_home(&mut t.disp, sim.period());
                if ghost_contacts(t, d, phys, &mut ghosts, remotes, sim) {
                    replay_contact = Some(quant);
                }
                fold_home(&mut t.disp, sim.period());
            },
        );
        if let Reconciled::Replayed(_) = outcome {
            self.ghosts = ghosts;
            // The replay is the prediction now, contacts included.
            if replay_contact.is_some() {
                self.contact_at = replay_contact;
            }
        }
        if first {
            if outcome == Reconciled::Stale {
                // The snapshot predates what we kept: take it as is.
                snap_to_server(LocalCar {
                    transform: car.transform,
                    dynamo: car.dynamo,
                    phys: car.phys,
                });
                self.prediction.clear();
            }
            // From wherever the car stood before the join: a teleport.
            self.render_offset = RenderOffset::default();
            return (SyncEvent::FirstSync, bumped);
        }
        let contact = bumped || touching || recent || replay_contact.is_some();
        // A replay that crossed a fold moved the car by whole periods too;
        // only the rest is a correction.
        let before = near(&before, car.transform.disp, period);
        let moved = before.disp.distance(car.transform.disp);
        let mut shown_at_once = false;
        if let Reconciled::Replayed(_) | Reconciled::Ahead = outcome {
            shown_at_once = !if self.no_fade {
                self.render_offset.absorb_unfaded(&before, car.transform)
            } else {
                self.render_offset.absorb(&before, car.transform)
            };
        }
        (
            SyncEvent::Reconciled {
                outcome,
                moved,
                shown_at_once,
                contact,
            },
            bumped,
        )
    }

    /// One frame of the render offset's decay.
    pub fn decay(&mut self, dt: f32) -> f32 {
        self.render_offset.decay(dt)
    }

    /// The pose to draw (and follow) for the local car's physics pose.
    pub fn drawn(&self, t: &space::Transform) -> space::Transform {
        self.render_offset.apply(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_sim() -> (level::Level, config::common::Common) {
        let level = level::load(
            &level::LevelConfig::new_test(),
            &config::settings::Geometry::default(),
        );
        let mut common = config::common::Common::test_default();
        common.nature.time_delta0 = 0.1286;
        common.nature.num_calls_analysis = 5;
        (level, common)
    }

    fn server(x: f32, y: f32) -> ServerCar {
        ServerCar {
            transform: space::Transform {
                disp: Vec3::new(x, y, 10.0),
                rot: Quat::IDENTITY,
                scale: 1.0,
            },
            linear_velocity: Vec3::new(0.0, 3.0, 0.0),
            angular_velocity: Vec3::ZERO,
            traction: 0.5,
            rudder: 0.1,
            bumped: false,
        }
    }

    fn snapshot(car: ServerCar, ack: u32, bumps: u32) -> LocalSnapshot {
        LocalSnapshot { car, ack, bumps }
    }

    #[test]
    fn remote_snaps_first_then_plays_back_behind() {
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        assert!(r.ghost().is_none(), "no ghost before a pose");
        r.on_snapshot(&server(10.0, 10.0), None);
        assert_eq!(r.render_transform.disp, Vec3::new(10.0, 10.0, 10.0));
        // Evenly timed snapshots at 20 u/s.
        for n in 1..=20 {
            r.interpolate(0.05);
            r.on_snapshot(&server(10.0 + n as f32, 10.0), None);
        }
        r.interpolate(0.0);
        // Drawn the playback delay behind the newest snapshot...
        let behind = 30.0 - r.render_transform.disp.x;
        let expected = 20.0 * RemoteCar::DELAY_BASE;
        assert!(
            (behind - expected).abs() < 0.2,
            "{behind:.2}u behind, expected {expected:.2}u"
        );
        assert!(!r.dry);
        // ...while the ghost is at the latest server state.
        let g = r.ghost().unwrap();
        assert_eq!(g.transform.disp.x, 30.0);
        assert_eq!(g.dynamo.traction, 0.5);
        assert_eq!(g.dynamo.linear_velocity, Vec3::new(0.0, 3.0, 0.0));
    }

    #[test]
    fn remote_crossing_the_seam_stays_next_to_itself() {
        let period = Some(Vec2::new(256.0, 256.0));
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        r.on_snapshot(&server(254.0, 100.0), period);
        r.interpolate(0.05);
        // The server folded it onto the home tile at x=2: drawn at 258.
        r.on_snapshot(&server(2.0, 100.0), period);
        assert_eq!(r.target_transform.disp.x, 258.0);
        for _ in 0..6 {
            r.interpolate(0.025);
            let x = r.render_transform.disp.x;
            // Reckoned on at its 80 u/s, not swept across the world.
            assert!((254.0..=270.0).contains(&x), "drawn at {x}");
        }
    }

    /// A remote driving along +X at `speed` u/s, at snapshot tick `n`,
    /// turning at `turn` rad/s about Z.
    fn moving(n: u32, speed: f32, turn: f32) -> ServerCar {
        let t = n as f32 / RemoteCar::SNAPSHOT_RATE;
        let mut car = server(100.0 + speed * t, 50.0);
        car.transform.rot = Quat::from_rotation_z(turn * t);
        car
    }

    /// One drawn frame of [`feed`].
    struct Frame {
        time: f32,
        x: f32,
        moved: f32,
        playback: f64,
        delay: f32,
        dry: bool,
    }

    /// Drive `r` at `fps` through snapshot ticks `0..ticks` of `car`, sent
    /// one per 50 ms and arriving `late(n)` seconds after (in order, as
    /// over TCP).
    fn feed(
        r: &mut RemoteCar,
        fps: u32,
        ticks: u32,
        car: impl Fn(u32) -> ServerCar,
        late: impl Fn(u32) -> f32,
    ) -> Vec<Frame> {
        let mut arrivals = Vec::new();
        let mut last = 0.0f32;
        for n in 0..ticks {
            last = last.max(n as f32 / RemoteCar::SNAPSHOT_RATE + late(n));
            arrivals.push(last);
        }
        let end = last + 1.0;
        let mut sent = 0;
        let mut frames = Vec::new();
        let mut prev = r.render_transform.disp;
        for i in 0.. {
            let time = i as f32 / fps as f32;
            if time > end {
                break;
            }
            while sent < ticks && arrivals[sent as usize] <= time {
                r.on_snapshot(&car(sent), None);
                sent += 1;
            }
            r.interpolate(1.0 / fps as f32);
            let d = r.render_transform.disp;
            frames.push(Frame {
                time,
                x: d.x,
                moved: (d - prev).length(),
                playback: r.playback,
                delay: r.delay,
                dry: r.dry,
            });
            prev = d;
        }
        frames
    }

    /// Repeatable jitter in `0..max` seconds for snapshot `n`.
    fn jitter(n: u32, max: f32) -> f32 {
        let mut x = n.wrapping_mul(2_654_435_761) ^ 0x5bd1_e995;
        x ^= x >> 15;
        x = x.wrapping_mul(0x2c1b_3c6d);
        x ^= x >> 12;
        (x % 1000) as f32 / 1000.0 * max
    }

    const FPS: u32 = 60;

    #[test]
    fn remote_plays_jittered_snapshots_back_smoothly() {
        let speed = 40.0;
        let per_frame = speed / FPS as f32;
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        // ±20 ms around a 50 ms latency.
        let frames = feed(
            &mut r,
            FPS,
            100,
            |n| moving(n, speed, 0.0),
            |n| 0.03 + jitter(n, 0.04),
        );
        let settled = || frames.iter().filter(|f| f.time > 1.0 && f.time < 5.0);
        for f in settled() {
            // At most the playback skew off the car's speed.
            assert!(
                (f.moved - per_frame).abs() < per_frame * RemoteCar::SKEW_MAX + 0.01,
                "drawn moved {:.3}u at t={:.3}, speed x dt is {per_frame:.3}",
                f.moved,
                f.time
            );
            assert!(!f.dry, "ran dry at t={:.3}", f.time);
        }
        assert!(r.target_delay > RemoteCar::DELAY_BASE && r.target_delay < RemoteCar::DELAY_MAX);
    }

    #[test]
    fn remote_delay_adapts_without_a_time_jump() {
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        // 3 s of up to 80 ms jitter, then 5 s of none.
        let frames = feed(
            &mut r,
            FPS,
            160,
            |n| moving(n, 40.0, 0.0),
            |n| 0.03 + if n < 60 { jitter(n, 0.08) } else { 0.0 },
        );
        let dt = 1.0 / FPS as f64;
        let skew = RemoteCar::SKEW_MAX as f64;
        for w in frames.windows(2).filter(|w| w[0].time > 0.5) {
            let step = w[1].playback - w[0].playback;
            assert!(
                step >= dt * (1.0 - skew) - 1e-6 && step <= dt * (1.0 + skew) + 1e-6,
                "playback stepped {step:.4}s at t={:.3}",
                w[1].time
            );
        }
        let jittery = frames.iter().find(|f| f.time >= 3.0).unwrap().delay;
        let calm = frames.last().unwrap().delay;
        assert!(calm < jittery - 0.03, "delay {jittery:.3}s -> {calm:.3}s");
        assert!(calm >= RemoteCar::DELAY_MIN && (calm - r.target_delay).abs() < 0.01);
    }

    #[test]
    fn remote_dead_reckons_through_a_gap_only_once_dry() {
        let speed = 40.0;
        let per_frame = speed / FPS as f32;
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        // Snapshots sent at 1.0..1.6 s are held for a retransmit, then
        // all arrive at once.
        let frames = feed(
            &mut r,
            FPS,
            60,
            |n| moving(n, speed, 0.0),
            |n| {
                let sent = n as f32 / RemoteCar::SNAPSHOT_RATE;
                if (1.0..1.6).contains(&sent) {
                    1.6 - sent
                } else {
                    0.03
                }
            },
        );
        let delay = RemoteCar::DELAY_BASE;
        for f in frames.iter().filter(|f| f.time > 0.5 && f.time < 3.0) {
            assert!(
                (f.moved - per_frame).abs() < 0.03,
                "drawn moved {:.3}u at t={:.3}, speed x dt is {per_frame:.3}",
                f.moved,
                f.time
            );
            // The buffer covers the start of the stall: the last
            // snapshot before it (sent at 0.95 s) is played until about
            // 0.98 + delay.
            if f.time < 0.95 + delay {
                assert!(!f.dry, "dry at t={:.3}", f.time);
            }
        }
        let dry = frames.iter().filter(|f| f.dry && f.time < 2.5).count();
        assert!(dry > 20 && dry < 40, "{dry} frames reckoned");
    }

    #[test]
    fn remote_resumes_after_a_gap_without_a_jump() {
        let speed = 40.0;
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        // Drives to tick 10 and stops there, but the snapshots after it
        // are held for a second.
        let stopped = |n: u32| moving(n.min(10), speed, 0.0);
        let frames = feed(&mut r, FPS, 40, stopped, |n| {
            if n > 10 && n < 30 {
                1.6 - n as f32 / 20.0
            } else {
                0.03
            }
        });
        // Reckoned on past where it stopped, at most the cap...
        let held = moving(10, speed, 0.0).transform.disp.x;
        let max_x = speed * RemoteCar::DEAD_RECKON_MAX;
        let ahead = frames.iter().map(|f| f.x).fold(f32::MIN, f32::max) - held;
        assert!(
            ahead > 0.5 * max_x && ahead < max_x + 0.1,
            "{ahead:.2}u ahead"
        );
        // ...then slid back over a few frames, not jumped.
        let worst = frames
            .iter()
            .filter(|f| f.time > 1.5)
            .map(|f| f.moved)
            .fold(0.0, f32::max);
        assert!(worst < 0.2 * max_x, "slid {worst:.2}u in a frame");
        let end = r.render_transform.disp.x;
        assert!(
            (end - held).abs() < 1e-2,
            "ends at {end}, stopped at {held}"
        );
    }

    /// A remote going round a circle of `radius` at `speed`, at tick `n`.
    fn circling(n: u32, speed: f32, radius: f32) -> ServerCar {
        let angle = speed / radius * n as f32 / RemoteCar::SNAPSHOT_RATE;
        let mut car = server(100.0 + radius * angle.sin(), 100.0 - radius * angle.cos());
        car.transform.rot = Quat::from_rotation_z(angle);
        car
    }

    #[test]
    fn remote_dead_reckoning_turns_and_holds() {
        let (speed, radius) = (40.0, 20.0);
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        for n in 0..4 {
            r.on_snapshot(&circling(n, speed, radius), None);
            r.interpolate(0.05);
        }
        // No more snapshots: played to the last, reckoned on round the
        // circle, held at the cap.
        for _ in 0..40 {
            r.interpolate(0.05);
        }
        assert!(r.dry);
        let truth = circling(3 + 14, speed, radius).transform;
        let off = (r.render_transform.disp - truth.disp).length();
        assert!(off < 0.5, "{off:.2}u off the circle");
        assert!(r.render_transform.rot.angle_between(truth.rot) < 0.02);
        let held = r.render_transform.disp;
        r.interpolate(1.0);
        assert!((r.render_transform.disp - held).length() < 1e-4);
    }

    #[test]
    fn remote_correction_spreads_over_frames_at_a_low_frame_rate() {
        let speed = 40.0;
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        let stopped = |n: u32| moving(n.min(10), speed, 0.0);
        // 8 fps: a whole decay would fit in one frame.
        let frames = feed(&mut r, 8, 40, stopped, |n| {
            if n > 10 && n < 30 {
                1.6 - n as f32 / 20.0
            } else {
                0.03
            }
        });
        let i = frames.iter().position(|f| f.time >= 1.6).unwrap();
        let error = speed * RemoteCar::DEAD_RECKON_MAX;
        for f in &frames[i..i + 3] {
            assert!(f.moved <= error / 3.0 + 0.1, "moved {:.2}u", f.moved);
        }
    }

    #[test]
    fn remote_plays_by_sender_time_across_a_skipped_tick() {
        let speed = 20.0;
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        // Tick 10 never comes; the rest arrive on time. Played by their
        // tick times, the car keeps its speed through the hole.
        let mut x_prev: Option<f32> = None;
        for n in 0..40u32 {
            if n != 10 {
                let time = n as f64 / RemoteCar::SNAPSHOT_RATE as f64;
                r.on_snapshot_at(&moving(n, speed, 0.0), None, time);
            }
            for _ in 0..3 {
                r.interpolate(1.0 / 60.0);
                let x = r.render_transform.disp.x;
                if let Some(p) = x_prev
                    && n > 5
                {
                    let moved = x - p;
                    assert!(
                        (moved - speed / 60.0).abs() < 0.05,
                        "moved {moved:.3}u in a frame at tick {n}"
                    );
                }
                x_prev = Some(x);
            }
        }
    }

    #[test]
    fn remote_forgets_an_early_burst() {
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        // The first snapshots come straight away, every later one 300 ms
        // on (a slower route): once the burst is out of the window the
        // buffer is sized for the new route, not dry.
        let car = |n: u32| moving(n, 20.0, 0.0);
        let frames = feed(&mut r, FPS, 200, car, |n| if n < 5 { 0.0 } else { 0.3 });
        for f in frames.iter().filter(|f| f.time > 6.0 && f.time < 9.5) {
            assert!(!f.dry, "dry at t={:.3}", f.time);
            assert!(
                f.delay < RemoteCar::DELAY_BASE + 0.03,
                "{:.3}s behind",
                f.delay
            );
        }
    }

    /// Quanta a second at the server's 4.5 a tick.
    const QPS: f32 = 90.0;

    /// The server's acks for a car whose owner's input stalls for ticks
    /// `stall` and then all comes in: it runs 4 or 5 quanta a tick (4.5 a
    /// tick of input), none while starved, then twice that to catch up.
    fn starved_acks(ticks: u32, stall: std::ops::Range<u32>) -> Vec<u32> {
        let (mut ack, mut carry, mut acks) = (0u32, 0.0f32, Vec::new());
        for n in 0..ticks {
            let received = if stall.contains(&n) {
                (4.5 * stall.start as f32) as u32
            } else {
                (4.5 * (n + 1) as f32) as u32
            };
            carry += 4.5;
            let budget = carry.floor() as u32;
            carry -= budget as f32;
            let pending = received - ack;
            let run = if pending > budget {
                (2 * budget).min(pending)
            } else {
                pending
            };
            ack += run;
            acks.push(ack);
        }
        acks
    }

    #[test]
    fn remote_by_ack_drives_through_a_starved_input_evenly() {
        let speed = 20.0;
        let acks = starved_acks(80, 20..28);
        // The server pose is where the car's input got it.
        let pose = |ack: u32| server(100.0 + speed * ack as f32 / QPS, 50.0);
        let play = |by_ack: bool| {
            let mut r = RemoteCar::new(CarPhysicsData::test_default());
            let mut moves = Vec::new();
            let mut prev = None;
            for i in 0..(80 * 3) {
                if i % 3 == 0 {
                    let n = i / 3;
                    let ack = acks[n as usize];
                    let time = if by_ack {
                        r.agent_time(ack, n, QPS)
                    } else {
                        n as f64 / RemoteCar::SNAPSHOT_RATE as f64
                    };
                    r.on_snapshot_at(&pose(ack), None, time);
                }
                r.interpolate(1.0 / 60.0);
                let x = r.render_transform.disp.x;
                if let Some(p) = prev
                    && i > 30
                {
                    moves.push(x - p);
                }
                prev = Some(x);
            }
            moves
        };
        let step = speed / 60.0;
        // By the tick the car stands for the stall and then rushes.
        let by_tick = play(false);
        assert!(by_tick.iter().any(|&m| m < 0.1 * step));
        assert!(by_tick.iter().any(|&m| m > 1.5 * step));
        // By the ack it keeps going at about its speed.
        for (i, m) in play(true).into_iter().enumerate() {
            assert!(
                (m - step).abs() < 0.35 * step,
                "frame {i}: moved {m:.3}u, driving {step:.3}u a frame"
            );
        }
    }

    #[test]
    fn remote_ack_timeline_repins_on_a_jump() {
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        let t0 = r.agent_time(1000, 100, QPS);
        assert_eq!(t0, 5.0);
        // 4 or 5 quanta a tick: by the ack.
        let t1 = r.agent_time(1004, 101, QPS);
        assert!((t1 - (5.0 + 4.0 / 90.0)).abs() < 1e-9);
        // None run: the same moment again.
        assert_eq!(r.agent_time(1004, 102, QPS), t1);
        // A respawn moves the ack far on: pinned again next to the last
        // snapshot by the tick.
        let t2 = r.agent_time(500_000, 103, QPS);
        assert!((t2 - (t1 + 0.05)).abs() < 1e-9);
        let t3 = r.agent_time(500_009, 104, QPS);
        assert!((t3 - (t2 + 0.1)).abs() < 1e-9);
        // The ack wraps.
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        let t0 = r.agent_time(u32::MAX - 1, 0, QPS);
        let t1 = r.agent_time(3, 1, QPS);
        assert!((t1 - t0 - 5.0 / 90.0).abs() < 1e-9);
    }

    #[test]
    fn remote_teleport_is_shown_at_once() {
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        for n in 0..4 {
            r.on_snapshot(&moving(n, 40.0, 0.0), None);
            r.interpolate(0.05);
        }
        // Respawned far away: no slide across the map, no reckoning on.
        r.on_snapshot(&server(900.0, 900.0), None);
        let at = Vec3::new(900.0, 900.0, 10.0);
        assert_eq!(r.render_transform.disp, at);
        r.interpolate(0.3);
        assert!((r.render_transform.disp - at).length() < 1e-3);
    }

    #[test]
    fn frame_input_puts_one_shots_on_the_last_quantum() {
        let held = QuantInput {
            motor: 1.0,
            brake: true,
            turbo: true,
            jump: Some(3.0),
            roll: -1.0,
            ..Default::default()
        };
        let first = ClientSync::frame_input(&held, 0, 3);
        assert_eq!(first.jump, None);
        assert_eq!(first.roll, 0.0);
        assert!(first.brake && first.turbo);
        assert_eq!(ClientSync::frame_input(&held, 2, 3), held);
        let c = net_control(&held);
        assert_eq!(c.jump, Some(3.0));
        assert_eq!(c.roll, -1.0);
        assert!(c.brake && c.turbo);
    }

    #[test]
    fn first_snapshot_syncs_and_later_ones_reconcile() {
        let (level, common) = test_sim();
        let sim = Sim {
            level: &level,
            common: &common,
            max_quant: 0.1,
        };
        let phys = CarPhysicsData::test_default();
        let remotes: HashMap<PlayerId, RemoteCar> = HashMap::new();
        let mut sync = ClientSync::default();
        let mut t = space::Transform::IDENTITY;
        let mut d = Dynamo::default();
        let s = server(64.0, 64.0);
        let (event, bumped) = sync.on_snapshot(
            &snapshot(s, 0, 0),
            LocalCar {
                transform: &mut t,
                dynamo: &mut d,
                phys: &phys,
            },
            &remotes,
            sim,
            false,
        );
        assert_eq!(event, SyncEvent::FirstSync);
        assert!(!bumped);
        assert_eq!(t.disp, s.transform.disp);
        assert!(sync.synced);

        // Step a few quanta and send the server's version of the same
        // quanta back: in sync.
        let mut server_t = t;
        let mut server_d = d;
        let input = QuantInput {
            motor: 1.0,
            ..Default::default()
        };
        for _ in 0..5 {
            sync.step_quantum(
                LocalCar {
                    transform: &mut t,
                    dynamo: &mut d,
                    phys: &phys,
                },
                &input,
                &remotes,
                sim,
                None,
            );
            step_input(
                &mut server_d,
                &mut server_t,
                &input,
                0.1,
                QuantClock::input_factor(0.1, &common),
                &phys,
                &level,
                &common,
                None,
            );
            fold_home(&mut server_t.disp, level.period());
        }
        let snap = ServerCar {
            transform: server_t,
            linear_velocity: server_d.linear_velocity,
            angular_velocity: server_d.angular_velocity,
            traction: server_d.traction,
            rudder: server_d.rudder,
            bumped: false,
        };
        let car = LocalCar {
            transform: &mut t,
            dynamo: &mut d,
            phys: &phys,
        };
        let (event, _) = sync.on_snapshot(&snapshot(snap, 5, 0), car, &remotes, sim, false);
        assert!(
            matches!(
                event,
                SyncEvent::Reconciled {
                    outcome: Reconciled::InSync,
                    ..
                }
            ),
            "{event:?}"
        );

        // A server pushed 1u aside: replayed, the move hidden in the offset.
        let mut pushed = snap;
        pushed.transform.disp.x += 1.0;
        let car = LocalCar {
            transform: &mut t,
            dynamo: &mut d,
            phys: &phys,
        };
        let (event, bumped) = sync.on_snapshot(&snapshot(pushed, 5, 1), car, &remotes, sim, false);
        assert!(bumped, "bump counter changed");
        let SyncEvent::Reconciled {
            outcome,
            moved,
            shown_at_once,
            ..
        } = event
        else {
            panic!("{event:?}");
        };
        assert_eq!(outcome, Reconciled::Replayed(0));
        assert!((moved - 1.0).abs() < 1e-3);
        assert!(!shown_at_once);
        let drawn = sync.drawn(&t);
        assert!((drawn.disp.x - (t.disp.x - 1.0)).abs() < 1e-3);

        // Held: untouched.
        let before = t;
        let car = LocalCar {
            transform: &mut t,
            dynamo: &mut d,
            phys: &phys,
        };
        pushed.transform.disp.x += 5.0;
        let (event, _) = sync.on_snapshot(&snapshot(pushed, 5, 1), car, &remotes, sim, true);
        assert_eq!(event, SyncEvent::Held);
        assert_eq!(t.disp, before.disp);
    }

    #[test]
    fn step_quantum_folds_home_and_reports_the_shift() {
        let (level, common) = test_sim();
        let sim = Sim {
            level: &level,
            common: &common,
            max_quant: 0.1,
        };
        let phys = CarPhysicsData::test_default();
        let remotes: HashMap<PlayerId, RemoteCar> = HashMap::new();
        let mut sync = ClientSync::default();
        let period = level.period();
        let mut t = space::Transform {
            disp: Vec3::new(-0.01, 20.0, level.get((0, 20)).high() + 5.0),
            rot: Quat::IDENTITY,
            scale: 1.0,
        };
        let mut d = Dynamo::default();
        let before = t.disp;
        let shift = sync.step_quantum(
            LocalCar {
                transform: &mut t,
                dynamo: &mut d,
                phys: &phys,
            },
            &QuantInput::default(),
            &remotes,
            sim,
            None,
        );
        assert!((0.0..period.x).contains(&t.disp.x));
        assert!(shift.x == period.x || shift.x == 0.0, "{shift:?}");
        assert!((t.disp.x - shift.x - before.x).abs() < 1.0);
        assert_eq!(sync.prediction.next(), 1);
        assert_eq!(sync.prediction.len(), 1);
    }

    #[test]
    fn ghosts_lead_to_the_present_and_push_back() {
        let (level, common) = test_sim();
        let sim = Sim {
            level: &level,
            common: &common,
            max_quant: 0.1,
        };
        let phys = CarPhysicsData::test_default();
        let (x, y) = (level.size.0 / 4, level.size.1 / 4);
        let z = level.get((x, y)).high() + 5.0;
        let spot = |dx: f32| space::Transform {
            disp: Vec3::new(x as f32 + dx, y as f32, z),
            rot: Quat::IDENTITY,
            scale: 1.0,
        };
        let mut remotes = HashMap::new();
        let mut r = RemoteCar::new(phys.clone());
        r.on_snapshot(
            &ServerCar {
                transform: spot(0.0),
                linear_velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
                traction: 0.0,
                rudder: 0.0,
                bumped: false,
            },
            Some(level.period()),
        );
        remotes.insert(7, r);
        let mut sync = ClientSync::default();
        let mut t = spot(0.5);
        let mut d = Dynamo::default();
        let me = ServerCar {
            transform: t,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            traction: 0.0,
            rudder: 0.0,
            bumped: false,
        };
        let car = LocalCar {
            transform: &mut t,
            dynamo: &mut d,
            phys: &phys,
        };
        sync.on_snapshot(&snapshot(me, 0, 0), car, &remotes, sim, false);
        assert_eq!(sync.ghosts.len(), 1);
        // Overlapping cars: the contact pushes ours away from the ghost.
        let start = t.disp;
        for _ in 0..3 {
            sync.step_quantum(
                LocalCar {
                    transform: &mut t,
                    dynamo: &mut d,
                    phys: &phys,
                },
                &QuantInput::default(),
                &remotes,
                sim,
                None,
            );
        }
        let ghost = sync.ghosts[&7];
        assert_eq!(ghost.lead, 3);
        let gap = wrapped_distance(t.disp, ghost.transform.disp, Some(level.period()));
        assert!(
            gap > start.distance(spot(0.0).disp),
            "overlap not resolved: gap {gap}"
        );
    }
}
