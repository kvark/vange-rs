//! Web entry point for vange-rs level viewer with test level.
//! Compiled with `cargo build --target wasm32-unknown-unknown --features web --bin web`
//!
//! If the `VANGERS_SERVER_WS` environment variable is set at compile time,
//! the viewer will attempt to connect to that WebSocket address on startup.
//! If the connection fails, it continues as a standalone viewer.

use wasm_bindgen::prelude::*;

use vangers::{
    config::{self, settings},
    data, level, model, physics,
    render::{
        self, Batcher, DEPTH_FORMAT, DirtyRect, GraphicsContext, Rect, Render, ScreenTargets,
    },
    space,
    vfs::Vfs,
};

/// Default level to try loading from the release if neither the URL
/// hash nor the level picker have selected one. If the release asset
/// is missing (404) we fall back to the procedural test level.
const DEFAULT_LEVEL: &str = "fostral";

/// How far above the ground the chase camera is held. Small enough that
/// the framing barely moves on flat terrain, large enough that the near
/// plane does not clip into the surface on a slope.
const CAMERA_CLEARANCE: f32 = 4.0;

/// INI path inside the per-level zip. Each `<id>.zip` stores the level
/// files at the archive root (no `<id>/` prefix), so the INI key is
/// just `"world.ini"`.
fn level_ini_path(_level_id: &str) -> String {
    "world.ini".to_string()
}

/// JS bridge for the loading-screen UI. The HTML defines these on
/// `window`; they update a progress bar and status text. The `catch`
/// attribute makes every call a no-op when the JS function is missing,
/// so the WASM binary works unchanged on pages without the overlay.
///
/// The sequence is:
///   vangePhase("Connecting to GPU…")     ← opaque step, spinner
///   vangePhase("Downloading fostral.zip") ← indeterminate
///   vangeProgress("fostral.zip", 1234, 5678)  ← byte progress
///   vangePhase("Mounting archives…")     ← spinner
///   vangePhase("Building renderer…")     ← spinner
///   vangeProgressDone()                   ← hide overlay
/// or, on failure:
///   vangeProgressError("…")               ← red banner, auto-hide
#[wasm_bindgen]
extern "C" {
    /// Set the top-line status text and switch the bar to indeterminate.
    #[wasm_bindgen(js_namespace = window, js_name = vangePhase, catch)]
    fn js_phase(label: &str) -> Result<(), JsValue>;

    /// Update the progress bar with byte counts. `total < 0` means
    /// Content-Length was missing; the bar stays indeterminate.
    #[wasm_bindgen(js_namespace = window, js_name = vangeProgress, catch)]
    fn js_progress(label: &str, loaded: f64, total: f64) -> Result<(), JsValue>;

    #[wasm_bindgen(js_namespace = window, js_name = vangeProgressDone, catch)]
    fn js_progress_done() -> Result<(), JsValue>;

    #[wasm_bindgen(js_namespace = window, js_name = vangeProgressError, catch)]
    fn js_progress_error(message: &str) -> Result<(), JsValue>;

    #[wasm_bindgen(js_namespace = window, js_name = vangeSelectedLevel, catch)]
    fn js_selected_level() -> Result<JsValue, JsValue>;
}

/// Khox is the smallest stock level (2048 × 8192) and the only one
/// that fits in an 8192-px texture. Used as the fallback when the
/// adapter can't allocate a 16384-px texture for the larger levels.
const SMALL_LEVEL: &str = "khox";

/// Largest map dimension the stock big levels expect (Fostral, Glorx,
/// Necross, etc. are 2048 × 16384). If the adapter can't go this high
/// we have to fall back to a smaller level.
const LARGE_LEVEL_TEXTURE_DIM: u32 = 16384;

/// If the adapter can't allocate a [`LARGE_LEVEL_TEXTURE_DIM`] texture,
/// override the selection to [`SMALL_LEVEL`] and tell the user why.
/// Returns the (possibly substituted) level id.
fn pick_level_for_adapter(requested: String, max_texture_dim: u32) -> String {
    if max_texture_dim >= LARGE_LEVEL_TEXTURE_DIM || requested == SMALL_LEVEL {
        return requested;
    }
    let msg = format!(
        "GPU caps texture dimension at {} px; the big levels need {} px. Loading {} instead.",
        max_texture_dim, LARGE_LEVEL_TEXTURE_DIM, SMALL_LEVEL
    );
    log::warn!(
        "Substituting '{}' for requested level '{}' ({})",
        SMALL_LEVEL,
        requested,
        msg
    );
    let _ = js_progress_error(&msg);
    SMALL_LEVEL.to_string()
}

/// Which terrain renderer the page asked for.
///
/// Selected by URL, so the site can offer the same scene under three
/// pipelines and you compare them by switching the address:
///
///   * `/`       the full site, on the triangle mesh
///   * `/mesh`   triangle mesh, bare page
///   * `/voxel`  voxel ray tracing, needs compute, so WebGPU only
///   * `/ray`    height-field ray tracing
///
/// `#terrain=mesh|voxel|ray` overrides the path. The mesh is the default
/// because it is the only one that does not care which backend it got:
/// fitted on the CPU and drawn with the plain raster pipeline, it runs
/// the same on WebGL2 as on WebGPU.
#[derive(Clone, Copy, PartialEq)]
enum TerrainChoice {
    Mesh,
    Voxel,
    Ray,
}

fn terrain_choice() -> TerrainChoice {
    let Some(window) = web_sys::window() else {
        return TerrainChoice::Mesh;
    };
    if let Ok(hash) = window.location().hash() {
        for pair in hash.trim_start_matches('#').split('&') {
            match pair {
                "terrain=mesh" => return TerrainChoice::Mesh,
                "terrain=voxel" => return TerrainChoice::Voxel,
                "terrain=ray" => return TerrainChoice::Ray,
                _ => {}
            }
        }
    }
    let path = window.location().pathname().unwrap_or_default();
    match path.trim_end_matches('/').rsplit('/').next() {
        Some("voxel") => TerrainChoice::Voxel,
        Some("ray") => TerrainChoice::Ray,
        // `/mesh` and the site root both land here.
        _ => TerrainChoice::Mesh,
    }
}

/// Read the selected level id from JS (set by the level picker UI),
/// falling back to the URL fragment `#level=<id>`, then to
/// [`DEFAULT_LEVEL`].
fn selected_level_id() -> String {
    if let Ok(val) = js_selected_level()
        && let Some(s) = val.as_string()
        && !s.is_empty()
    {
        return s;
    }
    if let Some(window) = web_sys::window()
        && let Ok(hash) = window.location().hash()
    {
        for pair in hash.trim_start_matches('#').split('&') {
            if let Some(rest) = pair.strip_prefix("level=")
                && !rest.is_empty()
            {
                return rest.to_string();
            }
        }
    }
    DEFAULT_LEVEL.to_string()
}

/// Fetch `common.zip` and `<level_id>.zip` from the release and mount
/// both into a VFS, reporting download progress to the JS UI. Returns
/// `None` on any error; the caller falls back to a procedural test level.
async fn fetch_release_level(level_id: &str) -> Option<(Vfs, String)> {
    let mut vfs = Vfs::new();

    let mut report = |label: &str, loaded: u64, total: Option<u64>| {
        let total_f = total.map_or(-1.0, |v| v as f64);
        let _ = js_progress(label, loaded as f64, total_f);
    };

    // common.zip holds cross-level assets. A level-only run is fine
    // without it, so we log+continue on failure.
    let _ = js_phase(&format!("Downloading {}", data::COMMON_ARCHIVE));
    if let Err(e) = data::fetch_and_mount(&mut vfs, data::COMMON_ARCHIVE, &mut report).await {
        log::warn!("Couldn't fetch {}: {}", data::COMMON_ARCHIVE, e);
    }

    let archive = data::level_archive_name(level_id);
    let _ = js_phase(&format!("Downloading {}", archive));
    if let Err(e) = data::fetch_and_mount(&mut vfs, &archive, &mut report).await {
        log::warn!(
            "Couldn't fetch {}: {}. Falling back to test level.",
            archive,
            e
        );
        let _ = js_progress_error(&format!("{}: {}", archive, e));
        return None;
    }

    let ini_path = level_ini_path(level_id);
    if !vfs.contains(&ini_path) {
        log::warn!(
            "{} did not contain {}. Falling back to test level.",
            archive,
            ini_path
        );
        let _ = js_progress_error(&format!("{} missing {}", archive, ini_path));
        return None;
    }

    log::info!(
        "Loaded release level '{}' from VFS ({} entries)",
        level_id,
        vfs.len()
    );
    Some((vfs, ini_path))
}

use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::{self, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::Window,
};

/// Compile-time server address for multiplayer. Set via:
///   VANGERS_SERVER_WS=ws://host:port cargo build ...
const SERVER_WS: Option<&str> = option_env!("VANGERS_SERVER_WS");

/// How many collision-shape samples per face to tessellate when
/// uploading a vehicle's collision mesh. The native build takes this
/// from `settings.ron`; on web we hardcode a balanced value.
const SHAPE_SAMPLING: u8 = 3;

/// Default body color for the spawned player vehicle.
const PLAYER_COLOR: render::object::BodyColor = render::object::BodyColor::Green;

/// Minimal controller state the web build feeds into the physics
/// integrator. Mirrors the private struct of the same name in
/// `bin/road/game.rs` — kept local because the native game has its
/// own copy with more fields (roll, etc.) we don't need here.
#[derive(Default)]
struct Control {
    motor: f32,
    rudder: f32,
    roll: f32,
    brake: bool,
    turbo: bool,
    /// `Some(power)` while the jump key is held; charge increases each
    /// frame. Consumed (set to `None`) on key release.
    jump_charge: Option<f32>,
    /// Pending jump to fire this frame, produced on key release.
    jump: Option<f32>,
}

/// The player vehicle + its per-frame physics state. Built once at
/// startup (when the VFS happens to contain enough Vangers data for
/// it) and then stepped each frame.
struct Agent {
    car: config::car::CarInfo,
    phys_data: physics::CarPhysicsData,
    transform: space::Transform,
    dynamo: physics::Dynamo,
    control: Control,
    color: render::object::BodyColor,
}

impl Agent {
    /// Apply control inputs with the same time scaling as the native
    /// build's `cpu_apply_control`. `input_factor` is
    /// `delta / MAIN_LOOP_TIME`, NOT raw dt.
    fn apply_control(&mut self, input_factor: f32, common: &config::common::Common) {
        if self.control.rudder != 0.0 {
            let angle = self.dynamo.rudder
                + common.car.rudder_step * 2.0 * input_factor * self.control.rudder;
            self.dynamo.rudder = angle.clamp(-common.car.rudder_max, common.car.rudder_max);
        }
        if self.control.motor != 0.0 {
            self.dynamo
                .change_traction(self.control.motor * input_factor * common.car.traction_incr);
        }
        if self.control.brake && self.dynamo.traction != 0.0 {
            self.dynamo.traction *= (-input_factor).exp2();
        }
    }

    /// Integrate one physics step. `physics_dt` uses the same scaling
    /// as the native build: `delta * fps * time_delta0 * num_calls`.
    fn physics_step(
        &mut self,
        physics_dt: f32,
        level: &level::Level,
        common: &config::common::Common,
    ) {
        let f_turbo = if self.control.turbo {
            common.global.k_traction_turbo
        } else {
            1.0
        };
        physics::step(
            &mut self.dynamo,
            &mut self.transform,
            physics_dt,
            &self.phys_data,
            level,
            common,
            f_turbo,
            if self.control.brake { 1.0 } else { 0.0 },
            self.control.jump.take(),
            self.control.roll,
            None, // line_buffer
        );
    }
}

/// Try to load the first main vehicle listed in `car.prm` out of the
/// VFS. Returns `None` on any missing asset — the caller uses free
/// camera movement in that case (same UX as before gameplay was wired).
fn spawn_default_agent(
    vfs: &Vfs,
    level: &level::Level,
    device: &wgpu::Device,
    object: &render::object::Context,
) -> Option<Agent> {
    use std::io::Cursor;

    let game_lst = vfs.read("game.lst")?;
    let registry = config::game::Registry::load_reader(Cursor::new(&*game_lst));

    let car_prm = vfs.read("car.prm")?;
    let (car_name, stats_data) = config::car::first_main_entry(Cursor::new(&*car_prm));

    let model_info = registry.model_infos.get(&car_name)?;
    let m3d_bytes = vfs.read(&model_info.path)?;

    // Per-vehicle physics file lives next to the m3d. If it's missing,
    // fall back to `default.prm` in the same directory.
    let prm_path = std::path::Path::new(&model_info.path).with_extension("prm");
    let prm_key = prm_path.to_str()?;
    let (prm_bytes, is_default) = if let Some(bytes) = vfs.read(prm_key) {
        (bytes, false)
    } else {
        let mut default = std::path::Path::new(&model_info.path).to_path_buf();
        default.set_file_name("default.prm");
        (vfs.read(default.to_str()?)?, true)
    };

    let car_physics = config::car::CarPhysics::load_reader(Cursor::new(&*prm_bytes));
    let scale = if is_default {
        model_info.scale
    } else {
        car_physics.scale_size
    };

    let visual = model::load_m3d_bytes(&m3d_bytes, device, object, SHAPE_SAMPLING);
    let phys_data =
        physics::CarPhysicsData::from_bytes(&m3d_bytes, &prm_bytes, scale, SHAPE_SAMPLING);

    let car = config::car::CarInfo {
        kind: config::car::Kind::Main,
        stats: config::car::CarStats::new(&stats_data),
        physics: car_physics,
        model: visual,
        scale,
    };

    // Spawn at the level center, snapped to the terrain.
    let coords = (level.size.0 / 2, level.size.1 / 2);
    let height = level.get(coords).high() + 5.0;
    let transform = space::Transform {
        scale,
        disp: glam::Vec3::new(coords.0 as f32, coords.1 as f32, height),
        rot: glam::Quat::IDENTITY,
    };

    Some(Agent {
        car,
        phys_data,
        transform,
        dynamo: physics::Dynamo::default(),
        control: Control::default(),
        color: PLAYER_COLOR,
    })
}

struct WebApp {
    render: Render,
    level: level::Level,
    cam: space::Camera,
    batcher: Batcher,
    /// Physics constants loaded from `common.prm`; `test_default` when
    /// the archive isn't available.
    common: config::common::Common,
    /// The player vehicle. `None` means the VFS didn't contain enough
    /// data to build one; we fall back to free-camera mode.
    agent: Option<Agent>,
    /// Follow-camera parameters (radius/height/smoothing).
    follow: space::Follow,
    /// Longest physics step taken in one go. The integrator is not
    /// linear in `dt` - drag and collision response saturate - so a
    /// long frame has to be split, not scaled up.
    max_quant: f32,
    /// True when running on WebGPU (vs WebGL2 fallback).
    is_webgpu: bool,
    moving_land: level::moving::MovingLand,
    /// Sensors and location engines that steer the moving land.
    triggers: level::trigger::Triggers,
    /// Time carried over towards the next moving-land quant.
    moving_land_time: f32,
    /// Scratch buffer for the rectangles a moving-land quant touched.
    moving_land_regions: Vec<level::moving::Region>,
}

impl WebApp {
    /// Build the app with a procedural test level. Used as a fallback
    /// when the release data can't be fetched (404, offline, etc.).
    // Embed settings.template.ron — the tracked version in the repo.
    // (config/settings.ron is gitignored as a per-developer override,
    // so it isn't present in CI checkouts.)
    const SETTINGS_RON: &str = include_str!("../../config/settings.template.ron");

    fn load_settings() -> settings::Settings {
        let mut s: settings::Settings =
            ron::de::from_str(Self::SETTINGS_RON).expect("Failed to parse embedded settings.ron");
        // Vertical heightmap scale for the web build. True 3D heightmap
        // rendering looks taller than original Vangers (which projected
        // the heightmap as a flat shaded plane); 0x80/0x100 = 0.5
        // brings the silhouette close to the 1998 look without
        // flattening it. Hard-coded here so native + the FFI keep the
        // settings.ron default (0x100).
        s.game.geometry.height = 0x80;
        // The focal-length FOV is wider than the legacy 45° default,
        // so the template's 2000-unit far plane clipped terrain near
        // the top of the screen. Push it out and widen the fog band
        // proportionally so the new horizon still fades smoothly
        // instead of cutting hard at the far plane.
        s.game.camera.depth_range = (10.0, 6000.0);

        {
            // GTA-style chase cam: far enough behind that the ~30-unit
            // mechos does not fill the frame (offset 26 sat on the tail),
            // high enough to see over the body, looking at a point ahead
            // of the nose so the car sits in the lower third. `angle` is
            // unused once `look_ahead` is set (90 looks straight down,
            // 0 along the horizon) and is kept as a fallback pitch.
            s.game.camera.angle = 12;
            s.game.camera.offset = 56.0;
            s.game.camera.height = 18.0;
            s.game.camera.look_ahead = 40.0;
            // Snappier than the default so the camera stays behind the
            // car through turns instead of trailing wide.
            s.game.camera.speed = 8.0;
            // At this height the ground is a few units from the eye, and
            // the 10-unit near plane would clip it away - rasterized
            // terrain then shows its own underside through the gap.
            s.game.camera.depth_range = (1.0, 2000.0);
        }
        s.render.fog.depth = 1000.0;
        s
    }

    fn new(gfx: &GraphicsContext, is_webgpu: bool) -> Self {
        let settings = Self::load_settings();
        let level_config = level::LevelConfig::new_test();
        let level = level::load(&level_config, &settings.game.geometry);
        Self::build(gfx, level_config, level, None, is_webgpu, &settings)
    }

    /// Build the app from a real level in a [`Vfs`]. `ini_path` is the
    /// VFS key of the world INI (e.g. `"fostral/world.ini"`).
    fn new_from_vfs(gfx: &GraphicsContext, vfs: &Vfs, ini_path: &str, is_webgpu: bool) -> Self {
        let settings = Self::load_settings();
        let level_config = level::LevelConfig::load_from_vfs(vfs, ini_path);
        let level = level::load_from_vfs(vfs, &level_config, &settings.game.geometry);
        Self::build(gfx, level_config, level, Some(vfs), is_webgpu, &settings)
    }

    fn build(
        gfx: &GraphicsContext,
        level_config: level::LevelConfig,
        level: level::Level,
        vfs: Option<&Vfs>,
        is_webgpu: bool,
        settings: &settings::Settings,
    ) -> Self {
        let objects_palette = vfs
            .and_then(|v| v.read("resource/pal/objects.pal"))
            .map(|b| level::read_palette_bytes(&b, None))
            .unwrap_or([[0xFF; 4]; 0x100]);

        let cam_config = &settings.game.camera;
        // Camera location is fixed up after the agent spawns below;
        // (0, 0, 200) is just a sane starting placeholder for the
        // free-camera fallback.
        let mut cam = space::Camera {
            loc: glam::vec3(0.0, 0.0, 200.0),
            rot: glam::Quat::IDENTITY,
            scale: glam::vec3(1.0, -1.0, 1.0),
            proj: {
                let h = gfx.screen_size.height.max(1) as f32;
                let focal = space::DEFAULT_FOCAL_PX;
                space::Projection::Perspective(space::PerspectiveParams {
                    fovy: space::PerspectiveParams::fov_from_focal_px(focal, h),
                    aspect: gfx.screen_size.width as f32 / h,
                    near: cam_config.depth_range.0,
                    far: cam_config.depth_range.1,
                    focal_px: Some(focal),
                })
            },
        };

        // On WebGPU, override terrain to RayVoxelTraced (needs compute).
        // On WebGL2, force RayTraced (fragment-only).
        let mut render_settings = settings.render.clone();
        // Voxel tracing needs compute, so it falls back to the height
        // field on WebGL2 - the mesh and the ray tracer run anywhere.
        let choice = match terrain_choice() {
            TerrainChoice::Voxel if !is_webgpu => {
                log::warn!("Voxel tracing needs WebGPU; falling back to the height-field tracer");
                TerrainChoice::Ray
            }
            other => other,
        };
        if choice == TerrainChoice::Mesh {
            // Quality 0.25 is the cheapest setting that still matches the
            // other renderers on open ground; see the terrain-mesh post.
            render_settings.terrain = settings::Terrain::Mesh { quality: 0.25 };
            // Ray-traced shadows work on both backends and do not depend
            // on the terrain renderer.
            render_settings.light.shadow.terrain = settings::ShadowTerrain::RayTraced;
        } else if choice == TerrainChoice::Voxel {
            render_settings.terrain = settings::Terrain::RayVoxelTraced {
                voxel_size: [2, 4, 1],
                // 40 was too low: rays that travel far exhaust the budget and return
                // background, so solid terrain reads as sky. Measured against a CPU
                // raycast on Fostral, first person, the fraction of solid terrain
                // drawn as sky fell from 21.8% to 2.1% on an open river view going
                // from 40 steps to 250, and from 9.3% to 0.1% on an open ridge.
                // Enclosed views were already fine and cost almost nothing extra,
                // because rays there terminate long before the budget runs out.
                max_outer_steps: 200,
                max_inner_steps: 200,
                max_update_texels: 1_000_000,
            };
            // Reuse the same voxel grid for shadow casting. Step counts
            // are halved compared to the main pass — shadows are lower-
            // frequency and don't need the same precision.
            render_settings.light.shadow.terrain = settings::ShadowTerrain::RayVoxelTraced {
                max_outer_steps: 20,
                max_inner_steps: 20,
            };
        } else {
            render_settings.terrain = settings::Terrain::RayTraced;
            render_settings.light.shadow.terrain = settings::ShadowTerrain::RayTraced;
        }
        let geometry = settings.game.geometry;
        let render = Render::new(
            gfx,
            &level_config,
            &objects_palette,
            &render_settings,
            &geometry,
            cam.front_face(),
        );

        // If the VFS has `common.prm` and a vehicle registry, spawn a
        // player agent. Any gap (missing common.prm, missing car.prm,
        // missing m3d/prm for the first vehicle) leaves `agent = None`
        // and the app falls back to free-camera mode.
        let common = vfs
            .and_then(|v| v.read("common.prm"))
            .map(|b| config::common::load_reader(std::io::Cursor::new(&*b)))
            .unwrap_or_else(config::common::Common::test_default);
        let agent = vfs.and_then(|v| spawn_default_agent(v, &level, &gfx.device, &render.object));
        if agent.is_none() {
            log::info!("No player agent — running in free-camera mode");
        }

        // Camera follow params from settings.ron, same conversion as
        // native (bin/road/game.rs CameraStyle::new).
        let follow = space::Follow {
            angle_x: (cam_config.angle as f32).to_radians() - std::f32::consts::FRAC_PI_2,
            offset: glam::vec3(0.0, cam_config.offset, cam_config.height),
            speed: cam_config.speed,
            look_ahead: cam_config.look_ahead,
        };

        let (moving_land, triggers) = match vfs {
            Some(v) => {
                let dir = level_config
                    .path_moving_land()
                    .and_then(|p| p.to_str().map(str::to_string))
                    .unwrap_or_else(|| "data.vot".into());
                let mut land = level::moving::MovingLand::load_from_vfs(
                    v,
                    &dir,
                    level_config.terrains.len() as i32,
                );
                let triggers = level::trigger::Triggers::load_from_vfs(v, &dir, &land);
                triggers.reset_locations(&mut land);
                if !land.is_empty() {
                    log::info!(
                        "Loaded moving land: {} locations, {} engines, {} sensors",
                        land.locations.len(),
                        triggers.engines.len(),
                        triggers.sensors.len()
                    );
                }
                (land, triggers)
            }
            None => Default::default(),
        };

        // Settle the follow camera at the agent's spawn pose. Without
        // this, the camera starts at the placeholder above and the slow
        // exponential follow (k = exp(-dt) ≈ 0.98 per frame at 60 Hz)
        // takes seconds to close the gap, looking like the camera "isn't
        // catching up". Same trick as `tests/net_physics.rs`.
        if let Some(ref a) = agent {
            cam.loc = a.transform.disp + glam::vec3(0.0, 0.0, 200.0);
            for _ in 0..120 {
                cam.follow(&a.transform, 1.0 / 60.0, &follow);
                cam.keep_above_ground(&level, CAMERA_CLEARANCE);
            }
        }

        WebApp {
            render,
            level,
            cam,
            batcher: Batcher::new(),
            common,
            agent,
            follow,
            max_quant: settings.game.physics.max_quant,
            is_webgpu,
            moving_land,
            triggers,
            moving_land_time: 0.0,
            moving_land_regions: Vec::new(),
        }
    }

    fn draw_ui(&self, ctx: &egui::Context) {
        egui::Window::new("Settings").show(ctx, |ui| {
            ui.label(format!(
                "Backend: {}",
                if self.is_webgpu { "WebGPU" } else { "WebGL2" }
            ));
            if let Some(ref agent) = self.agent {
                ui.separator();
                ui.label("Vehicle");
                let pos = agent.transform.disp;
                ui.label(format!(
                    "Position: ({:.0}, {:.0}, {:.0})",
                    pos.x, pos.y, pos.z
                ));
                ui.label(format!(
                    "Speed: {:.1}",
                    agent.dynamo.linear_velocity.length()
                ));
            }
            if !self.moving_land.is_empty() {
                ui.separator();
                ui.label(format!(
                    "Moving land: {} locations, {} engines",
                    self.moving_land.locations.len(),
                    self.triggers.engines.len()
                ));
            }
            ui.separator();
            ui.label("Camera");
            ui.label(format!(
                "Pos: ({:.0}, {:.0}, {:.0})",
                self.cam.loc.x, self.cam.loc.y, self.cam.loc.z
            ));
        });
    }

    fn resize(&mut self, extent: wgpu::Extent3d, device: &wgpu::Device) {
        self.render.resize(extent, device);
        if let space::Projection::Perspective(ref mut p) = self.cam.proj {
            p.aspect = extent.width as f32 / extent.height.max(1) as f32;
        }
    }

    /// Advances the moving land and hands the touched rectangles to the
    /// renderer. Same quantisation as the native game: animation speed is
    /// tied to `MAIN_LOOP_TIME`, not the frame rate.
    fn step_moving_land(&mut self, delta: f32) {
        if self.moving_land.is_empty() {
            return;
        }

        self.moving_land_time += delta;
        let quants = (self.moving_land_time / config::common::MAIN_LOOP_TIME) as u32;
        if quants == 0 {
            return;
        }
        self.moving_land_time -= quants as f32 * config::common::MAIN_LOOP_TIME;
        // After a long stall, drop the backlog instead of animating through it.
        const MAX_CATCH_UP: u32 = 4;

        self.moving_land_regions.clear();
        for _ in 0..quants.min(MAX_CATCH_UP) {
            if !self.triggers.is_empty() {
                let size = (self.level.size.0, self.level.size.1);
                if let Some(ref agent) = self.agent {
                    let pos = agent.transform.disp;
                    self.triggers.touch(
                        (pos.x as i32, pos.y as i32, pos.z as i32),
                        (agent.phys_data.bbox.radius * agent.car.scale) as i32,
                        size,
                    );
                }
                self.triggers.update(&mut self.moving_land);
            }
            self.moving_land
                .update(&mut self.level, &mut self.moving_land_regions);
        }

        self.moving_land_regions.sort_unstable();
        self.moving_land_regions.dedup();

        let z_range = 0..self.level.geometry.height as u16;
        for r in self.moving_land_regions.drain(..) {
            self.render.terrain.dirty_rects.push(DirtyRect {
                rect: Rect {
                    x: r.x as u16,
                    y: r.y as u16,
                    w: r.w as u16,
                    h: r.h as u16,
                },
                z_range: z_range.clone(),
                need_upload: true,
            });
        }
    }

    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        targets: ScreenTargets,
    ) -> wgpu::CommandBuffer {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("World"),
        });

        self.batcher.clear();
        if let Some(ref agent) = self.agent {
            self.batcher
                .add_model(&agent.car.model, &agent.transform, None, agent.color);
        }

        self.render.draw_world(
            &mut encoder,
            &mut self.batcher,
            &self.level,
            &self.cam,
            targets,
            None,
            device,
            queue,
        );
        encoder.finish()
    }
}

// --- Multiplayer WebSocket client (WASM only) ---

mod net_ws {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use vangers_net::{ClientMessage, PlayerId, ServerMessage, decode, encode};
    use wasm_bindgen::closure::Closure;

    pub struct WsClient {
        ws: web_sys::WebSocket,
        pub received: Rc<RefCell<Vec<ServerMessage>>>,
        pub connected: Rc<RefCell<bool>>,
        pub player_id: Option<PlayerId>,
        _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
        _on_open: Closure<dyn FnMut(JsValue)>,
        _on_error: Closure<dyn FnMut(web_sys::ErrorEvent)>,
        _on_close: Closure<dyn FnMut(web_sys::CloseEvent)>,
    }

    impl WsClient {
        pub fn connect(url: &str) -> Result<Self, JsValue> {
            log::info!("Connecting to WebSocket server: {}", url);
            let ws = web_sys::WebSocket::new(url)?;
            ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

            let received = Rc::new(RefCell::new(Vec::<ServerMessage>::new()));
            let connected = Rc::new(RefCell::new(false));

            // on_message: decode binary frames
            let recv_clone = received.clone();
            let on_message = Closure::wrap(Box::new(move |e: web_sys::MessageEvent| {
                if let Ok(buf) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                    let array = js_sys::Uint8Array::new(&buf);
                    let data = array.to_vec();
                    // Our protocol is length-prefixed; the server sends framed messages
                    let mut offset = 0;
                    while let Some((msg, consumed)) = decode::<ServerMessage>(&data[offset..]) {
                        recv_clone.borrow_mut().push(msg);
                        offset += consumed;
                    }
                }
            }) as Box<dyn FnMut(web_sys::MessageEvent)>);
            ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

            // on_open: send Join
            let ws_clone = ws.clone();
            let conn_clone = connected.clone();
            let on_open = Closure::wrap(Box::new(move |_: JsValue| {
                log::info!("WebSocket connected");
                *conn_clone.borrow_mut() = true;
                let msg = encode(&ClientMessage::Join {
                    player_name: "WebPlayer".into(),
                    car_name: "TestCar".into(),
                    color: 21,
                });
                let _ = ws_clone.send_with_u8_array(&msg);
            }) as Box<dyn FnMut(JsValue)>);
            ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));

            // on_error
            let on_error = Closure::wrap(Box::new(move |_: web_sys::ErrorEvent| {
                log::warn!("WebSocket error");
            }) as Box<dyn FnMut(web_sys::ErrorEvent)>);
            ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));

            // on_close
            let conn_close = connected.clone();
            let on_close = Closure::wrap(Box::new(move |_: web_sys::CloseEvent| {
                log::info!("WebSocket closed");
                *conn_close.borrow_mut() = false;
            }) as Box<dyn FnMut(web_sys::CloseEvent)>);
            ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));

            Ok(WsClient {
                ws,
                received,
                connected,
                player_id: None,
                _on_message: on_message,
                _on_open: on_open,
                _on_error: on_error,
                _on_close: on_close,
            })
        }

        pub fn send_input(&self, motor: f32, rudder: f32) {
            if !*self.connected.borrow() {
                return;
            }
            let msg = encode(&ClientMessage::Input {
                sequence: 0,
                control: vangers_net::NetControl {
                    motor,
                    rudder,
                    roll: 0.0,
                    brake: false,
                    turbo: false,
                    jump: None,
                },
            });
            let _ = self.ws.send_with_u8_array(&msg);
        }

        pub fn poll(&mut self) -> Vec<ServerMessage> {
            self.received.borrow_mut().drain(..).collect()
        }

        pub fn is_connected(&self) -> bool {
            *self.connected.borrow()
        }
    }
}

use web_time::Instant;

/// Pointer events collected from the document, so egui still works when
/// the canvas is not the event target (itch.io iframe, HTML overlays).
struct UiPointer {
    pos: [f32; 2],
    /// `None` = move, `Some(true)` = press, `Some(false)` = release.
    press: Option<bool>,
}

/// Closures registered on `document`. Kept alive for the page lifetime.
struct DomInputHooks {
    _keydown: Closure<dyn FnMut(web_sys::KeyboardEvent)>,
    _keyup: Closure<dyn FnMut(web_sys::KeyboardEvent)>,
    _pointerdown: Closure<dyn FnMut(web_sys::PointerEvent)>,
    _pointerup: Closure<dyn FnMut(web_sys::PointerEvent)>,
    _pointermove: Closure<dyn FnMut(web_sys::PointerEvent)>,
}

fn key_from_code(code: &str) -> Option<KeyCode> {
    Some(match code {
        "KeyW" => KeyCode::KeyW,
        "KeyA" => KeyCode::KeyA,
        "KeyS" => KeyCode::KeyS,
        "KeyD" => KeyCode::KeyD,
        "KeyQ" => KeyCode::KeyQ,
        "KeyE" => KeyCode::KeyE,
        "KeyZ" => KeyCode::KeyZ,
        "KeyX" => KeyCode::KeyX,
        "Space" => KeyCode::Space,
        "ShiftLeft" | "ShiftRight" => KeyCode::ShiftLeft,
        "AltLeft" | "AltRight" => KeyCode::AltLeft,
        _ => return None,
    })
}

fn pointer_pos(canvas: &web_sys::HtmlCanvasElement, event: &web_sys::PointerEvent) -> [f32; 2] {
    let rect = canvas.get_bounding_client_rect();
    [
        (event.client_x() as f64 - rect.left()) as f32,
        (event.client_y() as f64 - rect.top()) as f32,
    ]
}

fn in_level_picker(event: &web_sys::PointerEvent) -> bool {
    let Some(target) = event.target() else {
        return false;
    };
    let Ok(el) = target.dyn_into::<web_sys::Element>() else {
        return false;
    };
    el.closest("#level-picker").ok().flatten().is_some()
}

/// GPU resources initialized asynchronously on WASM.
struct GpuState {
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth_view: wgpu::TextureView,
    app: WebApp,
    window: Arc<Window>,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
}

struct WebHandler {
    window: Option<Arc<Window>>,
    gpu: Option<GpuState>,
    /// Shared slot for async WASM GPU init to deliver results.
    gpu_pending: std::rc::Rc<std::cell::RefCell<Option<GpuState>>>,
    screen_size: wgpu::Extent3d,
    keys_pressed: std::rc::Rc<std::cell::RefCell<std::collections::HashSet<KeyCode>>>,
    ui_pointer: std::rc::Rc<std::cell::RefCell<Vec<UiPointer>>>,
    /// Document-level input listeners. itch.io hosts us in an iframe and
    /// winit only hears keys/clicks on the canvas, so we take them from
    /// `document` instead.
    _dom_input: Option<DomInputHooks>,
    last_frame: Option<Instant>,
    ws_client: Option<net_ws::WsClient>,
    /// Status text overlay (used in multiplayer logging)
    #[allow(dead_code)]
    mp_status: String,
}

impl WebHandler {
    fn new() -> Self {
        let (ws_client, mp_status) = match SERVER_WS {
            Some(url) if !url.is_empty() => {
                // Auto-upgrade ws:// to wss:// when page is served over HTTPS
                let url = {
                    let is_https = web_sys::window()
                        .and_then(|w| w.location().protocol().ok())
                        .is_some_and(|p| p == "https:");
                    if is_https && url.starts_with("ws://") {
                        let upgraded = format!("wss://{}", &url[5..]);
                        log::info!("HTTPS page: upgrading {} to {}", url, upgraded);
                        upgraded
                    } else {
                        url.to_string()
                    }
                };
                match net_ws::WsClient::connect(&url) {
                    Ok(client) => (Some(client), format!("Connecting to {}...", url)),
                    Err(e) => {
                        log::warn!("Failed to connect to {}: {:?}", url, e);
                        (None, "Standalone mode (connection failed)".into())
                    }
                }
            }
            _ => {
                log::info!("No server configured, running standalone");
                (None, String::new())
            }
        };

        WebHandler {
            window: None,
            gpu: None,
            gpu_pending: std::rc::Rc::new(std::cell::RefCell::new(None)),
            screen_size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            keys_pressed: std::rc::Rc::new(std::cell::RefCell::new(
                std::collections::HashSet::new(),
            )),
            ui_pointer: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            _dom_input: None,
            last_frame: None,
            ws_client,
            mp_status,
        }
    }

    /// Listen on `document` in the capture phase. winit binds key/pointer
    /// handlers to the canvas, which never sees them while an HTML overlay
    /// is on top or while the itch.io iframe has focused `<body>` instead
    /// of the canvas.
    fn install_dom_input(&mut self, canvas: web_sys::HtmlCanvasElement) {
        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("no document");

        let keys = self.keys_pressed.clone();
        let on_keydown = Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
            if let Some(key) = key_from_code(&event.code()) {
                keys.borrow_mut().insert(key);
            }
            let code = event.code();
            if code == "Space" || code.starts_with("Arrow") {
                event.prevent_default();
            }
        }) as Box<dyn FnMut(web_sys::KeyboardEvent)>);

        let keys = self.keys_pressed.clone();
        let on_keyup = Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
            if let Some(key) = key_from_code(&event.code()) {
                keys.borrow_mut().remove(&key);
            }
        }) as Box<dyn FnMut(web_sys::KeyboardEvent)>);

        let pointer = self.ui_pointer.clone();
        let canvas_ptr = canvas.clone();
        let push_pointer =
            std::rc::Rc::new(move |event: web_sys::PointerEvent, press: Option<bool>| {
                if in_level_picker(&event) {
                    return;
                }
                pointer.borrow_mut().push(UiPointer {
                    pos: pointer_pos(&canvas_ptr, &event),
                    press,
                });
            });

        let down = push_pointer.clone();
        let canvas_focus = canvas.clone();
        let on_pointerdown = Closure::wrap(Box::new(move |event: web_sys::PointerEvent| {
            let _ = canvas_focus.focus();
            if event.button() == 0 {
                down(event, Some(true));
            }
        }) as Box<dyn FnMut(web_sys::PointerEvent)>);

        let up = push_pointer.clone();
        let on_pointerup = Closure::wrap(Box::new(move |event: web_sys::PointerEvent| {
            if event.button() == 0 {
                up(event, Some(false));
            }
        }) as Box<dyn FnMut(web_sys::PointerEvent)>);

        let mv = push_pointer;
        let on_pointermove = Closure::wrap(Box::new(move |event: web_sys::PointerEvent| {
            mv(event, None);
        }) as Box<dyn FnMut(web_sys::PointerEvent)>);

        let capture = true;
        let _ = document.add_event_listener_with_callback_and_bool(
            "keydown",
            on_keydown.as_ref().unchecked_ref(),
            capture,
        );
        let _ = document.add_event_listener_with_callback_and_bool(
            "keyup",
            on_keyup.as_ref().unchecked_ref(),
            capture,
        );
        let _ = document.add_event_listener_with_callback_and_bool(
            "pointerdown",
            on_pointerdown.as_ref().unchecked_ref(),
            capture,
        );
        let _ = document.add_event_listener_with_callback_and_bool(
            "pointerup",
            on_pointerup.as_ref().unchecked_ref(),
            capture,
        );
        let _ = document.add_event_listener_with_callback_and_bool(
            "pointermove",
            on_pointermove.as_ref().unchecked_ref(),
            capture,
        );

        self._dom_input = Some(DomInputHooks {
            _keydown: on_keydown,
            _keyup: on_keyup,
            _pointerdown: on_pointerdown,
            _pointerup: on_pointerup,
            _pointermove: on_pointermove,
        });
    }
}

impl ApplicationHandler for WebHandler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Wait + canvas rAF only fires while the document is considered
        // visible and focused. itch.io hosts us in a cross-origin iframe
        // that often gets a single animation frame and then goes idle,
        // which looks like a frozen first present. Poll keeps the loop
        // alive with setTimeout / scheduler.postTask.
        event_loop.set_control_flow(ControlFlow::Poll);

        if self.window.is_some() {
            return;
        }

        let mut _init_width = 800u32;
        let mut _init_height = 600u32;

        let attrs = Window::default_attributes().with_title("Vangers Web");

        let canvas = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .get_element_by_id("canvas")
            .expect("missing <canvas id='canvas'>")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .unwrap();

        let attrs = {
            use winit::platform::web::WindowAttributesExtWebSys;

            // Read the CSS layout size and set canvas resolution to match.
            // Cap at 4096 to stay within WebGPU texture limits. An itch.io
            // iframe can report 0x0 on the first layout pass; fall back to
            // the window so we do not configure a 1x1 swapchain.
            let win = web_sys::window().unwrap();
            let dpr = win.device_pixel_ratio();
            let max_dim = 4096u32;
            let mut css_w = canvas.client_width();
            let mut css_h = canvas.client_height();
            if css_w <= 0 || css_h <= 0 {
                css_w = win
                    .inner_width()
                    .ok()
                    .and_then(|v| v.as_f64())
                    .unwrap_or(800.0) as i32;
                css_h = win
                    .inner_height()
                    .ok()
                    .and_then(|v| v.as_f64())
                    .unwrap_or(600.0) as i32;
            }
            let cw = ((css_w as f64 * dpr) as u32).clamp(1, max_dim);
            let ch = ((css_h as f64 * dpr) as u32).clamp(1, max_dim);
            canvas.set_width(cw);
            canvas.set_height(ch);
            _init_width = cw;
            _init_height = ch;
            log::info!(
                "Canvas size: {}x{} css={}x{} (dpr={:.1})",
                cw,
                ch,
                css_w,
                css_h,
                dpr
            );

            attrs.with_canvas(Some(canvas.clone())).with_focusable(true)
        };

        self.install_dom_input(canvas);

        self.screen_size = wgpu::Extent3d {
            width: _init_width,
            height: _init_height,
            depth_or_array_layers: 1,
        };

        let window = Arc::new(event_loop.create_window(attrs).unwrap());

        let init_future = {
            let window_clone = window.clone();
            async move {
                // Pick the backend BEFORE any `create_surface` call.
                // A canvas can only have one rendering context for
                // its lifetime, so the first `getContext()` (which
                // happens inside `create_surface`) is binding. We
                // probe WebGPU here without touching the canvas:
                // build a WebGPU-only Instance and ask for an
                // adapter without `compatible_surface`. WebGPU is
                // the only backend that allows surface-less adapter
                // requests, so this is safe; on success we commit
                // to WebGPU, on failure we drop the Instance and
                // start fresh with GL.
                //
                // (`navigator.gpu` exists in browsers where WebGPU
                // is exposed but not actually working, so the
                // namespace check alone is not enough.)
                //
                // The mesh skips the probe outright. It is fitted on
                // the CPU and drawn with the plain raster pipeline, so
                // WebGPU buys it nothing, and initializing a backend a
                // route will not use is a way to inherit that
                // backend's problems - a WGSL rule enforced by one
                // implementation and not another takes down a page
                // that had no need of it. The voxel tracer needs
                // compute and so still asks.
                let webgpu_adapter = if terrain_choice() == TerrainChoice::Mesh {
                    log::info!("Mesh terrain runs on WebGL2; skipping the WebGPU probe");
                    None
                } else {
                    let probe = wgpu::Instance::new(wgpu::InstanceDescriptor {
                        backends: wgpu::Backends::BROWSER_WEBGPU,
                        ..wgpu::InstanceDescriptor::new_without_display_handle()
                    });
                    let adapter = probe
                        .request_adapter(&wgpu::RequestAdapterOptions {
                            power_preference: wgpu::PowerPreference::HighPerformance,
                            compatible_surface: None,
                            force_fallback_adapter: false,
                        })
                        .await
                        .ok();
                    // Keep the probe instance only if it produced the
                    // adapter we are going to render with; the surface
                    // has to come from the same instance.
                    adapter.map(|a| (probe, a))
                };
                let is_webgpu = webgpu_adapter.is_some();

                let (instance, surface, adapter) =
                    if let Some((webgpu_probe, adapter)) = webgpu_adapter {
                        log::info!("Using WebGPU backend");
                        let surface = webgpu_probe
                            .create_surface(window_clone.clone())
                            .expect("Failed to create the canvas surface (WebGPU)");
                        (webgpu_probe, surface, adapter)
                    } else {
                        log::info!("Using WebGL2 backend");
                        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                            backends: wgpu::Backends::GL,
                            ..wgpu::InstanceDescriptor::new_without_display_handle()
                        });
                        // WebGL2 requires `compatible_surface` for adapter
                        // enumeration, so the canvas surface has to come
                        // first.
                        let surface = instance
                            .create_surface(window_clone.clone())
                            .expect("Failed to create the canvas surface (WebGL2)");
                        let adapter = match instance
                            .request_adapter(&wgpu::RequestAdapterOptions {
                                power_preference: wgpu::PowerPreference::HighPerformance,
                                compatible_surface: Some(&surface),
                                force_fallback_adapter: false,
                            })
                            .await
                        {
                            Ok(a) => a,
                            Err(e) => {
                                let msg = format!("No GPU adapter available ({:?})", e);
                                let _ = js_progress_error(&msg);
                                panic!("{}", msg);
                            }
                        };
                        (instance, surface, adapter)
                    };

                let adapter_limits = adapter.limits();
                let required_limits = if is_webgpu {
                    // The voxel grid for Fostral at voxel_size=[2,4,1]
                    // is ~146 MiB, just above the 128 MiB default. Ask
                    // for 256 MiB — enough headroom for the stock
                    // levels without demanding more than low-end
                    // adapters can give.
                    const VOXEL_BUFFER_CAP: u64 = 256 << 20;
                    wgpu::Limits {
                        max_texture_dimension_2d: adapter_limits.max_texture_dimension_2d,
                        max_storage_buffer_binding_size: VOXEL_BUFFER_CAP,
                        max_buffer_size: VOXEL_BUFFER_CAP,
                        ..wgpu::Limits::downlevel_defaults()
                    }
                } else {
                    wgpu::Limits {
                        max_texture_dimension_2d: adapter_limits.max_texture_dimension_2d,
                        ..wgpu::Limits::downlevel_webgl2_defaults()
                    }
                };

                let (device, queue) = adapter
                    .request_device(&wgpu::DeviceDescriptor {
                        label: None,
                        required_features: wgpu::Features::empty(),
                        required_limits,
                        memory_hints: wgpu::MemoryHints::default(),
                        trace: wgpu::Trace::Off,
                        experimental_features: Default::default(),
                    })
                    .await
                    .expect("Failed to create device");

                (
                    instance,
                    surface,
                    adapter,
                    device,
                    queue,
                    window_clone,
                    is_webgpu,
                )
            }
        };

        let screen_size = self.screen_size;

        // Build GPU state from the async init results. `vfs_level` is
        // `Some((vfs, ini_path))` when real level data has been fetched;
        // `None` falls back to the procedural test level.
        let build_gpu_state = move |instance: wgpu::Instance,
                                    surface: wgpu::Surface<'static>,
                                    adapter: &wgpu::Adapter,
                                    device: wgpu::Device,
                                    queue: wgpu::Queue,
                                    screen_size: wgpu::Extent3d,
                                    vfs_level: Option<(Vfs, String)>,
                                    is_webgpu: bool,
                                    window: Arc<Window>|
              -> GpuState {
            // winit's web backend manages canvas backing size via its
            // own ResizeObserver. By the time async GPU init resolves,
            // it may have re-sized the canvas to a slightly different
            // value than what we computed in `resumed()` from
            // `client_width * dpr`. Trusting `window.inner_size()` here
            // keeps the surface config and camera aspect consistent
            // with the actual canvas backing — otherwise WebGPU draws
            // into a buffer of the wrong shape and the browser
            // stretches it onto the canvas.
            let inner = window.inner_size();
            let screen_size = if inner.width > 0 && inner.height > 0 {
                wgpu::Extent3d {
                    width: inner.width,
                    height: inner.height,
                    depth_or_array_layers: 1,
                }
            } else {
                screen_size
            };
            let caps = surface.get_capabilities(adapter);
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: caps.formats[0],
                width: screen_size.width,
                height: screen_size.height,
                present_mode: wgpu::PresentMode::Fifo,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: Vec::new(),
                desired_maximum_frame_latency: 2,
            };
            surface.configure(&device, &config);

            let depth_view = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("Depth"),
                    size: screen_size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: DEPTH_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default());

            let gfx = GraphicsContext {
                downlevel_caps: adapter.get_downlevel_capabilities(),
                color_format: config.format,
                screen_size,
                device,
                queue,
            };
            let app = match vfs_level {
                Some((vfs, ini_path)) => WebApp::new_from_vfs(&gfx, &vfs, &ini_path, is_webgpu),
                None => WebApp::new(&gfx, is_webgpu),
            };

            let egui_ctx = egui::Context::default();
            let egui_state =
                egui_winit::State::new(egui_ctx, egui::ViewportId::ROOT, &window, None, None, None);
            let egui_renderer = egui_wgpu::Renderer::new(
                &gfx.device,
                config.format,
                egui_wgpu::RendererOptions {
                    depth_stencil_format: None,
                    ..Default::default()
                },
            );

            GpuState {
                _instance: instance,
                surface,
                device: gfx.device,
                queue: gfx.queue,
                config,
                depth_view,
                app,
                window,
                egui_state,
                egui_renderer,
            }
        };

        self.window = Some(window);
        let pending = self.gpu_pending.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = js_phase("Initializing GPU…");
            let (instance, surface, adapter, device, queue, window, is_webgpu) = init_future.await;
            log::info!(
                "GPU initialized ({})",
                if is_webgpu { "WebGPU" } else { "WebGL2" }
            );

            // Best-effort fetch of release data. On any failure we
            // fall back to the procedural test level.
            let requested = selected_level_id();
            let level_id =
                pick_level_for_adapter(requested, adapter.limits().max_texture_dimension_2d);
            log::info!("Selected level: {}", level_id);
            let vfs_level = fetch_release_level(&level_id).await;

            // Level construction and renderer setup are synchronous
            // but far from instant (the renderer builds several
            // shader pipelines and uploads the height/meta/palette
            // textures). Announce the phase so the user sees why
            // the screen is still blank.
            let _ = js_phase(if vfs_level.is_some() {
                "Building level from release data…"
            } else {
                "Building procedural test level…"
            });

            let state = build_gpu_state(
                instance,
                surface,
                &adapter,
                device,
                queue,
                screen_size,
                vfs_level,
                is_webgpu,
                window.clone(),
            );
            *pending.borrow_mut() = Some(state);
            // Wake the event loop — without this, ControlFlow::Wait
            // keeps the loop sleeping and gpu_pending is never picked up.
            window.request_redraw();

            let _ = js_progress_done();
            log::info!("Web app ready");
        });
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        // Forward events to egui; skip game input if egui consumed them.
        // Pointer events are fed from the document (see `install_dom_input`)
        // because winit only hears them on the canvas, which HTML overlays
        // and an unfocused itch.io iframe both steal.
        if let Some(ref mut gpu) = self.gpu {
            let pointer = matches!(
                event,
                WindowEvent::CursorMoved { .. }
                    | WindowEvent::CursorEntered { .. }
                    | WindowEvent::CursorLeft { .. }
                    | WindowEvent::MouseInput { .. }
                    | WindowEvent::MouseWheel { .. }
            );
            if !pointer {
                let response = gpu.egui_state.on_window_event(&gpu.window, &event);
                if response.consumed {
                    return;
                }
            }
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput {
                event:
                    event::KeyEvent {
                        physical_key: PhysicalKey::Code(key),
                        state,
                        ..
                    },
                ..
            } => match state {
                event::ElementState::Pressed => {
                    self.keys_pressed.borrow_mut().insert(key);
                }
                event::ElementState::Released => {
                    self.keys_pressed.borrow_mut().remove(&key);
                }
            },
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                self.screen_size = wgpu::Extent3d {
                    width: size.width,
                    height: size.height,
                    depth_or_array_layers: 1,
                };
                if let Some(ref mut gpu) = self.gpu {
                    gpu.config.width = size.width;
                    gpu.config.height = size.height;
                    gpu.surface.configure(&gpu.device, &gpu.config);
                    gpu.depth_view = gpu
                        .device
                        .create_texture(&wgpu::TextureDescriptor {
                            label: Some("Depth"),
                            size: self.screen_size,
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: DEPTH_FORMAT,
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                            view_formats: &[],
                        })
                        .create_view(&wgpu::TextureViewDescriptor::default());
                    gpu.app.resize(self.screen_size, &gpu.device);
                }
            }
            WindowEvent::RedrawRequested => {
                if self.gpu.is_none()
                    && let Some(state) = self.gpu_pending.borrow_mut().take()
                {
                    // Adopt the surface's actual dimensions — they may
                    // differ from the pre-cap math in `resumed()` if
                    // winit's ResizeObserver reshaped the canvas during
                    // async GPU init. If this isn't synced, the next
                    // size-mismatch branch in render() would reconfigure
                    // back to the stale value and break the aspect ratio.
                    self.screen_size = wgpu::Extent3d {
                        width: state.config.width,
                        height: state.config.height,
                        depth_or_array_layers: 1,
                    };
                    self.gpu = Some(state);
                }

                self.render();

                // Schedule the next frame. On web this calls
                // requestAnimationFrame; on native with ControlFlow::Poll
                // this is redundant but harmless.
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Poll);
        if let Some(ref window) = self.window {
            window.request_redraw();
        }
    }
}

impl WebHandler {
    fn render(&mut self) {
        let Some(ref mut gpu) = self.gpu else {
            return;
        };

        // If the screen was resized while GPU was initializing asynchronously,
        // the depth texture and surface config still have the old size.
        // Reconfigure now before the first render.
        if gpu.config.width != self.screen_size.width
            || gpu.config.height != self.screen_size.height
        {
            gpu.config.width = self.screen_size.width;
            gpu.config.height = self.screen_size.height;
            gpu.surface.configure(&gpu.device, &gpu.config);
            gpu.depth_view = gpu
                .device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("Depth"),
                    size: self.screen_size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: DEPTH_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default());
            gpu.app.resize(self.screen_size, &gpu.device);
        }

        // Compute delta time
        let now = Instant::now();
        let dt = match self.last_frame {
            Some(prev) => (now - prev).as_secs_f32(),
            None => 1.0 / 60.0,
        };
        self.last_frame = Some(now);
        let dt = dt.min(0.1);

        // Derive motor/rudder from the currently pressed keys. Same
        // axis mapping in both "driving a vehicle" and "server input"
        // modes, so the multiplayer branch below can use them too.
        let keys = self.keys_pressed.borrow();
        let mut motor = 0.0f32;
        let mut rudder = 0.0f32;
        if keys.contains(&KeyCode::KeyW) {
            motor = 1.0;
        }
        if keys.contains(&KeyCode::KeyS) {
            motor = -1.0;
        }
        if keys.contains(&KeyCode::KeyA) {
            rudder = 1.0;
        }
        if keys.contains(&KeyCode::KeyD) {
            rudder = -1.0;
        }
        let brake = keys.contains(&KeyCode::Space);
        let turbo = keys.contains(&KeyCode::ShiftLeft);
        // Roll direction matches native: Q/E are scaled by cam.scale.y
        // (which is -1) so Q → +1, E → -1.
        let mut roll = 0.0f32;
        if keys.contains(&KeyCode::KeyQ) {
            roll = -gpu.app.cam.scale.y;
        }
        if keys.contains(&KeyCode::KeyE) {
            roll = gpu.app.cam.scale.y;
        }
        let jump_held = keys.contains(&KeyCode::AltLeft);
        let free_z_up = keys.contains(&KeyCode::KeyZ);
        let free_z_down = keys.contains(&KeyCode::KeyX);
        let free_q = keys.contains(&KeyCode::KeyQ);
        let free_e = keys.contains(&KeyCode::KeyE);
        drop(keys);

        // Direct camera control is active whenever we aren't actually
        // synced with a multiplayer server. `ws_client` is `Some` from
        // the moment the JS WebSocket object is created; we need the
        // true "handshake finished and socket still open" state, which
        // `WsClient::is_connected` exposes. Otherwise a failed or
        // dropped connection would leave the camera locked.
        let connected = self.ws_client.as_ref().is_some_and(|c| c.is_connected());

        // Moving land first, so the car drives on this quant's surface
        // (same order as the native game).
        gpu.app.step_moving_land(dt);

        if gpu.app.agent.is_some() {
            // Drive the player vehicle. Keyboard feeds control; physics
            // integrates the dynamo; the camera chases the transform.
            // We take a mutable reborrow scope so the follow-camera
            // update below can read from gpu.app too.
            let common = gpu.app.common.clone();
            let level_ref = &gpu.app.level;
            let mut follow = gpu.app.follow;
            let max_quant = gpu.app.max_quant;
            if let Some(ref mut agent) = gpu.app.agent {
                agent.control.motor = motor;
                agent.control.rudder = rudder;
                agent.control.brake = brake;
                agent.control.turbo = turbo;
                agent.control.roll = roll;
                // Jump: charge while Alt is held, fire on release
                if jump_held {
                    let power = dt * common.speed.standard_frame_rate as f32;
                    let charge = agent.control.jump_charge.get_or_insert(0.0);
                    *charge = (*charge + power).min(common.force.max_jump_power);
                } else if let Some(power) = agent.control.jump_charge.take() {
                    agent.control.jump = Some(power);
                }
                // Match the native build's time scaling (see bin/road/game.rs):
                //   input_factor = delta / MAIN_LOOP_TIME
                //   physics_dt   = delta * fps * time_delta0 * num_calls
                let input_factor = dt / config::common::MAIN_LOOP_TIME;
                let physics_dt = dt * {
                    let n = &common.nature;
                    common.speed.standard_frame_rate as f32
                        * n.time_delta0
                        * n.num_calls_analysis as f32
                };
                agent.apply_control(input_factor, &common);
                // Sub-step, as the native build does. `physics::step`
                // is not linear in `dt` - it derives a speed
                // correction from `dt / time_delta0` and applies drag
                // and collision response once per call, both of which
                // saturate - so folding a long frame into one large
                // step loses speed rather than covering the same
                // ground. That showed up as the car crawling whenever
                // the frame rate dipped, which is exactly when the
                // camera swings and more terrain comes into view.
                let mut left = physics_dt;
                while left > max_quant {
                    agent.physics_step(max_quant, level_ref, &common);
                    left -= max_quant;
                }
                agent.physics_step(left, level_ref, &common);
                // Pull back and look further as speed rises so the car
                // stays in the lower third instead of running toward
                // the top of the frame.
                let speed_xy = {
                    let v = agent.dynamo.linear_velocity;
                    (v.x * v.x + v.y * v.y).sqrt()
                };
                follow.offset.y += (speed_xy * 0.18).min(22.0);
                follow.look_ahead += (speed_xy * 0.28).min(28.0);
                gpu.app.cam.follow(&agent.transform, dt, &follow);
                gpu.app.cam.keep_above_ground(level_ref, CAMERA_CLEARANCE);
            }
        } else if !connected {
            // No vehicle loaded — fall back to the free camera. Same
            // bindings as the level-viewer behaviour this build
            // shipped with before gameplay was wired in.
            let move_speed = 100.0;
            let rotation_speed = 1.0;
            if motor != 0.0 {
                let mut dir = gpu.app.cam.rot * glam::Vec3::Y;
                dir.z = 0.0;
                if dir.length_squared() > 0.0 {
                    gpu.app.cam.loc += move_speed * dt * motor * dir.normalize();
                }
            }
            if rudder != 0.0 {
                let mut dir = gpu.app.cam.rot * glam::Vec3::X;
                dir.z = 0.0;
                if dir.length_squared() > 0.0 {
                    gpu.app.cam.loc -= move_speed * dt * rudder * dir.normalize();
                }
            }
            if free_z_up {
                gpu.app.cam.loc.z += move_speed * dt;
            }
            if free_z_down {
                gpu.app.cam.loc.z -= move_speed * dt;
            }
            if free_q {
                let rotation = glam::Quat::from_rotation_z(rotation_speed * dt);
                gpu.app.cam.rot = rotation * gpu.app.cam.rot;
            }
            if free_e {
                let rotation = glam::Quat::from_rotation_z(-rotation_speed * dt);
                gpu.app.cam.rot = rotation * gpu.app.cam.rot;
            }
        }

        // Process multiplayer messages
        if let Some(ref mut ws) = self.ws_client {
            // Send input
            if motor != 0.0 || rudder != 0.0 {
                ws.send_input(motor, rudder);
            }

            // Process received messages
            for msg in ws.poll() {
                match msg {
                    vangers_net::ServerMessage::Welcome {
                        player_id,
                        level_name,
                        ..
                    } => {
                        self.mp_status =
                            format!("Connected (player {}, level '{}')", player_id, level_name);
                        ws.player_id = Some(player_id);
                        log::info!("{}", self.mp_status);
                    }
                    vangers_net::ServerMessage::PlayerJoined {
                        player_id,
                        player_name,
                        ..
                    } => {
                        log::info!("Player {} ({}) joined", player_id, player_name);
                    }
                    vangers_net::ServerMessage::PlayerLeft { player_id } => {
                        log::info!("Player {} left", player_id);
                    }
                    vangers_net::ServerMessage::WorldState { agents, .. } => {
                        // Move camera to follow our agent
                        if let Some(my_id) = ws.player_id
                            && let Some(me) = agents.iter().find(|a| a.player_id == my_id)
                        {
                            let pos = glam::Vec3::from(me.transform.position);
                            gpu.app.cam.loc = glam::vec3(pos.x, pos.y, pos.z + 200.0);
                        }
                    }
                }
            }
        }

        let frame = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            other => {
                log::warn!(
                    "surface texture not ready ({other:?}), size {}x{}",
                    self.screen_size.width,
                    self.screen_size.height
                );
                if matches!(
                    other,
                    wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost
                ) {
                    gpu.surface.configure(&gpu.device, &gpu.config);
                }
                return;
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let targets = ScreenTargets {
            extent: self.screen_size,
            color: &view,
            depth: &gpu.depth_view,
        };
        let command_buffer = gpu.app.draw(&gpu.device, &gpu.queue, targets);

        // --- egui UI pass ---
        let mut raw_input = gpu.egui_state.take_egui_input(&gpu.window);
        // Canvas focus is unreliable inside itch.io's iframe. Treat the
        // game as focused whenever we are drawing, and apply pointer
        // events captured on the document.
        raw_input.focused = true;
        for ev in self.ui_pointer.borrow_mut().drain(..) {
            let pos = egui::pos2(ev.pos[0], ev.pos[1]);
            raw_input.events.push(egui::Event::PointerMoved(pos));
            if let Some(pressed) = ev.press {
                raw_input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                });
            }
        }
        let full_output = gpu.egui_state.egui_ctx().run_ui(raw_input, |ctx| {
            gpu.app.draw_ui(ctx);
        });
        gpu.egui_state
            .handle_platform_output(&gpu.window, full_output.platform_output);

        let paint_jobs = gpu
            .egui_state
            .egui_ctx()
            .tessellate(full_output.shapes, full_output.pixels_per_point);
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.screen_size.width, self.screen_size.height],
            pixels_per_point: gpu.window.scale_factor() as f32,
        };

        for (id, delta) in &full_output.textures_delta.set {
            gpu.egui_renderer
                .update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
        gpu.egui_renderer.update_buffers(
            &gpu.device,
            &gpu.queue,
            &mut gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("egui upload"),
                }),
            &paint_jobs,
            &screen_descriptor,
        );

        let mut egui_encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("UI") });
        {
            let mut pass = egui_encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    ..Default::default()
                })
                .forget_lifetime();
            gpu.egui_renderer
                .render(&mut pass, &paint_jobs, &screen_descriptor);
        }

        gpu.queue
            .submit(vec![command_buffer, egui_encoder.finish()]);
        for &id in &full_output.textures_delta.free {
            gpu.egui_renderer.free_texture(&id);
        }

        frame.present();
    }
}

#[wasm_bindgen(start)]
pub fn web_main() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Info).unwrap();

    log::info!("Starting Vangers Web");
    if let Some(url) = SERVER_WS {
        log::info!("Multiplayer server: {}", url);
    } else {
        log::info!("Standalone mode (no VANGERS_SERVER_WS set)");
    }

    let event_loop = EventLoop::new().unwrap();
    let handler = WebHandler::new();

    use winit::platform::web::EventLoopExtWebSys;
    event_loop.spawn_app(handler);
}

fn main() {
    web_main();
}
