use ini::Ini;
use std::ops::Range;
use std::path::{Path, PathBuf};

#[derive(Copy, Clone)]
pub struct Power(pub i32);
impl Power {
    pub fn as_value(&self) -> i32 {
        1 << self.0
    }
    pub fn as_power(&self) -> i32 {
        self.0
    }
}

#[derive(Clone)]
pub struct TerrainConfig {
    pub shadow_offset: u8,
    pub height_shift: u8,
    pub colors: Range<u8>, // note: actually, this is inclusive range
}

pub struct LevelConfig {
    //pub name: String,
    pub path_palette: PathBuf,
    pub path_data: PathBuf,
    pub is_compressed: bool,
    pub size: (Power, Power),
    pub geo: Power,
    pub section: Power,
    pub min_square: Power,
    pub terrains: Box<[TerrainConfig]>,
}

impl LevelConfig {
    pub fn load(ini_path: &Path) -> Self {
        let ini = Ini::load_from_file(ini_path).unwrap_or_else(|error| {
            panic!("Unable to read the level's INI description: {:?} {:?}", ini_path, error)
        });
        let global = &ini["Global Parameters"];
        let storage = &ini["Storage"];
        let render = &ini["Rendering Parameters"];

        let terra_count = render
            .get("Terrain Max")
            .map_or(8, |value| value.parse::<usize>().unwrap());
        let mut terrains = (0..terra_count)
            .map(|_| TerrainConfig {
                shadow_offset: 0,
                height_shift: 0,
                colors: 0..0,
            })
            .collect::<Box<[_]>>();

        for (t, val) in terrains
            .iter_mut()
            .zip(render["Shadow Offsets"].split_whitespace())
        {
            t.shadow_offset = val.parse().unwrap();
        }
        for (t, val) in terrains
            .iter_mut()
            .zip(render["Height Shifts"].split_whitespace())
        {
            t.height_shift = val.parse().unwrap();
        }
        for (t, val) in terrains
            .iter_mut()
            .zip(render["Begin Colors"].split_whitespace())
        {
            t.colors.start = val.parse().unwrap();
        }
        for (t, val) in terrains
            .iter_mut()
            .zip(render["End Colors"].split_whitespace())
        {
            t.colors.end = val.parse().unwrap();
        }

        let path_data = ini_path.with_file_name(&storage["File Name"]);
        LevelConfig {
            path_data,
            path_palette: ini_path.with_file_name(&storage["Palette File"]),
            is_compressed: &storage["Compressed Format Using"] != "0",
            //name: self.game.level.clone(),
            size: (
                Power(global["Map Power X"].parse().unwrap()),
                Power(global["Map Power Y"].parse().unwrap()),
            ),
            geo: Power(global["GeoNet Power"].parse().unwrap()),
            section: Power(global["Section Size Power"].parse().unwrap()),
            min_square: Power(global["Minimal Square Power"].parse().unwrap()),
            terrains,
        }
    }
}
