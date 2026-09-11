use crate::config::text::Reader;

use serde_scan;

use std::fs::File;
use std::io::Read;

#[derive(Debug, Deserialize)]
pub struct Cycle {
    pub name: String,
    pub cirt_max: usize,
    pub radiance_time: usize,
    pub price: usize,
    pub palette_path: String,
}

#[derive(Debug)]
pub struct Bunch {
    pub escave: String,
    pub bios: String,
    pub cycles: Vec<Cycle>,
}

pub fn load(file: File) -> Vec<Bunch> {
    load_reader(file)
}

pub fn load_reader<R: Read>(reader: R) -> Vec<Bunch> {
    let mut bunches = Vec::new();
    let mut fi = Reader::new(reader);
    fi.advance();
    assert_eq!(fi.cur(), "uniVang-ParametersFile_Ver_1");

    while fi.advance() {
        let (escave, bios, count): (String, String, usize) = fi.scan();
        let mut cycles = Vec::with_capacity(count);
        info!("Escave {} has {} cycles", escave, count);
        for _ in 0..count {
            fi.advance();
            let cycle = {
                let mut elems = fi.cur().split('"');
                assert!(elems.next().unwrap().is_empty());
                let name = elems.next().unwrap().to_string();
                let leftover = elems.next().unwrap();
                let (cirt_max, radiance_time, price, palette_path) =
                    serde_scan::from_str(leftover).unwrap();
                Cycle {
                    name,
                    cirt_max,
                    radiance_time,
                    price,
                    palette_path,
                }
            };
            fi.advance();
            cycles.push(cycle);
        }
        bunches.push(Bunch {
            escave,
            bios,
            cycles,
        });
    }
    bunches
}
