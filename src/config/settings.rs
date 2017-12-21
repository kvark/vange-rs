use ini::Ini;
use level;
use std::fs::File;
use std::path::PathBuf;

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

#[derive(Clone, Deserialize)]
pub struct DebugRender {
    pub max_vertices: usize,
    pub collision_shapes: bool,
    pub impulses: bool,
}

#[derive(Deserialize)]
pub struct Render {
    pub debug: DebugRender,
}

#[derive(Deserialize)]
pub struct Settings {
    pub data_path: PathBuf,
    pub car: Car,
    pub game: Game,
    pub window: Window,
    pub render: Render,
}

impl Settings {
    pub fn load(path: &str) -> Self {
        use std::io::Read;
        use toml;

        let mut string = String::new();
        File::open(path)
            .expect("Unable to open the settings file")
            .read_to_string(&mut string)
            .unwrap();
        let set: Settings = toml::from_str(&string).expect("Unable to parse settings TOML");

        if !set.check_path("options.dat") {
            panic!(
                "Can't find the resources of the original Vangers game at {:?}, {}",
                set.data_path, "please check your `config/settings.xml`"
            );
        }

        set
    }

    pub fn open_relative(
        &self,
        path: &str,
    ) -> File {
        File::open(self.data_path.join(path)).expect(&format!("Unable to open game file: {}", path))
    }

    pub fn check_path(
        &self,
        path: &str,
    ) -> bool {
        self.data_path.join(path).exists()
    }

    pub fn get_screen_aspect(&self) -> f32 {
        self.window.size[0] as f32 / self.window.size[1] as f32
    }

    pub fn open_palette(&self) -> File {
        let path = self.data_path
            .join("resource")
            .join("pal")
            .join("objects.pal");
        File::open(path).expect("Unable to open palette")
    }

    pub fn _open_vehicle_model(
        &self,
        name: &str,
    ) -> File {
        let path = self.data_path
            .join("resource")
            .join("m3d")
            .join("mechous")
            .join(name)
            .with_extension("m3d");
        File::open(path).expect(&format!("Unable to open vehicle {}", name))
    }

    pub fn get_level(&self) -> Option<level::LevelConfig> {
        if self.game.level.is_empty() {
            return None;
        }
        let level_path = self.data_path.join("thechain").join(&self.game.level);

        let ini = Ini::load_from_file(level_path.join("world.ini"))
            .expect("Unable to read the level's INI description");
        let global = &ini["Global Parameters"];
        let storage = &ini["Storage"];
        let render = &ini["Rendering Parameters"];

        let mut terrains = [level::TerrainConfig {
            shadow_offset: 0,
            height_shift: 0,
            color_range: (0, 0),
        }; level::NUM_TERRAINS];
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

        let biname = &storage["File Name"];
        Some(level::LevelConfig {
            path_vpr: level_path.join(biname).with_extension("vpr"),
            path_vmc: level_path.join(biname).with_extension("vmc"),
            path_palette: level_path.join(&storage["Palette File"]),
            is_compressed: storage["Compressed Format Using"] != "0",
            name: self.game.level.clone(),
            size: (
                level::Power(global["Map Power X"].parse().unwrap()),
                level::Power(global["Map Power Y"].parse().unwrap()),
            ),
            geo: level::Power(global["GeoNet Power"].parse().unwrap()),
            section: level::Power(global["Section Size Power"].parse().unwrap()),
            min_square: level::Power(global["Minimal Square Power"].parse().unwrap()),
            terrains: terrains,
        })
    }
}
