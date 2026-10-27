//! Multiplayer client: the parts of the predicting client that the native
//! game (`bin/road`) and the web build (`bin/web`) share.
//!
//! - [`RemoteCar`]: another player's car as the client sees it: the latest
//!   server state (the source of its contact [`Ghost`]) and the pose drawn
//!   between the last two snapshots.
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
#[derive(Clone)]
pub struct RemoteCar {
    /// Contact shape for the ghost.
    pub phys_data: CarPhysicsData,
    /// Previous snapshot pose (interpolation source).
    pub prev_transform: space::Transform,
    /// Latest snapshot pose (interpolation target, ghost start).
    pub target_transform: space::Transform,
    /// The pose drawn this frame.
    pub render_transform: space::Transform,
    /// Interpolation progress, 0..=1.
    pub interp_t: f32,
    /// False until the first snapshot pose: nothing to lerp from yet.
    pub pose_ready: bool,
    /// Car-local velocities, traction and rudder of the latest snapshot.
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub traction: f32,
    pub rudder: f32,
    /// The held control the server last ran the car with: its ghost is
    /// driven on with it.
    pub control: QuantInput,
}

impl RemoteCar {
    /// Snapshots a second (the server tick), the rate the drawn pose
    /// moves from one snapshot to the next.
    pub const SNAPSHOT_RATE: f32 = 20.0;

    pub fn new(phys_data: CarPhysicsData) -> Self {
        RemoteCar {
            phys_data,
            prev_transform: space::Transform::IDENTITY,
            target_transform: space::Transform::IDENTITY,
            render_transform: space::Transform::IDENTITY,
            interp_t: 1.0,
            pose_ready: false,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            traction: 0.0,
            rudder: 0.0,
            control: QuantInput::default(),
        }
    }

    /// Take a `WorldState` entry of this car: its pose and dynamics
    /// ([`Self::on_snapshot`]) and its held control.
    pub fn on_agent(&mut self, agent: &vangers_net::AgentState, period: Option<Vec2>) {
        self.control = agent_control(agent);
        self.on_snapshot(&server_car(agent), period);
    }

    /// Take a snapshot of this car. The first one is shown at once; later
    /// ones become the new interpolation target, on the replica nearest
    /// the last one (the server folds every car onto the home tile, so a
    /// car crossing the seam would otherwise be drawn sweeping across the
    /// whole world for a tick).
    pub fn on_snapshot(&mut self, car: &ServerCar, period: Option<Vec2>) {
        self.linear_velocity = car.linear_velocity;
        self.angular_velocity = car.angular_velocity;
        self.traction = car.traction;
        self.rudder = car.rudder;
        if !self.pose_ready {
            self.prev_transform = car.transform;
            self.target_transform = car.transform;
            self.render_transform = car.transform;
            self.interp_t = 1.0;
            self.pose_ready = true;
        } else {
            let mut target = car.transform;
            if let Some(p) = period {
                let d = target.disp.truncate() - self.target_transform.disp.truncate();
                let fold = |d: f32, p: f32| if p > 0.0 { (d / p).round() * p } else { 0.0 };
                target.disp.x -= fold(d.x, p.x);
                target.disp.y -= fold(d.y, p.y);
            }
            self.prev_transform = self.target_transform;
            self.target_transform = target;
            self.interp_t = 0.0;
        }
    }

    /// Move the drawn pose `delta` seconds on toward the latest snapshot.
    pub fn interpolate(&mut self, delta: f32) {
        self.interp_t = (self.interp_t + delta * Self::SNAPSHOT_RATE).min(1.0);
        let t = self.interp_t;
        let (a, b) = (&self.prev_transform, &self.target_transform);
        self.render_transform = space::Transform {
            disp: a.disp.lerp(b.disp, t),
            rot: a.rot.slerp(b.rot, t),
            scale: a.scale + (b.scale - a.scale) * t,
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
        /// touching the server's pose): past `RenderOffset::TELEPORT` it is
        /// faded out over `RenderOffset::CONTACT_FADE` rather than shown at
        /// once, up to `RenderOffset::CONTACT_TELEPORT`.
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
    /// Treat contact corrections like any other (shown at once past
    /// `RenderOffset::TELEPORT`, decayed at the usual rate): the behaviour
    /// before contact fading, for comparisons.
    pub no_contact_fade: bool,
}

impl ClientSync {
    /// A predicted contact this many quanta before the ack still makes a
    /// correction a contact correction (the server's contact may come a
    /// little earlier or later than the prediction's).
    pub const CONTACT_RECENT: u32 = 32;
    /// A faded contact correction larger than this is worth a log line
    /// (`Server correction ...u in contact: faded`).
    pub const CONTACT_LOG: f32 = 10.0;

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

        if !self.synced {
            self.synced = true;
            *car.transform = server.transform;
            car.dynamo.linear_velocity = server.linear_velocity;
            car.dynamo.angular_velocity = server.angular_velocity;
            car.dynamo.traction = server.traction;
            car.dynamo.rudder = server.rudder;
            self.reset();
            return (SyncEvent::FirstSync, bumped);
        }
        if held {
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
            force,
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
        let contact = bumped || touching || recent || replay_contact.is_some();
        // A replay that crossed a fold moved the car by whole periods too;
        // only the rest is a correction.
        let before = near(&before, car.transform.disp, period);
        let moved = before.disp.distance(car.transform.disp);
        let mut shown_at_once = false;
        if let Reconciled::Replayed(_) | Reconciled::Ahead = outcome {
            shown_at_once = !if contact && !self.no_contact_fade {
                self.render_offset.absorb_contact(&before, car.transform)
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
    fn remote_snaps_first_then_interpolates() {
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        assert!(r.ghost().is_none(), "no ghost before a pose");
        r.on_snapshot(&server(10.0, 10.0), None);
        assert_eq!(r.render_transform.disp, Vec3::new(10.0, 10.0, 10.0));
        r.on_snapshot(&server(20.0, 10.0), None);
        r.interpolate(0.025);
        assert!((r.render_transform.disp.x - 15.0).abs() < 1e-4);
        r.interpolate(1.0);
        assert_eq!(r.render_transform.disp.x, 20.0);
        let g = r.ghost().unwrap();
        assert_eq!(g.transform.disp.x, 20.0);
        assert_eq!(g.dynamo.traction, 0.5);
        assert_eq!(g.dynamo.linear_velocity, Vec3::new(0.0, 3.0, 0.0));
    }

    #[test]
    fn remote_crossing_the_seam_stays_next_to_itself() {
        let period = Some(Vec2::new(256.0, 256.0));
        let mut r = RemoteCar::new(CarPhysicsData::test_default());
        r.on_snapshot(&server(254.0, 100.0), period);
        // The server folded it onto the home tile at x=2: drawn at 258.
        r.on_snapshot(&server(2.0, 100.0), period);
        assert_eq!(r.target_transform.disp.x, 258.0);
        r.interpolate(0.025);
        assert!((r.render_transform.disp.x - 256.0).abs() < 1e-3);
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
