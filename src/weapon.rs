//! Shots from equipped weapon bays: the original guns' bullet speed, life
//! and reload ([`crate::combat::weapons`]), drawn as streaks (no pixel
//! bullets).

use crate::combat::{torus_delta, weapons};
use crate::escave::{Inventory, Kind};
use crate::level::Level;
use glam::Vec3;

/// Speed of a shot from a gun the table does not know.
pub const SHOT_SPEED: f32 = 400.0;
/// Seconds such a shot flies before it is dropped.
pub const SHOT_LIFE: f32 = 1.5;
/// Length of the streak a shot is drawn as.
pub const SHOT_DRAW_LEN: f32 = 18.0;
/// Streak colour of the local player's shots (ABGR, as `LineBuffer` takes).
pub const SHOT_COLOR: u32 = 0xFF44_EEFF;
/// Streak colour of other players' shots.
pub const REMOTE_SHOT_COLOR: u32 = 0xFF22_66FF;
/// Weapon bays a `Fire` may name.
pub const BAYS: u8 = 3;
/// Farthest a reported muzzle may be from the server's pose of the car: the
/// client predicts ahead of the server by its input latency, and a fast car
/// covers 100u and more in that time.
pub const FIRE_REACH: f32 = 160.0;
/// Most shots one player may fire per server tick (one per fire key).
pub const FIRES_PER_TICK: u8 = 2;
/// A remote shot leaves the shooter's car as drawn here when that car is
/// within this distance of the reported muzzle.
pub const REMOTE_ANCHOR: f32 = 64.0;

/// Streak colour of a bullet kind (`BulletID`), by what the original
/// draws for it; `None` for a kind without one of its own.
pub fn bullet_color(bullet: &str) -> Option<u32> {
    Some(match bullet {
        // Machotine tracer.
        "Laser" => 0xFF40_FFFF,
        // Speetle rocket.
        "Rocket" => 0xFF20_90FF,
        // Ghorb fireball.
        "FireBall" => 0xFF10_30FF,
        "JumpBall" => 0xFFFF_C040,
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    pub pos: Vec3,
    pub vel: Vec3,
    /// Seconds it flies (its bullet's `LifeTime`).
    pub life: f32,
}

impl Shot {
    pub fn step(&mut self, dt: f32) {
        self.pos += self.vel * dt;
    }

    /// A bullet of `gun` (the default speed and life for none).
    pub fn new(pos: Vec3, dir: Vec3, gun: Option<&weapons::Gun>) -> Self {
        let dir = dir.try_normalize().unwrap_or(Vec3::Y);
        let (speed, life) =
            gun.map_or((SHOT_SPEED, SHOT_LIFE), |g| (g.shot_speed(), g.shot_life()));
        Shot {
            pos,
            vel: dir * speed,
            life,
        }
    }

    /// Seconds the shot flies before the terrain stops it, at most its
    /// `life`: the server ends it at the same rock
    /// (`combat::ServerShot::step`), so a streak doesn't go through hills.
    pub fn flight_time(&self, level: &Level) -> f32 {
        let end = self.pos + self.vel * self.life;
        level
            .segment_hits_terrain(self.pos, end)
            .map_or(self.life, |t| t * self.life)
    }

    /// Fly on by `dt`, but not past `life` seconds of `age` (the shot is
    /// then at the rock that stopped it). Returns the new age.
    pub fn step_within(&mut self, dt: f32, age: f32, life: f32) -> f32 {
        self.step(dt.min((life - age).max(0.0)));
        age + dt
    }

    /// The streak drawn for the shot: from its head back along its flight.
    pub fn streak(&self) -> (Vec3, Vec3) {
        let dir = self.vel.try_normalize().unwrap_or(Vec3::Y);
        (self.pos, self.pos + dir * SHOT_DRAW_LEN)
    }
}

/// Server check of a `Fire`: the bay exists, the numbers are finite, the
/// muzzle is within [`FIRE_REACH`] of `car` (the server's pose of the
/// shooter, wrap-aware) and `dir` has a direction. Returns the unit `dir`.
pub fn check_fire(
    bay: u8,
    origin: Vec3,
    dir: Vec3,
    car: Vec3,
    period: (f32, f32),
) -> Result<Vec3, &'static str> {
    if bay >= BAYS {
        return Err("no such bay");
    }
    if !origin.is_finite() || !dir.is_finite() {
        return Err("not finite");
    }
    if torus_delta(car, origin, period).length() > FIRE_REACH {
        return Err("muzzle too far from the car");
    }
    dir.try_normalize().ok_or("no direction")
}

/// Another player's shot, as relayed by the server: it leaves the shooter's
/// car where this client draws it (`drawn`) when that is near the reported
/// muzzle, so the streak comes out of the car on screen rather than out of
/// where the shooter's own prediction had it; otherwise the reported muzzle.
/// It flies as a bullet of `gun`, the shooter's gun in that bay.
pub fn remote_shot(
    origin: Vec3,
    dir: Vec3,
    drawn: Option<Vec3>,
    period: (f32, f32),
    gun: Option<&weapons::Gun>,
) -> Shot {
    // `drawn` may be another replica of the car on the torus: take its
    // offset, so the shot stays on the reported muzzle's tile.
    let pos = match drawn.map(|car| torus_delta(origin, car, period)) {
        Some(offset) if offset.length() <= REMOTE_ANCHOR => origin + offset,
        _ => origin,
    };
    Shot::new(pos, dir, gun)
}

/// The gun model in each weapon bay (cargo is not a gun), as a client
/// declares them (`ClientMessage::Join` / `Weapons`).
pub fn loadout(inventory: &Inventory) -> weapons::Loadout {
    std::array::from_fn(|bay| {
        inventory
            .bay(bay)
            .filter(|good| good.kind == Kind::Weapon)
            .map(|good| good.id.clone())
    })
}

/// Fire from a weapon bay: a bullet of its gun in `table`. Cargo and
/// empty bays produce no shot.
pub fn fire(
    inventory: &Inventory,
    table: &weapons::Table,
    bay: usize,
    origin: Vec3,
    forward: Vec3,
) -> Option<Shot> {
    let good = inventory.bay(bay)?;
    if good.kind != Kind::Weapon {
        return None;
    }
    Some(Shot::new(origin, forward, table.gun(&good.id)))
}

/// Per-bay reload: a bay that fired waits its gun's
/// [`weapons::Table::cooldown`] before it fires again. The client keeps
/// one to fire no faster than the server takes; the server keeps one per
/// player to drop shots that come too fast.
#[derive(Clone, Debug, Default)]
pub struct Reload {
    ready: [f64; weapons::BAYS],
}

impl Reload {
    /// Slack a shot may come early by on the server: the time between two
    /// shots as it receives them jitters with the network. Early shots
    /// do not add up (the next is due a full cooldown after this one was).
    pub const SLACK: f64 = 0.1;

    /// Take a shot from `bay` at `now` (seconds) if its gun has reloaded
    /// (`slack` seconds early at most); it then waits `cooldown` more.
    pub fn try_fire(&mut self, bay: usize, now: f64, cooldown: f32, slack: f64) -> bool {
        let Some(ready) = self.ready.get_mut(bay) else {
            return false;
        };
        if now + slack < *ready {
            return false;
        }
        *ready = ready.max(now) + cooldown as f64;
        true
    }

    /// Every bay reloaded (new guns).
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::escave::{Inventory, Shop};

    fn buy_laser() -> (Shop, Inventory, i32) {
        let mut shop = Shop::fostral();
        let mut inv = Inventory::default();
        let mut credits = 200;
        shop.buy("LightLaser", &mut inv, &mut credits).unwrap();
        (shop, inv, credits)
    }

    #[test]
    fn fire_from_a_bay_leaves_the_vehicle() {
        let (_shop, mut inv, _credits) = buy_laser();
        inv.equip(0, 0).unwrap();
        let origin = Vec3::new(10.0, 20.0, 30.0);
        let table = weapons::Table::original();
        let mut shot = fire(&inv, &table, 0, origin, Vec3::Y).expect("bay 0 should fire");
        assert_eq!(shot.pos, origin);
        shot.step(0.1);
        assert_ne!(shot.pos, origin, "the shot must leave the vehicle");
        // A Speetle light rocket: 25u a quantum for 20 quanta.
        assert_eq!(shot.vel, Vec3::Y * 500.0);
        assert_eq!(shot.life, 1.0);
        assert_eq!(loadout(&inv), [Some("LightLaser".to_string()), None, None]);
    }

    #[test]
    fn a_bay_reloads_for_its_cooldown() {
        let mut reload = Reload::default();
        assert!(reload.try_fire(0, 10.0, 0.35, 0.0));
        assert!(!reload.try_fire(0, 10.2, 0.35, 0.0), "reloading");
        assert!(reload.try_fire(1, 10.2, 1.05, 0.0), "bays reload apart");
        assert!(reload.try_fire(0, 10.35, 0.35, 0.0));
        // With slack: early by up to it, but early shots do not add up.
        let mut reload = Reload::default();
        assert!(reload.try_fire(0, 0.0, 1.0, 0.1));
        assert!(reload.try_fire(0, 0.95, 1.0, 0.1), "50 ms early");
        assert!(!reload.try_fire(0, 1.85, 1.0, 0.1), "due at 2.0");
        assert!(reload.try_fire(0, 1.9, 1.0, 0.1));
        assert!(
            !reload.try_fire(weapons::BAYS, 5.0, 1.0, 0.1),
            "no such bay"
        );
    }

    #[test]
    fn cargo_cannot_fire() {
        let (_shop, inv, _credits) = buy_laser();
        assert!(
            fire(&inv, &weapons::Table::original(), 0, Vec3::ZERO, Vec3::Y).is_none(),
            "a gun in cargo is not usable"
        );
    }

    #[test]
    fn server_checks_the_fire() {
        let period = (2048.0, 16384.0);
        let car = Vec3::new(100.0, 100.0, 50.0);
        let dir = check_fire(0, car, Vec3::new(0.0, 2.0, 0.0), car, period).unwrap();
        assert_eq!(dir, Vec3::Y, "dir comes back unit");
        assert!(check_fire(BAYS, car, Vec3::Y, car, period).is_err());
        assert!(check_fire(0, car, Vec3::ZERO, car, period).is_err());
        assert!(check_fire(0, Vec3::splat(f32::NAN), Vec3::Y, car, period).is_err());
        let far = car + Vec3::X * (FIRE_REACH + 1.0);
        assert!(check_fire(0, far, Vec3::Y, car, period).is_err());
        // Across the seam: x=2040 is 108u west of x=100, not 1940u east.
        let seam = Vec3::new(2040.0, 100.0, 50.0);
        assert!(check_fire(1, seam, Vec3::Y, car, period).is_ok());
    }

    #[test]
    fn remote_shots_leave_the_drawn_car() {
        let period = (2048.0, 16384.0);
        let origin = Vec3::new(500.0, 500.0, 40.0);
        let near = origin + Vec3::new(20.0, -10.0, 0.0);
        let shot = remote_shot(origin, Vec3::new(0.0, 3.0, 0.0), Some(near), period, None);
        assert_eq!(shot.pos, near);
        assert_eq!(shot.vel, Vec3::Y * SHOT_SPEED);
        // As the shooter's gun flies: a Machotine tracer, 600u/s.
        let table = weapons::Table::original();
        let gun = table.by_bullet(11);
        let tracer = remote_shot(origin, Vec3::Y, Some(near), period, gun);
        assert_eq!((tracer.vel, tracer.life), (Vec3::Y * 600.0, 0.35));
        // A car drawn far away (or not at all): the reported muzzle.
        let far = origin + Vec3::X * (REMOTE_ANCHOR + 1.0);
        assert_eq!(
            remote_shot(origin, Vec3::Y, Some(far), period, None).pos,
            origin
        );
        assert_eq!(remote_shot(origin, Vec3::Y, None, period, None).pos, origin);
        // The car drawn one world-width west (another replica): the
        // same offset, on the muzzle's tile.
        let replica = near - Vec3::X * period.0;
        let shot_r = remote_shot(origin, Vec3::Y, Some(replica), period, None);
        assert!((shot_r.pos - near).length() < 1e-3, "{:?}", shot_r.pos);
        // The streak trails the head along the flight.
        let (from, to) = shot.streak();
        assert_eq!(from, near);
        assert_eq!(to, near + Vec3::Y * SHOT_DRAW_LEN);
    }

    #[test]
    fn empty_bays_cannot_fire() {
        let inv = Inventory::default();
        let table = weapons::Table::original();
        assert!(fire(&inv, &table, 0, Vec3::ZERO, Vec3::Y).is_none());
        assert!(fire(&inv, &table, 1, Vec3::ZERO, Vec3::Y).is_none());
        assert_eq!(loadout(&inv), [None, None, None]);
    }
}
