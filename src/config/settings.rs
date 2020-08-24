use crate::render::object::BodyColor;

use std::fs::File;
use std::path::PathBuf;

#[derive(Deserialize)]
pub struct Car {
    pub id: String,
    pub color: BodyColor,
    pub slots: Vec<String>,
}

#[derive(Deserialize)]
pub enum View {
    Flat,
    Perspective,
}

#[derive(Deserialize)]
pub struct Camera {
    pub angle: u8,
    pub height: f32,
    pub target_height_offset: f32,
    pub speed: f32,
    pub depth_range: (f32, f32),
}

#[derive(Deserialize)]
pub enum SpawnAt {
    Player,
    Random,
}

#[derive(Deserialize)]
pub struct Other {
    pub count: usize,
    pub spawn_at: SpawnAt,
}

#[derive(Deserialize)]
pub struct GpuCollision {
    pub max_objects: usize,
    pub max_polygons_total: usize,
    pub max_raster_size: (u32, u32),
}

#[derive(Deserialize)]
pub struct Physics {
    pub max_quant: f32,
    pub shape_sampling: u8,
    pub gpu_collision: Option<GpuCollision>,
}

#[derive(Deserialize)]
pub struct Game {
    pub level: String,
    pub cycle: String,
    pub view: View,
    pub camera: Camera,
    pub other: Other,
    pub physics: Physics,
}

#[derive(Deserialize)]
pub struct Window {
    pub title: String,
    pub size: [u32; 2],
    pub reload_on_focus: bool,
}

#[derive(Deserialize)]
pub enum Backend {
    Auto,
    Metal,
    Vulkan,
    DX12,
    DX11,
}

impl Backend {
    pub fn to_wgpu(&self) -> wgpu::BackendBit {
        match *self {
            Backend::Auto => wgpu::BackendBit::PRIMARY,
            Backend::Metal => wgpu::BackendBit::METAL,
            Backend::Vulkan => wgpu::BackendBit::VULKAN,
            Backend::DX12 => wgpu::BackendBit::DX12,
            Backend::DX11 => wgpu::BackendBit::DX11,
        }
    }
}

#[derive(Clone, Deserialize)]
pub struct DebugRender {
    pub max_vertices: usize,
    pub collision_shapes: bool,
    pub collision_map: bool,
    pub impulses: bool,
}

#[derive(Clone, Deserialize)]
pub struct Light {
    pub pos: [f32; 4],
    pub color: [f32; 4],
    pub shadow_size: u32,
}

#[derive(Clone, Deserialize)]
pub enum Terrain {
    RayTraced,
    RayMipTraced {
        mip_count: u32,
        max_jumps: u32,
        max_steps: u32,
        debug: bool,
    },
    Tessellated {
        screen_space: bool,
    },
    Sliced,
    Painted,
    Scattered {
        density: [u32; 3],
    },
}

#[derive(Deserialize)]
pub struct Render {
    pub light: Light,
    pub terrain: Terrain,
    pub debug: DebugRender,
}

#[derive(Deserialize)]
pub struct Settings {
    pub data_path: PathBuf,
    pub car: Car,
    pub game: Game,
    pub window: Window,
    pub backend: Backend,
    pub render: Render,
}

impl Settings {
    pub fn load(path: &str) -> Self {
        use std::io::Read;

        let mut string = String::new();
        File::open(path)
            .expect("Unable to open the settings file")
            .read_to_string(&mut string)
            .unwrap();
        let set: Settings = match ron::de::from_str(&string) {
            Ok(set) => set,
            Err(e) => panic!("Unable to parse settings RON.\n\t{}\n\tError: {:?}",
                "Please check if `config/settings.template.ron` has changed and your local config needs to be adjusted.",
                e,
            ),
        };

        if !set.check_path("options.dat") {
            panic!(
                "Can't find the resources of the original Vangers game at {:?}, {}",
                set.data_path, "please check your `config/settings.ron`"
            );
        }

        set
    }

    pub fn open_relative(&self, path: &str) -> File {
        File::open(self.data_path.join(path)).expect(&format!("Unable to open game file: {}", path))
    }

    pub fn check_path(&self, path: &str) -> bool {
        self.data_path.join(path).exists()
    }

    pub fn open_palette(&self) -> File {
        let path = self
            .data_path
            .join("resource")
            .join("pal")
            .join("objects.pal");
        File::open(path).expect("Unable to open palette")
    }

    pub fn _open_vehicle_model(&self, name: &str) -> File {
        let path = self
            .data_path
            .join("resource")
            .join("m3d")
            .join("mechous")
            .join(name)
            .with_extension("m3d");
        File::open(path).expect(&format!("Unable to open vehicle {}", name))
    }
}
