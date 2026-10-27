//! Server-side combat: shots flown on the server, hit against car spheres
//! with lag compensation, and armour.
//!
//! The server owns every car's armour. A shot it accepts (`weapon::check_fire`)
//! flies here at `weapon::SHOT_SPEED` for `weapon::SHOT_LIFE`, and each tick it
//! is tested against the other cars as the shooter saw them: rewound by how
//! far the shooter's view of the world was behind the server
//! ([`rewind_ticks`]), at most [`REWIND_CAP_SECS`].

use crate::weapon;
use glam::Vec3;
use std::collections::VecDeque;

/// Armour of a car whose `car.prm` stats the server does not have.
pub const DEFAULT_MAX_ARMOR: u16 = 100;
/// Farthest back a target is rewound for a shot, seconds: past this a
/// laggy shooter would hit cars that have long since moved on.
pub const REWIND_CAP_SECS: f32 = 0.2;
/// Pose history kept per car, seconds.
pub const HISTORY_SECS: f32 = 1.0;

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
    /// `period`. A spent shot (past `SHOT_LIFE`) hits nothing.
    pub fn step(&mut self, dt: f32, targets: &[Target<Id>], period: (f32, f32)) -> Option<Id> {
        if self.is_spent() {
            return None;
        }
        let from = self.pos;
        let to = from + self.vel * dt;
        self.pos = to;
        self.age += dt;
        targets
            .iter()
            .filter(|t| t.id != self.shooter)
            .filter_map(|t| {
                // The target's replica next to the shot.
                let center = from + torus_delta(from, t.center, period);
                segment_hits_sphere(from, to, center, t.radius).map(|f| (f, t.id))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id)| id)
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
            shot.step(0.05, &targets, PERIOD),
            Some(3),
            "20u: 3 is nearer than 2"
        );
        // Fired by 3 along +Y: the next car out is 2.
        let mut shot = ServerShot::new(3, 0, Vec3::new(0.0, 30.0, 0.0), Vec3::Y, 0.0);
        assert_eq!(shot.step(0.05, &targets, PERIOD), Some(2));
        // Alone, the shooter's own sphere is never hit.
        let mut shot = ServerShot::new(1, 0, Vec3::ZERO, -Vec3::Y, 0.0);
        for _ in 0..40 {
            assert_eq!(shot.step(0.05, &targets[..1], PERIOD), None);
        }
        assert!(shot.is_spent());
    }

    #[test]
    fn shots_hit_across_the_seam() {
        let targets = [Target {
            id: 2,
            center: Vec3::new(10.0, 500.0, 0.0),
            radius: 15.0,
        }];
        let mut shot = ServerShot::new(1, 0, Vec3::new(2030.0, 500.0, 0.0), Vec3::X, 0.0);
        assert_eq!(shot.step(0.1, &targets, PERIOD), Some(2));
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
        assert_eq!(shot.step(0.3, &[seen], PERIOD), Some(2));
        assert_eq!(
            unrewound.step(0.3, &[current], PERIOD),
            None,
            "it has moved on by now"
        );
    }

    #[test]
    fn damage_is_flat_per_bay() {
        assert!(bay_damage(0) > 0 && bay_damage(1) > 0 && bay_damage(2) > 0);
        assert_eq!(bay_damage(200), bay_damage(2));
    }
}
