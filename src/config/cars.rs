use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::str::FromStr;


pub type Price = u32;
pub type Time = u16;
pub type Shield = u16;

#[derive(Debug)]
pub struct Car {
    pub name: String,
    pub class: u8,
    pub price_total: Price,
    pub price_part: Price,
    pub size: [u8; 3],
    pub shield_regen: Shield,
    pub max_speed: u8,
    pub max_armor: u8,
    pub max_shield: Shield,
    //pub drop_energy: u16,
    //pub drop_time: Time,
    //pub max_fire: Time,
    //pub max_water: Time,
    pub max_oxygen: Time,
    //pub max_armor_destruction: u8,
    //pub max_damage: u8,
    pub spiral_capacity: u8,
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
            price_total: x[1] as Price,
            price_part: x[2] as Price,
            size: [x[3] as u8, x[4] as u8, x[5] as u8],
            shield_regen: x[6] as Shield,
            max_speed: x[7] as u8,
            max_armor: x[8] as u8,
            max_shield: x[9] as Shield,
            //drop_energy: x[8] as u16,
            //drop_time: x[9] as Time,
            //max_fire: x[10] as Time,
            //max_water: x[11] as Time,
            max_oxygen: x[15] as Time,
            //max_armor_destruction: x[13] as u8,
            //max_damage: x[14] as u8,
            spiral_capacity: x[18] as u8,
        })
    }
}

#[derive(Debug)]
pub struct Registry {
    pub main: Vec<Car>,
    pub ruffa: Vec<Car>,
    pub constructor: Vec<Car>,
}

struct Reader<I> {
    input: BufReader<I>,
    line: String,
}

impl<I: Read> Reader<I> {
    fn new(input: I) -> Reader<I> {
        Reader {
            input: BufReader::new(input),
            line: String::new(),
        }
    }

    fn cur(&self) -> &str {
        self.line.trim_right()
    }

    fn next(&mut self) -> &str {
        self.line.clear();
        self.input.read_line(&mut self.line).unwrap();
        self.cur()
    }

    fn skip_comments(&mut self) {
        if self.line.starts_with("/*") {
            while !self.cur().ends_with("*/") {
                self.next();
            }
        }
        while self.next().is_empty() {}
    }
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

        while fi.next().is_empty() {}
        fi.skip_comments();
        fi.skip_comments();

        let num_main: u8 = fi.cur().parse().unwrap();
        let num_ruffa: u8 = fi.next().parse().unwrap();
        let num_const: u8 = fi.next().parse().unwrap();
        info!("Reading {} main vehicles, {} ruffas, and {} constructors",
            num_main, num_ruffa, num_const);

        while fi.next().is_empty() {}
        fi.skip_comments();
        fi.skip_comments();

        for _ in 0 .. num_main {
            reg.main.push(fi.cur().parse().unwrap());
            fi.next();
        }
        for _ in 0 .. num_ruffa {
            reg.ruffa.push(fi.next().parse().unwrap());
        }
        fi.next();
        for _ in 0 .. num_const {
            reg.constructor.push(fi.next().parse().unwrap());
        }

        reg
    }
}
