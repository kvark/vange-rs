use crate::render::object::BodyColor;

use std::fs::File;
use std::path::{Path, PathBuf};

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
    /// World units ahead of the vehicle the chase camera looks at.
    /// Zero keeps the authored pitch (`angle`); a positive value is the
    /// GTA-style look-ahead that holds the car in the lower part of the
    /// frame instead of letting it run toward the horizon.
    #[serde(default)]
    pub look_ahead: f32,
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

#[derive(Copy, Clone, Debug, Deserialize)]
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
    GL,
}

impl Backend {
    pub fn to_wgpu(&self) -> wgpu::Backends {
        match *self {
            Backend::Auto => wgpu::Backends::PRIMARY,
            Backend::Metal => wgpu::Backends::METAL,
            Backend::Vulkan => wgpu::Backends::VULKAN,
            Backend::DX12 => wgpu::Backends::DX12,
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
    /// Triangulated irregular network: a real triangle mesh fitted to the
    /// height map by greedy Delaunay insertion. See `level::tin`.
    Mesh {
        /// Fit quality in `0..=1`. Higher means more triangles and a closer
        /// fit; see `level::tin::Config`.
        quality: f32,
    },
}

#[derive(Copy, Clone, Deserialize)]
pub struct Water {}

#[derive(Copy, Clone, Deserialize)]
pub struct Fog {
    pub color: [f32; 3],
    pub depth: f32,
}

impl Default for Render {
    fn default() -> Self {
        Render {
            wgpu_trace_path: String::new(),
            allow_tearing: false,
            light: Light {
                pos: [1.0, 2.0, 4.0, 0.0],
                color: [1.0, 1.0, 1.0],
                shadow: Shadow {
                    size: 0,
                    terrain: ShadowTerrain::RayTraced,
                },
            },
            terrain: Terrain::RayTraced,
            ray_steps: 128,
            water: Water {},
            fog: Fog {
                color: [0.1, 0.2, 0.3],
                depth: 50.0,
            },
            debug: DebugRender::default(),
        }
    }
}

#[derive(Clone, Deserialize)]
pub struct Render {
    #[serde(default)]
    pub wgpu_trace_path: String,
    #[serde(default)]
    pub allow_tearing: bool,
    pub light: Light,
    pub terrain: Terrain,
    /// Forward samples used by the plain height-field marcher. This stays
    /// outside `Terrain::RayTraced` so existing RON enum syntax remains valid.
    #[serde(default = "default_ray_steps")]
    pub ray_steps: u32,
    pub water: Water,
    pub fog: Fog,
    #[serde(default)]
    pub debug: DebugRender,
}

fn default_ray_steps() -> u32 {
    128
}

impl Render {
    pub fn get_device_limits(&self, adapter_limits: &wgpu::Limits, slices: u32) -> wgpu::Limits {
        let (max_width, max_height) = (2048usize, 16384usize);
        match self.terrain {
            // `Mesh` only needs plain vertex/index buffers on top of the
            // terrain texture, so it runs anywhere ray tracing does.
            Terrain::RayTraced | Terrain::Sliced | Terrain::Painted | Terrain::Mesh { .. } => {
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
                    max_storage_buffer_binding_size: voxel_storage_size as u64,
                    max_texture_dimension_2d: max_width.max(max_height) as u32,
                    ..wgpu::Limits::downlevel_defaults()
                }
            }
            // Full defaults, since the scatter pass wants compute and a
            // read-write storage buffer - but the terrain texture is as tall
            // as it is for every other mode, and `Limits::default` caps 2D
            // textures at 8192 against the 16384 the stock levels need.
            Terrain::Scattered { .. } => wgpu::Limits {
                max_texture_dimension_2d: adapter_limits.max_texture_dimension_2d,
                ..wgpu::Limits::default()
            },
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
        Self::load_from(path).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Like `load`, but returns `None` instead of panicking when the
    /// settings file is missing or the game data path is invalid.
    pub fn try_load(path: &str) -> Option<Self> {
        Self::load_from(path).ok()
    }

    fn load_from(path: &str) -> Result<Self, String> {
        const TEMPLATE: &str = "config/settings.template.ron";
        let string = read_settings_ron(path, TEMPLATE)?;
        let mut set: Settings = ron::de::from_str(&string).map_err(|e| {
            format!(
                "Unable to parse settings RON: {e:?}.\nPlease check if `{TEMPLATE}` has changed and your local config needs to be adjusted."
            )
        })?;
        set.data_path = resolve_data_path(&set.data_path)?;
        Ok(set)
    }

    pub fn open_relative(&self, path: &str) -> File {
        File::open(self.data_path.join(path))
            .unwrap_or_else(|_| panic!("Unable to open game file: {}", path))
    }

    pub fn check_path(&self, path: &str) -> bool {
        self.data_path.join(path).exists()
    }

    pub fn open_palette(&self) -> File {
        let objects = self
            .data_path
            .join("resource")
            .join("pal")
            .join("objects.pal");
        if let Ok(file) = File::open(&objects) {
            return file;
        }
        // The open-source Vangers tree ships Fostral's world palette.
        let fostral = self
            .data_path
            .join("thechain")
            .join("fostral")
            .join("harmony.pal");
        File::open(&fostral)
            .unwrap_or_else(|_| panic!("Unable to open palette at {:?} or {:?}", objects, fostral))
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

const TEMPLATE: &str = "config/settings.template.ron";
const LOCAL: &str = "config/settings.ron";

/// Markers that this directory is Vangers game data. `options.dat` is
/// the purchased install; the open-source tree dropped it for
/// `settings.toml` and publishes Fostral under `thechain/`.
const DATA_MARKERS: &[&str] = &[
    "options.dat",
    "wrlds.dat",
    "game.lst",
    "thechain/fostral/world.ini",
];

const DEFAULT_DATA_CANDIDATES: &[&str] = &["../Vangers/data", "../Vangers"];

fn read_settings_ron(path: &str, template: &str) -> Result<String, String> {
    use std::io::Read as _;
    if let Ok(mut file) = File::open(path) {
        let mut string = String::new();
        file.read_to_string(&mut string)
            .map_err(|e| format!("Unable to read settings file {path}: {e}"))?;
        return Ok(string);
    }
    let mut file = File::open(template).map_err(|e| {
        format!(
            "Unable to open the settings file {path:?} ({e}). Copy '{template}' to '{LOCAL}' and adjust 'data_path'."
        )
    })?;
    let mut string = String::new();
    file.read_to_string(&mut string)
        .map_err(|e| format!("Unable to read {template}: {e}"))?;
    log::info!("No {path}; using {template}");
    Ok(string)
}

pub(crate) fn looks_like_game_data(dir: &Path) -> bool {
    DATA_MARKERS.iter().any(|marker| dir.join(marker).exists())
}

fn normalize_data_dir(dir: PathBuf) -> PathBuf {
    let nested = dir.join("data");
    if looks_like_game_data(&nested) && !looks_like_game_data(&dir) {
        nested
    } else {
        dir
    }
}

fn resolve_data_path(configured: &Path) -> Result<PathBuf, String> {
    let mut tried = Vec::new();
    let mut consider = |p: PathBuf| -> Option<PathBuf> {
        if p.as_os_str().is_empty() {
            return None;
        }
        let p = normalize_data_dir(p);
        if looks_like_game_data(&p) {
            Some(p)
        } else {
            tried.push(p);
            None
        }
    };

    if let Some(found) = consider(configured.to_path_buf()) {
        return Ok(found);
    }
    if let Ok(env_path) = std::env::var("VANGERS_DATA")
        && let Some(found) = consider(PathBuf::from(env_path))
    {
        return Ok(found);
    }
    for candidate in DEFAULT_DATA_CANDIDATES {
        if let Some(found) = consider(PathBuf::from(candidate)) {
            return Ok(found);
        }
    }
    Err(format!(
        "Can't find Vangers game data at {:?}. Tried: {:?}. Set `data_path` in `{LOCAL}` (from `{TEMPLATE}`) to the original game or a KranX/Vangers checkout.",
        configured, tried
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("vange-rs-settings-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"").unwrap();
    }

    #[test]
    fn purchased_install_is_detected_by_options_dat() {
        let dir = scratch("options");
        touch(&dir.join("options.dat"));
        assert!(looks_like_game_data(&dir));
    }

    #[test]
    fn fostral_in_the_vangers_tree_is_enough() {
        let dir = scratch("fostral");
        touch(&dir.join("thechain/fostral/world.ini"));
        assert!(looks_like_game_data(&dir));
    }

    #[test]
    fn vangers_repo_root_resolves_to_data() {
        let root = scratch("repo");
        touch(&root.join("data/thechain/fostral/world.ini"));
        assert_eq!(normalize_data_dir(root.clone()), root.join("data"));
    }
}
