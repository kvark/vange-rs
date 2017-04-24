use std::fs::File;
use level;


#[derive(Deserialize)]
pub struct Car {
    pub id: String,
    pub slots: Vec<String>,
}

#[derive(Deserialize)]
pub struct Game {
    pub level: String,
}

#[derive(Deserialize)]
pub struct Window {
    pub title: String,
    pub size: [u32; 2],
}

#[derive(Deserialize)]
pub struct Settings {
    pub data_path: String,
    pub car: Car,
    pub game: Game,
    pub window: Window,
}

impl Settings {
    pub fn load(path: &str) -> Settings {
        use std::io::Read;
        use toml;

        let mut string = String::new();
        File::open(path).unwrap()
            .read_to_string(&mut string).unwrap();
        let set: Settings = toml::from_str(&string).unwrap();

        if !set.check_path("options.dat") {
            panic!("Can't find the resources of the original Vangers game at {}, {}",
               set.data_path, "please check your `config/settings.xml`");
        }

        set
    }

    pub fn open(&self, path: &str) -> File {
        let full = format!("{}/{}", self.data_path, path);
        File::open(full).unwrap()
    }

    pub fn check_path(&self, path: &str) -> bool {
        let full = format!("{}/{}", self.data_path, path);
        File::open(full).is_ok()
    }

    pub fn get_screen_aspect(&self) -> f32 {
        self.window.size[0] as f32 / self.window.size[1] as f32
    }

    pub fn get_object_palette_path(&self) -> String {
        format!("{}/resource/pal/objects.pal", self.data_path)
    }

    pub fn _get_vehicle_model_path(&self, name: &str) -> String {
        format!("{}/resource/m3d/mechous/{}.m3d", self.data_path, name)
    }

    pub fn get_level(&self) -> level::LevelConfig {
        use ini::Ini;
        let ini_path = format!("{}/thechain/{}/world.ini", self.data_path, self.game.level);
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
            path_vpr: format!("{}/thechain/{}/{}.vpr", self.data_path, self.game.level, biname),
            path_vmc: format!("{}/thechain/{}/{}.vmc", self.data_path, self.game.level, biname),
            path_palette: format!("{}/thechain/{}/{}", self.data_path, self.game.level, storage["Palette File"]),
            is_compressed: storage["Compressed Format Using"] != "0",
            name: self.game.level.clone(),
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
