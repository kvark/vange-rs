use level;

#[derive(RustcDecodable)]
pub struct WindowSettings {
    pub title: String,
    pub size: [u32; 2],
}

#[derive(RustcDecodable)]
pub struct Settings {
    pub game_path: String,
    pub level: String,
    pub window: WindowSettings,
}

impl Settings {
    pub fn load(path: &str) -> Settings {
        use std::io::{BufReader, Read};
        use std::fs::File;
        use toml;

        let mut string = String::new();
        BufReader::new(File::open(path).unwrap())
            .read_to_string(&mut string).unwrap();
        toml::decode_str(&string).unwrap()
    }

    pub fn get_level(&self) -> level::LevelConfig {
        use ini::Ini;
        let ini_path = format!("{}/thechain/{}/world.ini", self.game_path, self.level);
        let ini = Ini::load_from_file(&ini_path).unwrap();
        let global = &ini["Global Parameters"];
        let storage = &ini["Storage"];
        let render = &ini["Rendering Parameters"];
        let mut terrains = [level::TerrainConfig {
                shadow_offset: 0,
                height_shift: 0,
                color_range: (0, 0),
            }; level::NUM_TERRAINS];
        for (t, val) in terrains.iter_mut().zip(render["Shadow Offsets"].split_whitespace()) {
            t.shadow_offset = val.parse().unwrap();
        }
        for (t, val) in terrains.iter_mut().zip(render["Height Shifts"].split_whitespace()) {
            t.height_shift = val.parse().unwrap();
        }
        for (t, val) in terrains.iter_mut().zip(render["Begin Colors"].split_whitespace()) {
            t.color_range.0 = val.parse().unwrap();
        }
        for (t, val) in terrains.iter_mut().zip(render["End Colors"].split_whitespace()) {
            t.color_range.1 = val.parse().unwrap();
        }
        let biname = &storage["File Name"];
        level::LevelConfig {
            path_vpr: format!("{}/thechain/{}/{}.vpr", self.game_path, self.level, biname),
            path_vmc: format!("{}/thechain/{}/{}.vmc", self.game_path, self.level, biname),
            path_palette: format!("{}/thechain/{}/{}", self.game_path, self.level, storage["Palette File"]),
            is_compressed: storage["Compressed Format Using"] != "0",
            name: self.level.clone(),
            size: (
                level::Power(global["Map Power X"].parse().unwrap()),
                level::Power(global["Map Power Y"].parse().unwrap())
            ),
            geo: level::Power(global["GeoNet Power"].parse().unwrap()),
            section: level::Power(global["Section Size Power"].parse().unwrap()),
            min_square: level::Power(global["Minimal Square Power"].parse().unwrap()),
            terrains: terrains,
        }

    }
}
