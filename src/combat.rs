//! Server-side combat: shots flown on the server, hit against car spheres
//! with lag compensation, and armour.
//!
//! The server owns every car's armour. A shot it accepts (`weapon::check_fire`)
//! flies here at `weapon::SHOT_SPEED` for `weapon::SHOT_LIFE`, and each tick it
//! is tested against the other cars as the shooter saw them: rewound by how
//! far the shooter's view of the world was behind the server
//! ([`rewind_ticks`]), at most [`REWIND_CAP_SECS`]. The terrain stops it
//! ([`Level::segment_hits_terrain`]).
//!
//! A car whose armour runs out is a wreck ([`Wreck`]) for [`RESPAWN_SECS`]:
//! shots pass it and it fires none ([`fly_shots`]). Then the server puts it
//! back, undamaged, by the escave entrance nearest to where it died
//! ([`nearest_escave`]).

use crate::{level::Level, weapon};
use glam::Vec3;
use std::collections::VecDeque;

/// Armour of a car whose `car.prm` stats the server does not have.
pub const DEFAULT_MAX_ARMOR: u16 = 100;
/// Farthest back a target is rewound for a shot, seconds: past this a
/// laggy shooter would hit cars that have long since moved on.
pub const REWIND_CAP_SECS: f32 = 0.2;
/// Pose history kept per car, seconds.
pub const HISTORY_SECS: f32 = 1.0;

/// How long a wreck waits before the server respawns it, seconds.
pub const RESPAWN_SECS: f32 = 3.0;

/// Armour a hit from weapon bay `bay` takes off.
///
/// TODO: real per-weapon damage from the game data, once the client
/// declares its equipped weapons and the server checks them against the
/// car's bays. Until then the bay alone sets the damage.
pub fn bay_damage(bay: u8) -> u16 {
    match bay {
        0 => 10,
        1 => 15,
        _ => 20,
    }
}

/// Server ticks to rewind the targets of a shot fired by a client whose
/// newest snapshot was tick `view_tick`, arrived `view_ms` before it
/// fired: the cars it drew were carried on from that snapshot by about
/// that long. `now` is the tick the shot is taken in. Clamped to
/// `0..=REWIND_CAP_SECS`; a view from the future (a bad or stale client
/// tick) does not rewind.
pub fn rewind_ticks(now: u32, view_tick: u32, view_ms: u16, tick_secs: f32) -> f32 {
    if tick_secs <= 0.0 {
        return 0.0;
    }
    let view = view_tick as f32 + view_ms as f32 / 1000.0 / tick_secs;
    let cap = REWIND_CAP_SECS / tick_secs;
    (now as f32 - view).clamp(0.0, cap)
}

/// Wrap-aware `to - from` on a torus of `period` (XY only). A zero period
/// component does not wrap.
pub fn torus_delta(from: Vec3, to: Vec3, period: (f32, f32)) -> Vec3 {
    let wrap = |d: f32, span: f32| {
        if span > 0.0 {
            d - (d / span).round() * span
        } else {
            d
        }
    };
    let d = to - from;
    Vec3::new(wrap(d.x, period.0), wrap(d.y, period.1), d.z)
}

/// Where along `from -> to` (0..=1) the segment first enters the sphere at
/// `center`, or `None` if it misses. A segment starting inside hits at 0.
pub fn segment_hits_sphere(from: Vec3, to: Vec3, center: Vec3, radius: f32) -> Option<f32> {
    let d = to - from;
    let m = from - center;
    let c = m.length_squared() - radius * radius;
    if c <= 0.0 {
        return Some(0.0);
    }
    let a = d.length_squared();
    if a <= f32::EPSILON {
        return None;
    }
    let b = m.dot(d);
    if b >= 0.0 {
        return None; // moving away
    }
    let disc = b * b - a * c;
    if disc < 0.0 {
        return None;
    }
    let t = (-b - disc.sqrt()) / a;
    (t <= 1.0).then_some(t.max(0.0))
}

/// A car's recent positions, one per server tick, for rewinding.
#[derive(Clone, Debug, Default)]
pub struct PoseHistory {
    entries: VecDeque<(u32, Vec3)>,
}

impl PoseHistory {
    /// Note the car at `pos` after tick `tick`, keeping `keep` ticks.
    pub fn record(&mut self, tick: u32, pos: Vec3, keep: usize) {
        if self.entries.back().is_some_and(|&(t, _)| t >= tick) {
            self.entries.clear(); // ticks went backwards: start over
        }
        self.entries.push_back((tick, pos));
        while self.entries.len() > keep.max(1) {
            self.entries.pop_front();
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// The car at (fractional) tick `at`, interpolated between the ticks
    /// around it wrap-aware; held at the oldest / newest entry outside the
    /// history. A gap of more than `period`/2 between two ticks (a
    /// teleport) is not interpolated across: the nearer entry is taken.
    pub fn at(&self, at: f32, period: (f32, f32)) -> Option<Vec3> {
        let &(first_tick, first) = self.entries.front()?;
        if at <= first_tick as f32 {
            return Some(first);
        }
        for pair in self.entries.iter().zip(self.entries.iter().skip(1)) {
            let (&(t0, p0), &(t1, p1)) = pair;
            if at <= t1 as f32 {
                let f = (at - t0 as f32) / (t1 - t0) as f32;
                let d = torus_delta(p0, p1, period);
                if d.length() > crate::physics::client::RemoteCar::TELEPORT {
                    return Some(if f < 0.5 { p0 } else { p1 });
                }
                return Some(p0 + d * f);
            }
        }
        self.entries.back().map(|&(_, p)| p)
    }
}

/// A car that ran out of armour, waiting to respawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wreck<Id> {
    /// Where it died.
    pub at: Vec3,
    /// Whose shot emptied its armour.
    pub killer: Option<Id>,
    /// The server tick it respawns in ([`respawn_tick`]).
    pub respawn: u32,
}

/// The tick a car that died in tick `now` respawns in: [`RESPAWN_SECS`]
/// on, at least one tick. Wraps with the tick counter.
pub fn respawn_tick(now: u32, tick_secs: f32) -> u32 {
    let wait = if tick_secs > 0.0 {
        (RESPAWN_SECS / tick_secs).round().max(1.0) as u32
    } else {
        1
    };
    now.wrapping_add(wait)
}

/// Whether tick `now` has reached `due`, across the tick counter's wrap
/// (`due` less than half the counter's range away).
pub fn respawn_due(now: u32, due: u32) -> bool {
    now.wrapping_sub(due) as i32 >= 0
}

/// The escave entrance (map XY) nearest to `pos`, measured the short way
/// around a torus of `period`; the first listed on a tie, `None` with no
/// escaves.
pub fn nearest_escave(pos: Vec3, escaves: &[(i32, i32)], period: (f32, f32)) -> Option<(i32, i32)> {
    let dist = |&(x, y): &(i32, i32)| {
        let d = torus_delta(pos, Vec3::new(x as f32, y as f32, pos.z), period);
        d.x * d.x + d.y * d.y
    };
    escaves
        .iter()
        .min_by(|a, b| dist(a).total_cmp(&dist(b)))
        .copied()
}

/// How long after a respawn the server takes no pose from the client,
/// seconds: a `SetPose` or upright `Reset` sent from the wreck (or racing
/// the `Respawned`) must not undo the respawn.
pub const RESPAWN_POSE_LOCK_SECS: f32 = 1.0;
/// How far from the server's pose of the car the upright key may put it,
/// in world units: the client's prediction runs a little ahead of the
/// server, but not a car-length or more.
pub const RESET_REACH: f32 = 40.0;

/// Why the server ignored a pose the client set itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoseRefusal {
    /// `SetPose` on a server run without `--allow-setpose`.
    NotAllowed,
    /// The car is a wreck: it stays where it went up.
    Wreck,
    /// Within [`RESPAWN_POSE_LOCK_SECS`] of a respawn.
    JustRespawned,
}

impl std::fmt::Display for PoseRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match *self {
            Self::NotAllowed => "SetPose is debug only (--allow-setpose)",
            Self::Wreck => "a wreck",
            Self::JustRespawned => "just respawned",
        })
    }
}

/// Whether the server takes a pose the client set its car to itself in
/// tick `now`: a debug `SetPose` (`set_pose`) only when `allowed`
/// (`--allow-setpose`), an upright `Reset` always; neither for a wreck (`dead`) nor within
/// [`RESPAWN_POSE_LOCK_SECS`] after the tick it last respawned in.
pub fn check_client_pose(
    set_pose: bool,
    allowed: bool,
    dead: bool,
    respawned: Option<u32>,
    now: u32,
    tick_secs: f32,
) -> Result<(), PoseRefusal> {
    if set_pose && !allowed {
        return Err(PoseRefusal::NotAllowed);
    }
    if dead {
        return Err(PoseRefusal::Wreck);
    }
    if let Some(at) = respawned {
        let lock = if tick_secs > 0.0 {
            (RESPAWN_POSE_LOCK_SECS / tick_secs).round() as u32
        } else {
            0
        };
        if now.wrapping_sub(at) < lock {
            return Err(PoseRefusal::JustRespawned);
        }
    }
    Ok(())
}

/// Where the upright key puts a car the server has at `server`, the client
/// at `client`: the client's position (unfolded next to the server's)
/// within [`RESET_REACH`], else the server's own.
pub fn reset_position(client: Vec3, server: Vec3, period: (f32, f32)) -> Vec3 {
    let delta = torus_delta(server, client, period);
    if delta.is_finite() && delta.length() <= RESET_REACH {
        server + delta
    } else {
        server
    }
}

/// Fly every shot one tick of `dt` on and return its hits as
/// `(shooter, bay, target)`; a shot that hit or is spent is removed.
/// `targets(rewind)` lists the cars as a shot rewound by `rewind` ticks
/// sees them. `dead` tells the wrecks apart: a wreck's shots are dropped
/// (even those it fired before it died) and shots fly through a wreck.
pub fn fly_shots<Id: Copy + PartialEq>(
    shots: &mut Vec<ServerShot<Id>>,
    dt: f32,
    period: (f32, f32),
    level: Option<&Level>,
    targets: impl Fn(f32) -> Vec<Target<Id>>,
    dead: impl Fn(Id) -> bool,
) -> Vec<(Id, u8, Id)> {
    let mut hits = Vec::new();
    shots.retain_mut(|shot| {
        if dead(shot.shooter) {
            return false;
        }
        let mut targets = targets(shot.rewind);
        targets.retain(|t| !dead(t.id));
        match shot.step(dt, &targets, period, level) {
            Some(target) => {
                hits.push((shot.shooter, shot.bay, target));
                false
            }
            None => !shot.is_spent(),
        }
    });
    hits
}

/// A shot the server is flying.
#[derive(Clone, Debug)]
pub struct ServerShot<Id> {
    pub shooter: Id,
    pub bay: u8,
    pub pos: Vec3,
    pub vel: Vec3,
    pub age: f32,
    /// Ticks the targets are rewound by for this shot ([`rewind_ticks`]).
    pub rewind: f32,
}

/// A car a shot may hit: where it was at the shot's rewound tick and how
/// big it is.
#[derive(Clone, Copy, Debug)]
pub struct Target<Id> {
    pub id: Id,
    pub center: Vec3,
    pub radius: f32,
}

impl<Id: Copy + PartialEq> ServerShot<Id> {
    pub fn new(shooter: Id, bay: u8, origin: Vec3, dir: Vec3, rewind: f32) -> Self {
        ServerShot {
            shooter,
            bay,
            pos: origin,
            vel: dir.try_normalize().unwrap_or(Vec3::Y) * weapon::SHOT_SPEED,
            age: 0.0,
            rewind,
        }
    }

    /// Fly the shot on by `dt` and return the first car it hits on the
    /// way (never its shooter). Targets are tested wrap-aware on a torus of
    /// `period`. With a `level`, the terrain stops the shot: a car behind
    /// the first rock on the way is not hit, and the shot ends there (it
    /// flies on between the layers under a slab). A spent shot (past
    /// `SHOT_LIFE`, or stopped) hits nothing.
    pub fn step(
        &mut self,
        dt: f32,
        targets: &[Target<Id>],
        period: (f32, f32),
        level: Option<&Level>,
    ) -> Option<Id> {
        if self.is_spent() {
            return None;
        }
        let from = self.pos;
        let to = from + self.vel * dt;
        self.pos = to;
        self.age += dt;
        let rock = level.and_then(|level| level.segment_hits_terrain(from, to));
        let hit = targets
            .iter()
            .filter(|t| t.id != self.shooter)
            .filter_map(|t| {
                // The target's replica next to the shot.
                let center = from + torus_delta(from, t.center, period);
                segment_hits_sphere(from, to, center, t.radius).map(|f| (f, t.id))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        match (hit, rock) {
            (Some((f, _)), Some(r)) if r < f => self.stop(from.lerp(to, r)),
            (Some((_, id)), _) => return Some(id),
            (None, Some(r)) => self.stop(from.lerp(to, r)),
            (None, None) => {}
        }
        None
    }

    /// The shot hit the terrain at `at`: it ends there.
    fn stop(&mut self, at: Vec3) {
        self.pos = at;
        self.age = weapon::SHOT_LIFE;
    }

    pub fn is_spent(&self) -> bool {
        self.age >= weapon::SHOT_LIFE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERIOD: (f32, f32) = (2048.0, 16384.0);

    #[test]
    fn client_poses_are_refused_for_wrecks_and_just_after_a_respawn() {
        let tick = 0.05;
        // SetPose needs --allow-setpose; Reset does not.
        assert_eq!(
            check_client_pose(true, false, false, None, 100, tick),
            Err(PoseRefusal::NotAllowed)
        );
        assert_eq!(
            check_client_pose(true, true, false, None, 100, tick),
            Ok(())
        );
        assert_eq!(
            check_client_pose(false, false, false, None, 100, tick),
            Ok(())
        );
        // Never from a wreck.
        for set_pose in [true, false] {
            assert_eq!(
                check_client_pose(set_pose, true, true, None, 100, tick),
                Err(PoseRefusal::Wreck)
            );
        }
        // Locked for a second (20 ticks) after the respawn tick.
        for now in [100, 110, 119] {
            assert_eq!(
                check_client_pose(true, true, false, Some(100), now, tick),
                Err(PoseRefusal::JustRespawned),
                "tick {now}"
            );
            assert_eq!(
                check_client_pose(false, false, false, Some(100), now, tick),
                Err(PoseRefusal::JustRespawned),
                "tick {now}"
            );
        }
        assert_eq!(
            check_client_pose(false, false, false, Some(100), 120, tick),
            Ok(())
        );
        // Across the tick counter's wrap.
        assert_eq!(
            check_client_pose(false, false, false, Some(u32::MAX - 2), 5, tick),
            Err(PoseRefusal::JustRespawned)
        );
    }

    #[test]
    fn reset_keeps_the_client_position_only_within_reach() {
        let server = Vec3::new(100.0, 200.0, 10.0);
        let near = Vec3::new(110.0, 195.0, 12.0);
        assert_eq!(reset_position(near, server, PERIOD), near);
        let far = Vec3::new(1000.0, 200.0, 10.0);
        assert_eq!(reset_position(far, server, PERIOD), server);
        // Unfolded across the seam next to the server's pose.
        let seam = Vec3::new(2040.0, 200.0, 10.0);
        let folded = Vec3::new(5.0, 200.0, 10.0);
        assert_eq!(
            reset_position(folded, seam, PERIOD),
            Vec3::new(2053.0, 200.0, 10.0)
        );
        assert_eq!(reset_position(Vec3::NAN, server, PERIOD), server);
    }

    #[test]
    fn segment_sphere_hits_and_misses() {
        let c = Vec3::new(100.0, 0.0, 0.0);
        let hit = segment_hits_sphere(Vec3::ZERO, Vec3::new(200.0, 0.0, 0.0), c, 10.0);
        assert!((hit.unwrap() - 0.45).abs() < 1e-4, "{hit:?}");
        // Stops short, passes beside, points away.
        assert_eq!(
            segment_hits_sphere(Vec3::ZERO, Vec3::X * 80.0, c, 10.0),
            None
        );
        assert_eq!(
            segment_hits_sphere(Vec3::Y * 11.0, Vec3::new(200.0, 11.0, 0.0), c, 10.0),
            None
        );
        assert_eq!(
            segment_hits_sphere(Vec3::ZERO, -Vec3::X * 200.0, c, 10.0),
            None
        );
        // Starting inside counts at once.
        assert_eq!(segment_hits_sphere(c, c + Vec3::Y, c, 10.0), Some(0.0));
    }

    #[test]
    fn shots_hit_the_nearest_other_car_never_the_shooter() {
        let targets = [
            Target {
                id: 1,
                center: Vec3::ZERO,
                radius: 15.0,
            },
            Target {
                id: 2,
                center: Vec3::new(0.0, 60.0, 0.0),
                radius: 15.0,
            },
            Target {
                id: 3,
                center: Vec3::new(0.0, 30.0, 0.0),
                radius: 15.0,
            },
        ];
        // Player 1 fires from inside its own sphere along +Y.
        let mut shot = ServerShot::new(1, 0, Vec3::ZERO, Vec3::Y, 0.0);
        assert_eq!(
            shot.step(0.05, &targets, PERIOD, None),
            Some(3),
            "20u: 3 is nearer than 2"
        );
        // Fired by 3 along +Y: the next car out is 2.
        let mut shot = ServerShot::new(3, 0, Vec3::new(0.0, 30.0, 0.0), Vec3::Y, 0.0);
        assert_eq!(shot.step(0.05, &targets, PERIOD, None), Some(2));
        // Alone, the shooter's own sphere is never hit.
        let mut shot = ServerShot::new(1, 0, Vec3::ZERO, -Vec3::Y, 0.0);
        for _ in 0..40 {
            assert_eq!(shot.step(0.05, &targets[..1], PERIOD, None), None);
        }
        assert!(shot.is_spent());
    }

    #[test]
    fn the_nearest_escave_is_picked_the_short_way_round() {
        // Fostral's two: Incubator and Podish.
        let escaves = [(1657, 353), (1500, 15879)];
        let pos = |x, y| Vec3::new(x, y, 10.0);
        assert_eq!(
            nearest_escave(pos(590.0, 4140.0), &escaves, PERIOD),
            Some((1657, 353))
        );
        assert_eq!(
            nearest_escave(pos(1500.0, 14000.0), &escaves, PERIOD),
            Some((1500, 15879))
        );
        // Just under the Y seam, Incubator (357 on across it) beats
        // Podish (501 back), though it is 16000 off the naive way.
        assert_eq!(
            nearest_escave(pos(1660.0, 16380.0), &escaves, PERIOD),
            Some((1657, 353))
        );
        // And across the X seam: x=2040 is 17 from x=9.
        let near_seam = [(1000, 500), (9, 500)];
        assert_eq!(
            nearest_escave(pos(2040.0, 500.0), &near_seam, PERIOD),
            Some((9, 500))
        );
        assert_eq!(nearest_escave(pos(0.0, 0.0), &[], PERIOD), None);
    }

    #[test]
    fn a_wreck_respawns_after_the_delay_across_the_wrap() {
        let tick = 0.05; // 20 Hz
        let due = respawn_tick(100, tick);
        assert_eq!(due, 100 + (RESPAWN_SECS / tick).round() as u32);
        assert!(!respawn_due(100, due));
        assert!(!respawn_due(due - 1, due));
        assert!(respawn_due(due, due));
        assert!(respawn_due(due + 5, due));
        // The tick counter wraps while the wreck waits.
        let due = respawn_tick(u32::MAX - 10, tick);
        assert!(due < 100, "{due}");
        assert!(!respawn_due(u32::MAX - 1, due));
        assert!(respawn_due(due, due));
        // A degenerate tick still waits a tick.
        assert_eq!(respawn_tick(7, 0.0), 8);
        assert_eq!(respawn_tick(7, 10.0), 8);
    }

    #[test]
    fn wrecks_take_no_hits_and_fire_no_shots() {
        let targets = |_: f32| {
            vec![
                Target {
                    id: 1,
                    center: Vec3::ZERO,
                    radius: 15.0,
                },
                Target {
                    id: 2,
                    center: Vec3::new(0.0, 30.0, 0.0),
                    radius: 15.0,
                },
                Target {
                    id: 3,
                    center: Vec3::new(0.0, 60.0, 0.0),
                    radius: 15.0,
                },
            ]
        };
        let fly = |dead: &[u32]| {
            let mut shots = vec![
                // 1 fires up +Y through 2's sphere to 3.
                ServerShot::new(1, 0, Vec3::ZERO, Vec3::Y, 0.0),
                // 3 fires back down -Y at 2.
                ServerShot::new(3, 1, Vec3::new(0.0, 60.0, 0.0), -Vec3::Y, 0.0),
            ];
            let mut hits = Vec::new();
            for _ in 0..40 {
                hits.extend(fly_shots(&mut shots, 0.05, PERIOD, None, targets, |id| {
                    dead.contains(&id)
                }));
            }
            assert!(shots.is_empty(), "every shot hit or was spent");
            hits
        };
        // All alive: both shots hit 2, the car between them.
        assert_eq!(fly(&[]), vec![(1, 0, 2), (3, 1, 2)]);
        // 2 is a wreck: 1's shot flies through it into 3; 3's flies
        // through it into 1.
        assert_eq!(fly(&[2]), vec![(1, 0, 3), (3, 1, 1)]);
        // 3 is a wreck: its shot is dropped, and 1's still stops at 2.
        assert_eq!(fly(&[3]), vec![(1, 0, 2)]);
    }

    #[test]
    fn shots_hit_across_the_seam() {
        let targets = [Target {
            id: 2,
            center: Vec3::new(10.0, 500.0, 0.0),
            radius: 15.0,
        }];
        let mut shot = ServerShot::new(1, 0, Vec3::new(2030.0, 500.0, 0.0), Vec3::X, 0.0);
        assert_eq!(shot.step(0.1, &targets, PERIOD, None), Some(2));
    }

    #[test]
    fn rewind_follows_the_view_and_is_capped() {
        let tick = 0.05;
        // Newest snapshot 3 ticks old, drawn on 50 ms (one tick) since.
        assert!((rewind_ticks(100, 97, 50, tick) - 2.0).abs() < 1e-4);
        // A 1 s old view: capped at 200 ms = 4 ticks.
        assert!((rewind_ticks(100, 80, 0, tick) - 4.0).abs() < 1e-4);
        // A view from the future, or no time: no rewind.
        assert_eq!(rewind_ticks(100, 105, 0, tick), 0.0);
        assert_eq!(rewind_ticks(100, 90, 0, 0.0), 0.0);
    }

    #[test]
    fn history_rewinds_between_ticks() {
        let mut h = PoseHistory::default();
        assert_eq!(h.at(5.0, PERIOD), None);
        for t in 0..30u32 {
            h.record(t, Vec3::new(t as f32 * 10.0, 0.0, 0.0), 20);
        }
        // Only the last 20 ticks are kept: 10..=29.
        assert_eq!(h.at(0.0, PERIOD), Some(Vec3::new(100.0, 0.0, 0.0)));
        assert_eq!(h.at(25.5, PERIOD), Some(Vec3::new(255.0, 0.0, 0.0)));
        assert_eq!(h.at(40.0, PERIOD), Some(Vec3::new(290.0, 0.0, 0.0)));
        // Interpolated the short way across the seam.
        let mut h = PoseHistory::default();
        h.record(1, Vec3::new(2040.0, 0.0, 0.0), 20);
        h.record(2, Vec3::new(4.0, 0.0, 0.0), 20);
        let mid = h.at(1.5, PERIOD).unwrap();
        assert!((mid.x - 2046.0).abs() < 1e-3, "{mid:?}");
        // A teleport is not slid across.
        h.record(3, Vec3::new(900.0, 0.0, 0.0), 20);
        assert_eq!(h.at(2.4, PERIOD), Some(Vec3::new(4.0, 0.0, 0.0)));
        assert_eq!(h.at(2.6, PERIOD), Some(Vec3::new(900.0, 0.0, 0.0)));
    }

    #[test]
    fn a_rewound_target_is_hit_where_the_shooter_saw_it() {
        // The target drove +X at 200u/s (10u a tick); the shooter saw it 4
        // ticks (200 ms) back, at x=60, and aimed there.
        let mut h = PoseHistory::default();
        for t in 0..=10u32 {
            h.record(t, Vec3::new(t as f32 * 10.0 + 20.0, 100.0, 0.0), 20);
        }
        let now = 10;
        let rewind = rewind_ticks(now, 6, 0, 0.05);
        assert_eq!(rewind, 4.0);
        let mut shot = ServerShot::new(1, 0, Vec3::new(80.0, 0.0, 0.0), Vec3::Y, rewind);
        let seen = Target {
            id: 2,
            center: h.at(now as f32 - rewind, PERIOD).unwrap(),
            radius: 15.0,
        };
        let current = Target {
            id: 2,
            center: h.at(now as f32, PERIOD).unwrap(),
            radius: 15.0,
        };
        let mut unrewound = shot.clone();
        assert_eq!(shot.step(0.3, &[seen], PERIOD, None), Some(2));
        assert_eq!(
            unrewound.step(0.3, &[current], PERIOD, None),
            None,
            "it has moved on by now"
        );
    }

    #[test]
    fn damage_is_flat_per_bay() {
        assert!(bay_damage(0) > 0 && bay_damage(1) > 0 && bay_damage(2) > 0);
        assert_eq!(bay_damage(200), bay_damage(2));
    }

    /// A 256x256 level, flat at height `ground` (altitude scale 1).
    fn flat_level(ground: u8) -> Level {
        let mut level =
            crate::level::load(&crate::level::LevelConfig::new_test(), &Default::default());
        level.height.fill(ground);
        level.meta.fill(0);
        level
    }

    /// Turn texels `x0..x1` x `y0..y1` into a slab over a cave: floor `low`,
    /// ceiling `low + delta * 8` (default geometry), top `high`. Returns the
    /// ceiling as read back.
    fn add_slab(
        level: &mut Level,
        (x0, x1): (i32, i32),
        (y0, y1): (i32, i32),
        low: u8,
        delta: u8,
        high: u8,
    ) -> f32 {
        for y in y0..y1 {
            // Double-level texels come in even/odd pairs.
            for x in (x0 & !1..x1).step_by(2) {
                let i = level.wrap((x, y));
                level.height[i] = low;
                level.height[i + 1] = high;
                level.meta[i] = crate::level::DOUBLE_LEVEL | (delta >> 2);
                level.meta[i + 1] = crate::level::DOUBLE_LEVEL | (delta & 3);
            }
        }
        match level.get((x0, y0)) {
            crate::level::Texel::Dual { mid, .. } => mid,
            crate::level::Texel::Single(_) => panic!("no slab"),
        }
    }

    const SMALL: (f32, f32) = (256.0, 256.0);

    fn fly(shot: &mut ServerShot<u32>, targets: &[Target<u32>], level: &Level) -> Option<u32> {
        (0..40).find_map(|_| shot.step(0.05, targets, SMALL, Some(level)))
    }

    #[test]
    fn a_hill_between_blocks_the_shot() {
        let target = [Target {
            id: 2,
            center: Vec3::new(150.0, 100.0, 20.0),
            radius: 10.0,
        }];
        let mut level = flat_level(10);
        let mut shot = ServerShot::new(1, 0, Vec3::new(50.0, 100.0, 20.0), Vec3::X, 0.0);
        assert_eq!(fly(&mut shot, &target, &level), Some(2), "open ground: hit");

        for y in 90..110 {
            for x in 95..105 {
                let i = level.wrap((x, y));
                level.height[i] = 60;
            }
        }
        let mut shot = ServerShot::new(1, 0, Vec3::new(50.0, 100.0, 20.0), Vec3::X, 0.0);
        assert_eq!(fly(&mut shot, &target, &level), None, "the hill takes it");
        assert!(shot.is_spent(), "and the shot ends there");
        assert!(
            (shot.pos.x - 95.0).abs() <= 1.0,
            "at the hill: {:?}",
            shot.pos
        );
    }

    #[test]
    fn a_shot_under_a_slab_flies_between_the_layers() {
        let target = [Target {
            id: 2,
            center: Vec3::new(150.0, 100.0, 20.0),
            radius: 10.0,
        }];
        let mut level = flat_level(10);
        let mid = add_slab(&mut level, (80, 120), (90, 110), 10, 4, 80);
        assert!(mid > 30.0 && mid < 80.0, "a cave 10..{mid} under the slab");
        let mut shot = ServerShot::new(1, 0, Vec3::new(50.0, 100.0, 20.0), Vec3::X, 0.0);
        assert_eq!(fly(&mut shot, &target, &level), Some(2));
    }

    #[test]
    fn a_shot_into_the_underside_of_a_slab_stops() {
        let mut level = flat_level(10);
        let mid = add_slab(&mut level, (80, 120), (90, 110), 10, 4, 80);
        // Rising at 45 degrees from the cave towards a car above the slab.
        let from = Vec3::new(60.0, 100.0, 20.0);
        let dir = Vec3::new(1.0, 0.0, 1.0);
        let target = [Target {
            id: 2,
            center: from + dir * 70.0,
            radius: 10.0,
        }];
        let mut open = ServerShot::new(1, 0, from, dir, 0.0);
        assert_eq!(
            fly(&mut open, &target, &flat_level(10)),
            Some(2),
            "no slab: hit"
        );
        let mut shot = ServerShot::new(1, 0, from, dir, 0.0);
        assert_eq!(fly(&mut shot, &target, &level), None);
        assert!(shot.is_spent());
        assert!(
            (80.0..120.0).contains(&shot.pos.x) && (shot.pos.z - mid).abs() <= 1.0,
            "stopped at the slab's underside (z {mid}): {:?}",
            shot.pos
        );
    }

    #[test]
    fn the_client_streak_ends_where_the_server_shot_stops() {
        let mut level = flat_level(10);
        for y in 90..110 {
            for x in 95..105 {
                let i = level.wrap((x, y));
                level.height[i] = 60;
            }
        }
        let from = Vec3::new(50.0, 100.0, 20.0);
        let mut server = ServerShot::new(1, 0, from, Vec3::X, 0.0);
        assert_eq!(fly(&mut server, &[], &level), None);

        let mut shot = weapon::Shot {
            pos: from,
            vel: Vec3::X * weapon::SHOT_SPEED,
        };
        let life = shot.flight_time(&level);
        assert!(life < weapon::SHOT_LIFE);
        let mut age = 0.0;
        while age < life {
            age = shot.step_within(1.0 / 60.0, age, life);
        }
        assert!(
            (shot.pos - server.pos).length() <= 1.0,
            "client {:?} vs server {:?}",
            shot.pos,
            server.pos
        );
    }
}
