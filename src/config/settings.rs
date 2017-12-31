use std::fs::File;
use std::path::PathBuf;

#[derive(Deserialize)]
pub struct Car {
    pub id: String,
    pub slots: Vec<String>,
}

#[derive(Deserialize)]
pub enum View {
    Flat,
    Perspective,
}

#[derive(Deserialize)]
pub struct Game {
    pub level: String,
    pub cycle: String,
    pub view: View,
    pub other_vangers: usize,
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
}
