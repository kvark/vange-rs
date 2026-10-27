//! Multiplayer client-side prediction with server reconciliation.
//!
//! The client steps its own car one `max_quant` quantum at a time, numbers
//! every quantum, and sends the server the input of each one
//! ([`QuantInput`], batched per frame). The server feeds a player's car
//! exactly those inputs, in order, from an [`InputQueue`], and echoes in
//! each `WorldState` how many of them it has run (the *ack*). The client
//! keeps its inputs and the state it predicted after each quantum in a
//! [`Prediction`]; a snapshot is compared with the prediction *at the
//! acked quantum*, not with the present pose, which is up to a few ticks
//! further along. On a mismatch the client rewinds to the server state and
//! replays its buffered inputs up to now. What the replay moves the car by
//! goes into a [`RenderOffset`] that decays over a few frames, so the
//! correction is never seen as a snap.

use super::{CarPhysicsData, Dynamo};
use crate::{config, level, space};

use glam::{Quat, Vec2, Vec3};
use std::collections::VecDeque;

/// One quantum's worth of player input: everything `physics::step` and
/// the control update in front of it take from the player.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct QuantInput {
    pub motor: f32,
    pub rudder: f32,
    pub brake: bool,
    pub turbo: bool,
    /// Jump power released on this quantum.
    pub jump: Option<f32>,
    /// Side impulse direction on this quantum.
    pub roll: f32,
}

impl QuantInput {
    /// The same held controls without the one-shot jump and roll, for the
    /// other quanta of a frame and for the server holding a starved car.
    pub fn held(&self) -> Self {
        QuantInput {
            jump: None,
            roll: 0.0,
            ..*self
        }
    }
}

/// Rudder, traction and brake for one quantum (`input_factor` from
/// `QuantClock::input_factor`). The client and the server both run this.
pub fn apply_input(
    dynamo: &mut Dynamo,
    input: &QuantInput,
    input_factor: f32,
    common: &config::common::Common,
) {
    if input.rudder != 0.0 {
        let angle = dynamo.rudder + common.car.rudder_step * 2.0 * input_factor * input.rudder;
        dynamo.rudder = angle.clamp(-common.car.rudder_max, common.car.rudder_max);
    }
    if input.motor != 0.0 {
        dynamo.change_traction(input.motor * input_factor * common.car.traction_incr);
    }
    if input.brake && dynamo.traction != 0.0 {
        dynamo.traction *= (-input_factor).exp2();
    }
}

/// One quantum of a player's car: the control update, then `physics::step`.
#[allow(clippy::too_many_arguments)]
pub fn step_input(
    dynamo: &mut Dynamo,
    transform: &mut space::Transform,
    input: &QuantInput,
    quant: f32,
    input_factor: f32,
    car: &CarPhysicsData,
    level: &level::Level,
    common: &config::common::Common,
    tracks: Option<&mut level::terraform::Tracks>,
) {
    apply_input(dynamo, input, input_factor, common);
    super::step(
        dynamo,
        transform,
        quant,
        car,
        level,
        common,
        if input.turbo {
            common.global.k_traction_turbo
        } else {
            1.0
        },
        if input.brake {
            common.global.f_brake_max
        } else {
            0.0
        },
        input.jump,
        input.roll,
        None,
        tracks,
    );
}

/// How many quanta of a player's car the server runs this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TickBudget {
    /// From the client's inputs, acked.
    pub real: u32,
    /// With the last held control, after the client went quiet for
    /// [`InputQueue::STARVE_TICKS`]; not acked.
    pub held: u32,
}

impl TickBudget {
    pub fn total(&self) -> u32 {
        self.real + self.held
    }
}

/// Server side: the client's numbered quantum inputs, run in order.
///
/// A car whose inputs have not arrived yet waits (no step, no ack) rather
/// than guessing, so the server runs the same quanta the client predicted
/// and the reconcile finds nothing to correct. Otherwise it runs one
/// tick's worth per tick, steadily, even with a few frames queued: the
/// other cars run at that rate too, and a client bumping into them
/// predicts them on its own quanta, so the car must not race through a
/// burst of inputs (a long frame) and then stall. Past
/// [`Self::CATCHUP_TICKS`] of backlog (a lag spike) it runs two ticks'
/// worth per tick to catch up. A client that sends nothing for [`Self::STARVE_TICKS`] (a
/// script, a stalled window) gets its car driven on with the last held
/// control, as before per-quantum input.
#[derive(Clone, Debug, Default)]
pub struct InputQueue {
    pending: VecDeque<QuantInput>,
    /// Index of the next quantum to run: the ack.
    next: u32,
    last: QuantInput,
    starved_ticks: u32,
}

impl InputQueue {
    /// Ticks (at 20 Hz) without input before the server drives the car on.
    /// Longer than the slowest frame a real client sends a batch per: a
    /// browser on software WebGL draws about one frame a second, and a
    /// car driven on between its batches runs quanta the client never
    /// predicted (corrections of 50-200u every snapshot).
    pub const STARVE_TICKS: u32 = 60;
    /// Ticks of backlog past which the car catches up at double rate.
    pub const CATCHUP_TICKS: u32 = 8;
    /// Backlog cap; a client further ahead than this loses the oldest.
    pub const MAX_PENDING: usize = 4096;

    /// Quanta run so far: every input below this index is in the state.
    pub fn ack(&self) -> u32 {
        self.next
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Queue `count` quanta from index `first` with the held controls of
    /// `input`; its jump and roll go on the last of them, as the client
    /// stepped them. Quanta already queued or run are skipped; a gap
    /// (should not happen on an ordered stream) is jumped over.
    pub fn push(&mut self, first: u32, count: u32, input: QuantInput) {
        if count == 0 {
            return;
        }
        let end = first.wrapping_add(count);
        let queued_end = self.next.wrapping_add(self.pending.len() as u32);
        let start = if (first.wrapping_sub(queued_end) as i32) > 0 {
            // A gap: what was missed is gone, start over from `first`.
            self.pending.clear();
            self.next = first;
            first
        } else {
            queued_end
        };
        let mut index = start;
        while (end.wrapping_sub(index) as i32) > 0 {
            let last = index.wrapping_add(1) == end;
            self.pending
                .push_back(if last { input } else { input.held() });
            index = index.wrapping_add(1);
        }
        while self.pending.len() > Self::MAX_PENDING {
            self.pending.pop_front();
            self.next = self.next.wrapping_add(1);
        }
    }

    /// The client moved its car itself (reset / teleport) before quantum
    /// `next`: drop what is queued from before and expect `next` onward.
    pub fn reset(&mut self, next: u32) {
        self.pending.clear();
        self.next = next;
        self.starved_ticks = 0;
    }

    /// Plan this tick for a server that advances `tick_quanta` quanta.
    /// A tick that advances no quantum (a coarse `max_quant`) neither runs
    /// nor counts toward starvation.
    pub fn budget(&mut self, tick_quanta: u32) -> TickBudget {
        if tick_quanta == 0 {
            return TickBudget::default();
        }
        let n = self.pending.len() as u32;
        if n == 0 {
            self.starved_ticks += 1;
            let held = if self.starved_ticks > Self::STARVE_TICKS {
                tick_quanta
            } else {
                0
            };
            return TickBudget { real: 0, held };
        }
        self.starved_ticks = 0;
        let real = if n > Self::CATCHUP_TICKS * tick_quanta {
            2 * tick_quanta
        } else {
            n.min(tick_quanta)
        };
        TickBudget { real, held: 0 }
    }

    /// The held control the car last ran with (no jump or roll), sent to
    /// the other clients for their contact ghost of this car.
    pub fn last(&self) -> QuantInput {
        self.last.held()
    }

    /// The input for one quantum of the planned budget: the next queued
    /// one (acked), or once it runs out the held control.
    pub fn pop(&mut self) -> QuantInput {
        match self.pending.pop_front() {
            Some(input) => {
                self.next = self.next.wrapping_add(1);
                self.last = input;
                input
            }
            None => self.last.held(),
        }
    }
}

/// The server's copy of the local car with the ack it was taken at.
#[derive(Clone, Copy, Debug)]
pub struct ServerSnapshot {
    pub car: super::ServerCar,
    /// Quanta of this client's input in `car`.
    pub ack: u32,
}

/// One predicted quantum: its input and the state after it.
#[derive(Clone, Copy, Debug)]
struct Predicted {
    input: QuantInput,
    transform: space::Transform,
    dynamo: Dynamo,
}

/// What [`Prediction::reconcile`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reconciled {
    /// The server agrees with the prediction at the ack.
    InSync,
    /// Rewound to the server state and replayed this many quanta.
    Replayed(u32),
    /// The snapshot is older than the buffer (e.g. from before a reset):
    /// ignored.
    Stale,
    /// The server is past the prediction (a rejoin with a fresh client
    /// counter): its state taken as is.
    Ahead,
}

/// Client side: the local car's inputs and predicted states per quantum.
#[derive(Clone, Debug, Default)]
pub struct Prediction {
    history: VecDeque<Predicted>,
    /// Index of the next quantum to be stepped.
    next: u32,
}

impl Prediction {
    /// Quanta of history kept (~6 s on Fostral, minutes on the test level).
    pub const HISTORY: usize = 2048;
    /// Prediction error at the ack that is left alone.
    pub const POS_TOLERANCE: f32 = 0.02;
    pub const ROT_TOLERANCE: f32 = 0.002;
    pub const VEL_TOLERANCE: f32 = 0.05;

    /// Index of the next quantum: the `first` of the next input batch.
    pub fn next(&self) -> u32 {
        self.next
    }

    pub fn len(&self) -> usize {
        self.history.len()
    }

    pub fn is_empty(&self) -> bool {
        self.history.is_empty()
    }

    /// Record a stepped quantum: its input and the state it left.
    pub fn push(&mut self, input: QuantInput, transform: &space::Transform, dynamo: &Dynamo) {
        self.history.push_back(Predicted {
            input,
            transform: *transform,
            dynamo: *dynamo,
        });
        if self.history.len() > Self::HISTORY {
            self.history.pop_front();
        }
        self.next = self.next.wrapping_add(1);
    }

    /// Forget the history (the car was moved outside the inputs); the
    /// numbering goes on.
    pub fn clear(&mut self) {
        self.history.clear();
    }

    /// `count` quanta pass with the car moved outside its inputs (held on
    /// a train ride): the history goes and the numbering moves on. Returns
    /// the first of them.
    pub fn skip(&mut self, count: u32) -> u32 {
        let first = self.next;
        if count > 0 {
            self.history.clear();
            self.next = self.next.wrapping_add(count);
        }
        first
    }

    /// Correct the car (`transform`, `dynamo`: its present predicted state)
    /// from `server`. When the prediction at the ack is off, or `force`,
    /// rewind to the server state and replay every buffered input after
    /// the ack through `step` (the same quantum the live loop runs,
    /// contacts included; `step` also gets the index of the quantum it
    /// runs, to place remote ghosts on that quantum). `period` is the level's wrap: the server folds
    /// positions onto the home tile, the client keeps a nearby replica.
    pub fn reconcile(
        &mut self,
        server: &ServerSnapshot,
        force: bool,
        period: Option<Vec2>,
        transform: &mut space::Transform,
        dynamo: &mut Dynamo,
        mut step: impl FnMut(&mut space::Transform, &mut Dynamo, &QuantInput, u32),
    ) -> Reconciled {
        let ack = server.ack;
        let ahead = ack.wrapping_sub(self.next) as i32;
        if ahead > 0 {
            *transform = near(&server.car.transform, transform.disp, period);
            adopt(dynamo, &server.car);
            self.history.clear();
            self.next = ack;
            return Reconciled::Ahead;
        }
        // `ack - 1` is the quantum whose state the server sent.
        let behind = self.next.wrapping_sub(ack) as usize;
        if behind > self.history.len() {
            return Reconciled::Stale;
        }
        let base = self.history.len() - behind;
        // Nothing before `ack - 1` is needed again.
        let drop = base.saturating_sub(1);
        self.history.drain(..drop);
        let base = base - drop;
        let reference = (base > 0).then(|| self.history[base - 1]);
        let near_disp = reference.map_or(transform.disp, |p| p.transform.disp);
        let server_transform = near(&server.car.transform, near_disp, period);
        if !force
            && let Some(predicted) = reference
            && agrees(&predicted, &server_transform, &server.car)
        {
            return Reconciled::InSync;
        }
        let mut t = server_transform;
        let mut d = reference.map_or(*dynamo, |p| p.dynamo);
        adopt(&mut d, &server.car);
        let replayed = self.history.len() - base;
        for (i, predicted) in self.history.iter_mut().skip(base).enumerate() {
            step(&mut t, &mut d, &predicted.input, ack.wrapping_add(i as u32));
            predicted.transform = t;
            predicted.dynamo = d;
        }
        // Back onto the replica the car is on now: the history may predate
        // a whole-period rebase of the client's coordinates.
        let shift = near(&t, transform.disp, period).disp - t.disp;
        if shift != Vec3::ZERO {
            t.disp += shift;
            for predicted in self.history.iter_mut() {
                predicted.transform.disp += shift;
            }
        }
        *transform = t;
        *dynamo = d;
        Reconciled::Replayed(replayed as u32)
    }
}

/// Most quanta a remote ghost is led forward (about 1.4 s of real play:
/// a round trip of a laggy link plus a long frame, with room to spare).
pub const GHOST_LEAD_MAX: u32 = 128;

/// A remote car as the local prediction bumps into it.
///
/// The server pose of a remote car is a latency old, while the local car
/// is predicted a latency ahead; contact against the bare snapshot pose
/// hits the car where it was a round trip ago, and a ghost that never
/// gives way pushes the local car off by the whole overlap where the
/// server splits it. So a ghost starts at its snapshot pose on the local
/// quantum of the same snapshot (our ack in it) and is then carried along
/// with the local car one quantum at a time: full physics from its
/// snapshot velocities, traction and rudder, driven by the held control
/// the server last ran it with (motor, rudder, brake and turbo: its
/// driver's next inputs are unknown, the last ones are the best guess, as
/// the server's own guess for a starved car), for at most
/// [`GHOST_LEAD_MAX`] quanta, and it takes its share of every contact.
#[derive(Clone, Copy, Debug)]
pub struct Ghost {
    pub transform: space::Transform,
    pub dynamo: Dynamo,
    /// The held control it is driven with (never a jump or roll).
    pub control: QuantInput,
    /// Quanta led since the snapshot.
    pub lead: u32,
}

impl Ghost {
    /// A ghost coasting with no control.
    pub fn new(transform: space::Transform, dynamo: Dynamo) -> Self {
        Self::driven(transform, dynamo, QuantInput::default())
    }

    /// A ghost driven on with the held part of `control`.
    pub fn driven(transform: space::Transform, dynamo: Dynamo, control: QuantInput) -> Self {
        Ghost {
            transform,
            dynamo,
            control: control.held(),
            lead: 0,
        }
    }

    /// One quantum forward.
    pub fn advance(
        &mut self,
        quant: f32,
        car: &CarPhysicsData,
        level: &level::Level,
        common: &config::common::Common,
    ) {
        if self.lead >= GHOST_LEAD_MAX {
            return;
        }
        self.lead += 1;
        step_input(
            &mut self.dynamo,
            &mut self.transform,
            &self.control,
            quant,
            super::QuantClock::input_factor(quant, common),
            car,
            level,
            common,
            None,
        );
    }
}

/// Ghosts closer than this to the local car make every snapshot replay
/// the local inputs (`force`), so the ghosts are carried through the same
/// quanta as the car.
pub const GHOST_REPLAY_RANGE: f32 = 100.0;

/// Fold a position onto the home tile `[0, period)`, the way the server
/// does after every quantum (and the predicting client with it: the same
/// float operation, so both step the car on identical coordinates).
pub fn fold_home(disp: &mut Vec3, period: Vec2) {
    if period.x > 0.0 && period.y > 0.0 {
        disp.x = disp.x.rem_euclid(period.x);
        disp.y = disp.y.rem_euclid(period.y);
    }
}

/// Horizontal distance on a level wrapping with `period`.
pub fn wrapped_distance(a: Vec3, b: Vec3, period: Option<Vec2>) -> f32 {
    let mut d = a.truncate() - b.truncate();
    if let Some(p) = period {
        let fold = |d: f32, p: f32| if p > 0.0 { d - (d / p).round() * p } else { d };
        d = Vec2::new(fold(d.x, p.x), fold(d.y, p.y));
    }
    d.length()
}

/// `server` moved to the replica of its position closest to `disp`.
pub(crate) fn near(
    server: &space::Transform,
    disp: Vec3,
    period: Option<Vec2>,
) -> space::Transform {
    let mut t = *server;
    if let Some(p) = period {
        let d = t.disp.truncate() - disp.truncate();
        let fold = |d: f32, p: f32| if p > 0.0 { (d / p).round() * p } else { 0.0 };
        t.disp.x -= fold(d.x, p.x);
        t.disp.y -= fold(d.y, p.y);
    }
    t
}

/// Take the dynamics the server sends; the rest stays predicted.
fn adopt(dynamo: &mut Dynamo, car: &super::ServerCar) {
    dynamo.linear_velocity = car.linear_velocity;
    dynamo.angular_velocity = car.angular_velocity;
    dynamo.traction = car.traction;
    dynamo.rudder = car.rudder;
}

fn agrees(predicted: &Predicted, server: &space::Transform, car: &super::ServerCar) -> bool {
    let p = &predicted.transform;
    let d = &predicted.dynamo;
    p.disp.distance(server.disp) <= Prediction::POS_TOLERANCE
        // Normalized: a car's rotation drifts off unit length, and
        // `angle_between` reads that as a few milliradians.
        && p.rot.normalize().angle_between(server.rot.normalize()) <= Prediction::ROT_TOLERANCE
        && (p.rot * d.linear_velocity).distance(server.rot * car.linear_velocity)
            <= Prediction::VEL_TOLERANCE
        && (d.angular_velocity - car.angular_velocity).length() <= Prediction::VEL_TOLERANCE
}

/// The visible part of a correction, decaying back to the physics pose.
///
/// The replay moves the simulated car at once; the drawn car (and the
/// camera following it) keeps its old pose plus this offset, which shrinks
/// with time constant [`Self::TAU`] but by no more than
/// [`Self::MAX_SPEED`] per second and [`Self::MAX_STEP`] per frame, so a
/// frame never shows a pop.
///
/// A correction from car contact (a shove, a launch) is often far larger
/// than a misprediction: contact is chaotic, and the ghost of the other car
/// only guesses its driver's inputs. Such a correction is absorbed up to
/// [`Self::CONTACT_TELEPORT`]; once the offset is past [`Self::TELEPORT`]
/// (one big launch, or a sustained push piling it up) it is faded out
/// within [`Self::CONTACT_FADE`], but over at least
/// [`Self::CONTACT_FADE_FRAMES`] frames, whatever the speed and step caps:
/// the car slides there instead of popping.
#[derive(Clone, Copy, Debug)]
pub struct RenderOffset {
    pub disp: Vec3,
    pub rot: Quat,
    /// Contact fade: the offset shrinks at least this fast, world units
    /// per second, until it is gone.
    pub fade_speed: f32,
}

impl Default for RenderOffset {
    fn default() -> Self {
        RenderOffset {
            disp: Vec3::ZERO,
            rot: Quat::IDENTITY,
            fade_speed: 0.0,
        }
    }
}

impl RenderOffset {
    /// Seconds for the offset to shrink to ~37%.
    pub const TAU: f32 = 0.1;
    /// Fastest the offset shrinks, world units per second: above the
    /// ~20 u/s of corrections a sustained push against another car makes
    /// at a few ticks of latency, so the offset does not pile up.
    pub const MAX_SPEED: f32 = 30.0;
    /// Most the offset shrinks in one frame (long frames), world units.
    pub const MAX_STEP: f32 = 1.5;
    /// A correction this large is a teleport, not a misprediction: shown
    /// at once.
    pub const TELEPORT: f32 = 50.0;
    /// A correction coinciding with car contact is shown at once only past
    /// this (a respawn or a real teleport during a fight still snaps).
    pub const CONTACT_TELEPORT: f32 = 200.0;
    /// Seconds a contact correction takes to fade out at most.
    pub const CONTACT_FADE: f32 = 0.3;
    /// Fewest frames a contact fade is spread over, for frames longer than
    /// a third of [`Self::CONTACT_FADE`] (a 1-4 fps client).
    pub const CONTACT_FADE_FRAMES: f32 = 3.0;

    /// Keep drawing the car where it was before the physics pose moved
    /// from `before` to `after`. Returns `false` (and clears the offset)
    /// for a teleport.
    pub fn absorb(&mut self, before: &space::Transform, after: &space::Transform) -> bool {
        self.absorb_within(before, after, Self::TELEPORT)
    }

    /// [`Self::absorb`] for a correction caused by car contact: up to
    /// [`Self::CONTACT_TELEPORT`]; an offset past [`Self::TELEPORT`] is
    /// faded out within [`Self::CONTACT_FADE`].
    pub fn absorb_contact(&mut self, before: &space::Transform, after: &space::Transform) -> bool {
        if !self.absorb_within(before, after, Self::CONTACT_TELEPORT) {
            return false;
        }
        // Smaller ones decay as usual (a frame never shows more than
        // `MAX_STEP`); past the plain teleport limit, fade.
        let len = self.disp.length();
        if len > Self::TELEPORT {
            self.fade_speed = self.fade_speed.max(len / Self::CONTACT_FADE);
        }
        true
    }

    fn absorb_within(
        &mut self,
        before: &space::Transform,
        after: &space::Transform,
        teleport: f32,
    ) -> bool {
        let disp = self.disp + before.disp - after.disp;
        if disp.length() > teleport {
            *self = Self::default();
            return false;
        }
        self.disp = disp;
        self.rot = (self.rot * before.rot * after.rot.inverse()).normalize();
        true
    }

    /// Shrink by one frame of `dt`; returns how far the drawn car moved
    /// toward the physics pose.
    pub fn decay(&mut self, dt: f32) -> f32 {
        let len = self.disp.length();
        let keep = (-dt / Self::TAU).exp();
        let step = (len * (1.0 - keep))
            .min(Self::MAX_SPEED * dt)
            .min(Self::MAX_STEP)
            .max(
                (self.fade_speed * dt)
                    .min(self.fade_speed * Self::CONTACT_FADE / Self::CONTACT_FADE_FRAMES),
            )
            .min(len);
        if len > 0.0 {
            self.disp *= (len - step) / len;
        }
        self.rot = Quat::IDENTITY.slerp(self.rot, keep);
        if self.disp.length() < 1e-4 {
            self.disp = Vec3::ZERO;
            self.fade_speed = 0.0;
        }
        step
    }

    /// The pose to draw for the physics pose `t`.
    pub fn apply(&self, t: &space::Transform) -> space::Transform {
        space::Transform {
            disp: t.disp + self.disp,
            rot: (self.rot * t.rot).normalize(),
            scale: t.scale,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(motor: f32, jump: Option<f32>) -> QuantInput {
        QuantInput {
            motor,
            jump,
            roll: 1.0,
            ..QuantInput::default()
        }
    }

    #[test]
    fn queue_puts_jump_and_roll_on_the_last_quantum() {
        let mut q = InputQueue::default();
        q.push(0, 3, input(1.0, Some(4.0)));
        assert_eq!(q.budget(10), TickBudget { real: 3, held: 0 });
        let got: Vec<_> = (0..3).map(|_| q.pop()).collect();
        assert_eq!(got[0], input(1.0, None).held());
        assert_eq!(got[1], input(1.0, None).held());
        assert_eq!(got[2], input(1.0, Some(4.0)));
        assert_eq!(q.ack(), 3);
    }

    #[test]
    fn queue_skips_overlap_and_jumps_gaps() {
        let mut q = InputQueue::default();
        q.push(0, 4, input(1.0, None));
        q.push(2, 4, input(0.5, None));
        assert_eq!(q.pending(), 6);
        q.push(10, 2, input(0.2, None));
        assert_eq!((q.ack(), q.pending()), (10, 2));
        q.reset(20);
        assert_eq!((q.ack(), q.pending()), (20, 0));
    }

    #[test]
    fn starved_car_waits_then_drives_on() {
        let mut q = InputQueue::default();
        q.push(0, 2, input(1.0, Some(3.0)));
        assert_eq!(q.budget(4).real, 2);
        q.pop();
        q.pop();
        for _ in 0..InputQueue::STARVE_TICKS {
            assert_eq!(q.budget(4), TickBudget::default());
        }
        assert_eq!(q.budget(4), TickBudget { real: 0, held: 4 });
        // Held: no repeated jump or roll, and no ack.
        assert_eq!(q.pop(), input(1.0, None).held());
        assert_eq!(q.ack(), 2);
    }

    #[test]
    fn backlog_catches_up_at_twice_the_rate() {
        let mut q = InputQueue::default();
        q.push(0, 40, input(1.0, None));
        assert_eq!(q.budget(4).real, 8);
        // A few frames' worth queued (long frames): steady, one tick each.
        let mut q = InputQueue::default();
        q.push(0, 18, input(1.0, None));
        assert_eq!(q.budget(4).real, 4);
    }

    fn car(x: f32) -> crate::physics::ServerCar {
        crate::physics::ServerCar {
            transform: space::Transform {
                disp: Vec3::new(x, 0.0, 0.0),
                ..space::Transform::IDENTITY
            },
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            traction: 0.0,
            rudder: 0.0,
            bumped: false,
        }
    }

    /// A 1D stand-in for physics: each quantum moves the car by `motor`.
    fn slide(t: &mut space::Transform, _: &mut Dynamo, i: &QuantInput, _: u32) {
        t.disp.x += i.motor;
    }

    fn predict(p: &mut Prediction, t: &mut space::Transform, d: &mut Dynamo, motors: &[f32]) {
        for &m in motors {
            let i = input(m, None);
            slide(t, d, &i, 0);
            p.push(i, t, d);
        }
    }

    #[test]
    fn compares_at_the_ack_not_the_present() {
        let mut p = Prediction::default();
        let (mut t, mut d) = (space::Transform::IDENTITY, Dynamo::default());
        predict(&mut p, &mut t, &mut d, &[1.0; 10]);
        // The server has run 4 quanta: x = 4, while the client is at 10.
        let snap = ServerSnapshot {
            car: car(4.0),
            ack: 4,
        };
        let r = p.reconcile(&snap, false, None, &mut t, &mut d, slide);
        assert_eq!(r, Reconciled::InSync);
        assert_eq!(t.disp.x, 10.0);
    }

    #[test]
    fn replays_from_the_server_state() {
        let mut p = Prediction::default();
        let (mut t, mut d) = (space::Transform::IDENTITY, Dynamo::default());
        predict(&mut p, &mut t, &mut d, &[1.0; 10]);
        // A bump the client did not see: 3 more at quantum 4.
        let snap = ServerSnapshot {
            car: car(7.0),
            ack: 4,
        };
        let r = p.reconcile(&snap, false, None, &mut t, &mut d, slide);
        assert_eq!(r, Reconciled::Replayed(6));
        assert_eq!(t.disp.x, 13.0);
        // The replayed states are the new prediction.
        let snap = ServerSnapshot {
            car: car(9.0),
            ack: 6,
        };
        assert_eq!(
            p.reconcile(&snap, false, None, &mut t, &mut d, slide),
            Reconciled::InSync
        );
        // Older than the buffer: ignored.
        let snap = ServerSnapshot {
            car: car(0.0),
            ack: 1,
        };
        assert_eq!(
            p.reconcile(&snap, false, None, &mut t, &mut d, slide),
            Reconciled::Stale
        );
    }

    #[test]
    fn wrapped_server_position_is_the_same_car() {
        let mut p = Prediction::default();
        let mut t = space::Transform {
            disp: Vec3::new(2046.0, 0.0, 0.0),
            ..space::Transform::IDENTITY
        };
        let mut d = Dynamo::default();
        predict(&mut p, &mut t, &mut d, &[1.0; 4]);
        // Server folded 2048 onto 0.
        let snap = ServerSnapshot {
            car: car(0.0),
            ack: 2,
        };
        let period = Some(Vec2::new(2048.0, 4096.0));
        let r = p.reconcile(&snap, false, period, &mut t, &mut d, slide);
        assert_eq!(r, Reconciled::InSync);
        assert_eq!(t.disp.x, 2050.0);
    }

    #[test]
    fn replay_lands_on_the_present_replica() {
        let mut p = Prediction::default();
        let (mut t, mut d) = (space::Transform::IDENTITY, Dynamo::default());
        predict(&mut p, &mut t, &mut d, &[1.0; 6]);
        // The client rebased its coordinates by a whole period since.
        t.disp.x -= 2048.0;
        let snap = ServerSnapshot {
            car: car(3.5),
            ack: 3,
        };
        let period = Some(Vec2::new(2048.0, 4096.0));
        let r = p.reconcile(&snap, false, period, &mut t, &mut d, slide);
        assert_eq!(r, Reconciled::Replayed(3));
        assert_eq!(t.disp.x, 6.5 - 2048.0);
    }

    #[test]
    fn offset_hides_the_correction_and_decays() {
        let mut o = RenderOffset::default();
        let before = space::Transform::IDENTITY;
        let after = space::Transform {
            disp: Vec3::new(5.0, 0.0, 0.0),
            ..before
        };
        assert!(o.absorb(&before, &after));
        assert_eq!(o.apply(&after).disp, before.disp);
        let mut moved = 0.0;
        for _ in 0..120 {
            let step = o.decay(1.0 / 60.0);
            assert!(step <= RenderOffset::MAX_SPEED / 60.0 + 1e-5);
            moved += step;
        }
        assert!((moved - 5.0).abs() < 1e-3 && o.disp == Vec3::ZERO);
        let far = space::Transform {
            disp: Vec3::new(500.0, 0.0, 0.0),
            ..before
        };
        assert!(!o.absorb(&before, &far));
    }

    #[test]
    fn contact_offset_fades_within_the_window() {
        let before = space::Transform::IDENTITY;
        let after = space::Transform {
            disp: Vec3::new(120.0, 0.0, 0.0),
            ..before
        };
        // A plain correction this large is a teleport.
        assert!(!RenderOffset::default().absorb(&before, &after));
        // From contact it fades: gone within `CONTACT_FADE` at 60 fps...
        let mut o = RenderOffset::default();
        assert!(o.absorb_contact(&before, &after));
        let frames = (RenderOffset::CONTACT_FADE * 60.0).ceil() as usize;
        for _ in 0..frames {
            assert!(o.decay(1.0 / 60.0) <= 120.0 / frames as f32 + 1e-3);
        }
        assert_eq!(o.disp, Vec3::ZERO);
        assert_eq!(o.fade_speed, 0.0);
        // ...and over a few frames at 1 fps rather than one.
        assert!(o.absorb_contact(&before, &after));
        for _ in 0..RenderOffset::CONTACT_FADE_FRAMES as usize {
            assert!(o.decay(1.0) < 120.0 / RenderOffset::CONTACT_FADE_FRAMES + 1e-3);
        }
        assert!(o.disp.length() < 1e-3);
        // A respawn mid-fight still snaps.
        let respawn = space::Transform {
            disp: Vec3::new(1000.0, 0.0, 0.0),
            ..before
        };
        assert!(!o.absorb_contact(&before, &respawn));
    }
}
