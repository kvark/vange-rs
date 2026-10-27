//! Web entry point for vange-rs level viewer with test level.
//! Compiled with `cargo build --target wasm32-unknown-unknown --features web --bin web`
//!
//! If the `VANGERS_SERVER_WS` environment variable is set at compile time
//! (e.g. `ws://127.0.0.1:7801`), the viewer connects to that WebSocket on
//! startup. Story-cycle state (`CycleState` on `WorldState`) is applied from
//! the server so web and native clients stay aligned. If the connection
//! fails, it continues as a standalone viewer with local cycle stepping.

use wasm_bindgen::prelude::*;

use vangers::{
    config::{self, settings},
    creature, data, escave, level, life, minimap, model, physics,
    render::{
        self, Batcher, DEPTH_FORMAT, GraphicsContext, Render, ScreenTargets, debug::LineBuffer,
        object::Instance,
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

/// Longest frame whose quanta a multiplayer client still steps, seconds.
/// Past it (a stall, not a slow page) the car falls behind real time; the
/// server waits [`physics::prediction::InputQueue::STARVE_TICKS`] (3 s)
/// before driving it on without input.
const MP_FRAME_MAX: f32 = 2.5;

/// On-page multiplayer diagnostics, for testers without DevTools: the
/// last few join / bump / correction lines as the console logs them, and
/// the local car's predicted pose next to the server's. On by default
/// when the page joins a server, or with `#debug=1` (`#debug=0` hides it);
/// F2 or the Settings checkbox toggles it.
#[derive(Default)]
struct NetDebug {
    visible: bool,
    lines: std::collections::VecDeque<String>,
    /// The local car in the newest snapshot: position, its ack, speed.
    server: Option<(glam::Vec3, u32, f32)>,
    /// What the newest snapshot did to the prediction.
    outcome: String,
}

impl NetDebug {
    const LINES: usize = 8;

    fn new() -> Self {
        let visible = match hash_param("debug").as_deref() {
            Some("0") => false,
            Some(_) => true,
            None => server_url().is_some(),
        };
        NetDebug {
            visible,
            ..NetDebug::default()
        }
    }

    /// Log `line` to the console and keep it for the overlay, stamped
    /// with UTC wall time to match the server log.
    fn note(&mut self, level: log::Level, line: String) {
        log::log!(level, "{}", line);
        let iso = String::from(js_sys::Date::new_0().to_iso_string());
        let stamp = iso.get(11..19).unwrap_or("");
        if self.lines.len() == Self::LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(format!("{stamp}Z {line}"));
    }
}

/// The value of `key=` in the URL fragment (`#level=fostral&name=Bob`).
fn hash_param(key: &str) -> Option<String> {
    let hash = web_sys::window()?.location().hash().ok()?;
    hash.trim_start_matches('#').split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key && !v.is_empty()).then(|| {
            js_sys::decode_uri_component(v)
                .ok()
                .map(String::from)
                .unwrap_or_else(|| v.to_string())
        })
    })
}

/// The multiplayer server to join: `#server=ws://host:port`, else the
/// compile-time [`SERVER_WS`]. `None` plays solo.
fn server_url() -> Option<String> {
    hash_param("server").or_else(|| SERVER_WS.filter(|url| !url.is_empty()).map(str::to_string))
}

/// Player name and body color for `Join`: `#name=Bob&color=red`
/// (`green`, `red`, `blue`, `yellow`, `gray`, or the palette number).
fn player_identity() -> (String, render::object::BodyColor) {
    use render::object::BodyColor;
    let name = hash_param("name").unwrap_or_else(|| "WebPlayer".to_string());
    let color = match hash_param("color").as_deref().map(str::to_ascii_lowercase) {
        None => PLAYER_COLOR,
        Some(c) => match c.as_str() {
            "green" => BodyColor::Green,
            "red" => BodyColor::Red,
            "blue" => BodyColor::Blue,
            "yellow" => BodyColor::Yellow,
            "gray" | "grey" => BodyColor::Gray,
            other => other
                .parse::<u8>()
                .map(BodyColor::from_value)
                .unwrap_or(PLAYER_COLOR),
        },
    };
    (name, color)
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

use std::collections::HashMap;
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

/// How many collision-shape samples per face to tessellate for models
/// that only draw (the Bug). Cars take `settings.game.physics.shape_sampling`
/// like the native game and the server, so their physics match.
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
    /// The `car.prm` name, sent in the multiplayer `Join`.
    car_name: String,
    car: config::car::CarInfo,
    phys_data: physics::CarPhysicsData,
    transform: space::Transform,
    dynamo: physics::Dynamo,
    control: Control,
    color: render::object::BodyColor,
    /// Stretches each wheel covered since the last frame, cut into the
    /// ground afterwards - see `WebApp::step_tracks`.
    tracks: level::terraform::Tracks,
    /// Cirt gathered toward the world's story cycles.
    cirtainer: level::cycle::Cirtainer,
    armor: u16,
    max_armor: u16,
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
            Some(&mut self.tracks),
        );
    }
}

/// The vehicles of `car.prm`, built on demand out of the VFS: the
/// player's car, and remote players' cars by name as they join.
struct CarLib {
    vfs: Vfs,
    registry: config::game::Registry,
    entries: Vec<(String, config::car::Kind, Vec<u32>)>,
    shape_sampling: u8,
    cache: HashMap<String, config::car::CarInfo>,
}

impl CarLib {
    fn new(vfs: &Vfs, shape_sampling: u8) -> Option<Self> {
        use std::io::Cursor;
        let game_lst = vfs.read("game.lst")?;
        let registry = config::game::Registry::load_reader(Cursor::new(&*game_lst));
        let car_prm = vfs.read("car.prm")?;
        let entries = config::car::entries(Cursor::new(&*car_prm));
        Some(CarLib {
            vfs: vfs.clone(),
            registry,
            entries,
            shape_sampling,
            cache: HashMap::new(),
        })
    }

    fn first_main(&self) -> Option<&str> {
        self.entries
            .iter()
            .find(|(_, kind, _)| matches!(kind, config::car::Kind::Main))
            .map(|(name, _, _)| name.as_str())
    }

    /// The car named `name`, `None` on a name `car.prm` lacks or a missing
    /// asset.
    fn get(
        &mut self,
        name: &str,
        device: &wgpu::Device,
        object: &render::object::Context,
    ) -> Option<config::car::CarInfo> {
        use std::io::Cursor;
        if let Some(car) = self.cache.get(name) {
            return Some(car.clone());
        }
        let (_, kind, stats_data) = self.entries.iter().find(|(n, _, _)| n == name)?;
        let model_info = self.registry.model_infos.get(name)?;
        let m3d_bytes = self.vfs.read(&model_info.path)?;

        // Per-vehicle physics file lives next to the m3d. If it's missing,
        // fall back to `default.prm` in the same directory.
        let prm_path = std::path::Path::new(&model_info.path).with_extension("prm");
        let prm_key = prm_path.to_str()?;
        let (prm_bytes, is_default) = if let Some(bytes) = self.vfs.read(prm_key) {
            (bytes, false)
        } else {
            let mut default = std::path::Path::new(&model_info.path).to_path_buf();
            default.set_file_name("default.prm");
            (self.vfs.read(default.to_str()?)?, true)
        };

        let physics = config::car::CarPhysics::load_reader(Cursor::new(&*prm_bytes));
        let scale = if is_default {
            model_info.scale
        } else {
            physics.scale_size
        };
        let car = config::car::CarInfo {
            kind: *kind,
            stats: config::car::CarStats::new(stats_data),
            physics,
            model: model::load_m3d_bytes(&m3d_bytes, device, object, self.shape_sampling),
            scale,
        };
        self.cache.insert(name.to_string(), car.clone());
        Some(car)
    }
}

/// Build the first main vehicle listed in `car.prm`. Returns `None` on any
/// missing asset — the caller uses free camera movement in that case
/// (same UX as before gameplay was wired).
fn spawn_default_agent(
    cars: &mut CarLib,
    level: &level::Level,
    device: &wgpu::Device,
    object: &render::object::Context,
    color: render::object::BodyColor,
) -> Option<Agent> {
    let car_name = cars.first_main()?.to_string();
    let car = cars.get(&car_name, device, object)?;
    // The same physics data the native game builds (and the server, from
    // the same files and `shape_sampling`).
    let phys_data = physics::CarPhysicsData::from_car_info(&car);
    let scale = car.scale;

    // Spawn at the level center, snapped to the terrain.
    let coords = (level.size.0 / 2, level.size.1 / 2);
    let height = level.get(coords).high() + 5.0;
    let transform = space::Transform {
        scale,
        disp: glam::Vec3::new(coords.0 as f32, coords.1 as f32, height),
        rot: glam::Quat::IDENTITY,
    };

    let armor = car.stats.max_armor as u16;
    Some(Agent {
        car_name,
        car,
        phys_data,
        transform,
        dynamo: physics::Dynamo::default(),
        control: Control::default(),
        color,
        tracks: level::terraform::Tracks::default(),
        cirtainer: level::cycle::Cirtainer::default(),
        armor,
        max_armor: armor,
    })
}

/// A remote player, drawn where the server last put it (interpolated).
struct WebRemote {
    name: String,
    car: config::car::CarInfo,
    color: render::object::BodyColor,
    net: physics::client::RemoteCar,
}

impl std::borrow::Borrow<physics::client::RemoteCar> for WebRemote {
    fn borrow(&self) -> &physics::client::RemoteCar {
        &self.net
    }
}

fn load_shop_spins(
    vfs: &Vfs,
    shop: &escave::Shop,
    slots: &[String],
) -> HashMap<String, escave::SpinMesh> {
    use std::io::Cursor;

    let Some(game_lst) = vfs.read("game.lst") else {
        return HashMap::new();
    };
    let registry = config::game::Registry::load_reader(Cursor::new(&*game_lst));
    let mut meshes = HashMap::new();
    let mut load = |id: &str, model_id: &str| {
        if meshes.contains_key(id) {
            return;
        }
        let Some(info) = registry.model_infos.get(model_id) else {
            return;
        };
        let Some(bytes) = vfs.read(&info.path) else {
            return;
        };
        let Some(spin) = escave::SpinMesh::load_bytes(&bytes) else {
            return;
        };
        meshes.insert(id.to_string(), spin);
    };
    for good in shop.stock() {
        load(&good.id, good.mesh_id());
    }
    for sid in slots {
        load(sid, sid);
    }
    meshes
}

fn load_bug(
    vfs: &Vfs,
    device: &wgpu::Device,
    object: &render::object::Context,
) -> Option<(Vec<std::sync::Arc<model::Mesh>>, f32)> {
    use std::io::Cursor;

    let game_lst = vfs.read("game.lst")?;
    let registry = config::game::Registry::load_reader(Cursor::new(&*game_lst));
    let info = registry.model_infos.get("Bug")?;
    let bytes = vfs.read(&info.path)?;
    let frames = if info.path.rsplit('.').next() == Some("a3d") {
        model::load_a3d_frames_bytes(&bytes, device)
    } else {
        vec![model::load_m3d_bytes(&bytes, device, object, SHAPE_SAMPLING).body]
    };
    if frames.is_empty() {
        return None;
    }
    Some((frames, info.scale.max(0.2) / 3.0))
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
    /// World name used for story cycles / fauna (e.g. "Fostral").
    world_name: String,
    /// Story cycles when the world has an escave bunch.
    cycle: Option<level::cycle::Bunch>,
    cycle_time: f32,
    /// Multiplayer client prediction of the local car (quantum clock,
    /// numbered history, remote ghosts, render offset), shared with the
    /// native game (`physics::client`).
    mp: physics::client::ClientSync,
    /// Other players of the multiplayer session.
    remotes: HashMap<vangers_net::PlayerId, WebRemote>,
    /// On-page multiplayer diagnostics.
    net_debug: NetDebug,
    /// Vehicles by name, for the remote players' cars.
    cars: Option<CarLib>,
    moving: level::moving::MovingWorld,
    /// Terrain-editing effects the car is allowed to leave behind. Same
    /// defaults as the native game: tread on, hull-press and mole mounds off.
    terraform: level::terraform::Config,
    life: life::World,
    line_buffer: LineBuffer,
    space_held: bool,
    ride: Option<Ride>,
    /// Original Bug `.a3d` frames, if `game.lst` has one in the VFS.
    bug: Option<(Vec<std::sync::Arc<model::Mesh>>, f32)>,
    inventory: escave::Inventory,
    shop: escave::Shop,
    spin_meshes: HashMap<String, escave::SpinMesh>,
    screen: escave::Screen,
    approach_cam: Option<(glam::Vec3, glam::Vec3)>,
    escave_note: Option<String>,
    escave_selected: Option<String>,
    data_path: std::path::PathBuf,
    /// Packed `resource/iscreen/ldata/` from the world zip. Empty when the
    /// release has no cave maps (open-source tree, test level).
    iscreen: Vfs,
    cave_boot: CaveBoot,
    cave: Option<CaveView>,
    /// `escaves.prm` + `spots.prm` so VLC names like `Escave1` map to VigBoo.
    cave_pads: Vec<escave::cave::Pad>,
    /// On-screen stick (motor, rudder), only while a thumb is on it.
    stick: (f32, f32),
    /// On-screen Use, same edge as Space.
    use_held: bool,
    /// Phone / tablet: coarse pointer or a narrow touch screen.
    touch_stick: bool,
    minimap: minimap::Minimap,
}

struct CaveBoot {
    render: settings::Render,
    geometry: settings::Geometry,
    downlevel_caps: wgpu::DownlevelCapabilities,
    color_format: wgpu::TextureFormat,
    front_face: wgpu::FrontFace,
}

struct CaveView {
    name: String,
    level: level::Level,
    render: Render,
    cam: space::Camera,
    target: glam::Vec3,
    distance: f32,
    yaw: f32,
}

struct Ride {
    t: f32,
    duration: f32,
    start: glam::Vec3,
    dest: glam::Vec3,
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
        // A multiplayer build keeps the full height: the server steps the
        // cars on 0x100 terrain, and the prediction has to drive on the
        // same ground or every snapshot would pull the car back.
        if server_url().is_none() {
            s.game.geometry.height = 0x80;
        }
        // The focal-length FOV is wider than the legacy 45° default,
        // so the template's 2000-unit far plane clipped terrain near
        // the top of the screen. Push it out and widen the fog band
        // proportionally so the new horizon still fades smoothly
        // instead of cutting hard at the far plane.
        s.game.camera.depth_range = (10.0, 6000.0);

        {
            // Close third-person chase cam. The mechos is roughly 30 world
            // units long; with the wide focal-512 FOV a long chase shrinks
            // it into the screen, so this sits under a body length behind
            // the centre and looks just beyond the nose. `height` is the
            // base; `build` then adds one body height so the view sits
            // over the hull instead of staring at the dirt under it.
            // `angle` is unused once `look_ahead` is set.
            s.game.camera.angle = 12;
            s.game.camera.offset = 24.0;
            s.game.camera.height = 9.0;
            s.game.camera.look_ahead = 10.0;
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
        Self::build(
            gfx,
            level_config,
            level,
            None,
            is_webgpu,
            &settings,
            "fostral",
        )
    }

    /// Build the app from a real level in a [`Vfs`]. `ini_path` is the
    /// VFS key of the world INI (e.g. `"fostral/world.ini"`).
    fn new_from_vfs(
        gfx: &GraphicsContext,
        vfs: &Vfs,
        ini_path: &str,
        world: &str,
        is_webgpu: bool,
    ) -> Self {
        let settings = Self::load_settings();
        let t = Instant::now();
        let level_config = level::LevelConfig::load_from_vfs(vfs, ini_path);
        let level = level::load_from_vfs(vfs, &level_config, &settings.game.geometry);
        log::info!(
            "[startup] level '{ini_path}' loaded in {:.0} ms ({} texels)",
            t.elapsed().as_secs_f32() * 1e3,
            level.size.0 as u64 * level.size.1 as u64 / 1024 * 1024
        );
        let t = Instant::now();
        let app = Self::build(
            gfx,
            level_config,
            level,
            Some(vfs),
            is_webgpu,
            &settings,
            world,
        );
        log::info!(
            "[startup] app built in {:.0} ms",
            t.elapsed().as_secs_f32() * 1e3
        );
        app
    }

    fn build(
        gfx: &GraphicsContext,
        level_config: level::LevelConfig,
        level: level::Level,
        vfs: Option<&Vfs>,
        is_webgpu: bool,
        settings: &settings::Settings,
        world: &str,
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
            chase_yaw: None,
            under_slab: 0.0,
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
            // Same quality as native `tin::Config` default. Wasm still
            // scaffolds and drains instead of blocking startup on a full
            // parallel fit; the drain is sized so nearby chunks catch up.
            render_settings.terrain = settings::Terrain::Mesh { quality: 0.75 };
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
        let t = Instant::now();
        let render = Render::new(
            gfx,
            &level_config,
            &objects_palette,
            &render_settings,
            &geometry,
            cam.front_face(),
        );
        log::info!(
            "[startup] renderer built in {:.0} ms",
            t.elapsed().as_secs_f32() * 1e3
        );

        // If the VFS has `common.prm` and a vehicle registry, spawn a
        // player agent. Any gap (missing common.prm, missing car.prm,
        // missing m3d/prm for the first vehicle) leaves `agent = None`
        // and the app falls back to free-camera mode.
        let common = vfs
            .and_then(|v| v.read("common.prm"))
            .map(|b| config::common::load_reader(std::io::Cursor::new(&*b)))
            .unwrap_or_else(config::common::Common::test_default);
        let t = Instant::now();
        let mut cars = vfs.and_then(|v| CarLib::new(v, settings.game.physics.shape_sampling));
        let (_, player_color) = player_identity();
        let agent = cars.as_mut().and_then(|cars| {
            spawn_default_agent(cars, &level, &gfx.device, &render.object, player_color)
        });
        if agent.is_some() {
            log::info!(
                "[startup] player vehicle built in {:.0} ms",
                t.elapsed().as_secs_f32() * 1e3
            );
        } else {
            log::info!("No player agent — running in free-camera mode");
        }

        // Camera follow params from settings.ron, same conversion as
        // native (bin/road/game.rs CameraStyle::new).
        let mut follow = space::Follow {
            angle_x: (cam_config.angle as f32).to_radians() - std::f32::consts::FRAC_PI_2,
            offset: glam::vec3(0.0, cam_config.offset, cam_config.height),
            speed: cam_config.speed,
            look_ahead: cam_config.look_ahead,
        };
        if let Some(ref a) = agent {
            let b = &a.phys_data.bbox;
            let body_h = (b.max[2] - b.min[2]).abs() * a.car.scale;
            let extra = if body_h > 1.0 {
                body_h.clamp(6.0, 28.0)
            } else {
                12.0
            };
            follow.offset.z += extra;
            log::info!("chase camera raised by {extra:.0} (one body height)");
        }

        let moving_t = Instant::now();
        let moving = level::moving::MovingWorld::load(&level_config, vfs);
        log::info!(
            "[startup] moving world loaded in {:.0} ms",
            moving_t.elapsed().as_secs_f32() * 1e3
        );

        // Settle the follow camera at the agent's spawn pose. Without
        // this, the camera starts at the placeholder above and the slow
        // exponential follow (k = exp(-dt) ≈ 0.98 per frame at 60 Hz)
        // takes seconds to close the gap, looking like the camera "isn't
        // catching up". Same trick as `tests/net_physics.rs`.
        if let Some(ref a) = agent {
            cam.loc = a.transform.disp + glam::vec3(0.0, 0.0, 200.0);
            let reach = space::Subject::reach_of(&a.phys_data.bbox, a.car.scale);
            let subject = space::Subject::unanchored(&a.transform, reach);
            for _ in 0..120 {
                cam.chase_anchored(
                    &level,
                    &a.transform,
                    subject,
                    1.0 / 60.0,
                    &follow,
                    CAMERA_CLEARANCE,
                );
            }
        }

        // Story cycles from VFS bunches/escaves + stage palettes.
        let mut level = level;
        let cycle = vfs.and_then(|v| {
            let bunches =
                config::bunches::load_reader(std::io::Cursor::new(v.read("bunches.prm")?));
            let mut escaves = v
                .read("escaves.prm")
                .map(|b| config::escaves::load_reader(std::io::Cursor::new(b)))
                .unwrap_or_default();
            if let Some(bytes) = v.read("spots.prm") {
                escaves.extend(config::escaves::load_reader(std::io::Cursor::new(bytes)));
            }
            level::cycle::Bunch::load(world, &level, &bunches, &escaves, |path| {
                v.read(path)
                    .or_else(|| v.read(&format!("resource/{path}")))
                    .or_else(|| {
                        let trimmed = path.trim_start_matches("./");
                        v.read(trimmed)
                    })
                    .map(|bytes| bytes.as_ref().to_vec())
            })
        });
        if let Some(ref bunch) = cycle {
            level.palette = *bunch.settled_palette();
            log::info!(
                "Story cycle loaded for '{world}': {} stages ({})",
                bunch.stages.len(),
                bunch.escave
            );
            // Light matches the opening stage.
            // (render is already built; modulation is applied below.)
        } else {
            log::info!("No story cycle for '{world}'");
        }

        let mut life = life::World::spawn(world, &level, std::path::Path::new(""));
        life.beebs = 500;
        let bug = vfs.and_then(|v| load_bug(v, &gfx.device, &render.object));
        if bug.is_none() {
            log::info!("No Bug model in the VFS; beebs draw as ticks");
        }
        let shop = escave::Shop::fostral();
        let spin_meshes = {
            let spins = vfs
                .map(|v| load_shop_spins(v, &shop, &settings.car.slots))
                .unwrap_or_default();
            if spins.is_empty() {
                log::warn!("No shop item meshes; the preview turntable will be empty");
            } else {
                log::info!("Loaded {} shop preview meshes", spins.len());
            }
            spins
        };
        let mut app = WebApp {
            render,
            level,
            cam,
            batcher: Batcher::new(),
            common,
            agent,
            follow,
            max_quant: settings.game.physics.max_quant,
            is_webgpu,
            // Without level data this is the procedural test level,
            // whatever `world` names for the fauna.
            world_name: match vfs {
                Some(_) => world.to_string(),
                None => "test".to_string(),
            },
            cycle,
            cycle_time: 0.0,
            mp: Default::default(),
            remotes: HashMap::new(),
            net_debug: NetDebug::new(),
            cars,
            moving,
            terraform: level::terraform::Config::default(),
            life,
            line_buffer: LineBuffer::new(),
            space_held: false,
            ride: None,
            bug,
            inventory: {
                let boards = vfs
                    .and_then(|v| {
                        let a = v.read("actint/actint.inc")?;
                        let n = v.read("actint/a_str.inc")?;
                        Some(escave::Catalog::parse(
                            &String::from_utf8_lossy(&a),
                            &String::from_utf8_lossy(&n),
                        ))
                    })
                    .unwrap_or_else(|| escave::Catalog::load(&settings.data_path));
                if boards.is_empty() {
                    log::warn!(
                        "No actint mechos boards; shop hex grid will be empty (need actint/actint.inc)"
                    );
                }
                escave::Inventory::for_car(&settings.car.id, &boards)
            },
            shop,
            spin_meshes,
            screen: escave::Screen::new(),
            approach_cam: None,
            escave_note: None,
            escave_selected: None,
            data_path: std::path::PathBuf::new(),
            iscreen: {
                let slice = vfs
                    .map(|v| v.prefix("resource/iscreen/ldata"))
                    .unwrap_or_default();
                if slice.is_empty() {
                    log::info!("No iscreen ldata in VFS; escave interiors stay 2D");
                } else {
                    log::info!("Kept {} iscreen file(s) for escave voxels", slice.len());
                }
                slice
            },
            cave_boot: CaveBoot {
                render: escave::cave::render_settings(&render_settings),
                geometry,
                downlevel_caps: gfx.downlevel_caps.clone(),
                color_format: gfx.color_format,
                front_face: cam.front_face(),
            },
            cave: None,
            cave_pads: {
                let mut pads = Vec::new();
                if let Some(v) = vfs {
                    for key in ["escaves.prm", "spots.prm"] {
                        if let Some(bytes) = v.read(key) {
                            pads.extend(escave::cave::pads_from_prm(&String::from_utf8_lossy(
                                &bytes,
                            )));
                        }
                    }
                }
                if pads.is_empty() {
                    log::info!("No escaves.prm/spots.prm; VLC names like Escave1 will not map");
                } else {
                    log::info!("Loaded {} escave/spot pads for iscreen", pads.len());
                }
                pads
            },
            stick: (0.0, 0.0),
            use_held: false,
            touch_stick: uses_touch_stick(),
            minimap: minimap::Minimap::new(),
        };
        if let Some(ref bunch) = app.cycle {
            app.render.set_light_modulation(bunch.light());
            app.render.dirty_palette(0..0x100);
        }
        app
    }

    fn draw_ui(&mut self, ctx: &egui::Context) {
        if let Some(visit) = self.screen.visit() {
            let see_through = self.cave.is_some();
            let action = escave::draw_interior(
                ctx,
                visit,
                &self.shop,
                &self.inventory,
                self.life.beebs,
                self.escave_note.as_deref(),
                &mut self.escave_selected,
                &self.spin_meshes,
                see_through,
            );
            if let Some(visit) = self.screen.visit_mut() {
                let (note, _slots) = action.apply(
                    visit,
                    &mut self.shop,
                    &mut self.inventory,
                    &mut self.life.beebs,
                );
                if let Some(note) = note {
                    self.escave_note = Some(note);
                }
            }
            if action.leave {
                self.escave_selected = None;
                self.screen.begin_leave();
            }
        }
        let shutter = self.screen.shutter();
        if shutter > 0.0 {
            escave::draw_shutters(ctx, shutter);
        }

        if self.touch_stick {
            self.use_held = draw_use_button(ctx);
        }

        if !self.screen.is_world() {
            self.stick = (0.0, 0.0);
            return;
        }

        self.draw_minimap(ctx);
        self.draw_net_debug(ctx);

        if self.touch_stick {
            self.stick = draw_drive_stick(ctx);
            self.draw_camera_window(ctx, true);
            return;
        }

        const PANEL_WIDTH: f32 = 340.0;

        let window = egui::Window::new("Settings")
            // Coordinate strings change width as their digits and signs
            // change. Letting the window auto-size to them makes the whole
            // diagnostics panel visibly jitter while driving.
            .min_width(PANEL_WIDTH)
            .max_width(PANEL_WIDTH)
            .resizable(false);
        window.show(ctx, |ui| {
            ui.label(format!(
                "Backend: {}",
                if self.is_webgpu { "WebGPU" } else { "WebGL2" }
            ));
            ui.checkbox(&mut self.net_debug.visible, "Net debug overlay (F2)");
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
            if !self.moving.is_empty() {
                ui.separator();
                ui.label(format!(
                    "Moving land: {} locations, {} engines",
                    self.moving.land.locations.len(),
                    self.moving.triggers.engines.len()
                ));
            }
            ui.separator();
            ui.label("Camera");
            self.draw_camera_controls(ui);
        });
    }

    fn draw_camera_window(&mut self, ctx: &egui::Context, compact: bool) {
        egui::Window::new("Camera")
            .collapsible(true)
            .default_open(!compact)
            .resizable(false)
            .show(ctx, |ui| {
                self.draw_camera_controls(ui);
            });
    }

    fn draw_minimap(&mut self, ctx: &egui::Context) {
        let (center, heading) = if let Some(ref agent) = self.agent {
            let mut f = agent.transform.rot * glam::Vec3::Y;
            f.z = 0.0;
            (agent.transform.disp.truncate(), f.truncate())
        } else {
            (
                self.cam.loc.truncate(),
                self.cam.ground_forward().truncate(),
            )
        };
        let mut marks = Vec::new();
        for pad in &self.cave_pads {
            marks.push(minimap::Mark {
                pos: glam::Vec2::new(pad.pos.0 as f32, pad.pos.1 as f32),
                color: egui::Color32::from_rgb(220, 160, 40),
                large: true,
            });
        }
        self.minimap.show(ctx, &self.level, center, heading, &marks);
    }

    fn draw_net_debug(&mut self, ctx: &egui::Context) {
        if !self.net_debug.visible {
            return;
        }
        let debug = &self.net_debug;
        egui::Window::new("Net debug (F2)")
            .anchor(egui::Align2::RIGHT_TOP, [-8.0, 56.0])
            .resizable(false)
            .collapsible(true)
            .default_width(420.0)
            .show(ctx, |ui| {
                let mono = |text: String| egui::RichText::new(text).monospace().size(11.0);
                match self.agent {
                    Some(ref agent) => {
                        let p = agent.transform.disp;
                        ui.label(mono(format!(
                            "predicted ({:.1}, {:.1}, {:.1}) speed {:.1} next q{}",
                            p.x,
                            p.y,
                            p.z,
                            agent.dynamo.linear_velocity.length(),
                            self.mp.prediction.next()
                        )));
                    }
                    None => {
                        ui.label(mono("no local car".to_string()));
                    }
                }
                match debug.server {
                    Some((p, ack, speed)) => {
                        ui.label(mono(format!(
                            "server    ({:.1}, {:.1}, {:.1}) speed {:.1} ack q{}",
                            p.x, p.y, p.z, speed, ack
                        )));
                    }
                    None => {
                        ui.label(mono("server    (no snapshot yet)".to_string()));
                    }
                }
                if !debug.outcome.is_empty() {
                    ui.label(mono(format!("last snapshot: {}", debug.outcome)));
                }
                if self.ride.is_some() {
                    ui.label(mono("train ride: car held".to_string()));
                }
                ui.separator();
                for line in &debug.lines {
                    ui.label(mono(line.clone()));
                }
            });
    }

    fn draw_camera_controls(&mut self, ui: &mut egui::Ui) {
        let follow = if self.agent.is_some() {
            Some(&mut self.follow)
        } else {
            None
        };
        self.cam.draw_controls(ui, follow, None);
    }

    fn resize(&mut self, extent: wgpu::Extent3d, device: &wgpu::Device) {
        self.render.resize(extent, device);
        if let space::Projection::Perspective(ref mut p) = self.cam.proj {
            p.aspect = extent.width as f32 / extent.height.max(1) as f32;
        }
        if let Some(ref mut cave) = self.cave {
            cave.cam
                .proj
                .update(extent.width as u16, extent.height as u16);
            cave.render.resize(extent, device);
        }
    }

    fn step_moving_land(&mut self, delta: f32, networked: bool) {
        // Multiplayer: server owns playback via `MovingLandState` on WorldState.
        if networked {
            return;
        }
        let height = self.level.geometry.height as u16;
        let touches = self.agent.as_ref().map(|a| {
            let pos = a.transform.disp;
            level::moving::Touch {
                pos: (pos.x as i32, pos.y as i32, pos.z as i32),
                radius: (a.phys_data.bbox.radius * a.car.scale) as i32,
            }
        });
        let regions = self.moving.step(&mut self.level, delta, touches);
        self.render.dirty_terrain(regions, height);
    }

    /// Apply server moving-land playback (frame / phase / step per location).
    fn apply_moving_land_state(&mut self, state: &vangers_net::MovingLandState) {
        let height = self.level.geometry.height as u16;
        let playback: Vec<level::moving::PlaybackState> = state
            .locations
            .iter()
            .map(|p| level::moving::PlaybackState {
                frame: p.frame as usize,
                stage: p.stage,
                go_phase: p.go_phase,
                step: p.step,
            })
            .collect();
        let regions = self.moving.sync_authority(&playback, &mut self.level);
        self.render.dirty_terrain(regions, height);
    }

    /// Solo: gather / deliver / advance. Networked: fade visuals only.
    fn step_cycle(&mut self, delta: f32, networked: bool) {
        let bunch = match self.cycle {
            Some(ref mut bunch) => bunch,
            None => return,
        };
        self.cycle_time += delta;
        let quants = (self.cycle_time / config::common::MAIN_LOOP_TIME) as u32;
        if quants == 0 {
            return;
        }
        self.cycle_time -= quants as f32 * config::common::MAIN_LOOP_TIME;

        let mut range = 0..0;
        for _ in 0..quants.min(4) {
            let step = if networked {
                bunch.step_fade_quant(&mut self.level)
            } else {
                if let Some(ref mut agent) = self.agent {
                    let coord = (agent.transform.disp.x as i32, agent.transform.disp.y as i32);
                    bunch.gather(coord, &mut agent.cirtainer);
                    bunch.deliver(coord, &mut agent.cirtainer);
                }
                bunch.quant(&mut self.level)
            };
            if step.start != step.end {
                range = step;
            }
        }
        if range.start != range.end {
            self.render.dirty_palette(range);
            self.render.set_light_modulation(bunch.light());
        }
    }

    fn apply_cycle_state(&mut self, state: &vangers_net::CycleState) {
        let fade = state.fade.as_ref().map(|f| (f.target as usize, f.left));
        let applied = if let Some(ref mut bunch) = self.cycle {
            let range = bunch.sync_authority(
                &mut self.level,
                state.current as usize,
                &state.banked,
                state.light,
                fade,
            );
            Some((range, bunch.light()))
        } else {
            None
        };
        if let Some((range, light)) = applied {
            if range.start != range.end {
                self.render.dirty_palette(range);
            }
            self.render.set_light_modulation(light);
        }
    }

    fn apply_player_cirt(
        &mut self,
        my_id: Option<vangers_net::PlayerId>,
        state: &vangers_net::CycleState,
    ) {
        let Some(ref mut agent) = self.agent else {
            return;
        };
        for pc in &state.players {
            if Some(pc.player_id) == my_id {
                agent.cirtainer.set_held(&pc.held);
                break;
            }
        }
    }

    /// Cuts the stretches the wheels covered since the last frame into the
    /// level, and hands the touched rectangles to the renderer. Port of the
    /// native game's `step_tracks` (bin/road/game.rs); the physics records
    /// the wheels' stretches over an immutable level, and the cutting here
    /// runs where the level can be borrowed mutably.
    fn step_tracks(&mut self) {
        let Some(agent) = self.agent.as_mut() else {
            return;
        };
        let mut regions = Vec::new();
        let reach = (agent.phys_data.bbox.radius * agent.car.scale).max(8.0) as i32;
        let treads = level::terraform::apply_vehicle(
            &mut self.level,
            &self.terraform,
            &mut agent.tracks,
            reach,
            &mut regions,
        );
        for track in treads {
            self.life.particles.from_track(&track, &self.level);
        }
        if regions.is_empty() {
            return;
        }
        regions.sort_unstable();
        regions.dedup();
        let height = self.level.geometry.height as u16;
        self.render.dirty_terrain(&regions, height);
    }

    fn step_life(&mut self, delta: f32) {
        let Some(ref agent) = self.agent else {
            return;
        };
        let wheels = agent.phys_data.wheel_points(&agent.transform);
        let contact = life::Contact {
            pos: agent.transform.disp,
            wheels: &wheels,
            radius: agent.phys_data.bbox.radius * agent.car.scale,
            armor: agent.armor,
            max_armor: agent.max_armor,
        };
        let nibble = self.life.step(&self.level, delta, contact, &[]);
        if nibble != 0
            && let Some(ref mut agent) = self.agent
        {
            agent.armor = agent.armor.saturating_sub(nibble);
        }
    }

    fn start_enter(&mut self, name: String, dest: glam::Vec3) {
        if self.screen.blocks_drive() {
            return;
        }
        self.approach_cam = Some((self.cam.loc, dest));
        self.escave_note = None;
        self.escave_selected = None;
        self.screen.begin_enter(name);
    }

    fn step_screen(&mut self, delta: f32) {
        let leaving = !self.screen.is_world();
        let had_interior = self.screen.visit().is_some();
        self.screen.step(delta, &self.data_path);
        if let Some(blend) = self.screen.camera_blend()
            && let Some((start, dest)) = self.approach_cam
        {
            self.cam.loc = start.lerp(dest, blend);
        }
        if had_interior && self.screen.visit().is_none() {
            self.eject_from_escave();
        }
        if leaving && self.screen.is_world() {
            self.approach_cam = None;
        }
        if let Some(ref mut cave) = self.cave {
            cave.yaw += delta * 0.12;
            escave::cave::orbit(
                &mut cave.cam,
                cave.target,
                cave.distance,
                cave.yaw,
                escave::cave::ELEVATION,
            );
        }
    }

    fn ensure_cave(
        &mut self,
        name: &str,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        extent: wgpu::Extent3d,
    ) {
        if self.cave.as_ref().is_some_and(|c| c.name == name) {
            return;
        }
        let situation = escave::cave::Situation::Shop;
        let Some((config, level)) =
            escave::cave::load_from_vfs(&self.iscreen, name, &self.cave_boot.geometry, situation)
        else {
            self.cave = None;
            return;
        };
        let gfx = GraphicsContext {
            device: device.clone(),
            queue: queue.clone(),
            downlevel_caps: self.cave_boot.downlevel_caps.clone(),
            color_format: self.cave_boot.color_format,
            screen_size: extent,
        };
        let pal = level.palette;
        let mut render = Render::new(
            &gfx,
            &config,
            &pal,
            &self.cave_boot.render,
            &self.cave_boot.geometry,
            self.cave_boot.front_face,
        );
        render.terrain.set_void_at_zero(true);
        let (mut cam, target, distance) = escave::cave::camera(&level, self.cam.proj, situation);
        cam.proj.update(extent.width as u16, extent.height as u16);
        let tile = situation.region();
        log::info!(
            "escave iscreen {name} {}x{} from {}x{}+{},{}",
            level.size.0,
            level.size.1,
            tile.w,
            tile.h,
            tile.x,
            tile.y
        );
        self.cave = Some(CaveView {
            name: name.to_string(),
            level,
            render,
            cam,
            target,
            distance,
            yaw: 0.35,
        });
    }

    fn try_use(&mut self) {
        if self.screen.visit().is_some() {
            self.screen.begin_leave();
            return;
        }
        if self.screen.blocks_drive() {
            return;
        }
        let Some(ref agent) = self.agent else {
            return;
        };
        let pos = agent.transform.disp;
        let radius = (agent.phys_data.bbox.radius * agent.car.scale) as i32;
        let at = (pos.x as i32, pos.y as i32, pos.z as i32);
        self.moving.use_at(at, radius, self.level.size);
        self.moving.triggers.touch(at, radius, self.level.size);
    }

    fn try_begin_escave_visit(&mut self) {
        if !self.screen.is_world() || self.ride.is_some() {
            return;
        }
        let Some(ref agent) = self.agent else {
            return;
        };
        if agent.dynamo.linear_velocity.z > 10.0 {
            return;
        }
        let pos = agent.transform.disp;
        let radius = (agent.phys_data.bbox.radius * agent.car.scale) as i32;
        let Some(arrival) = self.moving.triggers.escave_arrival_at(
            (pos.x as i32, pos.y as i32, pos.z as i32),
            radius,
            self.level.size,
        ) else {
            return;
        };
        let name = escave::cave::resolve_name(
            &arrival.name,
            (arrival.pos.0, arrival.pos.1),
            &self.cave_pads,
        );
        self.net_debug.note(
            log::Level::Info,
            format!("Escave arrival '{}' at ({:.0},{:.0})", name, pos.x, pos.y),
        );
        self.start_enter(
            name,
            glam::Vec3::new(arrival.pos.0 as f32, arrival.pos.1 as f32, pos.z),
        );
    }

    fn eject_from_escave(&mut self) {
        let Some(ref mut agent) = self.agent else {
            return;
        };
        let pos = agent.transform.disp;
        let radius = (agent.phys_data.bbox.radius * agent.car.scale) as i32 + 96;
        let at = (pos.x as i32, pos.y as i32, pos.z as i32);
        let size = self.level.size;
        self.moving.close_doors_at(at, radius, size);
        let kick = self.moving.triggers.impulse_at(at, radius, size);
        let vel = match kick {
            Some(k) => {
                let dir = glam::Vec3::new(
                    k.direction.0 as f32,
                    k.direction.1 as f32,
                    k.direction.2 as f32,
                );
                let n = dir.length();
                let dir = if n < 1e-3 { glam::Vec3::Z } else { dir / n };
                dir * (k.power as f32).max(24.0)
            }
            None => glam::Vec3::new(0.0, 24.0, 80.0),
        };
        agent.dynamo.linear_velocity += vel;
    }

    fn begin_train_ride(&mut self) -> bool {
        if self.ride.is_some() || self.screen.blocks_drive() {
            return false;
        }
        let Some(ref agent) = self.agent else {
            return false;
        };
        let pos = agent.transform.disp;
        let radius = (agent.phys_data.bbox.radius * agent.car.scale) as i32;
        let Some(ride) = self.moving.triggers.train_ride_at(
            (pos.x as i32, pos.y as i32, pos.z as i32),
            radius,
            self.level.size,
        ) else {
            return false;
        };
        let dest_z = self.level.get((ride.dest.0, ride.dest.1)).high() + 8.0;
        self.net_debug.note(
            log::Level::Info,
            format!(
                "Train ride from ({:.0},{:.0}) to ({},{})",
                pos.x, pos.y, ride.dest.0, ride.dest.1
            ),
        );
        self.ride = Some(Ride {
            t: 0.0,
            duration: (ride.quants as f32 * config::common::MAIN_LOOP_TIME).max(0.6),
            start: pos,
            dest: glam::Vec3::new(ride.dest.0 as f32, ride.dest.1 as f32, dest_z),
        });
        true
    }

    fn step_ride(&mut self, delta: f32) -> bool {
        let Some(ref mut ride) = self.ride else {
            return false;
        };
        ride.t = (ride.t + delta / ride.duration).min(1.0);
        let t = ride.t;
        let s = t * t * (3.0 - 2.0 * t);
        let pos = ride.start.lerp(ride.dest, s);
        let done = t >= 1.0;
        if let Some(ref mut agent) = self.agent {
            agent.transform.disp = pos;
            agent.dynamo.linear_velocity = glam::Vec3::ZERO;
            agent.dynamo.angular_velocity = glam::Vec3::ZERO;
            agent.dynamo.traction = 0.0;
        }
        if done {
            self.ride = None;
        }
        true
    }

    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        targets: ScreenTargets,
    ) -> wgpu::CommandBuffer {
        if let Some(name) = self.screen.cave_name().map(str::to_string) {
            self.ensure_cave(&name, device, queue, targets.extent);
        }
        if self.screen.visit().is_some()
            && let Some(ref mut cave) = self.cave
        {
            self.batcher.clear();
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Escave"),
            });
            cave.render
                .set_local_light(cave.target, 280.0, [1.0, 0.82, 0.52]);
            cave.render.draw_world(
                &mut encoder,
                &mut self.batcher,
                &cave.level,
                &cave.cam,
                targets,
                None,
                device,
                queue,
                None,
            );
            return encoder.finish();
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("World"),
        });

        self.batcher.clear();
        // Remote players at their interpolated server pose, on the copy of
        // the torus next to the eye (as in the native game).
        for remote in self.remotes.values() {
            if !remote.net.pose_ready {
                continue;
            }
            let mut drawn = remote.net.render_transform;
            drawn.disp = self.level.display_pos(drawn.disp, self.cam.loc);
            self.batcher
                .add_model(&remote.car.model, &drawn, None, remote.color);
        }
        if let Some(ref agent) = self.agent {
            // In multiplayer the last corrections are drawn as a decaying
            // offset; solo it is zero.
            let mut drawn = self.mp.drawn(&agent.transform);
            drawn.disp = self.level.display_pos(drawn.disp, self.cam.loc);
            self.batcher
                .add_model(&agent.car.model, &drawn, None, agent.color);
            let r = (agent.phys_data.bbox.radius * agent.car.scale).max(8.0);
            let p = drawn.disp;
            // Only punch the see-through cone when terrain still blocks
            // the car after camera avoidance — otherwise solid ground
            // (especially the long cam→car strip at turbo pullback).
            if space::Camera::terrain_blocks_view(&self.level, self.cam.loc, p, CAMERA_CLEARANCE) {
                self.render.set_focus(p, r * 1.15);
            } else {
                self.render.clear_focus();
            }
        } else {
            self.render.clear_focus();
        }
        let eye = self.cam.loc;
        if let Some((ref frames, scale)) = self.bug {
            for insect in self.life.swarm.near(eye, creature::ACTIVE_RADIUS) {
                let mesh = &frames[insect.frame(frames.len())];
                let transform = space::Transform {
                    scale,
                    disp: self.level.display_pos(insect.pos, eye),
                    rot: insect.rotation(),
                };
                let color = match insect.tier {
                    0 => m3d::ColorId::Custom1 as u8,
                    1 => m3d::ColorId::Custom2 as u8,
                    _ => m3d::ColorId::Custom4 as u8,
                };
                self.batcher
                    .add_mesh(mesh, Instance::new(&transform, 0.0, color));
            }
        }
        self.life
            .draw_fx(&mut self.line_buffer, eye, self.bug.is_none());

        self.render.draw_world(
            &mut encoder,
            &mut self.batcher,
            &self.level,
            &self.cam,
            targets,
            None,
            device,
            queue,
            Some(&self.line_buffer),
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
        /// `Join` went out on this connection.
        joined: bool,
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

            // on_open: the frame loop sends `Join` once the car is known.
            let conn_clone = connected.clone();
            let on_open = Closure::wrap(Box::new(move |_: JsValue| {
                log::info!("WebSocket connected");
                *conn_clone.borrow_mut() = true;
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
                joined: false,
                _on_message: on_message,
                _on_open: on_open,
                _on_error: on_error,
                _on_close: on_close,
            })
        }

        /// Join the session with the player's car, once per connection.
        pub fn join(&mut self, player_name: &str, car_name: &str, color: u8) {
            if self.joined || !self.is_connected() {
                return;
            }
            self.joined = true;
            log::info!(
                "Joining as {} in {} (color {})",
                player_name,
                car_name,
                color
            );
            let msg = encode(&ClientMessage::Join {
                player_name: player_name.to_string(),
                car_name: car_name.to_string(),
                color,
                protocol: vangers_net::PROTOCOL_VERSION,
            });
            let _ = self.ws.send_with_u8_array(&msg);
        }

        pub fn is_joined(&self) -> bool {
            self.joined && self.is_connected()
        }

        /// This frame's batch: `quanta` quanta from `first_quant` with the
        /// full controls (held ones for all, the jump and roll of the last).
        /// Sent every frame with quanta, released keys included: the server
        /// runs exactly the quanta it is sent.
        pub fn send_input(
            &mut self,
            first_quant: u32,
            quanta: u32,
            control: &vangers_net::NetControl,
        ) {
            if !self.is_joined() || quanta == 0 {
                return;
            }
            let msg = encode(&ClientMessage::Input {
                first_quant,
                quanta: quanta.min(u16::MAX as u32) as u16,
                control: control.clone(),
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
    _blur: Closure<dyn FnMut(web_sys::Event)>,
    _visibility: Closure<dyn FnMut(web_sys::Event)>,
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
        "ControlLeft" | "ControlRight" => KeyCode::ControlLeft,
        "ShiftLeft" | "ShiftRight" => KeyCode::ShiftLeft,
        "AltLeft" | "AltRight" => KeyCode::AltLeft,
        "KeyJ" => KeyCode::KeyJ,
        "F2" => KeyCode::F2,
        _ => return None,
    })
}

/// Phone / tablet: a coarse pointer, or a narrow screen that also has touch.
fn uses_touch_stick() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    if window
        .match_media("(pointer: coarse)")
        .ok()
        .flatten()
        .is_some_and(|m| m.matches())
    {
        return true;
    }
    let touch = window.navigator().max_touch_points() > 0;
    let narrow = window
        .inner_width()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(1024.0)
        < 900.0;
    touch && narrow
}

const STICK_RADIUS: f32 = 56.0;
const USE_RADIUS: f32 = 36.0;
const TOUCH_MARGIN: f32 = 20.0;
const TOUCH_GAP: f32 = 16.0;

/// Bottom-right analog stick. Returns (motor, rudder) in [-1, 1], matching WASD.
fn draw_drive_stick(ctx: &egui::Context) -> (f32, f32) {
    const RADIUS: f32 = STICK_RADIUS;
    const MARGIN: f32 = TOUCH_MARGIN;
    let mut drive = (0.0f32, 0.0f32);
    egui::Area::new(egui::Id::new("drive-stick"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-MARGIN, -MARGIN))
        .show(ctx, |ui| {
            let size = egui::vec2(RADIUS * 2.0, RADIUS * 2.0);
            let (resp, painter) = ui.allocate_painter(size, egui::Sense::drag());
            let center = resp.rect.center();
            let mut knob = egui::Vec2::ZERO;
            if resp.dragged()
                && let Some(pos) = resp.interact_pointer_pos()
            {
                let v = pos - center;
                let max = RADIUS - 6.0;
                let len = v.length();
                knob = if len > max { v * (max / len) } else { v };
            }
            painter.circle_filled(center, RADIUS, egui::Color32::from_black_alpha(90));
            painter.circle_stroke(
                center,
                RADIUS,
                egui::Stroke::new(2.0_f32, egui::Color32::from_white_alpha(70)),
            );
            painter.circle_filled(
                center + knob,
                RADIUS * 0.38,
                egui::Color32::from_white_alpha(150),
            );
            let reach = RADIUS - 6.0;
            let nx = (knob.x / reach).clamp(-1.0, 1.0);
            let ny = (knob.y / reach).clamp(-1.0, 1.0);
            const DEAD: f32 = 0.12;
            if nx.abs() > DEAD || ny.abs() > DEAD {
                // egui +Y is down; W is motor +, A is rudder +.
                drive = (-ny, -nx);
            }
        });
    drive
}

/// Use / Space, to the left of the stick so a thumb can hit it without
/// fighting the knob. Held like the key: press edge opens a pad, or
/// leaves the shop.
fn draw_use_button(ctx: &egui::Context) -> bool {
    const RADIUS: f32 = USE_RADIUS;
    let x = -(TOUCH_MARGIN + STICK_RADIUS * 2.0 + TOUCH_GAP);
    let mut held = false;
    egui::Area::new(egui::Id::new("use-button"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(x, -TOUCH_MARGIN))
        .show(ctx, |ui| {
            let size = egui::vec2(RADIUS * 2.0, RADIUS * 2.0);
            let (resp, painter) = ui.allocate_painter(size, egui::Sense::click_and_drag());
            held = resp.is_pointer_button_down_on();
            let fill = if held {
                egui::Color32::from_white_alpha(50)
            } else {
                egui::Color32::from_black_alpha(90)
            };
            let center = resp.rect.center();
            painter.circle_filled(center, RADIUS, fill);
            painter.circle_stroke(
                center,
                RADIUS,
                egui::Stroke::new(2.0_f32, egui::Color32::from_white_alpha(70)),
            );
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                "USE",
                egui::FontId::proportional(16.0),
                egui::Color32::from_white_alpha(200),
            );
        });
    held
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
    /// Shared with the visibility listener so a hidden tab can restart
    /// the rAF chain when it comes back (Wait does not poll).
    window_slot: std::rc::Rc<std::cell::RefCell<Option<Arc<Window>>>>,
    gpu: Option<GpuState>,
    /// Shared slot for async WASM GPU init to deliver results.
    gpu_pending: std::rc::Rc<std::cell::RefCell<Option<GpuState>>>,
    screen_size: wgpu::Extent3d,
    keys_pressed: std::rc::Rc<std::cell::RefCell<std::collections::HashSet<KeyCode>>>,
    /// Keys pressed since the last frame, even if already released: on a
    /// slow page (software WebGL runs at about 1 fps) a tap of Alt starts
    /// and ends between two frames and never shows in `keys_pressed`.
    keys_tapped: std::rc::Rc<std::cell::RefCell<std::collections::HashSet<KeyCode>>>,
    ui_pointer: std::rc::Rc<std::cell::RefCell<Vec<UiPointer>>>,
    /// Document-level input listeners. itch.io hosts us in an iframe and
    /// winit only hears keys/clicks on the canvas, so we take them from
    /// `document` instead.
    _dom_input: Option<DomInputHooks>,
    last_frame: Option<Instant>,
    /// Set when the tab hides so the next frame does not inherit a huge
    /// pause or a stuck Ctrl (brake) from Ctrl+Tab.
    clock_stale: std::rc::Rc<std::cell::Cell<bool>>,
    ws_client: Option<net_ws::WsClient>,
    /// The first frame builds the whole terrain TIN and uploads it; time
    /// it once so the console shows where startup actually goes.
    first_draw_measured: bool,
    /// Status text overlay (used in multiplayer logging)
    #[allow(dead_code)]
    mp_status: String,
}

impl WebHandler {
    fn new() -> Self {
        let (ws_client, mp_status) = match server_url() {
            Some(url) => {
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
            window_slot: std::rc::Rc::new(std::cell::RefCell::new(None)),
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
            keys_tapped: std::rc::Rc::new(
                std::cell::RefCell::new(std::collections::HashSet::new()),
            ),
            ui_pointer: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            _dom_input: None,
            last_frame: None,
            clock_stale: std::rc::Rc::new(std::cell::Cell::new(false)),
            first_draw_measured: false,
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
        let taps = self.keys_tapped.clone();
        let on_keydown = Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
            if let Some(key) = key_from_code(&event.code()) {
                keys.borrow_mut().insert(key);
                if !event.repeat() {
                    taps.borrow_mut().insert(key);
                }
            }
            if browser_eats(&event.code()) {
                event.prevent_default();
            }
        }) as Box<dyn FnMut(web_sys::KeyboardEvent)>);

        let keys = self.keys_pressed.clone();
        let on_keyup = Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
            if let Some(key) = key_from_code(&event.code()) {
                keys.borrow_mut().remove(&key);
            }
            if browser_eats(&event.code()) {
                event.prevent_default();
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
                if !in_level_picker(&event) {
                    event.prevent_default();
                }
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

        let keys = self.keys_pressed.clone();
        let stale = self.clock_stale.clone();
        let on_blur = Closure::wrap(Box::new(move |_event: web_sys::Event| {
            release_held_keys(&mut keys.borrow_mut());
            stale.set(true);
        }) as Box<dyn FnMut(web_sys::Event)>);

        let keys = self.keys_pressed.clone();
        let stale = self.clock_stale.clone();
        let slot = self.window_slot.clone();
        let on_visibility = Closure::wrap(Box::new(move |_event: web_sys::Event| {
            let hidden = web_sys::window()
                .and_then(|w| w.document())
                .is_some_and(|d| d.hidden());
            if hidden {
                release_held_keys(&mut keys.borrow_mut());
                stale.set(true);
            } else if let Some(ref window) = *slot.borrow() {
                window.request_redraw();
            }
        }) as Box<dyn FnMut(web_sys::Event)>);

        let win = web_sys::window().expect("no window");
        let _ = win.add_event_listener_with_callback("blur", on_blur.as_ref().unchecked_ref());
        let _ = document.add_event_listener_with_callback(
            "visibilitychange",
            on_visibility.as_ref().unchecked_ref(),
        );

        self._dom_input = Some(DomInputHooks {
            _keydown: on_keydown,
            _keyup: on_keyup,
            _pointerdown: on_pointerdown,
            _pointerup: on_pointerup,
            _pointermove: on_pointermove,
            _blur: on_blur,
            _visibility: on_visibility,
        });
    }
}

/// Keys the browser acts on by default: Space scrolls, the arrows scroll,
/// and a lone Alt (jump) moves focus to the browser's menu on Windows and
/// some Linux desktops, after which the page hears no keys at all.
fn browser_eats(code: &str) -> bool {
    matches!(code, "Space" | "AltLeft" | "AltRight" | "F2") || code.starts_with("Arrow")
}

/// Ctrl+Tab (and any other chord the browser eats) never delivers keyup.
/// ControlLeft is brake, so a leftover Ctrl parks the car until something
/// clears it. Jump still works because Alt is a fresh press.
fn release_held_keys(keys: &mut std::collections::HashSet<KeyCode>) {
    keys.clear();
}

impl ApplicationHandler for WebHandler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Wait + request_redraw is a requestAnimationFrame chain. Poll
        // on Firefox uses scheduler.postTask with a fresh AbortController
        // every iteration; dropping the previous one fires `abort` and
        // the cycle collector later walks millions of WebTasks.
        event_loop.set_control_flow(ControlFlow::Wait);

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
                                    world: String,
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
            let color_format = render::pick_surface_format(&caps.formats);
            log::info!("Color format: {color_format:?}");
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: color_format,
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
                Some((vfs, ini_path)) => {
                    WebApp::new_from_vfs(&gfx, &vfs, &ini_path, &world, is_webgpu)
                }
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

        self.window = Some(window.clone());
        *self.window_slot.borrow_mut() = Some(window.clone());
        window.request_redraw();
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
                level_id,
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
            WindowEvent::Focused(false) => {
                release_held_keys(&mut self.keys_pressed.borrow_mut());
                self.last_frame = None;
            }
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput {
                event:
                    event::KeyEvent {
                        physical_key: PhysicalKey::Code(key),
                        state,
                        repeat,
                        ..
                    },
                ..
            } => match state {
                event::ElementState::Pressed => {
                    self.keys_pressed.borrow_mut().insert(key);
                    if !repeat {
                        self.keys_tapped.borrow_mut().insert(key);
                    }
                    if key == KeyCode::KeyC
                        && let Some(ref mut gpu) = self.gpu
                        && let Some(ref mut agent) = gpu.app.agent
                    {
                        agent.dynamo.flotator = !agent.dynamo.flotator;
                        log::info!(
                            "cutterig {}",
                            if agent.dynamo.flotator { "on" } else { "off" }
                        );
                    }
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

                // Next frame via requestAnimationFrame. Do not also Poll:
                // that path is scheduler.postTask + AbortController.abort.
                if self.gpu.is_some()
                    && let Some(ref window) = self.window
                {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
        // Kick the rAF chain once GPU init has posted a pending state,
        // or we would sit in Wait until a DOM event.
        if self.gpu.is_none()
            && self.gpu_pending.borrow().is_some()
            && let Some(ref window) = self.window
        {
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
        if self.clock_stale.get() {
            self.clock_stale.set(false);
            self.last_frame = None;
        }
        let now = Instant::now();
        let dt = match self.last_frame {
            Some(prev) => (now - prev).as_secs_f32(),
            None => 1.0 / 60.0,
        };
        self.last_frame = Some(now);
        // Multiplayer quanta keep to real time even on a slow page
        // (software WebGL draws about one frame a second): the server runs
        // the car at real time too, and a client that sends less than that
        // starves its car there (see `InputQueue::STARVE_TICKS`).
        let mp_dt = dt.min(MP_FRAME_MAX);
        let dt = dt.min(0.1);

        // Derive motor/rudder from the currently pressed keys. Same
        // axis mapping in both "driving a vehicle" and "server input"
        // modes, so the multiplayer branch below can use them too.
        let keys = self.keys_pressed.borrow();
        let taps = std::mem::take(&mut *self.keys_tapped.borrow_mut());
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
        {
            let (sm, sr) = gpu.app.stick;
            if sm.abs() > 0.01 {
                motor = sm;
            }
            if sr.abs() > 0.01 {
                rudder = sr;
            }
        }
        // A tap shorter than a frame still counts for this frame.
        let down = |key: KeyCode| keys.contains(&key) || taps.contains(&key);
        let brake = down(KeyCode::ControlLeft);
        let space = keys.contains(&KeyCode::Space) || gpu.app.use_held;
        let turbo = down(KeyCode::ShiftLeft);
        // Roll direction matches native: Q/E are scaled by cam.scale.y
        // (which is -1) so Q → +1, E → -1.
        let mut roll = 0.0f32;
        if keys.contains(&KeyCode::KeyQ) {
            roll = -gpu.app.cam.scale.y;
        }
        if keys.contains(&KeyCode::KeyE) {
            roll = gpu.app.cam.scale.y;
        }
        // Jump: Alt as in the native game, or J.
        let jump_held = keys.contains(&KeyCode::AltLeft) || keys.contains(&KeyCode::KeyJ);
        let jump_tapped = taps.contains(&KeyCode::AltLeft) || taps.contains(&KeyCode::KeyJ);
        if taps.contains(&KeyCode::F2) {
            gpu.app.net_debug.visible = !gpu.app.net_debug.visible;
        }
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

        // Multiplayer: join with the player's car once the socket is open,
        // and start over when it closes.
        if let Some(ref mut ws) = self.ws_client
            && ws.is_connected()
            && !ws.is_joined()
        {
            let (name, color) = player_identity();
            let car_name = gpu.app.agent.as_ref().map_or("", |a| a.car_name.as_str());
            ws.join(&name, car_name, color as u8);
        }
        if !connected && (gpu.app.mp.synced || !gpu.app.remotes.is_empty()) {
            gpu.app.mp.disconnect();
            gpu.app.remotes.clear();
        }
        let mp_on = self.ws_client.as_ref().is_some_and(|c| c.is_joined());
        // Match the native build's time scaling (see bin/road/game.rs):
        //   physics_dt = delta * fps * time_delta0 * num_calls
        let physics_dt = if mp_on { mp_dt } else { dt } * {
            let common = &gpu.app.common;
            let n = &common.nature;
            common.speed.standard_frame_rate as f32 * n.time_delta0 * n.num_calls_analysis as f32
        };
        // Multiplayer steps the car in whole quanta, numbered for the
        // server's input queue and the reconcile (`physics::client`, as in
        // the native game).
        let quanta = if mp_on {
            gpu.app.mp.due_quanta(physics_dt, gpu.app.max_quant)
        } else {
            0
        };
        // This frame's input batch: first quantum and the last quantum's
        // input (held controls with the jump and roll).
        let mut batch: Option<(u32, physics::prediction::QuantInput)> = None;
        // The car is not driven this frame (escave screen, train ride).
        let mut held = true;

        // Moving land first, so the car drives on this quant's surface
        // (same order as the native game).
        gpu.app.step_moving_land(dt, connected);
        if space && !gpu.app.space_held {
            gpu.app.try_use();
        }
        gpu.app.space_held = space;
        if !gpu.app.screen.is_world() {
            gpu.app.step_screen(dt);
        }
        if gpu.app.screen.blocks_drive() {
            if gpu.app.screen.follows_camera()
                && let Some(ref agent) = gpu.app.agent
            {
                let follow = gpu.app.follow;
                let target = agent.transform;
                let reach = space::Subject::reach_of(&agent.phys_data.bbox, agent.car.scale);
                gpu.app.cam.chase_anchored(
                    &gpu.app.level,
                    &target,
                    space::Subject::unanchored(&target, reach),
                    dt,
                    &follow,
                    CAMERA_CLEARANCE,
                );
            }
        } else {
            let riding = gpu.app.step_ride(dt);
            if !riding {
                gpu.app.begin_train_ride();
            }
            if gpu.app.ride.is_none() {
                gpu.app.try_begin_escave_visit();
            }

            held = riding;
            if gpu.app.agent.is_some() {
                // Drive the player vehicle. Keyboard feeds control; physics
                // integrates the dynamo; the camera chases the transform.
                // We take a mutable reborrow scope so the follow-camera
                // update below can read from gpu.app too.
                let common = gpu.app.common;
                let level_ref = &gpu.app.level;
                let mut follow = gpu.app.follow;
                let max_quant = gpu.app.max_quant;
                if let Some(ref mut agent) = gpu.app.agent {
                    agent.control.motor = motor;
                    agent.control.rudder = rudder;
                    agent.control.brake = brake;
                    agent.control.turbo = turbo;
                    agent.control.roll = roll;
                    // Jump: charge while Alt is held, fire on release. A
                    // tap within one frame fires with one frame's charge.
                    let frame_power = dt * common.speed.standard_frame_rate as f32;
                    if jump_held {
                        let charge = agent.control.jump_charge.get_or_insert(0.0);
                        *charge = (*charge + frame_power).min(common.force.max_jump_power);
                    } else if let Some(power) = agent.control.jump_charge.take() {
                        agent.control.jump = Some(power);
                    } else if jump_tapped {
                        agent.control.jump = Some(frame_power.min(common.force.max_jump_power));
                    }
                    // Match the native build's time scaling (see bin/road/game.rs):
                    //   input_factor = delta / MAIN_LOOP_TIME
                    let input_factor = dt / config::common::MAIN_LOOP_TIME;
                    if mp_on && !riding {
                        // Whole quanta, as the server runs them: the jump
                        // and roll on the last one, each quantum folded
                        // home and bumped off the remote ghosts, then kept
                        // for the reconcile.
                        let input = physics::prediction::QuantInput {
                            motor,
                            rudder,
                            brake,
                            turbo,
                            jump: if quanta > 0 {
                                agent.control.jump.take()
                            } else {
                                None
                            },
                            roll,
                        };
                        let first = gpu.app.mp.prediction.next();
                        let sim = physics::client::Sim {
                            level: level_ref,
                            common: &common,
                            max_quant,
                        };
                        for i in 0..quanta {
                            let input = physics::client::ClientSync::frame_input(&input, i, quanta);
                            let shift = gpu.app.mp.step_quantum(
                                physics::client::LocalCar {
                                    transform: &mut agent.transform,
                                    dynamo: &mut agent.dynamo,
                                    phys: &agent.phys_data,
                                },
                                &input,
                                &gpu.app.remotes,
                                sim,
                                Some(&mut agent.tracks),
                            );
                            // The car moved to another copy of the torus:
                            // the camera goes along.
                            gpu.app.cam.loc += shift;
                            batch = Some((first, input));
                        }
                    } else if !riding {
                        agent.apply_control(input_factor, &common);
                        let mut left = physics_dt;
                        while left > max_quant {
                            agent.physics_step(max_quant, level_ref, &common);
                            left -= max_quant;
                        }
                        agent.physics_step(left, level_ref, &common);
                    }
                    // A small speed-sensitive pullback gives a sense of speed
                    // without shrinking the vehicle into the distance.
                    let speed_xy = {
                        let v = agent.dynamo.linear_velocity;
                        (v.x * v.x + v.y * v.y).sqrt()
                    };
                    follow.offset.y += (speed_xy * 0.03).min(3.0);
                    follow.look_ahead += (speed_xy * 0.04).min(4.0);
                    let target = gpu.app.mp.drawn(&agent.transform);
                    let reach = space::Subject::reach_of(&agent.phys_data.bbox, agent.car.scale);
                    gpu.app.cam.chase_anchored(
                        level_ref,
                        &target,
                        space::Subject::unanchored(&target, reach),
                        dt,
                        &follow,
                        CAMERA_CLEARANCE,
                    );
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

            // Cut the wheels' tracks into the level after the physics ran.
            gpu.app.step_tracks();
            gpu.app.step_life(dt);
        }

        // Story cycles run whether or not the escave UI is up so solo
        // palette fades and MP fade catch-up stay consistent.
        gpu.app.step_cycle(dt, connected);

        // Multiplayer: a car that was not driven still uses up its quanta
        // (numbered, no history), with a released input.
        if mp_on && batch.is_none() && (held || gpu.app.agent.is_none()) {
            let first = gpu.app.mp.skip(quanta);
            batch = Some((first, physics::prediction::QuantInput::default()));
        }

        // Process multiplayer messages
        if let Some(ref mut ws) = self.ws_client {
            if let Some((first, input)) = batch {
                ws.send_input(first, quanta, &physics::client::net_control(&input));
            }

            let messages = ws.poll();
            // Only the newest snapshot reconciles the local car.
            let newest_world = messages
                .iter()
                .rposition(|m| matches!(m, vangers_net::ServerMessage::WorldState { .. }));
            for (index, msg) in messages.into_iter().enumerate() {
                match msg {
                    vangers_net::ServerMessage::Welcome {
                        player_id,
                        level_name,
                        ..
                    } => {
                        self.mp_status =
                            format!("Connected (player {}, level '{}')", player_id, level_name);
                        ws.player_id = Some(player_id);
                        gpu.app
                            .net_debug
                            .note(log::Level::Info, self.mp_status.clone());
                        // The page picks its own level (and swaps big
                        // levels for Khox on GPUs capped at 8192 px, as
                        // software WebGL is). Physics on another map
                        // can't agree with the server, so say so.
                        let here = gpu.app.world_name.clone();
                        if !level_name.eq_ignore_ascii_case(&here) {
                            let line = format!(
                                "Server runs '{}' but this page loaded '{}'; positions won't match",
                                level_name, here
                            );
                            self.mp_status += &format!(" - level mismatch, here '{}'", here);
                            gpu.app.net_debug.note(log::Level::Warn, line);
                        }
                    }
                    vangers_net::ServerMessage::PlayerJoined {
                        player_id,
                        player_name,
                        car_name,
                        color,
                    } => {
                        gpu.app.net_debug.note(
                            log::Level::Info,
                            format!(
                                "Player {} ({}) joined in {}",
                                player_id, player_name, car_name
                            ),
                        );
                        if Some(player_id) == ws.player_id {
                            continue;
                        }
                        let car = gpu
                            .app
                            .cars
                            .as_mut()
                            .and_then(|cars| {
                                cars.get(&car_name, &gpu.device, &gpu.app.render.object)
                            })
                            .or_else(|| {
                                log::warn!("No car '{}' here, drawing our own", car_name);
                                gpu.app.agent.as_ref().map(|a| a.car.clone())
                            });
                        let Some(car) = car else {
                            continue;
                        };
                        let phys = physics::CarPhysicsData::from_car_info(&car);
                        let color = render::object::BodyColor::from_value(color);
                        gpu.app
                            .remotes
                            .entry(player_id)
                            .and_modify(|remote| {
                                remote.name = player_name.clone();
                                remote.net.phys_data = phys.clone();
                                remote.car = car.clone();
                                remote.color = color;
                            })
                            .or_insert_with(|| WebRemote {
                                name: player_name.clone(),
                                car: car.clone(),
                                color,
                                net: physics::client::RemoteCar::new(phys.clone()),
                            });
                    }
                    vangers_net::ServerMessage::PlayerLeft { player_id } => {
                        let name = gpu.app.remotes.remove(&player_id).map(|r| r.name);
                        gpu.app.net_debug.note(
                            log::Level::Info,
                            format!("Player {} ({:?}) left", player_id, name),
                        );
                    }
                    vangers_net::ServerMessage::WorldState {
                        agents,
                        cycle,
                        moving,
                        ..
                    } => {
                        if let Some(ref cycle_state) = cycle {
                            gpu.app.apply_cycle_state(cycle_state);
                            gpu.app.apply_player_cirt(ws.player_id, cycle_state);
                        }
                        if let Some(ref moving_state) = moving {
                            gpu.app.apply_moving_land_state(moving_state);
                        }
                        // Remote cars first: a replay of the local car
                        // bumps against their ghosts.
                        let period = Some(gpu.app.level.period());
                        for agent_state in &agents {
                            if Some(agent_state.player_id) == ws.player_id {
                                continue;
                            }
                            if let Some(remote) = gpu.app.remotes.get_mut(&agent_state.player_id) {
                                remote
                                    .net
                                    .on_snapshot(&physics::client::server_car(agent_state), period);
                            }
                        }
                        if Some(index) != newest_world {
                            continue;
                        }
                        let Some(me) = ws
                            .player_id
                            .and_then(|my_id| agents.iter().find(|a| a.player_id == my_id))
                        else {
                            continue;
                        };
                        let snap = physics::client::LocalSnapshot::from_agent(me);
                        gpu.app.net_debug.server = Some((
                            snap.car.transform.disp,
                            snap.ack,
                            snap.car.linear_velocity.length(),
                        ));
                        if let Some(ref mut agent) = gpu.app.agent {
                            // Compare with what we predicted at the ack; on
                            // a mismatch rewind there and replay our inputs
                            // since, drawing the move as a decaying offset.
                            let (event, bumped) = gpu.app.mp.on_snapshot(
                                &snap,
                                physics::client::LocalCar {
                                    transform: &mut agent.transform,
                                    dynamo: &mut agent.dynamo,
                                    phys: &agent.phys_data,
                                },
                                &gpu.app.remotes,
                                physics::client::Sim {
                                    level: &gpu.app.level,
                                    common: &gpu.app.common,
                                    max_quant: gpu.app.max_quant,
                                },
                                held,
                            );
                            let debug = &mut gpu.app.net_debug;
                            match event {
                                physics::client::SyncEvent::FirstSync => {
                                    gpu.app.cam.focus_on(&snap.car.transform);
                                    debug.outcome = "first sync".to_string();
                                }
                                physics::client::SyncEvent::Reconciled {
                                    outcome,
                                    moved,
                                    shown_at_once,
                                } => {
                                    debug.outcome = format!("{:?}, moved {:.1}u", outcome, moved);
                                    if shown_at_once {
                                        debug.note(
                                            log::Level::Warn,
                                            format!(
                                                "Server correction {:.1}u: shown at once",
                                                moved
                                            ),
                                        );
                                    }
                                    if bumped {
                                        debug.note(
                                            log::Level::Info,
                                            format!(
                                                "Server bump: {:?} bumps={}",
                                                outcome, me.bumps
                                            ),
                                        );
                                    }
                                }
                                physics::client::SyncEvent::Held => {
                                    debug.outcome = "held (not reconciled)".to_string();
                                }
                            }
                        } else {
                            // No local vehicle mesh — at least park the camera.
                            let pos = snap.car.transform.disp;
                            gpu.app.cam.loc = glam::vec3(pos.x, pos.y, pos.z + 200.0);
                        }
                    }
                }
            }
        }
        for remote in gpu.app.remotes.values_mut() {
            remote.net.interpolate(dt);
        }
        if connected {
            gpu.app.mp.decay(dt);
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
        let t = Instant::now();
        let command_buffer = gpu.app.draw(&gpu.device, &gpu.queue, targets);
        if !self.first_draw_measured {
            self.first_draw_measured = true;
            // The mesh renderer builds the TIN lazily on the first update,
            // so the first frame pays for the whole level's fit + upload.
            log::info!(
                "[startup] first frame (terrain TIN build + upload) took {:.0} ms",
                t.elapsed().as_secs_f32() * 1e3
            );
        }

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
        let full_output = {
            let app = &mut gpu.app;
            gpu.egui_state.egui_ctx().run_ui(raw_input, |ctx| {
                app.draw_ui(ctx);
            })
        };
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

    use winit::platform::web::{EventLoopExtWebSys, PollStrategy};
    // If anything sets ControlFlow::Poll, skip scheduler.postTask (the
    // default). IdleCallback cancels with cancelIdleCallback, not abort.
    event_loop.set_poll_strategy(PollStrategy::IdleCallback);
    event_loop.spawn_app(handler);
}

fn main() {
    web_main();
}

#[cfg(test)]
mod input_tests {
    use super::*;

    #[test]
    fn hiding_the_tab_releases_brake_and_drive() {
        let mut keys = std::collections::HashSet::from([KeyCode::ControlLeft, KeyCode::KeyW]);
        release_held_keys(&mut keys);
        assert!(keys.is_empty());
    }
}
