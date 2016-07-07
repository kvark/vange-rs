use std::collections::HashMap;
use std::fs::File;
use super::text::Reader;


pub type BoxSize = u8;
pub type Price = u32;
pub type Time = u16;
pub type Shield = u16;

#[derive(Clone, Copy, Debug)]
pub struct Car {
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

impl Car {
    fn new(d: &[u32]) -> Car {
        assert_eq!(d.len(), 19);
        Car {
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

#[derive(Debug)]
pub struct Registry {
    pub main: HashMap<String, Car>,
    pub ruffa: HashMap<String, Car>,
    pub constructor: HashMap<String, Car>,
}

impl Registry {
    pub fn load(file: File) -> Registry {
        let mut reg = Registry {
            main: HashMap::new(),
            ruffa: HashMap::new(),
            constructor: HashMap::new(),
        };
        let mut fi = Reader::new(file);
        fi.advance();
        assert_eq!(fi.cur(), "uniVang-ParametersFile_Ver_1");

        let num_main: u8 = fi.next_value();
        let num_ruffa: u8 = fi.next_value();
        let num_const: u8 = fi.next_value();
        info!("Reading {} main vehicles, {} ruffas, and {} constructors",
            num_main, num_ruffa, num_const);

        for _ in 0 .. num_main {
            let (name, data) = fi.next_entry();
            reg.main.insert(name.to_owned(), Car::new(&data));
        }
        for _ in 0 .. num_ruffa {
            let (name, data) = fi.next_entry();
            reg.ruffa.insert(name.to_owned(), Car::new(&data));
        }
        for _ in 0 .. num_const {
            let (name, data) = fi.next_entry();
            reg.constructor.insert(name.to_owned(), Car::new(&data));
        }

        reg
    }
}
