use crate::render::object::BodyColor;

use std::fs::File;
use std::path::PathBuf;

#[derive(Deserialize)]
pub struct Car {
    pub id: String,
    pub color: BodyColor,
    pub slots: Vec<String>,
    pub pos: Option<(i32, i32)>,
}

#[derive(Copy, Clone, Deserialize)]
pub enum Projection {
    Flat,
    Perspective,
}

#[derive(Copy, Clone, Deserialize)]
pub struct Camera {
    pub angle: u8,
    pub height: f32,
    pub offset: f32,
    pub speed: f32,
    pub depth_range: (f32, f32),
    pub projection: Projection,
}

#[derive(Copy, Clone, Deserialize)]
pub enum SpawnAt {
    Player,
    Random,
}

#[derive(Copy, Clone, Deserialize)]
pub struct Other {
    pub count: usize,
    pub spawn_at: SpawnAt,
}

#[derive(Copy, Clone, Deserialize)]
pub struct Physics {
    pub max_quant: f32,
    pub shape_sampling: u8,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Geometry {
    pub height: u32,
    pub delta_mask: u32,
    pub delta_power: u8,
    pub delta_const: u8,
}

impl Default for Geometry {
    fn default() -> Self {
        // Note: these values match the original game logic
        Self {
            height: 0x100,
            delta_mask: 0xFFFF,
            delta_power: 3,
            delta_const: 1,
        }
    }
}

#[derive(Deserialize)]
pub struct Game {
    pub level: String,
    pub cycle: String,
    pub geometry: Geometry,
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

#[derive(Copy, Clone, Deserialize)]
pub enum Backend {
    Auto,
    Metal,
    Vulkan,
    DX12,
    DX11,
    GL,
}

impl Backend {
    pub fn to_wgpu(&self) -> wgpu::Backends {
        match *self {
            Backend::Auto => wgpu::Backends::PRIMARY,
            Backend::Metal => wgpu::Backends::METAL,
            Backend::Vulkan => wgpu::Backends::VULKAN,
            Backend::DX12 => wgpu::Backends::DX12,
            Backend::DX11 => wgpu::Backends::DX11,
            Backend::GL => wgpu::Backends::GL,
        }
    }
}

#[derive(Copy, Clone, Default, Deserialize)]
pub struct DebugRender {
    pub max_vertices: usize,
    pub collision_shapes: bool,
    pub collision_map: bool,
    pub impulses: bool,
}

#[derive(Copy, Clone, Deserialize)]
pub enum ShadowTerrain {
    RayTraced,
    RayVoxelTraced {
        max_outer_steps: u32,
        max_inner_steps: u32,
    },
}

#[derive(Copy, Clone, Deserialize)]
pub struct Shadow {
    pub size: u32,
    pub terrain: ShadowTerrain,
}

#[derive(Copy, Clone, Deserialize)]
pub struct Light {
    pub pos: [f32; 4],
    pub color: [f32; 3],
    pub shadow: Shadow,
}

#[derive(Copy, Clone, Deserialize)]
pub enum Terrain {
    RayTraced,
    RayVoxelTraced {
        voxel_size: [u32; 3],
        max_outer_steps: u32,
        max_inner_steps: u32,
        max_update_texels: usize,
    },
    Sliced,
    Painted,
    Scattered {
        density: [u32; 3],
    },
}

#[derive(Copy, Clone, Deserialize)]
pub struct Water {}

#[derive(Copy, Clone, Deserialize)]
pub struct Fog {
    pub color: [f32; 3],
    pub depth: f32,
}

#[derive(Clone, Deserialize)]
pub struct Render {
    #[serde(default)]
    pub wgpu_trace_path: String,
    pub allow_tearing: bool,
    pub light: Light,
    pub terrain: Terrain,
    pub water: Water,
    pub fog: Fog,
    #[serde(default)]
    pub debug: DebugRender,
}

impl Render {
    pub fn get_device_limits(&self, adapter_limits: &wgpu::Limits, slices: u32) -> wgpu::Limits {
        let (max_width, max_height) = (2048usize, 16384usize);
        match self.terrain {
            Terrain::RayTraced { .. } | Terrain::Sliced { .. } | Terrain::Painted { .. } => {
                wgpu::Limits {
                    max_texture_dimension_2d: adapter_limits.max_texture_dimension_2d,
                    ..wgpu::Limits::downlevel_webgl2_defaults()
                }
            }
            Terrain::RayVoxelTraced { voxel_size, .. } => {
                let voxel_points = voxel_size[0] * voxel_size[1] * voxel_size[2];
                let max_voxels = max_width * max_height * slices as usize / voxel_points as usize;
                // Note: 1/7 is roughly the sum size of all the mips
                // Division by 8 is because we have 8 bits per byte.
                // The extra space is for rounding and such.
                let voxel_storage_size = (max_voxels * 8 / 7) / 8 + 4096;
                info!(
                    "Estimating {} MB for voxel storage",
                    voxel_storage_size >> 20
                );
                wgpu::Limits {
                    max_storage_buffer_binding_size: voxel_storage_size as u32,
                    max_texture_dimension_2d: max_width.max(max_height) as u32,
                    ..wgpu::Limits::downlevel_defaults()
                }
            }
            Terrain::Scattered { .. } => wgpu::Limits::default(),
        }
    }
}

#[derive(Copy, Clone, Default, Deserialize)]
pub struct Ui {
    pub enabled: bool,
    pub frame_history: usize,
}

#[derive(Deserialize)]
pub struct Settings {
    pub data_path: PathBuf,
    pub car: Car,
    pub game: Game,
    pub window: Window,
    pub backend: Backend,
    pub render: Render,
    pub ui: Ui,
}

impl Settings {
    pub fn load(path: &str) -> Self {
        use std::io::Read;

        const TEMPLATE: &str = "config/settings.template.ron";
        const PATH: &str = "config/settings.ron";
        let mut string = String::new();
        File::open(path)
            .unwrap_or_else(|e| panic!("Unable to open the settings file: {:?}.\nPlease copy '{}' to '{}' and adjust 'data_path'",
                e, TEMPLATE, PATH))
            .read_to_string(&mut string)
            .unwrap();
        let set: Settings = match ron::de::from_str(&string) {
            Ok(set) => set,
            Err(e) => panic!(
                "Unable to parse settings RON: {:?}.\nPlease check if `{}` has changed and your local config needs to be adjusted.",
                e,
                TEMPLATE,
            ),
        };

        if !set.check_path("options.dat") {
            panic!(
                "Can't find the resources of the original Vangers game at {:?}, please check your `{}`",
                set.data_path, PATH,
            );
        }

        set
    }

    pub fn open_relative(&self, path: &str) -> File {
        File::open(self.data_path.join(path))
            .unwrap_or_else(|_| panic!("Unable to open game file: {}", path))
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
        File::open(path).unwrap_or_else(|_| panic!("Unable to open vehicle {}", name))
    }
}
