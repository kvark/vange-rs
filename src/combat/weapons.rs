//! The original guns: `device.lst` names them and their size, `game.lst`
//! gives each its bullet (power, speed, life, reload) and every car its
//! weapon slots. One table, shared by the server (which resolves the hits)
//! and the clients (which draw the shots and keep the same reloads).
//!
//! Units are the original's:
//! - armour is kept in hundredths of a `car.prm` armour point
//!   ([`ARMOR_UNIT`]): the original keeps `MaxArmor << 16` and takes
//!   `(FirstPower << 16) / 100` off per bullet, so a bullet's `FirstPower`
//!   *is* its damage in these units (a Machotine bullet, 250, takes 2.5
//!   points);
//! - times are legacy game quanta, [`LEGACY_QUANTS_PER_SEC`] a second
//!   (`UnitTimeConst 20`); `Speed` is world units per quant.
//!
//! [`Table::original`] is a built-in copy of the shipped data, so a server
//! with no game files (the CI tests) runs the same guns. With the files,
//! [`Table::parse`] reads them, and a test checks the two agree.

use std::collections::HashMap;

/// Weapon bays a car may have (the model's first three slots).
pub const BAYS: usize = 3;
/// Armour is kept in hundredths of a `car.prm` point: bullet power units.
pub const ARMOR_UNIT: u32 = 100;
/// Game quanta per second of the original logic (`UnitTimeConst`).
pub const LEGACY_QUANTS_PER_SEC: f32 = 20.0;
/// Slot size of every bay of a car the table does not list (test cars):
/// any gun but the size-0 specials fits.
pub const UNKNOWN_CAR_SLOT: u8 = 7;

/// Armour (or damage) in [`ARMOR_UNIT`]s as points: `9750` is `97.50`.
pub fn points(armor: u32) -> String {
    format!("{}.{:02}", armor / ARMOR_UNIT, armor % ARMOR_UNIT)
}

/// One gun (`device.lst` `DeviceID Gun`) with its bullet (`game.lst`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gun {
    /// `ModelID`: what the shop sells and a client declares.
    pub model: String,
    /// `UvsDeviceName`.
    pub device: String,
    /// `SizeID`: fits a slot at least this big; 0 only a size-0 slot.
    pub size: u8,
    /// `GunID`: the bullet's index in `game.lst`.
    pub bullet_id: u8,
    /// The bullet's `BulletID` (`Laser`, `Rocket`, `FireBall`, ...).
    pub bullet: String,
    /// `LifeTime`, legacy quanta.
    pub life_time: u32,
    /// `FirstPower`: the damage, in armour hundredths.
    pub power: u32,
    /// `LastPower` (the same as `power` for every shipped gun).
    pub last_power: u32,
    /// `Speed`, world units per legacy quantum.
    pub speed: u32,
    /// `WaitTime`, in 1/256 of `WeaponWaitTime` quanta.
    pub wait_time: u32,
}

impl Gun {
    /// Armour (hundredths) one bullet takes off.
    pub fn damage(&self) -> u32 {
        self.power
    }

    /// World units per second.
    pub fn shot_speed(&self) -> f32 {
        self.speed as f32 * LEGACY_QUANTS_PER_SEC
    }

    /// Seconds a bullet flies.
    pub fn shot_life(&self) -> f32 {
        self.life_time as f32 / LEGACY_QUANTS_PER_SEC
    }

    /// Whether it fits a slot of `slot_size` (`mechos.cpp`: a sized gun
    /// in any slot as big, a size-0 one only in a size-0 slot).
    pub fn fits(&self, slot_size: u8) -> bool {
        if self.size == 0 {
            slot_size == 0
        } else {
            self.size <= slot_size
        }
    }
}

/// The guns and the cars' slots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    /// `WeaponWaitTime` (`game.lst`).
    pub weapon_wait_time: u32,
    /// In `device.lst` order.
    pub guns: Vec<Gun>,
    /// Slot size of each bay, per `car.prm` name; `None`: no such bay.
    pub cars: Vec<(String, [Option<u8>; BAYS])>,
}

/// A declared set of guns, one per bay, by `ModelID`.
pub type Loadout = [Option<String>; BAYS];

impl Table {
    /// Seconds between two shots of a bay: the original's
    /// `(WeaponWaitTime * WaitTime) >> 8` quanta of `GUN_WAIT`, plus the
    /// quantum it turns `GUN_READY` on.
    pub fn cooldown(&self, gun: &Gun) -> f32 {
        let quanta = ((self.weapon_wait_time * gun.wait_time) >> 8) + 1;
        quanta as f32 / LEGACY_QUANTS_PER_SEC
    }

    pub fn gun(&self, model: &str) -> Option<&Gun> {
        self.guns.iter().find(|g| g.model == model)
    }

    /// Any gun firing bullet `bullet_id` (they fly alike).
    pub fn by_bullet(&self, bullet_id: u8) -> Option<&Gun> {
        self.guns.iter().find(|g| g.bullet_id == bullet_id)
    }

    /// Slot size of each bay of `car`; a car the table lacks gets
    /// [`UNKNOWN_CAR_SLOT`] bays.
    pub fn bays(&self, car: &str) -> [Option<u8>; BAYS] {
        self.cars
            .iter()
            .find(|entry| entry.0 == car)
            .map_or([Some(UNKNOWN_CAR_SLOT); BAYS], |&(_, bays)| bays)
    }

    /// Check a declared `loadout` against `car`'s bays: every gun known,
    /// in a bay the car has, that it fits. Returns each bay's gun.
    pub fn check<'a>(
        &'a self,
        car: &str,
        loadout: &Loadout,
    ) -> Result<[Option<&'a Gun>; BAYS], String> {
        let bays = self.bays(car);
        let mut guns = [None; BAYS];
        for (bay, model) in loadout.iter().enumerate() {
            let Some(model) = model.as_ref() else {
                continue;
            };
            let gun = self
                .gun(model)
                .ok_or_else(|| format!("bay {bay}: no gun {model:?}"))?;
            match bays[bay] {
                None => return Err(format!("bay {bay}: {car} has no such bay")),
                Some(size) if !gun.fits(size) => {
                    return Err(format!(
                        "bay {bay}: {model} (size {}) does not fit slot size {size}",
                        gun.size
                    ));
                }
                Some(_) => guns[bay] = Some(gun),
            }
        }
        Ok(guns)
    }

    /// Read `game.lst` (bullets, `WeaponWaitTime`, unit slot matrices) and
    /// `device.lst` (guns); `cars` are the `car.prm` names in file order,
    /// which pick each car's slot matrix.
    pub fn parse(game_lst: &str, device_lst: &str, cars: &[String]) -> Result<Self, String> {
        let game = tokens(game_lst);
        let weapon_wait_time = value_after(&game, "WeaponWaitTime")?;
        let bullets = parse_bullets(&game)?;
        let matrices = parse_matrices(&game)?;
        let guns = parse_devices(&tokens(device_lst))
            .into_iter()
            .map(|(model, device, size, bullet_id)| {
                let b = bullets
                    .get(bullet_id as usize)
                    .ok_or_else(|| format!("{model}: no bullet {bullet_id}"))?;
                Ok(Gun {
                    model,
                    device,
                    size,
                    bullet_id,
                    bullet: b.kind.clone(),
                    life_time: b.life_time,
                    power: b.power,
                    last_power: b.last_power,
                    speed: b.speed,
                    wait_time: b.wait_time,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let cars = cars
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let slots = matrix_of(index)
                    .and_then(|m| matrices.get(&m))
                    .cloned()
                    .unwrap_or_default();
                let mut bays = [None; BAYS];
                for (size, slot) in slots {
                    if let Some(bay) = bays.get_mut(slot as usize) {
                        *bay = Some(size);
                    }
                }
                (name.clone(), bays)
            })
            .collect();
        Ok(Table {
            weapon_wait_time,
            guns,
            cars,
        })
    }

    /// The shipped `game.lst`, `device.lst` and `car.prm`, built in.
    pub fn original() -> Self {
        /// `game.lst` and `device.lst` fields of one gun.
        type Row = (
            &'static str,
            &'static str,
            u8,
            u8,
            &'static str,
            u32,
            u32,
            u32,
            u32,
        );
        #[rustfmt::skip]
        const GUNS: [Row; 13] = [
            // ModelID, UvsDeviceName, SizeID, GunID, BulletID, LifeTime, Power, Speed, WaitTime
            ("Crustest", "CRUSTEST_CANNON", 7, 20, "JumpBall", 150, 20000, 10, 2560),
            ("JumpBug", "BEEBBANOZA_BLOCKADE", 6, 19, "JumpBall", 100, 500, 10, 256),
            ("LightLaser", "SPEETLE_SYSTEM_LIGHT", 5, 17, "Rocket", 20, 3000, 25, 256),
            ("HeavyLaser", "SPEETLE_SYSTEM_HEAVY", 7, 18, "Rocket", 30, 7000, 35, 256),
            ("LightFireBall", "GHORB_GEAR_LIGHT", 3, 15, "FireBall", 30, 1250, 20, 128),
            ("HeavyFireBall", "GHORB_GEAR_HEAVY", 7, 16, "FireBall", 40, 2500, 30, 128),
            ("Terminator", "TERMINATOR", 0, 13, "Terminator", 150, 1000, 5, 128),
            ("LightMissile", "MACHOTINE_GUN_LIGHT", 3, 11, "Laser", 7, 250, 30, 85),
            ("HeavyMissile", "MACHOTINE_GUN_HEAVY", 6, 12, "Laser", 10, 333, 40, 85),
            ("Wasp", "TERMINATOR2", 0, 14, "Horde", 200, 1500, 10, 128),
            ("Amputat", "AMPUTATOR", 0, 21, "Hypnotise", 300, 20000, 10, 1),
            ("Degradat", "DEGRADATOR", 0, 21, "Hypnotise", 300, 20000, 10, 1),
            ("Scope", "MECHOSCOPE", 0, 21, "Hypnotise", 300, 20000, 10, 1),
        ];
        #[rustfmt::skip]
        const CARS: [(&str, [u8; BAYS]); 24] = [
            // `car.prm` order; 0: no such bay.
            ("OxidizeMonk", [3, 3, 0]), ("BladeKeeper", [5, 5, 0]), ("SpreadSpot", [5, 5, 0]),
            ("TheRipper", [3, 5, 6]), ("HeavyLady", [5, 5, 7]), ("AncientDemon", [5, 6, 0]),
            ("IronShadow", [0, 5, 7]), ("atTractor", [5, 0, 7]), ("Arcan", [5, 5, 0]),
            ("MadSurgeon", [0, 0, 5]), ("Zippax", [3, 5, 0]), ("RiverBier", [6, 7, 5]),
            ("Piercator", [3, 5, 0]), ("Excorps", [5, 5, 0]),
            ("BLOCH", [0; BAYS]), ("SMOK", [0; BAYS]), ("MOTOK", [0; BAYS]),
            ("SHROT", [0; BAYS]), ("DOCHOD", [0; BAYS]),
            ("Sandoll", [5, 5, 0]), ("LastMoggy", [7, 7, 0]), ("QueenFrog", [5, 5, 0]),
            ("LawnMower", [5, 5, 0]), ("Wormaster", [5, 5, 0]),
        ];
        Table {
            weapon_wait_time: 20,
            guns: GUNS
                .iter()
                .map(
                    |&(
                        model,
                        device,
                        size,
                        bullet_id,
                        bullet,
                        life_time,
                        power,
                        speed,
                        wait_time,
                    )| Gun {
                        model: model.to_string(),
                        device: device.to_string(),
                        size,
                        bullet_id,
                        bullet: bullet.to_string(),
                        life_time,
                        power,
                        last_power: power,
                        speed,
                        wait_time,
                    },
                )
                .collect(),
            cars: CARS
                .iter()
                .map(|&(name, sizes)| (name.to_string(), sizes.map(|s| (s != 0).then_some(s))))
                .collect(),
        }
    }

    /// The table from the game files under `data_path` (`game.lst`,
    /// `device.lst`, `car.prm`), or [`Table::original`] when they are
    /// missing or unreadable.
    pub fn load(data_path: &std::path::Path) -> Self {
        let read = |name: &str| {
            std::fs::read(data_path.join(name))
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .map_err(|e| format!("{name}: {e}"))
        };
        let parsed = (|| {
            let car_prm = read("car.prm")?;
            let cars: Vec<String> = crate::config::car::entries(car_prm.as_bytes())
                .into_iter()
                .map(|(name, _, _)| name)
                .collect();
            Table::parse(&read("game.lst")?, &read("device.lst")?, &cars)
        })();
        match parsed {
            Ok(table) => table,
            Err(e) => {
                log::warn!("Weapons: {e}; using the built-in table");
                Table::original()
            }
        }
    }
}

/// `UnitMatrixData` index of the car at `index` in `car.prm`
/// (`mechos.cpp`: the main cars from `UNIT_SUB_MATRIX_NUM`, the Ruffas
/// share one, the specials have their own).
fn matrix_of(index: usize) -> Option<u32> {
    match index {
        0..=13 => Some(9 + index as u32),
        14..=18 => Some(23),
        19 => Some(25),
        20 => Some(26),
        21 => Some(24),
        22 => Some(27),
        23 => Some(28),
        _ => None,
    }
}

/// Whitespace tokens with `//` comments dropped and a `{ ... }` group as
/// one token.
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut group: Option<Vec<&str>> = None;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("");
        for word in line.split_whitespace() {
            match (group.as_mut(), word) {
                (None, "{") => group = Some(Vec::new()),
                (Some(words), "}") => {
                    out.push(format!("{{ {} }}", words.join(" ")));
                    group = None;
                }
                (Some(words), w) => words.push(w),
                (None, w) => out.push(w.to_string()),
            }
        }
    }
    out
}

fn value_after<T: std::str::FromStr>(tokens: &[String], key: &str) -> Result<T, String> {
    let at = tokens
        .iter()
        .position(|t| t == key)
        .ok_or_else(|| format!("no {key}"))?;
    tokens
        .get(at + 1)
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| format!("bad {key}"))
}

/// The value after `key` in `block`, as `T`.
fn field<T: std::str::FromStr>(block: &[String], key: &str) -> Result<T, String> {
    value_after(block, key)
}

/// `block` split at every `start` token (anything before the first is
/// dropped).
fn blocks<'a>(tokens: &'a [String], start: &str) -> Vec<&'a [String]> {
    let starts: Vec<usize> = tokens
        .iter()
        .enumerate()
        .filter(|&(_, t)| t == start)
        .map(|(i, _)| i)
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(n, &s)| &tokens[s..starts.get(n + 1).copied().unwrap_or(tokens.len())])
        .collect()
}

struct Bullet {
    kind: String,
    life_time: u32,
    power: u32,
    last_power: u32,
    speed: u32,
    wait_time: u32,
}

/// The `GameBulletNumType` bullets of `game.lst`, in order.
fn parse_bullets(game: &[String]) -> Result<Vec<Bullet>, String> {
    let count: usize = value_after(game, "GameBulletNumType")?;
    let from = game
        .iter()
        .position(|t| t == "GameBulletNumType")
        .unwrap_or(0);
    let list = blocks(&game[from..], "BulletID");
    if list.len() < count {
        return Err(format!("{} bullets of {count}", list.len()));
    }
    list.iter()
        .take(count)
        .enumerate()
        .map(|(i, b)| {
            let bad = |e: String| format!("bullet {i}: {e}");
            Ok(Bullet {
                kind: b.get(1).cloned().ok_or_else(|| bad("no BulletID".into()))?,
                life_time: field(b, "LifeTime").map_err(bad)?,
                power: field(b, "FirstPower").map_err(bad)?,
                last_power: field(b, "LastPower").map_err(bad)?,
                speed: field(b, "Speed").map_err(bad)?,
                wait_time: field(b, "WaitTime").map_err(bad)?,
            })
        })
        .collect()
}

/// Each `UnitMatrixID`'s slots as `(SlotSize, 0-based slot)`.
fn parse_matrices(game: &[String]) -> Result<HashMap<u32, Vec<(u8, u8)>>, String> {
    let mut out = HashMap::new();
    for block in blocks(game, "UnitMatrixID") {
        let id: u32 = field(block, "UnitMatrixID")?;
        let num_id: u32 = field(block, "NumID")?;
        // The original reads the slots only of a matrix with `NumID`.
        let num_slot: usize = if num_id > 0 {
            field(block, "NumSlot")?
        } else {
            0
        };
        let list = |key: &str| -> Result<Vec<u8>, String> {
            let at = block
                .iter()
                .position(|t| t == key)
                .ok_or_else(|| format!("matrix {id}: no {key}"))?;
            block[at + 1..]
                .iter()
                .take(num_slot)
                .map(|v| v.parse().map_err(|_| format!("matrix {id}: bad {key}")))
                .collect()
        };
        let slots = if num_slot > 0 {
            let sizes = list("SlotSize")?;
            let slots = list("nSlot")?;
            sizes
                .into_iter()
                .zip(slots)
                .map(|(size, n)| (size, n.saturating_sub(1)))
                .collect()
        } else {
            Vec::new()
        };
        out.insert(id, slots);
    }
    Ok(out)
}

/// The `device.lst` guns as `(ModelID, UvsDeviceName, SizeID, GunID)`.
fn parse_devices(device: &[String]) -> Vec<(String, String, u8, u8)> {
    blocks(device, "DeviceID")
        .into_iter()
        .filter(|b| b.get(1).map(String::as_str) == Some("Gun"))
        .filter_map(|b| {
            Some((
                field(b, "ModelID").ok()?,
                field(b, "UvsDeviceName").ok()?,
                field(b, "SizeID").ok()?,
                field(b, "GunID").ok()?,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `device.lst` as shipped, the Machotine light gun's entry.
    const DEVICE_LST: &str = "NumDeviceType\t67\n\n\
DeviceID\tGun\nUvsDeviceName \tMACHOTINE_GUN_LIGHT\nModelID \tLightMissile\nSizeID\t\t3\nGunID\t\t11\n\n\
DeviceID\tOther\nUvsDeviceName \tSPEETLE_SYSTEM_AMMO0\nModelID \tAmmoGun\nSizeID\t\t0\n";

    /// `game.lst` as shipped: its reload unit, the Machotine light bullet
    /// (bullet 11) and the Retro slots (OxidizeMonk's).
    const GAME_LST_PARTS: [&str; 3] = [
        "UnitTimeConst\t20\nWeaponWaitTime\t20\n",
        "MACHOTINE_GUN_LIGHT\nBulletID\tLaser\nLifeTime\t7\nFirstPower\t250\nLastPower\t250\n\
CraterType\t4\nBulletMode\t{ Fly Speed }\nSpeed\t\t30\nShowID\t\tLaser\nShowType\t0\n\
ExtentionShowType\t0\nPrecision\t4\nTargetMode\tNone\nBulletScale\t256\nAltOffset\t0\n\
BulletRadius\t5\nTapeSize\t2\nWaitTime\t85\n",
        "//Retro\nUnitMatrixID\t9\nNumID\t4\nDataID\t3 0 0 0\nNumSlot 3\nSlotSize 0 3 3\nnSlot\t 4 1 2\n",
    ];

    /// `game.lst` with bullets 0..=10 stubbed before the real bullet 11.
    fn game_lst() -> String {
        let stub = "BulletID\tStub\nLifeTime\t1\nFirstPower\t1\nLastPower\t1\n\
BulletMode\t{ Fly }\nSpeed\t\t1\nWaitTime\t0\n";
        let [head, bullet, matrix] = GAME_LST_PARTS;
        format!(
            "{matrix}{head}GameBulletNumType 12\n{}{bullet}",
            stub.repeat(11)
        )
    }

    #[test]
    fn a_machotine_bullet_takes_two_and_a_half_points() {
        let cars = ["OxidizeMonk".to_string()];
        let table = Table::parse(&game_lst(), DEVICE_LST, &cars).unwrap();
        assert_eq!(table.guns.len(), 1, "only the Gun devices");
        let gun = table.gun("LightMissile").unwrap();
        assert_eq!(gun, Table::original().gun("LightMissile").unwrap());
        assert_eq!(
            (gun.bullet_id, gun.bullet.as_str(), gun.size),
            (11, "Laser", 3)
        );
        // FirstPower 250: 2.5 points of armour.
        assert_eq!(gun.damage(), 250);
        assert_eq!(gun.damage() as f32 / ARMOR_UNIT as f32, 2.5);
        // 30u a quantum for 7 quanta.
        assert_eq!((gun.shot_speed(), gun.shot_life()), (600.0, 0.35));
        // (20 * 85) >> 8 = 6 quanta waiting, ready on the 7th.
        assert_eq!(table.cooldown(gun), 0.35);
        assert_eq!(table.bays("OxidizeMonk"), [Some(3), Some(3), None]);
    }

    #[test]
    fn loadouts_are_checked_against_the_car_slots() {
        let table = Table::original();
        let set = |guns: [Option<&str>; BAYS]| guns.map(|g| g.map(str::to_string));
        // OxidizeMonk: two size-3 bays.
        let ok = table
            .check(
                "OxidizeMonk",
                &set([Some("LightMissile"), Some("LightFireBall"), None]),
            )
            .unwrap();
        assert_eq!(
            ok.map(|g| g.map(|g| g.damage())),
            [Some(250), Some(1250), None]
        );
        assert!(table.check("OxidizeMonk", &set([None, None, None])).is_ok());
        // A Speetle (size 5) does not fit, nor does anything in bay 2.
        assert!(
            table
                .check("OxidizeMonk", &set([Some("LightLaser"), None, None]))
                .is_err()
        );
        assert!(
            table
                .check("OxidizeMonk", &set([None, None, Some("LightMissile")]))
                .is_err()
        );
        assert!(
            table
                .check("OxidizeMonk", &set([Some("Nope"), None, None]))
                .is_err()
        );
        // HeavyLady carries the heavy Speetle in its size-7 bay 2.
        assert!(
            table
                .check("HeavyLady", &set([None, None, Some("HeavyLaser")]))
                .is_ok()
        );
        assert!(
            table
                .check("HeavyLady", &set([Some("HeavyLaser"), None, None]))
                .is_err()
        );
        // The size-0 specials fit no bay; Ruffas have none.
        assert!(
            table
                .check("LastMoggy", &set([Some("Terminator"), None, None]))
                .is_err()
        );
        assert!(
            table
                .check("BLOCH", &set([Some("LightMissile"), None, None]))
                .is_err()
        );
        // A car the table lacks (the test cars): any sized gun anywhere.
        assert!(
            table
                .check("TestCar", &set([Some("HeavyLaser"); BAYS]))
                .is_ok()
        );
    }

    #[test]
    fn the_original_guns_differ() {
        let table = Table::original();
        let gun = |m| table.gun(m).unwrap();
        // The two Fostral shop guns and the Ghorb.
        assert_eq!(gun("LightLaser").damage(), 3000);
        assert_eq!(table.cooldown(gun("LightLaser")), 1.05);
        assert_eq!(gun("LightFireBall").damage(), 1250);
        assert_eq!(table.cooldown(gun("LightFireBall")), 0.55);
        assert_eq!(table.by_bullet(17).unwrap().model, "LightLaser");
    }

    /// With the game files (not on CI): the built-in table is theirs.
    #[test]
    fn the_built_in_table_matches_the_game_files() {
        let Some(path) = ["../vange-data", "../Vangers/data", "/workspace/vange-data"]
            .iter()
            .map(std::path::Path::new)
            .find(|p| p.join("game.lst").exists() && p.join("device.lst").exists())
        else {
            eprintln!("no game data: skipped");
            return;
        };
        let read = |name: &str| {
            String::from_utf8_lossy(&std::fs::read(path.join(name)).unwrap()).into_owned()
        };
        let cars: Vec<String> = crate::config::car::entries(read("car.prm").as_bytes())
            .into_iter()
            .map(|(name, _, _)| name)
            .collect();
        let parsed = Table::parse(&read("game.lst"), &read("device.lst"), &cars).unwrap();
        let original = Table::original();
        assert_eq!(parsed.weapon_wait_time, original.weapon_wait_time);
        assert_eq!(parsed.guns, original.guns);
        assert_eq!(parsed.cars, original.cars);
        assert_eq!(Table::load(path), original);
    }
}
