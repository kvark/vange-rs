//! The energy shield (the original's `Energy`): a hit drains it before the
//! armour, and it fills back at the car's own rate.
//!
//! As in the original (KranX/Vangers, 71e95b8):
//! - `car.prm` gives each car `MaxEnergy` and `DeltaEnergy`, the two
//!   columns after `MaxArmor` (`src/uvs/univang.cpp:1201-1203`);
//! - a car starts full: `MaxEnergy = sc->MaxEnergy << 16`,
//!   `dEnergy = ((MaxEnergy / 10) * sc->DeltaEnergy) / (UnitGlobalTime * 100)`,
//!   `Energy = MaxEnergy` (`src/units/mechos.cpp:7985-7994`), with
//!   `UnitGlobalTime` = `UnitTimeConst` = 20 (`src/units/hobj.cpp:497-498`,
//!   `game.lst`);
//! - each quant of a car with armour left, `Energy += round(dEnergy *
//!   XTCORE_FRAME_NORMAL)` up to `MaxEnergy` (`src/units/mechos.cpp:8624-8627`);
//!   `XTCORE_FRAME_NORMAL` is the frame time over 0.05 s
//!   (`lib/xtool/xtcore.cpp:223`), 1 at 20 Hz;
//! - a bullet's power comes off `Energy` first, and only what it lacks off
//!   `Armor` (`VangerUnit::BulletCollision`, `src/units/mechos.cpp:969-973`);
//! - the HUD shows `Energy >> 16`, whole points
//!   (`src/units/mechos.cpp:3263-3265`).
//!
//! So a car regains `MaxEnergy * DeltaEnergy / 1000` points a second: 4.8
//! for the OxidizeMonk (120, 40), filling from empty in 25 s.
//!
//! Kept like the armour, in hundredths of a point ([`ARMOR_UNIT`]), where a
//! bullet's `FirstPower` is its damage. Not carried over: the jump's cost
//! in energy (`mechos.cpp:9408`) and the device discharge below
//! `DropEnergy` (`mechos.cpp:8628-8641`). The devices add nothing while
//! carried: `UnitEnergy` / `UnitDeletaEnergy` of `items.cpp:383-388` are
//! read and never used.

use super::ARMOR_UNIT;

/// `UnitTimeConst` of `game.lst`: the original logic's quanta a second.
pub const UNIT_GLOBAL_TIME: u32 = 20;

/// A car's shield, from its `car.prm` row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shield {
    /// `MaxEnergy`, in [`ARMOR_UNIT`]s.
    pub max: u32,
    /// `dEnergy`: refill per original quant (1/20 s), in [`ARMOR_UNIT`]s.
    pub regen: u32,
}

impl Shield {
    /// From `car.prm` `MaxEnergy` and `DeltaEnergy` (points).
    pub fn new(max_energy: u16, delta_energy: u16) -> Self {
        Shield {
            max: max_energy as u32 * ARMOR_UNIT,
            regen: regen_per_quant(max_energy, delta_energy),
        }
    }

    /// No shield: every hit goes to the armour. A car whose `car.prm`
    /// stats the server does not have (the test cars) gets this.
    pub const NONE: Shield = Shield { max: 0, regen: 0 };
}

/// `dEnergy` in [`ARMOR_UNIT`]s a quant: `MaxEnergy * DeltaEnergy /
/// (10 * UnitGlobalTime * 100)` points, to the nearest hundredth (the
/// original's 16.16 value is within a hundredth of it).
pub fn regen_per_quant(max_energy: u16, delta_energy: u16) -> u32 {
    let num = max_energy as u32 * delta_energy as u32 * ARMOR_UNIT;
    let den = 10 * UNIT_GLOBAL_TIME * 100;
    (num + den / 2) / den
}

/// A hit of `power` ([`ARMOR_UNIT`]s): off the energy first, the rest off
/// the armour, neither below 0 (`BulletCollision`). Returns what went to
/// the armour.
pub fn take_hit(energy: &mut u32, armor: &mut u32, power: u32) -> u32 {
    let through = power.saturating_sub(*energy);
    *energy = energy.saturating_sub(power);
    *armor = armor.saturating_sub(through);
    through
}

/// One step of refill, `frame_normal` original quanta long
/// (`XTCORE_FRAME_NORMAL`: 1 at a 20 Hz tick). Only a car with armour
/// left refills.
pub fn regenerate(energy: &mut u32, armor: u32, shield: Shield, frame_normal: f32) {
    if armor == 0 || *energy >= shield.max {
        *energy = (*energy).min(shield.max);
        return;
    }
    let step = (shield.regen as f32 * frame_normal).round() as u32;
    *energy = (*energy + step).min(shield.max);
}

/// The HUD line, on the original's scale: whole `car.prm` points, as
/// `aciCurEnergy = Energy >> 16` (the bar's `aciMaxEnergy` is that `+ 1`).
pub fn hud_line(energy: u32, max_energy: u32) -> String {
    format!("Energy {}/{}", energy / ARMOR_UNIT, max_energy / ARMOR_UNIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The OxidizeMonk's (`car.prm`: MaxEnergy 120, DeltaEnergy 40).
    const OXIDIZE_MONK: Shield = Shield {
        max: 120 * ARMOR_UNIT,
        regen: 24,
    };

    /// The original's `dEnergy`, 16.16, as `mechos.cpp:7986` computes it.
    fn original_d_energy(max_energy: u16, delta_energy: u16) -> i64 {
        let max = (max_energy as i64) << 16;
        ((max / 10) * delta_energy as i64) / (UNIT_GLOBAL_TIME as i64 * 100)
    }

    #[test]
    fn oxidize_monk_regen_matches_the_original() {
        // car.prm: OxidizeMonk ... MaxArmor 100, MaxEnergy 120, DeltaEnergy 40.
        let shield = Shield::new(120, 40);
        assert_eq!(shield.max, 12_000);
        assert_eq!(shield, OXIDIZE_MONK);
        // 0.24 points a quant, 4.8 a second; the original: 15728 / 65536.
        assert_eq!(shield.regen, 24);
        assert_eq!(original_d_energy(120, 40), 15728);
        let per_sec = shield.regen * UNIT_GLOBAL_TIME;
        assert_eq!(per_sec, 480);
        assert_eq!(per_sec, 120 * 40 * ARMOR_UNIT / 1000);
    }

    #[test]
    fn regen_is_within_a_hundredth_of_the_original() {
        // (MaxEnergy, DeltaEnergy) of every car in the shipped car.prm.
        for (max, delta) in [
            (120, 40),
            (350, 40),
            (300, 45),
            (300, 30),
            (400, 30),
            (260, 50),
            (200, 35),
            (150, 30),
            (250, 40),
            (180, 40),
            (50, 100),
            (250, 55),
            (500, 25),
            (200, 70),
        ] {
            let ours = regen_per_quant(max, delta) as f64 / ARMOR_UNIT as f64;
            let original = original_d_energy(max, delta) as f64 / 65536.0;
            assert!(
                (ours - original).abs() <= 0.01,
                "{max}/{delta}: {ours} vs {original}"
            );
        }
    }

    #[test]
    fn hits_drain_energy_before_armour() {
        let (mut energy, mut armor) = (1_000, 10_000);
        assert_eq!(take_hit(&mut energy, &mut armor, 250), 0);
        assert_eq!((energy, armor), (750, 10_000));
        assert_eq!(take_hit(&mut energy, &mut armor, 1_000), 250);
        assert_eq!((energy, armor), (0, 9_750));
        assert_eq!(take_hit(&mut energy, &mut armor, 20_000), 20_000);
        assert_eq!((energy, armor), (0, 0));
    }

    #[test]
    fn regen_fills_to_max_only_with_armour_left() {
        let shield = OXIDIZE_MONK;
        let mut energy = 0;
        regenerate(&mut energy, 0, shield, 1.0);
        assert_eq!(energy, 0, "a wreck does not refill");
        // 25 s at 20 Hz fills it from empty.
        for _ in 0..499 {
            regenerate(&mut energy, 100, shield, 1.0);
        }
        assert_eq!(energy, 11_976);
        regenerate(&mut energy, 100, shield, 1.0);
        assert_eq!(energy, shield.max);
        regenerate(&mut energy, 100, shield, 1.0);
        assert_eq!(energy, shield.max);
        // A 30 Hz tick is 2/3 of a quant.
        let mut energy = 0;
        regenerate(&mut energy, 100, shield, 20.0 / 30.0);
        assert_eq!(energy, 16);
    }

    #[test]
    fn hud_shows_whole_points() {
        assert_eq!(
            hud_line(OXIDIZE_MONK.max, OXIDIZE_MONK.max),
            "Energy 120/120"
        );
        // 87.99 points show as 87, as Energy >> 16 does.
        assert_eq!(hud_line(8_799, 12_000), "Energy 87/120");
    }
}
