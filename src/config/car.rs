use config::Settings;
use config::text::Reader;
use gfx;
use model;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;

pub type BoxSize = u8;
pub type Price = u32;
pub type Time = u16;
pub type Shield = u16;

#[derive(Clone, Debug)]
pub enum Kind {
    Main,
    Ruffa,
    Constructor,
}

#[derive(Clone, Debug)]
pub struct CarStats {
    pub class: u8,
    pub price_buy: Price,
    pub price_sell: Price,
    pub size: [BoxSize; 4],
    pub max_speed: u8,
    pub max_armor: u8,
    pub shield_max: Shield,
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
    fn new(d: &[u32]) -> CarStats {
        CarStats {
            class: d[0] as u8,
            price_buy: d[1] as Price,
            price_sell: d[2] as Price,
            size: [
                d[3] as BoxSize,
                d[4] as BoxSize,
                d[5] as BoxSize,
                d[6] as BoxSize,
            ],
            max_speed: d[7] as u8,
            max_armor: d[8] as u8,
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
    fn load(file: File) -> CarPhysics {
        let mut fi = Reader::new(file);
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
pub struct CarInfo<R: gfx::Resources> {
    pub kind: Kind,
    pub stats: CarStats,
    pub physics: CarPhysics,
    pub model: model::Model<R>,
    pub scale: f32,
}

pub fn load_registry<R: gfx::Resources, F: gfx::Factory<R>>(
    settings: &Settings,
    reg: &super::game::Registry,
    factory: &mut F,
) -> HashMap<String, CarInfo<R>> {
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

    for i in 0 .. num_main + num_ruffa + num_const {
        let (name, data) = fi.next_entry();
        let mi = &reg.model_infos[name];
        let mut prm_path = settings.data_path
            .join(&mi.path)
            .with_extension("prm");
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
        let mut file = BufReader::new(settings.open_relative(&mi.path));
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
                physics: physics,
                model: model::load_m3d(&mut file, factory),
                scale: scale,
            },
        );
    }

    map
}
