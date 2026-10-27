use crate::{
    config::Settings, config::text::Reader, model, render::object::Context as ObjectContext,
};

use wgpu;

use std::{collections::HashMap, fs::File, io::Read};

pub type BoxSize = u8;
pub type Price = u32;
pub type Time = u16;
pub type Shield = u16;

#[derive(Copy, Clone, Debug)]
pub enum Kind {
    Main,
    Ruffa,
    Constructor,
}

#[derive(Copy, Clone, Debug)]
pub struct CarStats {
    pub class: u8,
    pub price_buy: Price,
    pub price_sell: Price,
    pub size: [BoxSize; 4],
    pub max_speed: u8,
    /// `MaxArmor`, in armour points (up to 500, LastMoggy).
    pub max_armor: u16,
    /// `MaxEnergy`: the energy shield, points (`combat::shield`).
    pub shield_max: Shield,
    /// `DeltaEnergy`: the shield's refill rate (`combat::shield`).
    pub shield_regen: Shield,
    pub shield_drop: Shield,
    pub drop_time: Time,
    pub max_fire: Time,
    pub max_water: Time,
    pub max_oxygen: Time,
    pub max_fly: Time,
    pub max_damage: u8,
    pub max_teleport: u8,
}

impl CarStats {
    pub fn new(d: &[u32]) -> Self {
        CarStats {
            class: d[0] as u8,
            price_buy: d[1],
            price_sell: d[2],
            size: [
                d[3] as BoxSize,
                d[4] as BoxSize,
                d[5] as BoxSize,
                d[6] as BoxSize,
            ],
            max_speed: d[7] as u8,
            max_armor: d[8] as u16,
            shield_max: d[9] as Shield,
            shield_regen: d[10] as Shield,
            shield_drop: d[11] as Shield,
            drop_time: d[12] as Time,
            max_fire: d[13] as Time,
            max_water: d[14] as Time,
            max_oxygen: d[15] as Time,
            max_fly: d[16] as Time,
            max_damage: d[17] as u8,
            max_teleport: d[18] as u8,
        }
    }
}

#[repr(u8)]
#[derive(Copy, Clone)]
pub enum _Side {
    Front,
    Back,
    Side,
    Upper,
    Lower,
}

pub const NUM_SIDES: usize = 5;

#[derive(Clone, Debug)]
pub struct CarPhysics {
    pub name: String,
    // base
    pub scale_size: f32,
    pub scale_bound: f32,
    pub scale_box: f32,
    pub z_offset_of_mass_center: f32,
    // car
    pub speed_factor: f32,
    pub mobility_factor: f32,
    // devices
    pub water_speed_factor: f32,
    pub air_speed_factor: f32,
    pub underground_speed_factor: f32,
    // ship
    pub k_archimedean: f32,
    pub k_water_traction: f32,
    pub k_water_rudder: f32,
    // grader
    pub terra_mover_sx: [f32; 3],
    // defence & ram
    pub defence: [u16; NUM_SIDES],
    pub ram_power: [u16; NUM_SIDES],
}

impl CarPhysics {
    pub fn load(file: File) -> Self {
        Self::load_reader(file)
    }

    /// Reader-based variant of [`CarPhysics::load`]. Useful when the
    /// source is a zip entry in memory rather than a real file.
    pub fn load_reader<R: Read>(reader: R) -> Self {
        let mut fi = Reader::new(reader);
        fi.advance();
        CarPhysics {
            name: fi.cur().split_whitespace().nth(1).unwrap().to_owned(),
            scale_size: fi.next_key_value("scale_size:"),
            scale_bound: fi.next_key_value("scale_bound:"),
            scale_box: fi.next_key_value("scale_box:"),
            z_offset_of_mass_center: fi.next_key_value("z_offset_of_mass_center:"),
            speed_factor: fi.next_key_value("speed_factor:"),
            mobility_factor: fi.next_key_value("mobility_factor:"),
            water_speed_factor: fi.next_key_value("water_speed_factor:"),
            air_speed_factor: fi.next_key_value("air_speed_factor:"),
            underground_speed_factor: fi.next_key_value("underground_speed_factor:"),
            k_archimedean: fi.next_key_value("k_archimedean:"),
            k_water_traction: fi.next_key_value("k_water_traction:"),
            k_water_rudder: fi.next_key_value("k_water_rudder:"),
            terra_mover_sx: [
                fi.next_key_value("TerraMoverSx:"),
                fi.next_key_value("TerraMoverSy:"),
                fi.next_key_value("TerraMoverSz:"),
            ],
            defence: [
                fi.next_key_value("FrontDefense:"),
                fi.next_key_value("BackDefense:"),
                fi.next_key_value("SideDefense:"),
                fi.next_key_value("UpperDefense:"),
                fi.next_key_value("LowerDefense:"),
            ],
            ram_power: [
                fi.next_key_value("FrontRamPower:"),
                fi.next_key_value("BackRamPower:"),
                fi.next_key_value("SideRamPower:"),
                fi.next_key_value("UpperRamPower:"),
                fi.next_key_value("LowerRamPower:"),
            ],
        }
    }
}

#[derive(Clone)]
pub struct CarInfo {
    pub kind: Kind,
    pub stats: CarStats,
    pub physics: CarPhysics,
    pub model: model::VisualModel,
    pub scale: f32,
}

/// Parse `car.prm` and return `(name, stats)` for the first "main"
/// vehicle entry. The web build uses this to spawn a default player
/// vehicle without the full `load_registry` dance (which needs
/// `Settings` + on-disk paths).
pub fn first_main_entry<R: Read>(reader: R) -> (String, Vec<u32>) {
    let mut fi = Reader::new(reader);
    fi.advance();
    assert_eq!(fi.cur(), "uniVang-ParametersFile_Ver_1");
    let _num_main: u8 = fi.next_value();
    let _num_ruffa: u8 = fi.next_value();
    let _num_const: u8 = fi.next_value();
    let (name, data) = fi.next_entry::<u32>();
    (name.to_owned(), data)
}

/// Every vehicle of `car.prm` in file order: its name, kind and raw
/// stats. For a client that builds cars by name on demand (the web build
/// loading a remote player's car).
pub fn entries<R: Read>(reader: R) -> Vec<(String, Kind, Vec<u32>)> {
    let mut fi = Reader::new(reader);
    fi.advance();
    assert_eq!(fi.cur(), "uniVang-ParametersFile_Ver_1");
    let num_main: u8 = fi.next_value();
    let num_ruffa: u8 = fi.next_value();
    let num_const: u8 = fi.next_value();
    let total = num_main as usize + num_ruffa as usize + num_const as usize;
    (0..total)
        .map(|i| {
            let (name, data) = fi.next_entry::<u32>();
            let kind = if i < num_main as usize {
                Kind::Main
            } else if i < num_main as usize + num_ruffa as usize {
                Kind::Ruffa
            } else {
                Kind::Constructor
            };
            (name.to_owned(), kind, data)
        })
        .collect()
}

pub fn load_registry(
    settings: &Settings,
    reg: &super::game::Registry,
    device: &wgpu::Device,
    object: &ObjectContext,
) -> HashMap<String, CarInfo> {
    let mut map = HashMap::new();
    let mut fi = Reader::new(settings.open_relative("car.prm"));
    fi.advance();
    assert_eq!(fi.cur(), "uniVang-ParametersFile_Ver_1");

    let num_main: u8 = fi.next_value();
    let num_ruffa: u8 = fi.next_value();
    let num_const: u8 = fi.next_value();
    info!(
        "Reading {} main vehicles, {} ruffas, and {} constructors",
        num_main, num_ruffa, num_const
    );

    for i in 0..num_main + num_ruffa + num_const {
        let (name, data) = fi.next_entry();
        let mi = &reg.model_infos[name];
        let mut prm_path = settings.data_path.join(&mi.path).with_extension("prm");
        let is_default = !prm_path.exists();
        if is_default {
            warn!("Vehicle {} doesn't have parameters, using defaults", name);
            prm_path.set_file_name("default");
        }
        let physics = CarPhysics::load(File::open(prm_path).unwrap());
        let scale = if is_default {
            mi.scale
        } else {
            physics.scale_size
        };
        let file = settings.open_relative(&mi.path);
        let model = model::load_m3d(file, device, object, settings.game.physics.shape_sampling);
        map.insert(
            name.to_owned(),
            CarInfo {
                kind: if i < num_main {
                    Kind::Main
                } else if i < num_main + num_ruffa {
                    Kind::Ruffa
                } else {
                    Kind::Constructor
                },
                stats: CarStats::new(&data),
                physics,
                model,
                scale,
            },
        );
    }

    map
}

/// Load car physics data for all cars without GPU resources (for headless servers).
pub fn load_physics_registry(
    data_path: &std::path::Path,
    reg: &super::game::Registry,
    shape_sampling: u8,
) -> HashMap<String, crate::physics::CarPhysicsData> {
    use crate::physics::CarPhysicsData;

    let open = |rel: &str| -> File {
        File::open(data_path.join(rel))
            .unwrap_or_else(|_| panic!("Unable to open game file: {}", rel))
    };

    let mut map = HashMap::new();
    let mut fi = Reader::new(open("car.prm"));
    fi.advance();
    assert_eq!(fi.cur(), "uniVang-ParametersFile_Ver_1");

    let num_main: u8 = fi.next_value();
    let num_ruffa: u8 = fi.next_value();
    let num_const: u8 = fi.next_value();

    for _ in 0..num_main + num_ruffa + num_const {
        let (name, _data) = fi.next_entry::<i32>();
        let mi = &reg.model_infos[name];
        let mut prm_path = data_path.join(&mi.path).with_extension("prm");
        let is_default = !prm_path.exists();
        if is_default {
            prm_path.set_file_name("default");
        }
        let m3d_file = open(&mi.path);
        let prm_file = File::open(&prm_path).unwrap();
        let phys = CarPhysicsData::load_from_files(m3d_file, prm_file, mi.scale, shape_sampling);
        let final_scale = if is_default {
            mi.scale
        } else {
            phys.physics.scale_size
        };
        map.insert(
            name.to_owned(),
            CarPhysicsData {
                scale: final_scale,
                ..phys
            },
        );
    }

    map
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `car.prm`, as shipped, cut down to three cars.
    const CAR_PRM: &str = "uniVang-ParametersFile_Ver_1\n3\n0\n0\nOxidizeMonk 1\t  5477\t 5477  0       0       3\t4\t 13\t 100\t 120\t 40\t 1\t50\t0\t2000\t250\t50\t 1\t 2\nHeavyLady   4\t   129456  129456 4\t  3\t  1\t   4\t    11\t    380     400     30\t    1\t   40\t   2\t   2000    900\t   50\t   3\t   5\nLastMoggy 5    262350 29150  2\t     3\t     2\t      4        10      500     500     25     1      40      2\t     9000    1000     50 \t 3\t 6\n";

    #[test]
    fn max_armor_is_read_whole() {
        let armor: Vec<(String, u16)> = entries(CAR_PRM.as_bytes())
            .into_iter()
            .map(|(name, _, data)| (name, CarStats::new(&data).max_armor))
            .collect();
        // Past 255: HeavyLady and LastMoggy used to wrap around in a u8.
        assert_eq!(
            armor,
            [
                ("OxidizeMonk".to_string(), 100),
                ("HeavyLady".to_string(), 380),
                ("LastMoggy".to_string(), 500),
            ]
        );
    }

    #[test]
    fn energy_columns_follow_max_armor() {
        // univang.cpp:1201-1203: MaxArmor, MaxEnergy, DeltaEnergy.
        let shields: Vec<(String, u16, u16)> = entries(CAR_PRM.as_bytes())
            .into_iter()
            .map(|(name, _, data)| {
                let stats = CarStats::new(&data);
                (name, stats.shield_max, stats.shield_regen)
            })
            .collect();
        assert_eq!(
            shields,
            [
                ("OxidizeMonk".to_string(), 120, 40),
                ("HeavyLady".to_string(), 400, 30),
                ("LastMoggy".to_string(), 500, 25),
            ]
        );
    }
}
