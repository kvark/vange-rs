use std::fs::File;
use std::str::FromStr;
use super::text::Reader;


pub type Price = u32;
pub type Time = u16;
pub type Shield = u16;

#[derive(Debug)]
pub struct Car {
    pub name: String,
    pub class: u8,
    pub price_buy: Price,
    pub price_sell: Price,
    pub size: [u8; 4],
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

impl FromStr for Car {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        let mut items = s.split_whitespace();
        let name = items.next().unwrap().to_owned();
        let x: Vec<u32> = items.map(|i| i.parse().unwrap()).collect();
        assert_eq!(x.len(), 19);
        Ok(Car {
            name: name,
            class: x[0] as u8,
            price_buy: x[1] as Price,
            price_sell: x[2] as Price,
            size: [x[3] as u8, x[4] as u8, x[5] as u8, x[6] as u8],
            max_speed: x[7] as u8,
            max_armor: x[8] as u8,
            shield_max: x[9] as Shield,
            shield_regen: x[10] as Shield,
            shield_drop: x[11] as Shield,
            drop_time: x[12] as Time,
            max_fire: x[13] as Time,
            max_water: x[14] as Time,
            max_oxygen: x[15] as Time,
            max_fly: x[16] as Time,
            max_damage: x[17] as u8,
            max_teleport: x[18] as u8,
        })
    }
}

#[derive(Debug)]
pub struct Registry {
    pub main: Vec<Car>,
    pub ruffa: Vec<Car>,
    pub constructor: Vec<Car>,
}

impl Registry {
    pub fn load(file: File) -> Registry {
        let mut reg = Registry {
            main: Vec::new(),
            ruffa: Vec::new(),
            constructor: Vec::new(),
        };
        let mut fi = Reader::new(file);

        assert_eq!(fi.next(), "uniVang-ParametersFile_Ver_1");

        let num_main: u8 = fi.next().parse().unwrap();
        let num_ruffa: u8 = fi.next().parse().unwrap();
        let num_const: u8 = fi.next().parse().unwrap();
        info!("Reading {} main vehicles, {} ruffas, and {} constructors",
            num_main, num_ruffa, num_const);

        for _ in 0 .. num_main {
            reg.main.push(fi.next().parse().unwrap());
        }
        for _ in 0 .. num_ruffa {
            reg.ruffa.push(fi.next().parse().unwrap());
        }
        for _ in 0 .. num_const {
            reg.constructor.push(fi.next().parse().unwrap());
        }

        reg
    }
}
