//! Multiplayer client: the parts of the predicting client that the native
//! game (`bin/road`) and the web build (`bin/web`) share.
//!
//! - [`RemoteCar`]: another player's car as the client sees it: the latest
//!   server state (the source of its contact [`Ghost`]) and the pose drawn
//!   from it: each snapshot blended in from the drawn pose, then dead
//!   reckoned through gaps.
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

/// Another player's car.
///
/// Drawn by dead reckoning with a blend: from each snapshot the car is
/// carried on at the motion of its last two snapshots, and a new snapshot
/// is blended in from wherever the car is drawn when it arrives, both
/// moving on while they blend, over [`Self::BLEND`]. Past
/// [`Self::DEAD_RECKON_MAX`] without a snapshot the car holds. Over TCP a
/// lost segment stalls every snapshot behind it for a retransmit (~RTT +
/// 200 ms); drawn between the last two snapshots instead, the car stood
/// still for the stall and then jumped the whole way at once.
#[derive(Clone)]
pub struct RemoteCar {
    /// Contact shape for the ghost.
    pub phys_data: CarPhysicsData,
    /// The pose drawn when the latest snapshot arrived (blend source).
    pub prev_transform: space::Transform,
    /// Latest snapshot pose (blend target, ghost start).
    pub target_transform: space::Transform,
    /// The pose drawn this frame.
    pub render_transform: space::Transform,
    /// Snapshot intervals since the latest snapshot arrived.
    pub interp_t: f32,
    /// False until the first snapshot pose: nothing to blend from yet.
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
    /// The motion the blend source carries on with: the one the car was
    /// drawn with when the latest snapshot arrived.
    prev_velocity: Vec3,
    prev_spin: Vec3,
}

impl RemoteCar {
    /// Snapshots a second (the server tick).
    pub const SNAPSHOT_RATE: f32 = 20.0;
    /// Seconds a new snapshot takes to blend in. Two intervals rather than
    /// one: with ±20 ms of jitter the drawn car peaks at 1.3x its speed
    /// instead of 1.7x, for a few frames more to settle from rest.
    pub const BLEND: f32 = 2.0 / Self::SNAPSHOT_RATE;
    /// Longest the drawn car is carried on past a snapshot's arrival
    /// before it holds: covers a retransmit stall at a 250 ms RTT (~0.45
    /// s) plus the interval, not a dropped connection.
    pub const DEAD_RECKON_MAX: f32 = 0.7;
    /// A snapshot this far from the last one, or from the drawn car, is a
    /// respawn or teleport: drawn there at once, not slid to.
    pub const TELEPORT: f32 = 64.0;

    pub fn new(phys_data: CarPhysicsData) -> Self {
        RemoteCar {
            phys_data,
            prev_transform: space::Transform::IDENTITY,
            target_transform: space::Transform::IDENTITY,
            render_transform: space::Transform::IDENTITY,
            interp_t: 0.0,
            pose_ready: false,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            traction: 0.0,
            rudder: 0.0,
            control: QuantInput::default(),
            reckon_velocity: Vec3::ZERO,
            reckon_spin: Vec3::ZERO,
            prev_velocity: Vec3::ZERO,
            prev_spin: Vec3::ZERO,
        }
    }

    /// Take a server snapshot of this car (see `on_snapshot`).
    pub fn on_agent(&mut self, agent: &vangers_net::AgentState, period: Option<Vec2>) {
        self.control = agent_control(agent);
        self.on_snapshot(&server_car(agent), period);
    }

    /// Take a snapshot of this car. The first one (and a teleport) is
    /// shown at once; later ones are blended in from the pose drawn now,
    /// on the replica nearest the last one (the server folds every car
    /// onto the home tile, so a car crossing the seam would otherwise be
    /// drawn sweeping across the whole world).
    pub fn on_snapshot(&mut self, car: &ServerCar, period: Option<Vec2>) {
        self.linear_velocity = car.linear_velocity;
        self.angular_velocity = car.angular_velocity;
        self.traction = car.traction;
        self.rudder = car.rudder;
        let mut target = car.transform;
        if self.pose_ready
            && let Some(p) = period
        {
            let d = target.disp.truncate() - self.target_transform.disp.truncate();
            let fold = |d: f32, p: f32| if p > 0.0 { (d / p).round() * p } else { 0.0 };
            target.disp.x -= fold(d.x, p.x);
            target.disp.y -= fold(d.y, p.y);
        }
        let step = target.disp - self.target_transform.disp;
        if !self.pose_ready
            || step.length() > Self::TELEPORT
            || (target.disp - self.render_transform.disp).length() > Self::TELEPORT
        {
            self.prev_transform = target;
            self.render_transform = target;
            self.prev_velocity = Vec3::ZERO;
            self.prev_spin = Vec3::ZERO;
            self.reckon_velocity = Vec3::ZERO;
            self.reckon_spin = Vec3::ZERO;
            self.pose_ready = true;
        } else {
            let mut turn = target.rot * self.target_transform.rot.inverse();
            if turn.w < 0.0 {
                turn = -turn;
            }
            // The blend source carries on as the drawn car was carried
            // (still, once held past the cap).
            let since = self.interp_t / Self::SNAPSHOT_RATE;
            let blend = (since / Self::BLEND).min(1.0);
            (self.prev_velocity, self.prev_spin) = if since < Self::DEAD_RECKON_MAX {
                (
                    self.prev_velocity.lerp(self.reckon_velocity, blend),
                    self.prev_spin.lerp(self.reckon_spin, blend),
                )
            } else {
                (Vec3::ZERO, Vec3::ZERO)
            };
            self.prev_transform = self.render_transform;
            self.reckon_velocity = step * Self::SNAPSHOT_RATE;
            self.reckon_spin = turn.to_scaled_axis() * Self::SNAPSHOT_RATE;
        }
        self.target_transform = target;
        self.interp_t = 0.0;
    }

    /// Move the drawn pose `delta` seconds on.
    pub fn interpolate(&mut self, delta: f32) {
        self.interp_t += delta * Self::SNAPSHOT_RATE;
        let since = self.interp_t / Self::SNAPSHOT_RATE;
        let ahead = since.min(Self::DEAD_RECKON_MAX);
        let blend = (since / Self::BLEND).min(1.0);
        let carry = |t: &space::Transform, v: Vec3, w: Vec3| space::Transform {
            disp: t.disp + v * ahead,
            rot: (Quat::from_scaled_axis(w * ahead) * t.rot).normalize(),
            scale: t.scale,
        };
        let a = carry(&self.prev_transform, self.prev_velocity, self.prev_spin);
        let b = carry(
            &self.target_transform,
            self.reckon_velocity,
            self.reckon_spin,
        );
        self.render_transform = space::Transform {
            disp: a.disp.lerp(b.disp, blend),
            rot: a.rot.slerp(b.rot, blend),
            scale: a.scale + (b.scale - a.scale) * blend,
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
        self.prev_transform.disp -= by;
        self.target_transform.disp -= by;
        self.render_transform.disp -= by;
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
    fn remote_snaps_first_then_blends() {
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        assert!(r.ghost().is_none(), "no ghost before a pose");
        r.on_snapshot(&server(10.0, 10.0), None);
        assert_eq!(r.render_transform.disp, Vec3::new(10.0, 10.0, 10.0));
        r.on_snapshot(&server(11.0, 10.0), None);
        // Not moved by the arrival, only by the frames after it.
        assert_eq!(r.render_transform.disp.x, 10.0);
        r.interpolate(0.05);
        // Half blended between standing at 10 and 11 carried on at 20 u/s.
        assert!((r.render_transform.disp.x - 11.0).abs() < 1e-4);
        r.interpolate(0.05);
        assert!((r.render_transform.disp.x - 13.0).abs() < 1e-4);
        let g = r.ghost().unwrap();
        assert_eq!(g.transform.disp.x, 11.0);
        assert_eq!(g.dynamo.traction, 0.5);
        assert_eq!(g.dynamo.linear_velocity, Vec3::new(0.0, 3.0, 0.0));
        // Drawn a unit ahead at 20 u/s: the next snapshot blends in from
        // there, both carried on.
        r.on_snapshot(&server(12.0, 10.0), None);
        r.interpolate(0.05);
        assert!((r.render_transform.disp.x - 13.5).abs() < 1e-4);
    }

    #[test]
    fn remote_crossing_the_seam_stays_next_to_itself() {
        let period = Some(Vec2::new(256.0, 256.0));
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        r.on_snapshot(&server(254.0, 100.0), period);
        // The server folded it onto the home tile at x=2: drawn at 258.
        r.on_snapshot(&server(2.0, 100.0), period);
        assert_eq!(r.target_transform.disp.x, 258.0);
        r.interpolate(0.05);
        let x = r.render_transform.disp.x;
        assert!(x > 254.0 && x < 262.0, "drawn at {x}");
    }

    /// A remote driving along +X at `speed` u/s, at snapshot tick `n`,
    /// turning at `turn` rad/s about Z.
    fn moving(n: u32, speed: f32, turn: f32) -> ServerCar {
        let t = n as f32 / RemoteCar::SNAPSHOT_RATE;
        let mut car = server(100.0 + speed * t, 50.0);
        car.transform.rot = Quat::from_rotation_z(turn * t);
        car
    }

    const FPS: u32 = 60;

    /// Drive `r` at 60 fps through snapshot ticks `0..ticks`, each sent on
    /// the frame it is due, except that none arrive in frames `stall`
    /// (the held ones all arrive on the frame after). Returns the drawn
    /// move of every frame, with the frame's time.
    fn drive(
        r: &mut RemoteCar,
        ticks: u32,
        speed: f32,
        stall: std::ops::Range<u32>,
    ) -> Vec<(f32, f32)> {
        let per_tick = FPS / RemoteCar::SNAPSHOT_RATE as u32;
        let mut sent = 0;
        let mut moves = Vec::new();
        let mut last = r.render_transform.disp;
        for frame in 0.. {
            if sent == ticks {
                break;
            }
            if !stall.contains(&frame) {
                while sent <= frame / per_tick && sent < ticks {
                    r.on_snapshot(&moving(sent, speed, 0.0), None);
                    sent += 1;
                }
            }
            r.interpolate(1.0 / FPS as f32);
            let d = r.render_transform.disp;
            moves.push((frame as f32 / FPS as f32, (d - last).length()));
            last = d;
        }
        moves
    }

    #[test]
    fn remote_dead_reckons_steady_motion_through_a_gap() {
        let speed = 40.0;
        let per_frame = speed / FPS as f32;
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        // 0.6 s without a snapshot from t=1 s, then 12 at once.
        let moves = drive(&mut r, 60, speed, FPS..FPS * 8 / 5);
        // Once settled from the start at rest (overlapping blends take a
        // few snapshots), every frame moves the car by its speed: through
        // the gap and out of it.
        for &(t, m) in moves.iter().filter(|&&(t, _)| t > 0.75) {
            assert!(
                (m - per_frame).abs() < 0.02,
                "drawn moved {m:.3}u at t={t:.3}, speed x dt is {per_frame:.3}"
            );
        }
        // On the last snapshot's line, carried on by less than a snapshot.
        let last = moving(59, speed, 0.0).transform.disp;
        let ahead = r.render_transform.disp - last;
        assert!(ahead.y.abs() < 1e-3 && ahead.x >= 0.0 && ahead.x < speed / 20.0);
    }

    #[test]
    fn remote_resumes_after_a_gap_without_a_jump() {
        let speed = 40.0;
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        drive(&mut r, 11, speed, 0..0);
        // The car stopped where its last snapshot (tick 10) showed it, and
        // a 0.8 s stall followed: drawn on to the cap, then held.
        let held = moving(10, speed, 0.0);
        for _ in 0..48 {
            r.interpolate(1.0 / FPS as f32);
        }
        let ahead = r.render_transform.disp.x - held.transform.disp.x;
        assert!(ahead > 0.0 && ahead <= speed * RemoteCar::DEAD_RECKON_MAX + 0.1);
        let before = r.render_transform.disp;
        // The stall ends: it was standing there all along.
        for _ in 0..16 {
            r.on_snapshot(&held, None);
        }
        assert_eq!(r.render_transform.disp, before, "no jump on arrival");
        let frames = (RemoteCar::BLEND * FPS as f32).round() as usize;
        let mut last = before;
        for _ in 0..frames {
            r.interpolate(1.0 / FPS as f32);
            let step = (r.render_transform.disp - last).length();
            assert!(
                step <= ahead / frames as f32 * 2.0,
                "slid {step:.2}u in a frame"
            );
            last = r.render_transform.disp;
        }
        assert!((r.render_transform.disp - held.transform.disp).length() < 1e-3);
        // Standing still: nothing to reckon on.
        r.interpolate(0.5);
        assert!((r.render_transform.disp - held.transform.disp).length() < 1e-3);
    }

    #[test]
    fn remote_dead_reckoning_turns_and_holds() {
        let (speed, turn) = (40.0, 1.0);
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        for n in 0..4 {
            r.on_snapshot(&moving(n, speed, turn), None);
            r.interpolate(0.05);
        }
        // 0.3 s later in all: turned on by 0.3 rad from tick 3.
        r.interpolate(0.25);
        let yaw = r.render_transform.rot.to_euler(glam::EulerRot::ZYX).0;
        assert!((yaw - turn * 0.45).abs() < 1e-3, "yaw {yaw}");
        // Past the cap it holds.
        r.interpolate(1.0);
        let held = r.render_transform;
        r.interpolate(1.0);
        assert_eq!(r.render_transform.disp, held.disp);
        let reckoned = held.disp.x - moving(3, speed, turn).transform.disp.x;
        assert!((reckoned - speed * RemoteCar::DEAD_RECKON_MAX).abs() < 1e-3);
    }

    #[test]
    fn remote_teleport_is_shown_at_once() {
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        r.on_snapshot(&moving(0, 40.0, 0.0), None);
        r.on_snapshot(&moving(1, 40.0, 0.0), None);
        r.interpolate(0.2);
        // Respawned far away: no slide across the map, no reckoning on.
        r.on_snapshot(&server(900.0, 900.0), None);
        assert_eq!(r.render_transform.disp, Vec3::new(900.0, 900.0, 10.0));
        r.interpolate(0.3);
        assert_eq!(r.render_transform.disp, Vec3::new(900.0, 900.0, 10.0));
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
