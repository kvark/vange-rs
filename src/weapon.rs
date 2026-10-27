//! Shots from equipped weapon bays. No original ammo tables or pixel bullets.

use crate::escave::{Inventory, Kind};
use glam::Vec3;

pub const SHOT_SPEED: f32 = 400.0;
/// Seconds a shot flies before it is dropped.
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    pub pos: Vec3,
    pub vel: Vec3,
}

impl Shot {
    pub fn step(&mut self, dt: f32) {
        self.pos += self.vel * dt;
    }

    /// The streak drawn for the shot: from its head back along its flight.
    pub fn streak(&self) -> (Vec3, Vec3) {
        let dir = self.vel.try_normalize().unwrap_or(Vec3::Y);
        (self.pos, self.pos + dir * SHOT_DRAW_LEN)
    }
}

/// Wrap-aware `to - from` on a torus of `period` (XY; Z is not wrapped).
/// A zero period component does not wrap.
fn torus_delta(from: Vec3, to: Vec3, period: (f32, f32)) -> Vec3 {
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
pub fn remote_shot(origin: Vec3, dir: Vec3, drawn: Option<Vec3>, period: (f32, f32)) -> Shot {
    // `drawn` may be another replica of the car on the torus: take its
    // offset, so the shot stays on the reported muzzle's tile.
    let pos = match drawn.map(|car| torus_delta(origin, car, period)) {
        Some(offset) if offset.length() <= REMOTE_ANCHOR => origin + offset,
        _ => origin,
    };
    Shot {
        pos,
        vel: dir.try_normalize().unwrap_or(Vec3::Y) * SHOT_SPEED,
    }
}

/// Fire from a weapon bay. Cargo and empty bays produce no shot.
pub fn fire(inventory: &Inventory, bay: usize, origin: Vec3, forward: Vec3) -> Option<Shot> {
    let good = inventory.bay(bay)?;
    if good.kind != Kind::Weapon {
        return None;
    }
    let dir = {
        let len = forward.length();
        if len < 1e-4 { Vec3::Y } else { forward / len }
    };
    Some(Shot {
        pos: origin,
        vel: dir * SHOT_SPEED,
    })
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
        let mut shot = fire(&inv, 0, origin, Vec3::Y).expect("bay 0 should fire");
        assert_eq!(shot.pos, origin);
        shot.step(0.1);
        assert_ne!(shot.pos, origin, "the shot must leave the vehicle");
        assert!(shot.vel.length() > 0.0);
    }

    #[test]
    fn cargo_cannot_fire() {
        let (_shop, inv, _credits) = buy_laser();
        assert!(
            fire(&inv, 0, Vec3::ZERO, Vec3::Y).is_none(),
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
        let shot = remote_shot(origin, Vec3::new(0.0, 3.0, 0.0), Some(near), period);
        assert_eq!(shot.pos, near);
        assert_eq!(shot.vel, Vec3::Y * SHOT_SPEED);
        // A car drawn far away (or not at all): the reported muzzle.
        let far = origin + Vec3::X * (REMOTE_ANCHOR + 1.0);
        assert_eq!(remote_shot(origin, Vec3::Y, Some(far), period).pos, origin);
        assert_eq!(remote_shot(origin, Vec3::Y, None, period).pos, origin);
        // The car drawn one world-width west (another replica): the
        // same offset, on the muzzle's tile.
        let replica = near - Vec3::X * period.0;
        let shot_r = remote_shot(origin, Vec3::Y, Some(replica), period);
        assert!((shot_r.pos - near).length() < 1e-3, "{:?}", shot_r.pos);
        // The streak trails the head along the flight.
        let (from, to) = shot.streak();
        assert_eq!(from, near);
        assert_eq!(to, near + Vec3::Y * SHOT_DRAW_LEN);
    }

    #[test]
    fn empty_bays_cannot_fire() {
        let inv = Inventory::default();
        assert!(fire(&inv, 0, Vec3::ZERO, Vec3::Y).is_none());
        assert!(fire(&inv, 1, Vec3::ZERO, Vec3::Y).is_none());
    }
}
