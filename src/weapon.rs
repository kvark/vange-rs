//! Shots from equipped weapon bays. No original ammo tables or pixel bullets.

use crate::escave::{Inventory, Kind};
use glam::Vec3;

pub const SHOT_SPEED: f32 = 400.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    pub pos: Vec3,
    pub vel: Vec3,
}

impl Shot {
    pub fn step(&mut self, dt: f32) {
        self.pos += self.vel * dt;
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
    fn empty_bays_cannot_fire() {
        let inv = Inventory::default();
        assert!(fire(&inv, 0, Vec3::ZERO, Vec3::Y).is_none());
        assert!(fire(&inv, 1, Vec3::ZERO, Vec3::Y).is_none());
    }
}
