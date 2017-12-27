use super::NUM_TERRAINS;

use ini::Ini;
use std::path::PathBuf;


pub struct Power(pub i32);
impl Power {
    pub fn as_value(&self) -> i32 {
        1 << self.0
    }
    pub fn as_power(&self) -> i32 {
        self.0
    }
}

#[derive(Clone, Copy)]
pub struct TerrainConfig {
    pub shadow_offset: u8,
    pub height_shift: u8,
    pub color_range: (u8, u8),
}

pub struct LevelConfig {
    //pub name: String,
    pub path_palette: PathBuf,
    pub path_vpr: PathBuf,
    pub path_vmc: PathBuf,
    pub is_compressed: bool,
    pub size: (Power, Power),
    pub geo: Power,
    pub section: Power,
    pub min_square: Power,
    pub terrains: [TerrainConfig; NUM_TERRAINS],
}

impl LevelConfig {
    pub fn load(ini_path: &PathBuf) -> Self {
        let ini = Ini::load_from_file(ini_path)
            .expect("Unable to read the level's INI description");
        let global = &ini["Global Parameters"];
        let storage = &ini["Storage"];
        let render = &ini["Rendering Parameters"];

        let mut terrains = [TerrainConfig {
            shadow_offset: 0,
            height_shift: 0,
            color_range: (0, 0),
        }; NUM_TERRAINS];
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
            t.color_range.0 = val.parse().unwrap();
        }
        for (t, val) in terrains
            .iter_mut()
            .zip(render["End Colors"].split_whitespace())
        {
            t.color_range.1 = val.parse().unwrap();
        }

        let file_path = ini_path.with_file_name(&storage["File Name"]);
        LevelConfig {
            path_vpr: file_path.with_extension("vpr"),
            path_vmc: file_path.with_extension("vmc"),
            path_palette: ini_path.with_file_name(&storage["Palette File"]),
            is_compressed: storage["Compressed Format Using"] != "0",
            //name: self.game.level.clone(),
            size: (
                Power(global["Map Power X"].parse().unwrap()),
                Power(global["Map Power Y"].parse().unwrap()),
            ),
            geo: Power(global["GeoNet Power"].parse().unwrap()),
            section: Power(global["Section Size Power"].parse().unwrap()),
            min_square: Power(global["Minimal Square Power"].parse().unwrap()),
            terrains: terrains,
        }
    }
}
