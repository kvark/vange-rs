use vangers::{
    combat, config, level,
    physics::{
        self,
        prediction::{InputQueue, QuantInput, TickBudget},
        CarPhysicsData, Dynamo,
    },
    space, weapon,
};
use vangers_net::{
    decode, encode, AgentState, ClientMessage, CycleFade, CycleState, LocationPhase,
    MovingLandState, NetControl, NetDynamo, NetTransform, PlayerCirt, PlayerId, ServerMessage,
    PROTOCOL_VERSION,
};

use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use glam::{Vec2, Vec3};
use log::{debug, error, info, warn};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    time::{self, Duration},
};
use tokio_tungstenite::tungstenite;

/// Vangers headless multiplayer server
#[derive(Parser)]
struct Cli {
    /// TCP port to listen on
    #[arg(short, long, default_value = "7800")]
    port: u16,

    /// WebSocket port to listen on (for WASM clients)
    #[arg(long, default_value = "7801")]
    ws_port: u16,

    /// Maximum number of players
    #[arg(long, default_value = "8")]
    max_players: usize,

    /// Server tick rate in Hz
    #[arg(long, default_value = "20")]
    tick_rate: u32,

    /// Level to host: `test`, a `world.ini` path, or a world name from `wrlds.dat`.
    /// When omitted, uses `settings.game.level` (resolved to `world.ini`) if game
    /// data is present; otherwise the procedural `test` level (CI / no data).
    #[arg(short, long)]
    level: Option<String>,

    /// Path to settings file (default: config/settings.ron)
    #[arg(short, long, default_value = "config/settings.ron")]
    settings: String,

    /// Dev / ram-test affordance: place the first two fresh joiners
    /// `DUEL_SPACING` apart on `--duel-center`, facing each other along X
    /// (west car faces east, east car faces west). Later joiners and
    /// reclaimed poses are unaffected. Off by default.
    #[arg(long)]
    duel_spawn: bool,

    /// Map XY midpoint for `--duel-spawn`. The default is open flat sand on
    /// Fostral (terrain height 0-2, no slabs across a 50x16 box).
    #[arg(long, value_parser = parse_xy, default_value = "590,4151")]
    duel_center: (i32, i32),

    /// Dev / camera-test affordance: place the first fresh joiner at map
    /// `X,Y`, on the top surface there (a roof, not the cave under it),
    /// facing -Y. Overrides `--duel-spawn` for that joiner only. Fostral's
    /// roof over the rock channel east of the arch: `1030,3985`.
    #[arg(long, value_parser = parse_xy)]
    spawn_at: Option<(i32, i32)>,

    /// An escave entrance at map `X,Y` for wrecks to respawn by, on top of
    /// the hosted world's own from `escaves.prm` / `spots.prm` (the `test`
    /// level has none). Repeatable.
    #[arg(long = "escave", value_parser = parse_xy)]
    escaves: Vec<(i32, i32)>,

    /// Debug: hold every message this many milliseconds each way, to try
    /// the multiplayer prediction on localhost under a real-world ping
    /// (the round trip is twice this plus jitter).
    #[arg(long, default_value = "0")]
    sim_latency_ms: u64,

    /// Debug: up to this many extra milliseconds per message, random (the
    /// order is kept), on top of `--sim-latency-ms`.
    #[arg(long, default_value = "0")]
    sim_jitter_ms: u64,

    /// Debug: take `ClientMessage::SetPose` (the Tweaks Position apply,
    /// `--autodrive tp:`): a client may then put its car anywhere. Off by
    /// default; still never for a wreck or just after a respawn.
    #[arg(long)]
    allow_setpose: bool,
}

/// Parse `X,Y` map coordinates.
fn parse_xy(s: &str) -> Result<(i32, i32), String> {
    let (x, y) = s
        .split_once(',')
        .ok_or_else(|| format!("expected X,Y, got {:?}", s))?;
    let parse = |v: &str| {
        v.trim()
            .parse::<i32>()
            .map_err(|e| format!("{:?}: {}", v, e))
    };
    Ok((parse(x)?, parse(y)?))
}

/// Events from client connection tasks to the main game loop.
///
/// `conn_id` is assigned at accept time. On `Join`, the server may map it to a
/// previously used `player_id` when the same `--name` reconnects (see
/// `name_ids` / `conn_to_player` in the game loop).
enum SessionEvent {
    Connected {
        conn_id: PlayerId,
        sender: mpsc::UnboundedSender<Vec<u8>>,
    },
    Message {
        conn_id: PlayerId,
        msg: ClientMessage,
    },
    Disconnected {
        conn_id: PlayerId,
    },
}

/// How long a disconnected player's last pose / cycle carry is kept for reclaim.
const RECONNECT_POSE_GRACE: Duration = Duration::from_secs(5 * 60);

/// Last per-player snapshot after leave/disconnect, restored on same-name reclaim.
///
/// Holds WorldState pose (transform/dynamo), story-cycle carry (`cirtainer`),
/// and spiral charge. Shared world cycle banks stay on the server `Bunch`.
struct LastPose {
    transform: space::Transform,
    dynamo: Dynamo,
    cirtainer: level::cycle::Cirtainer,
    spiral_charge: u8,
    armor: u16,
    disconnected_at: Instant,
}

/// Optional test-only spiral charge (`VANGERS_TEST_SEED_SPIRAL=3`).
///
/// Fresh spawn only — reclaim must restore `LastPose.spiral_charge`.
fn test_seed_spiral() -> Option<u8> {
    let raw = std::env::var("VANGERS_TEST_SEED_SPIRAL").ok()?;
    raw.trim().parse::<u8>().ok()
}

/// Optional test-only cirtainer seed (`VANGERS_TEST_SEED_CIRT=7,0,2`).
///
/// Applied on **fresh** spawn only so integration tests can prove reclaim
/// restores `LastPose.cirtainer` rather than re-applying the seed.
fn test_seed_cirtainer() -> Option<level::cycle::Cirtainer> {
    let raw = std::env::var("VANGERS_TEST_SEED_CIRT").ok()?;
    let held: Vec<i32> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<i32>())
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if held.is_empty() {
        None
    } else {
        Some(level::cycle::Cirtainer::from_held(held))
    }
}

/// Server-side agent with full physics state.
struct ServerAgent {
    name: String,
    car_name: String,
    color: u8,
    /// The client's numbered per-quantum inputs, run in order; its `ack`
    /// goes out in `AgentState::ack` for the client's reconcile.
    inputs: InputQueue,
    transform: space::Transform,
    dynamo: Dynamo,
    phys_data: CarPhysicsData,
    cirtainer: level::cycle::Cirtainer,
    /// Spiral charge slots filled; capacity is client-local / car-dependent.
    spiral_charge: u8,
    sender: mpsc::UnboundedSender<Vec<u8>>,
    joined: bool,
    /// Felt car-car bumps taken (`physics::FELT_BUMP_DV`), sent in
    /// `AgentState::bumps` so the owning client adopts the hit.
    bumps: u32,
    /// Follow-up pose logs after the last jump.
    jump_trace: Option<JumpTrace>,
    /// Throttled log of the brake/turbo/jump the client sends.
    input_log: InputLog,
    /// When this car's last `Pose` line went out.
    pose_logged: Option<Instant>,
    /// Quanta this car was driven on with its held control since its
    /// client went quiet (`InputQueue::STARVE_TICKS`), unacked: motion the
    /// client never predicted. `None` while its inputs arrive.
    starved: Option<u32>,
    /// Shots taken from this player this tick (`weapon::FIRES_PER_TICK`).
    fires: u8,
    /// Armour left (`AgentState::armor`); shots resolved here take it
    /// down to 0.
    armor: u16,
    max_armor: u16,
    /// The car's positions over the last `combat::HISTORY_SECS`, to test
    /// shots against it where their shooters saw it.
    history: combat::PoseHistory,
    /// Throttled summaries of this player's shots and of the hits it took.
    fire_log: FireLog,
    hit_log: HitLog,
    /// Out of armour: a wreck until `Wreck::respawn` (`AgentState::dead`).
    dead: Option<combat::Wreck<PlayerId>>,
    /// The tick it last respawned in: no pose from the client is taken
    /// for `combat::RESPAWN_POSE_LOCK_SECS` after.
    respawned: Option<u32>,
}

/// Diagnostic: a player's accepted shots, summed into at most one info
/// line per [`INPUT_LOG_EVERY`] (each shot is a debug line; at two a tick
/// they would flood the log).
#[derive(Default)]
struct FireLog {
    last: Option<Instant>,
    shots: u32,
    rewind_max: f32,
}

impl FireLog {
    fn note(&mut self, player_id: PlayerId, name: &str, rewind: f32) {
        self.shots += 1;
        self.rewind_max = self.rewind_max.max(rewind);
        let now = Instant::now();
        if self.last.is_some_and(|t| now - t < INPUT_LOG_EVERY) {
            return;
        }
        info!(
            "Fire: p{} ({}) {} shot(s) since the last line, rewind up to {:.2} ticks",
            player_id, name, self.shots, self.rewind_max
        );
        *self = FireLog {
            last: Some(now),
            ..FireLog::default()
        };
    }
}

/// Diagnostic: hits a car took, summed into at most one info line per
/// [`HIT_LOG_EVERY`], and always the one that empties its armour.
#[derive(Default)]
struct HitLog {
    last: Option<Instant>,
    hits: u32,
    damage: u32,
}

const HIT_LOG_EVERY: Duration = Duration::from_millis(500);

impl HitLog {
    /// `emptied`: this hit took the last of the armour.
    fn note(
        &mut self,
        target: (PlayerId, &str),
        shooter: &str,
        damage: u16,
        armor: (u16, u16),
        emptied: bool,
    ) {
        self.hits += 1;
        self.damage += damage as u32;
        let now = Instant::now();
        if !emptied && self.last.is_some_and(|t| now - t < HIT_LOG_EVERY) {
            return;
        }
        info!(
            "Hit: p{} ({}) took {} hit(s) for {} since the last line, last from {}; armour {}/{}",
            target.0, target.1, self.hits, self.damage, shooter, armor.0, armor.1
        );
        *self = HitLog {
            last: Some(now),
            ..HitLog::default()
        };
    }
}

/// Diagnostic: the brake, turbo and jump a client sends, logged at info so
/// a session can be checked from the server log alone. A jump is logged
/// every time; held brake/turbo at most once per [`INPUT_LOG_EVERY`].
#[derive(Default)]
struct InputLog {
    last: Option<Instant>,
    brake: u32,
    turbo: u32,
}

const INPUT_LOG_EVERY: Duration = Duration::from_secs(1);
/// Between `Pose` lines of a car (its server pose, to compare with what a
/// client shows).
const POSE_LOG_EVERY: Duration = Duration::from_secs(5);

impl InputLog {
    fn note(
        &mut self,
        player_id: PlayerId,
        name: &str,
        first_quant: u32,
        quanta: u16,
        control: &NetControl,
    ) {
        if let Some(power) = control.jump {
            info!(
                "Input p{} ({}) jump power={:.1} at quant {}",
                player_id,
                name,
                power,
                first_quant.wrapping_add(quanta.saturating_sub(1) as u32)
            );
        }
        if !control.brake && !control.turbo {
            return;
        }
        if control.brake {
            self.brake += quanta as u32;
        }
        if control.turbo {
            self.turbo += quanta as u32;
        }
        let now = Instant::now();
        if self.last.is_some_and(|t| now - t < INPUT_LOG_EVERY) {
            return;
        }
        info!(
            "Input p{} ({}) brake={} turbo={} (quanta held since the last line)",
            player_id, name, self.brake, self.turbo
        );
        *self = InputLog {
            last: Some(now),
            ..InputLog::default()
        };
    }
}

/// Diagnostic: where a jump took the car, logged at info.
struct JumpTrace {
    tick: u32,
    start: Vec3,
}

/// Ticks between `Jump trace` lines (1 s at 20 Hz).
const JUMP_TRACE_EVERY: u32 = 20;
/// Stop tracing after this many ticks even if the car keeps moving.
const JUMP_TRACE_MAX: u32 = 200;
/// World speed under which a traced car counts as at rest.
const JUMP_TRACE_REST_SPEED: f32 = 0.2;

impl ServerAgent {
    fn to_agent_state(&self, player_id: PlayerId) -> AgentState {
        AgentState {
            player_id,
            transform: NetTransform {
                position: self.transform.disp.into(),
                rotation: [
                    self.transform.rot.x,
                    self.transform.rot.y,
                    self.transform.rot.z,
                    self.transform.rot.w,
                ],
                scale: self.transform.scale,
            },
            dynamo: NetDynamo {
                traction: self.dynamo.traction,
                rudder: self.dynamo.rudder,
                linear_velocity: self.dynamo.linear_velocity.into(),
                angular_velocity: self.dynamo.angular_velocity.into(),
            },
            spiral_charge: self.spiral_charge,
            bumps: self.bumps,
            ack: self.inputs.ack(),
            control: net_control_of(&self.inputs.last()),
            armor: self.armor,
            max_armor: self.max_armor,
            dead: self.dead.is_some(),
        }
    }

    fn world_velocity(&self) -> Vec3 {
        self.transform.rot * self.dynamo.linear_velocity
    }

    /// One-shot `Jump` line at the quantum the jump impulse is applied
    /// (pose before it, velocity after it).
    fn log_jump(&mut self, player_id: PlayerId, tick: u32, power: f32, before: Vec3) {
        let v = self.world_velocity();
        info!(
            "Jump tick {} p{} power={:.1} pos=({:.1},{:.1},{:.1}) vel=({:.2},{:.2},{:.2})",
            tick, player_id, power, before.x, before.y, before.z, v.x, v.y, v.z
        );
        self.jump_trace = Some(JumpTrace {
            tick,
            start: before,
        });
    }

    /// Log when the server starts driving this car on without its client's
    /// input (the client went quiet for `InputQueue::STARVE_TICKS`), and
    /// how far that went once input is back: the client did not predict
    /// those quanta, so its next snapshot corrects it.
    fn note_budget(&mut self, player_id: PlayerId, budget: &TickBudget) {
        match self.starved {
            None if budget.held > 0 => {
                info!(
                    "Input p{} ({}) starved: no input for {} ticks, driving on with the held control",
                    player_id,
                    self.name,
                    InputQueue::STARVE_TICKS
                );
                self.starved = Some(budget.held);
            }
            Some(ref mut held) if budget.real == 0 => *held += budget.held,
            Some(held) => {
                info!(
                    "Input p{} ({}) resumed after {} unacked held quanta",
                    player_id, self.name, held
                );
                self.starved = None;
            }
            None => {}
        }
    }

    /// A `Pose` line every [`POSE_LOG_EVERY`] while joined: the server's
    /// pose of the car, to hold against a client's HUD or debug overlay.
    fn log_pose(&mut self, player_id: PlayerId) {
        if !self.joined {
            return;
        }
        let now = Instant::now();
        if self.pose_logged.is_some_and(|t| now - t < POSE_LOG_EVERY) {
            return;
        }
        self.pose_logged = Some(now);
        let p = self.transform.disp;
        info!(
            "Pose p{} ({}) pos=({:.1},{:.1},{:.1}) speed={:.1} ack={}",
            player_id,
            self.name,
            p.x,
            p.y,
            p.z,
            self.dynamo.linear_velocity.length(),
            self.inputs.ack()
        );
    }

    /// `Jump trace` lines every second after a jump: at +1 s, +2 s, then
    /// on until the car is at rest (or `JUMP_TRACE_MAX`).
    fn trace_jump(&mut self, player_id: PlayerId, tick: u32, wrap: (f32, f32)) {
        let Some(ref trace) = self.jump_trace else {
            return;
        };
        let elapsed = tick.wrapping_sub(trace.tick);
        if elapsed == 0 || elapsed % JUMP_TRACE_EVERY != 0 {
            return;
        }
        let shortest = |d: f32, size: f32| d - size * (d / size).round();
        let pos = self.transform.disp;
        let d = pos - trace.start;
        let moved = Vec3::new(shortest(d.x, wrap.0), shortest(d.y, wrap.1), d.z);
        let v = self.world_velocity();
        let at_rest = v.length() < JUMP_TRACE_REST_SPEED;
        info!(
            "Jump trace +{:.1}s tick {} p{} pos=({:.1},{:.1},{:.1}) vel=({:.2},{:.2},{:.2}) moved=({:.1},{:.1},{:.1}){}",
            elapsed as f32 / JUMP_TRACE_EVERY as f32,
            tick,
            player_id,
            pos.x,
            pos.y,
            pos.z,
            v.x,
            v.y,
            v.z,
            moved.x,
            moved.y,
            moved.z,
            if at_rest { " at rest" } else { "" }
        );
        if (elapsed >= 2 * JUMP_TRACE_EVERY && at_rest) || elapsed >= JUMP_TRACE_MAX {
            self.jump_trace = None;
        }
    }
}

/// Resolved terrain + the name reported in Welcome / used for story cycles.
struct HostedLevel {
    /// Absolute or relative `world.ini`, or `None` for the procedural test level.
    ini_path: Option<std::path::PathBuf>,
    /// Welcome / cycle world name (`"test"`, `"Fostral"`, …).
    name: String,
}

impl HostedLevel {
    fn test() -> Self {
        Self {
            ini_path: None,
            name: "test".into(),
        }
    }

    fn is_test(&self) -> bool {
        self.ini_path.is_none()
    }
}

/// Resolve `--level` / settings into terrain + a name that matches what clients load.
fn resolve_hosted_level(
    cli_level: Option<&str>,
    settings: Option<&config::Settings>,
) -> HostedLevel {
    match cli_level {
        Some("test") => HostedLevel::test(),
        Some(arg) => resolve_level_arg(arg, settings),
        None => resolve_default_level(settings),
    }
}

fn resolve_default_level(settings: Option<&config::Settings>) -> HostedLevel {
    let Some(settings) = settings else {
        return HostedLevel::test();
    };
    let name = settings.game.level.trim();
    if name.is_empty() {
        return HostedLevel::test();
    }
    match resolve_world_ini(settings, name) {
        Some(hosted) => hosted,
        None => {
            warn!(
                "settings.game.level={:?} but world.ini not found under {:?} — falling back to test",
                name, settings.data_path
            );
            HostedLevel::test()
        }
    }
}

fn resolve_level_arg(arg: &str, settings: Option<&config::Settings>) -> HostedLevel {
    let path = std::path::Path::new(arg);
    let looks_like_path =
        path.is_file() || arg.ends_with(".ini") || arg.contains('/') || arg.contains('\\');
    if looks_like_path {
        let resolved = resolve_ini_filesystem_path(path, settings);
        let name = world_name_for_ini_path(&resolved, settings).unwrap_or_else(|| arg.to_string());
        return HostedLevel {
            ini_path: Some(resolved),
            name,
        };
    }
    // World name (e.g. Fostral / Glorx)
    if let Some(settings) = settings {
        if let Some(hosted) = resolve_world_ini(settings, arg) {
            return hosted;
        }
        warn!(
            "Unknown world name {:?} under {:?} — treating as world.ini path",
            arg, settings.data_path
        );
    }
    HostedLevel {
        ini_path: Some(path.to_path_buf()),
        name: arg.to_string(),
    }
}

/// Prefer an existing path as given; otherwise try under `settings.data_path`.
fn resolve_ini_filesystem_path(
    path: &std::path::Path,
    settings: Option<&config::Settings>,
) -> std::path::PathBuf {
    if path.is_file() {
        return path.to_path_buf();
    }
    if let Some(settings) = settings {
        let under_data = settings.data_path.join(path);
        if under_data.is_file() {
            return under_data;
        }
    }
    path.to_path_buf()
}

fn resolve_world_ini(settings: &config::Settings, name: &str) -> Option<HostedLevel> {
    let worlds = config::worlds::try_load_from_path(&settings.data_path)?;
    let path = config::worlds::ini_path_if_present(&settings.data_path, &worlds, name)?;
    let display = config::worlds::canonical_name(&worlds, name)
        .unwrap_or(name)
        .to_string();
    Some(HostedLevel {
        ini_path: Some(path),
        name: display,
    })
}

/// Prefer the settings / wrlds.dat key when `ini` matches a known world.
fn world_name_for_ini_path(
    ini: &std::path::Path,
    settings: Option<&config::Settings>,
) -> Option<String> {
    let settings = settings?;
    let worlds = config::worlds::try_load_from_path(&settings.data_path)?;
    // Exact match against resolved paths
    for (name, rel) in &worlds {
        let candidate = settings.data_path.join(rel);
        if paths_equal(&candidate, ini) {
            return Some(name.clone());
        }
    }
    // settings.game.level if it resolves to the same file
    let level = settings.game.level.trim();
    if !level.is_empty() {
        if let Some(path) = config::worlds::ini_path_if_present(&settings.data_path, &worlds, level)
        {
            if paths_equal(&path, ini) {
                return Some(
                    config::worlds::canonical_name(&worlds, level)
                        .unwrap_or(level)
                        .to_string(),
                );
            }
        }
    }
    None
}

fn paths_equal(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

/// Distance between the two `--duel-spawn` car centres. A mechos contact
/// box is ~40u long (OxidizeMonk: 224 model units x 0.227 x `scale_box`
/// 0.8), so 70 leaves a ~30u gap between the bumpers; the old 30 had the
/// noses overlapping once car-car contact exists.
const DUEL_SPACING: i32 = 70;

/// Map XY and yaw (radians about Z) for a fresh join.
///
/// Default: `find_spawn_point` grid, yaw PI (facing -Y). With `duel`, joiner
/// 0 sits west of the center facing +X and joiner 1 east facing -X, so a ram
/// test needs no long drive. `spawn_at` puts joiner 0 there instead, facing
/// -Y. The car's forward axis is local +Y. `occupied`: the map XY of the
/// cars in the world, which a grid spawn keeps clear of.
fn spawn_pose(
    level: &level::Level,
    index: usize,
    duel: Option<(i32, i32)>,
    spawn_at: Option<(i32, i32)>,
    occupied: &[Vec2],
) -> ((i32, i32), f32) {
    use std::f32::consts::{FRAC_PI_2, PI};
    if let (Some((x, y)), 0) = (spawn_at, index) {
        return ((x.rem_euclid(level.size.0), y.rem_euclid(level.size.1)), PI);
    }
    match (duel, index) {
        (Some((cx, cy)), 0 | 1) => {
            let (dx, yaw) = if index == 0 {
                (-DUEL_SPACING / 2, -FRAC_PI_2)
            } else {
                (DUEL_SPACING / 2, FRAC_PI_2)
            };
            (
                (
                    (cx + dx).rem_euclid(level.size.0),
                    cy.rem_euclid(level.size.1),
                ),
                yaw,
            )
        }
        _ => (find_spawn_point(level, index, occupied), PI),
    }
}

/// Spacing of the default spawn grid, and the least distance a fresh
/// spawn keeps from a car already in the world.
const SPAWN_SPACING: i32 = 30;
/// Half-width in texels of the ground a fresh spawn must have clear: a
/// mechos contact box is ~40u long (OxidizeMonk), with a margin.
const SPAWN_FOOTPRINT: i32 = 24;
/// Steepest step in world units between neighbouring texels that spawn
/// ground may hold. Rough ground steps a few units; a ledge (Khox at a
/// quarter of the map: 123 next to 163) is many times that.
const SPAWN_MAX_STEP: f32 = 16.0;
/// How far, in texels, `find_spawn_point` looks for clear ground, and the
/// stride of its rings.
const SPAWN_SEARCH: i32 = 256;
const SPAWN_SEARCH_STEP: i32 = 8;

/// Find a spawn point on the level terrain: the grid point of `index`, or
/// the nearest clear ground to it (see `spawn_ground_clear`) at least
/// `SPAWN_SPACING` away from every car in `occupied` (map XY).
fn find_spawn_point(level: &level::Level, index: usize, occupied: &[Vec2]) -> (i32, i32) {
    let spacing = SPAWN_SPACING;
    let base_x = level.size.0 / 4;
    let base_y = level.size.1 / 4;
    let x = base_x + (index as i32 % 4) * spacing;
    let y = base_y + (index as i32 / 4) * spacing;
    let (x, y) = clear_ground_near(level, (x, y), occupied).unwrap_or((x, y));
    (x.rem_euclid(level.size.0), y.rem_euclid(level.size.1))
}

/// Whether the car's footprint around `center` is one layer with no ledge.
///
/// The car is dropped from the height of its center texel. With a ledge
/// or a slab under its body it starts inside the terrain and the contact
/// throws it out: on Khox the default spawn sat across a 40u ridge, and
/// the car flew ~75u up at 15u/s and tumbled for ~10s, deaf to the
/// throttle until it landed.
fn spawn_ground_clear(level: &level::Level, center: (i32, i32)) -> bool {
    let r = SPAWN_FOOTPRINT;
    let single = |x: i32, y: i32| match level.get((x, y)) {
        level::Texel::Single(point) => Some(point.0),
        level::Texel::Dual { .. } => None,
    };
    for y in center.1 - r..=center.1 + r {
        for x in center.0 - r..=center.0 + r {
            let Some(h) = single(x, y) else {
                return false;
            };
            for (nx, ny) in [(x + 1, y), (x, y + 1)] {
                if nx > center.0 + r || ny > center.1 + r {
                    continue;
                }
                match single(nx, ny) {
                    Some(n) if (n - h).abs() <= SPAWN_MAX_STEP => {}
                    _ => return false,
                }
            }
        }
    }
    true
}

/// `base` when its ground is clear and free, else the nearest such point
/// on rings of `SPAWN_SEARCH_STEP` around it, out to `SPAWN_SEARCH`.
fn clear_ground_near(
    level: &level::Level,
    base: (i32, i32),
    occupied: &[Vec2],
) -> Option<(i32, i32)> {
    let min_sq = (SPAWN_SPACING * SPAWN_SPACING) as f32;
    let free = |p: (i32, i32)| {
        let at = Vec2::new(p.0 as f32, p.1 as f32);
        occupied.iter().all(|&car| {
            let d = level.shortest_xy(car.extend(0.0), at.extend(0.0));
            d.length_squared() >= min_sq
        })
    };
    let usable = |p: (i32, i32)| free(p) && spawn_ground_clear(level, p);
    if usable(base) {
        return Some(base);
    }
    let step = SPAWN_SEARCH_STEP;
    for ring in 1..=SPAWN_SEARCH / step {
        let d = ring * step;
        let mut best: Option<(i32, (i32, i32))> = None;
        for i in -ring..=ring {
            let k = i * step;
            for (dx, dy) in [(k, -d), (k, d), (-d, k), (d, k)] {
                let p = (base.0 + dx, base.1 + dy);
                let dist = dx * dx + dy * dy;
                if best.is_some_and(|(b, _)| b <= dist) {
                    continue;
                }
                if usable(p) {
                    best = Some((dist, p));
                }
            }
        }
        if let Some((_, p)) = best {
            return Some(p);
        }
    }
    None
}

#[tokio::main]
async fn main() {
    env_logger::init();
    let cli = Cli::parse();

    // Try to load settings (same config file as the client).
    // Falls back to test defaults when the file or game data is missing (e.g. CI).
    let settings = config::Settings::try_load(&cli.settings);
    if settings.is_none() {
        warn!(
            "Could not load settings from '{}' — using test defaults",
            cli.settings
        );
    }

    // Load level: prefer settings.game.level (same as clients) when --level
    // is omitted and game data is present; `--level test` stays for CI.
    let hosted = resolve_hosted_level(cli.level.as_deref(), settings.as_ref());
    let level_src = match &hosted.ini_path {
        None => "procedural test".to_string(),
        Some(p) => p.display().to_string(),
    };
    info!("Loading level: {} ({})", hosted.name, level_src);
    let level_config = match &hosted.ini_path {
        None => level::LevelConfig::new_test(),
        Some(path) => level::LevelConfig::load(path),
    };
    let geometry = settings
        .as_ref()
        .map(|s| s.game.geometry)
        .unwrap_or_default();
    let mut level = level::load(&level_config, &geometry);
    info!(
        "Level loaded: {}x{} (test={})",
        level.size.0,
        level.size.1,
        hosted.is_test()
    );

    // Story cycles (cirt → escave → palette). Server is authoritative so
    // native TCP and web WS clients stay on the same stage / banks.
    // Name matches the hosted terrain (Welcome + cycle), not a mismatched
    // settings.game.level when `--level test` was forced.
    let cycle_world = hosted.name.clone();
    // Every world's escaves (and spots): the story cycle looks up its own,
    // and wrecks respawn by the hosted world's.
    let all_escaves = settings
        .as_ref()
        .map(|settings| {
            let mut escaves = config::escaves::load_optional(settings, "escaves.prm");
            escaves.extend(config::escaves::load_optional(settings, "spots.prm"));
            escaves
        })
        .unwrap_or_default();
    let mut respawn_escaves: Vec<(i32, i32)> = all_escaves
        .iter()
        .filter(|e| e.world.eq_ignore_ascii_case(&cycle_world))
        .map(|e| e.coordinates)
        .collect();
    respawn_escaves.extend(cli.escaves.iter().copied());
    info!(
        "Respawn escaves for '{}': {:?}",
        cycle_world, respawn_escaves
    );
    let mut cycle = if let Some(ref settings) = settings {
        if !settings.check_path("bunches.prm") {
            None
        } else {
            let bunches = config::bunches::load(settings.open_relative("bunches.prm"));
            level::cycle::Bunch::load(&cycle_world, &level, &bunches, &all_escaves, |path| {
                settings
                    .check_path(path)
                    .then(|| std::fs::read(settings.data_path.join(path)).ok())
                    .flatten()
            })
        }
    } else {
        None
    };
    if let Some(ref bunch) = cycle {
        level.palette = *bunch.settled_palette();
        info!(
            "Story cycle loaded for '{}': {} stages, escave {}",
            cycle_world,
            bunch.stages.len(),
            bunch.escave
        );
    } else {
        info!(
            "No story cycle for '{}' (test/bonus world or missing data)",
            cycle_world
        );
    }

    // Moving land (animated *.vot patches). Server owns playback so clients
    // share the same frame / phase instead of free-running independently.
    let mut moving = level::moving::MovingWorld::load(&level_config, None);
    if moving.is_empty() {
        info!("No moving land for '{}'", cycle_world);
    } else {
        info!(
            "Moving land loaded for '{}': {} locations, {} engines",
            cycle_world,
            moving.land.locations.len(),
            moving.triggers.engines.len()
        );
    }

    // Load physics constants and car data from game files when available.
    let (common, car_physics) = if let Some(ref settings) = settings {
        let common = config::common::load(settings.open_relative("common.prm"));
        let reg = config::game::Registry::load(settings);
        let cars = config::car::load_physics_registry(
            &settings.data_path,
            &reg,
            settings.game.physics.shape_sampling,
        );
        info!(
            "Loaded physics (gravity={}, frame_rate={}, {} cars)",
            common.nature.gravity,
            common.speed.standard_frame_rate,
            cars.len(),
        );
        (common, cars)
    } else {
        let common = config::common::Common::test_default();
        info!(
            "Using test physics (gravity={}, frame_rate={})",
            common.nature.gravity, common.speed.standard_frame_rate
        );
        (common, std::collections::HashMap::new())
    };

    // Undamaged armour per car, from `car.prm`.
    let car_armor: HashMap<String, u16> = settings
        .as_ref()
        .map(|settings| {
            config::car::entries(settings.open_relative("car.prm"))
                .into_iter()
                .map(|(name, _, data)| (name, config::car::CarStats::new(&data).max_armor as u16))
                .collect()
        })
        .unwrap_or_default();
    // Shots in flight, and the pose history each car keeps for them.
    let mut shots: Vec<combat::ServerShot<PlayerId>> = Vec::new();

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<SessionEvent>();
    let sim_lag = SimLag {
        latency: Duration::from_millis(cli.sim_latency_ms),
        jitter: Duration::from_millis(cli.sim_jitter_ms),
    };
    if sim_lag.is_on() {
        warn!(
            "Simulated network lag: {} ms each way + up to {} ms jitter",
            cli.sim_latency_ms, cli.sim_jitter_ms
        );
    }
    let next_id = Arc::new(AtomicU32::new(1));

    // TCP listener
    let tcp_addr = format!("0.0.0.0:{}", cli.port);
    let tcp_listener = TcpListener::bind(&tcp_addr)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind TCP to {}: {}", tcp_addr, e));
    info!("TCP listening on {}", tcp_addr);

    let tcp_tx = event_tx.clone();
    let tcp_next_id = next_id.clone();
    tokio::spawn(async move {
        loop {
            match tcp_listener.accept().await {
                Ok((stream, peer)) => {
                    let id = tcp_next_id.fetch_add(1, Ordering::Relaxed);
                    info!("TCP connection from {} assigned player_id={}", peer, id);
                    let tx = tcp_tx.clone();
                    tokio::spawn(handle_tcp_connection(stream, id, tx, sim_lag));
                }
                Err(e) => error!("TCP accept error: {}", e),
            }
        }
    });

    // WebSocket listener
    let ws_addr = format!("0.0.0.0:{}", cli.ws_port);
    let ws_listener = TcpListener::bind(&ws_addr)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind WS to {}: {}", ws_addr, e));
    info!("WebSocket listening on {}", ws_addr);

    let ws_tx = event_tx.clone();
    let ws_next_id = next_id;
    tokio::spawn(async move {
        loop {
            match ws_listener.accept().await {
                Ok((stream, peer)) => {
                    let id = ws_next_id.fetch_add(1, Ordering::Relaxed);
                    info!(
                        "WebSocket connection from {} assigned player_id={}",
                        peer, id
                    );
                    let tx = ws_tx.clone();
                    tokio::spawn(handle_ws_connection(stream, id, tx, sim_lag));
                }
                Err(e) => error!("WS accept error: {}", e),
            }
        }
    });

    info!(
        "Tick rate: {} Hz, max players: {}",
        cli.tick_rate, cli.max_players
    );

    // Main game loop
    let tick_duration = Duration::from_secs_f64(1.0 / cli.tick_rate as f64);
    let mut tick_interval = time::interval(tick_duration);
    let mut players: HashMap<PlayerId, ServerAgent> = HashMap::new();
    // Last player_id claimed by each Join name. Survives disconnect so a
    // client reconnecting with the same `--name` reclaims that identity.
    let mut name_ids: HashMap<String, PlayerId> = HashMap::new();
    // Accept-time conn_id → effective player_id (identity after name reclaim).
    let mut conn_to_player: HashMap<PlayerId, PlayerId> = HashMap::new();
    // Last known pose + cirtainer + spiral per player_id after leave (grace reclaim).
    let mut last_poses: HashMap<PlayerId, LastPose> = HashMap::new();
    let mut tick: u32 = 0;
    let mut bump_tracker = BumpTracker::default();
    let level_name = cycle_world.clone();
    let max_players = cli.max_players;
    let duel = cli.duel_spawn.then_some(cli.duel_center);
    if let Some((x, y)) = duel {
        info!(
            "Duel spawn: first two joiners {}u apart around ({},{}), facing each other",
            DUEL_SPACING, x, y
        );
    }
    let spawn_at = cli.spawn_at;
    if let Some((x, y)) = spawn_at {
        info!("Spawn at: first joiner on top of ({},{})", x, y);
    }
    // Same sub-step as the clients. `physics::step` applies contact impulses
    // and the rest threshold once per call, so a finer quant than the
    // client's makes the server car far stickier against terrain: a car
    // tipped against a tree cluster stays pinned on the server (and the
    // WorldState reconcile pins the local car to it) while the same input
    // frees it on the client.
    let max_quant = physics_max_quant(settings.as_ref());
    info!("Physics sub-step max_quant={}", max_quant);

    // Physics timing
    let dt_fixed = 1.0 / cli.tick_rate as f32;
    let input_factor = dt_fixed / config::common::MAIN_LOOP_TIME;
    let physics_dt = dt_fixed * {
        let n = &common.nature;
        let fps = common.speed.standard_frame_rate as f32;
        fps * n.time_delta0 * n.num_calls_analysis as f32
    };

    info!(
        "Starting game loop (physics_dt={:.4}, input_factor={:.2})",
        physics_dt, input_factor
    );
    let mut quant_clock = physics::QuantClock::default();
    let quant_input_factor = physics::QuantClock::input_factor(max_quant, &common);

    loop {
        tokio::select! {
            _ = tick_interval.tick() => {
                if players.is_empty() {
                    continue;
                }

                tick += 1;
                for agent in players.values_mut() {
                    agent.fires = 0;
                }

                // Whole `max_quant` steps only, the same split the
                // clients use for their own car (`physics::QuantClock`),
                // so both integrate a jump along the same path.
                let quanta = quant_clock.advance(physics_dt, max_quant);

                // Each player's car runs the client's own numbered quanta
                // (`InputQueue`), one quantum per slot, with car contact
                // resolved after every slot. The client predicted the same
                // quanta with the same inputs, so its prediction at the ack
                // matches unless something it could not see (another car)
                // got in the way; then it rewinds to this state and replays.
                // A car whose input is late waits rather than guessing.
                let budgets: HashMap<PlayerId, TickBudget> = players
                    .iter_mut()
                    .filter(|(_, a)| a.joined)
                    .map(|(&id, a)| (id, a.inputs.budget(quanta)))
                    .collect();
                for (id, budget) in budgets.iter() {
                    if let Some(agent) = players.get_mut(id) {
                        agent.note_budget(*id, budget);
                    }
                }
                let slots = budgets.values().map(TickBudget::total).max().unwrap_or(0);
                for i in 0..slots {
                    let last = i + 1 == slots;
                    for (&player_id, agent) in players.iter_mut() {
                        match budgets.get(&player_id) {
                            Some(budget) if i < budget.total() => {}
                            _ => continue,
                        }
                        let input = agent.inputs.pop();
                        // A wreck rolls on with no control (its client
                        // sends none either).
                        let input = if agent.dead.is_some() {
                            physics::prediction::QuantInput::default()
                        } else {
                            input
                        };
                        let before = agent.transform.disp;
                        // The server does not deform the terrain: the edits
                        // would have to be replicated for clients to agree
                        // on the ground they are driving over.
                        physics::prediction::step_input(
                            &mut agent.dynamo,
                            &mut agent.transform,
                            &input,
                            max_quant,
                            quant_input_factor,
                            &agent.phys_data,
                            &level,
                            &common,
                            None,
                        );
                        if let Some(power) = input.jump {
                            agent.log_jump(player_id, tick, power, before);
                        }

                        // Wrap coordinates
                        let size = level.size;
                        agent.transform.disp.x =
                            agent.transform.disp.x.rem_euclid(size.0 as f32);
                        agent.transform.disp.y =
                            agent.transform.disp.y.rem_euclid(size.1 as f32);
                    }

                    // Car-vs-car contact, server-authoritative. Each agent's
                    // `physics::step` only sees the terrain, so without this
                    // pass players drive straight through each other.
                    collide_players(
                        &mut players,
                        level.size,
                        &common,
                        tick,
                        last,
                        &mut bump_tracker,
                    );
                }
                bump_tracker.prune(tick);
                let wrap = (level.size.0 as f32, level.size.1 as f32);
                for (&player_id, agent) in players.iter_mut() {
                    agent.trace_jump(player_id, tick, wrap);
                    agent.log_pose(player_id);
                }
                let deaths = step_shots(&mut shots, &mut players, &level, tick, dt_fixed);
                for (player_id, killer, position) in deaths {
                    let msg = encode(&ServerMessage::Died {
                        player_id,
                        killer,
                        position: position.into(),
                    });
                    for agent in players.values().filter(|a| a.joined) {
                        let _ = agent.sender.send(msg.clone());
                    }
                }
                for (player_id, transform, next_quant) in
                    respawn_wrecks(&mut players, &level, &respawn_escaves, tick)
                {
                    let msg = encode(&ServerMessage::Respawned {
                        player_id,
                        transform,
                        next_quant,
                    });
                    for agent in players.values().filter(|a| a.joined) {
                        let _ = agent.sender.send(msg.clone());
                    }
                }

                // Story cycle: one quant per server tick (tick_rate 20 Hz
                // matches MAIN_LOOP_TIME = 0.05). All joined players
                // gather/deliver — including those that clients only
                // keep as remote_agents.
                let cycle_state = if let Some(ref mut bunch) = cycle {
                    for agent in players.values_mut() {
                        if !agent.joined {
                            continue;
                        }
                        let coord = (
                            agent.transform.disp.x as i32,
                            agent.transform.disp.y as i32,
                        );
                        bunch.gather(coord, &mut agent.cirtainer);
                        bunch.deliver(coord, &mut agent.cirtainer);
                    }
                    let _ = bunch.quant(&mut level);
                    Some(CycleState {
                        current: bunch.current() as u32,
                        banked: bunch.banked().to_vec(),
                        light: bunch.light(),
                        fade: bunch.fade_progress().map(|(target, left)| CycleFade {
                            target: target as u32,
                            left,
                        }),
                        players: players
                            .iter()
                            .filter(|(_, a)| a.joined)
                            .map(|(&id, a)| PlayerCirt {
                                player_id: id,
                                held: a.cirtainer.held().to_vec(),
                            })
                            .collect(),
                    })
                } else {
                    None
                };

                // Moving land: one quant per tick with all joined players as
                // sensor touches (doors / cyclics see everyone).
                let moving_state = if moving.is_empty() {
                    None
                } else {
                    let touches: Vec<level::moving::Touch> = players
                        .values()
                        .filter(|a| a.joined)
                        .map(|a| {
                            let pos = a.transform.disp;
                            level::moving::Touch {
                                pos: (pos.x as i32, pos.y as i32, pos.z as i32),
                                radius: (a.phys_data.bbox.radius * a.transform.scale) as i32,
                            }
                        })
                        .collect();
                    let _ = moving.run_quants(&mut level, 1, touches);
                    Some(MovingLandState {
                        locations: moving
                            .playback_states()
                            .into_iter()
                            .map(|p| LocationPhase {
                                frame: p.frame.min(u16::MAX as usize) as u16,
                                stage: p.stage,
                                go_phase: p.go_phase,
                                step: p.step,
                            })
                            .collect(),
                    })
                };

                // Collect agent states and broadcast
                let agents: Vec<AgentState> = players
                    .iter()
                    .filter(|(_, a)| a.joined)
                    .map(|(&id, agent)| agent.to_agent_state(id))
                    .collect();

                let msg = encode(&ServerMessage::WorldState {
                    tick,
                    agents,
                    cycle: cycle_state,
                    moving: moving_state,
                });

                let mut disconnected = Vec::new();
                for (&id, agent) in &players {
                    if agent.sender.send(msg.clone()).is_err() {
                        disconnected.push(id);
                    }
                }
                for id in disconnected {
                    remove_player(&mut players, &mut conn_to_player, &mut last_poses, id);
                }
            }

            Some(event) = event_rx.recv() => {
                match event {
                    SessionEvent::Connected { conn_id, sender } => {
                        if players.len() >= max_players {
                            warn!("Rejecting conn_id={}: server full", conn_id);
                            drop(sender);
                            continue;
                        }
                        // Create agent with placeholder state, wait for Join
                        conn_to_player.insert(conn_id, conn_id);
                        players.insert(conn_id, ServerAgent {
                            name: String::new(),
                            car_name: String::new(),
                            color: 0,
                            inputs: InputQueue::default(),
                            transform: space::Transform::IDENTITY,
                            dynamo: Dynamo::default(),
                            phys_data: CarPhysicsData::test_default(), // replaced on Join
                            cirtainer: level::cycle::Cirtainer::default(),
                            spiral_charge: 0,
                            sender,
                            joined: false,
                            bumps: 0,
                            jump_trace: None,
                            input_log: InputLog::default(),
                            pose_logged: None,
                            starved: None,
                            fires: 0,
                            armor: combat::DEFAULT_MAX_ARMOR,
                            max_armor: combat::DEFAULT_MAX_ARMOR,
                            history: combat::PoseHistory::default(),
                            fire_log: FireLog::default(),
                            hit_log: HitLog::default(),
                            dead: None,
                            respawned: None,
                        });
                    }

                    SessionEvent::Message { conn_id, msg } => {
                        match msg {
                            ClientMessage::Join { player_name, car_name, color, protocol } => {
                                if protocol != PROTOCOL_VERSION {
                                    warn!(
                                        "Rejecting conn_id={}: protocol {} (server speaks {})",
                                        conn_id, protocol, PROTOCOL_VERSION
                                    );
                                    remove_player(&mut players, &mut conn_to_player, &mut last_poses, conn_id);
                                    continue;
                                }
                                let player_id = reclaim_player_id(
                                    conn_id,
                                    &player_name,
                                    &mut players,
                                    &mut name_ids,
                                    &mut conn_to_player,
                                );

                                purge_expired_poses(&mut last_poses);
                                let restored = last_poses.remove(&player_id).filter(|pose| {
                                    pose.disconnected_at.elapsed() <= RECONNECT_POSE_GRACE
                                });

                                let spawn_index = players.values().filter(|a| a.joined).count();
                                let occupied: Vec<Vec2> = players
                                    .values()
                                    .filter(|a| a.joined)
                                    .map(|a| a.transform.disp.truncate())
                                    .collect();
                                let (coords, spawn_yaw) =
                                    spawn_pose(&level, spawn_index, duel, spawn_at, &occupied);
                                let height = level.get(coords).high() + 5.0;

                                // Log restored map XY when reclaiming; find_spawn_point is
                                // only used for brand-new / expired-pose joins.
                                let (log_x, log_y) = restored
                                    .as_ref()
                                    .map(|pose| {
                                        (
                                            pose.transform.disp.x as i32,
                                            pose.transform.disp.y as i32,
                                        )
                                    })
                                    .unwrap_or(coords);

                                let pose_note = if restored.is_some() {
                                    " (restored last pose/cirtainer/spiral)"
                                } else if player_id != conn_id {
                                    " (reclaimed id, fresh spawn)"
                                } else {
                                    ""
                                };
                                info!(
                                    "Player {} ({}) joined with car={}, color={}, spawn=({},{}){}",
                                    player_id,
                                    player_name,
                                    car_name,
                                    color,
                                    log_x,
                                    log_y,
                                    pose_note
                                );

                                if let Some(agent) = players.get_mut(&player_id) {
                                    if let Some(phys) = car_physics.get(&car_name) {
                                        agent.phys_data = phys.clone();
                                    } else {
                                        warn!("Unknown car '{}', using test physics", car_name);
                                    }
                                    agent.name = player_name.clone();
                                    agent.car_name = car_name.clone();
                                    agent.color = color;
                                    agent.joined = true;
                                    agent.inputs = InputQueue::default();
                                    agent.history.clear();
                                    agent.max_armor = car_armor
                                        .get(&car_name)
                                        .copied()
                                        .unwrap_or(combat::DEFAULT_MAX_ARMOR);
                                    agent.armor = agent.max_armor;
                                    agent.dead = None;
                                    if let Some(pose) = restored {
                                        agent.transform = pose.transform;
                                        agent.dynamo = pose.dynamo;
                                        agent.cirtainer = pose.cirtainer;
                                        agent.spiral_charge = pose.spiral_charge;
                                        // A wreck that left comes back whole.
                                        if pose.armor > 0 {
                                            agent.armor = pose.armor.min(agent.max_armor);
                                        }
                                        // Keep model scale in sync with the chosen car.
                                        agent.transform.scale = agent.phys_data.scale;
                                    } else {
                                        // Seed only brand-new ids so reclaim tests
                                        // cannot false-pass by re-applying the env.
                                        agent.cirtainer = if player_id == conn_id {
                                            test_seed_cirtainer().unwrap_or_default()
                                        } else {
                                            level::cycle::Cirtainer::default()
                                        };
                                        agent.spiral_charge = if player_id == conn_id {
                                            test_seed_spiral().unwrap_or(0)
                                        } else {
                                            0
                                        };
                                        agent.dynamo = Dynamo::default();
                                        agent.transform = space::Transform {
                                            scale: agent.phys_data.scale,
                                            disp: Vec3::new(
                                                coords.0 as f32,
                                                coords.1 as f32,
                                                height,
                                            ),
                                            rot: glam::Quat::from_rotation_z(spawn_yaw),
                                        };
                                    }

                                    // Send welcome
                                    let welcome = encode(&ServerMessage::Welcome {
                                        player_id,
                                        tick,
                                        level_name: level_name.clone(),
                                    });
                                    let _ = agent.sender.send(welcome);
                                }

                                // Tell new player about existing players
                                let new_sender = players.get(&player_id)
                                    .map(|a| a.sender.clone());
                                for (&id, other) in &players {
                                    if id != player_id && other.joined {
                                        if let Some(ref sender) = new_sender {
                                            let existing = encode(&ServerMessage::PlayerJoined {
                                                player_id: id,
                                                player_name: other.name.clone(),
                                                car_name: other.car_name.clone(),
                                                color: other.color,
                                            });
                                            let _ = sender.send(existing);
                                        }
                                    }
                                }

                                // Tell existing players about new player
                                let joined_msg = encode(&ServerMessage::PlayerJoined {
                                    player_id,
                                    player_name,
                                    car_name,
                                    color,
                                });
                                for (&id, other) in &players {
                                    if id != player_id && other.joined {
                                        let _ = other.sender.send(joined_msg.clone());
                                    }
                                }
                            }

                            ClientMessage::Input { first_quant, quanta, control } => {
                                let player_id = effective_player_id(&conn_to_player, conn_id);
                                if let Some(agent) = players.get_mut(&player_id) {
                                    agent.input_log.note(
                                        player_id,
                                        &agent.name,
                                        first_quant,
                                        quanta,
                                        &control,
                                    );
                                    agent.inputs.push(
                                        first_quant,
                                        quanta as u32,
                                        quant_input(&control),
                                    );
                                }
                            }

                            ClientMessage::SetPose { transform, next_quant } => {
                                let player_id = effective_player_id(&conn_to_player, conn_id);
                                if let Some(agent) = players.get_mut(&player_id) {
                                    let check = combat::check_client_pose(
                                        true,
                                        cli.allow_setpose,
                                        agent.dead.is_some(),
                                        agent.respawned,
                                        tick,
                                        dt_fixed,
                                    );
                                    if let Err(why) = check {
                                        warn!(
                                            "SetPose from player {} ({}) ignored: {}",
                                            player_id, agent.name, why
                                        );
                                    } else if agent.joined {
                                        // The client predicts from this pose
                                        // with its quanta from `next_quant`.
                                        agent.inputs.reset(next_quant);
                                        agent.transform.disp = Vec3::from(transform.position);
                                        agent.transform.rot = glam::Quat::from_xyzw(
                                            transform.rotation[0],
                                            transform.rotation[1],
                                            transform.rotation[2],
                                            transform.rotation[3],
                                        );
                                        // Keep car model scale authoritative.
                                        agent.transform.scale = agent.phys_data.scale;
                                        // Stop residual motion so the debug
                                        // teleport sticks in WorldState.
                                        agent.dynamo.linear_velocity = Vec3::ZERO;
                                        agent.dynamo.angular_velocity = Vec3::ZERO;
                                        // Don't rewind shots to before
                                        // the teleport.
                                        agent.history.clear();
                                    }
                                }
                            }

                            ClientMessage::Reset { position, next_quant } => {
                                let player_id = effective_player_id(&conn_to_player, conn_id);
                                if let Some(agent) = players.get_mut(&player_id) {
                                    let check = combat::check_client_pose(
                                        false,
                                        cli.allow_setpose,
                                        agent.dead.is_some(),
                                        agent.respawned,
                                        tick,
                                        dt_fixed,
                                    );
                                    if let Err(why) = check {
                                        warn!(
                                            "Reset from player {} ({}) ignored: {}",
                                            player_id, agent.name, why
                                        );
                                    } else if agent.joined {
                                        let period = level.period();
                                        let at = combat::reset_position(
                                            Vec3::from(position),
                                            agent.transform.disp,
                                            (period.x, period.y),
                                        );
                                        if at != Vec3::from(position) {
                                            warn!(
                                                "Reset from player {} ({}) at {:?}: too far from {:?}, kept there",
                                                player_id, agent.name, position, agent.transform.disp
                                            );
                                        }
                                        // Upright where it stands, still,
                                        // as the client did before
                                        // quantum `next_quant`.
                                        agent.inputs.reset(next_quant);
                                        agent.transform.disp = at;
                                        agent.transform.rot = glam::Quat::IDENTITY;
                                        agent.dynamo.linear_velocity = Vec3::ZERO;
                                        agent.dynamo.angular_velocity = Vec3::ZERO;
                                        agent.history.clear();
                                    }
                                }
                            }

                            ClientMessage::SetSpiral { charge } => {
                                let player_id = effective_player_id(&conn_to_player, conn_id);
                                if let Some(agent) = players.get_mut(&player_id) {
                                    if agent.joined {
                                        // Capacity is car-local on clients; store charge only.
                                        agent.spiral_charge = charge;
                                    }
                                }
                            }

                            ClientMessage::Fire { quant, bay, origin, dir, view_tick, view_ms } => {
                                let player_id = effective_player_id(&conn_to_player, conn_id);
                                let period = level.period();
                                let Some(agent) = players.get_mut(&player_id) else {
                                    continue;
                                };
                                if !agent.joined {
                                    continue;
                                }
                                if agent.dead.is_some() {
                                    debug!("Fire from player {}: a wreck, dropped", player_id);
                                    continue;
                                }
                                if agent.fires >= weapon::FIRES_PER_TICK {
                                    warn!("Fire from player {}: over {} a tick, dropped", player_id, weapon::FIRES_PER_TICK);
                                    continue;
                                }
                                let dir = match weapon::check_fire(
                                    bay,
                                    Vec3::from(origin),
                                    Vec3::from(dir),
                                    agent.transform.disp,
                                    (period.x, period.y),
                                ) {
                                    Ok(dir) => dir,
                                    Err(why) => {
                                        warn!(
                                            "Fire from player {} rejected: {} (muzzle {:?}, car {:?})",
                                            player_id, why, origin, agent.transform.disp
                                        );
                                        continue;
                                    }
                                };
                                agent.fires += 1;
                                let rewind = combat::rewind_ticks(tick, view_tick, view_ms, dt_fixed);
                                debug!(
                                    "Fire: player {} ({}) bay {} quant {} ack {} view {}+{}ms rewind {:.2} ticks from {:?} dir {:?}",
                                    player_id,
                                    agent.name,
                                    bay,
                                    quant,
                                    agent.inputs.ack(),
                                    view_tick,
                                    view_ms,
                                    rewind,
                                    origin,
                                    dir
                                );
                                agent.fire_log.note(player_id, &agent.name, rewind);
                                shots.push(combat::ServerShot::new(
                                    player_id,
                                    bay,
                                    Vec3::from(origin),
                                    dir,
                                    rewind,
                                ));
                                let shot = encode(&ServerMessage::Shot {
                                    shooter: player_id,
                                    tick,
                                    quant,
                                    bay,
                                    origin,
                                    dir: dir.into(),
                                });
                                for (&id, other) in &players {
                                    if id != player_id && other.joined {
                                        let _ = other.sender.send(shot.clone());
                                    }
                                }
                            }

                            ClientMessage::Leave => {
                                let player_id = effective_player_id(&conn_to_player, conn_id);
                                info!("Player {} leaving", player_id);
                                remove_player(&mut players, &mut conn_to_player, &mut last_poses, player_id);
                            }
                        }
                    }

                    SessionEvent::Disconnected { conn_id } => {
                        let player_id = effective_player_id(&conn_to_player, conn_id);
                        info!("Player {} disconnected (conn_id={})", player_id, conn_id);
                        remove_player(&mut players, &mut conn_to_player, &mut last_poses, player_id);
                    }
                }
            }
        }
    }
}

/// The physics input of an `Input` batch (held controls; the jump and roll
/// land on the batch's last quantum in `InputQueue::push`).
fn quant_input(control: &NetControl) -> QuantInput {
    QuantInput {
        motor: control.motor,
        rudder: control.rudder,
        brake: control.brake,
        turbo: control.turbo,
        jump: control.jump,
        roll: control.roll,
    }
}

/// The wire form of the held control a car last ran with.
fn net_control_of(input: &QuantInput) -> NetControl {
    NetControl {
        motor: input.motor,
        rudder: input.rudder,
        roll: 0.0,
        brake: input.brake,
        turbo: input.turbo,
        jump: None,
    }
}

/// `max_quant` used when no settings file is available; matches
/// `config/settings.template.ron` so CI clients and server agree.
const DEFAULT_MAX_QUANT: f32 = 0.1;

/// Ticks a pair of cars must stay apart before their next felt contact
/// counts as a new bump (0.5 s at 20 Hz).
const BUMP_REARM_TICKS: u32 = 10;

#[derive(Clone, Copy, Debug)]
struct PairContact {
    /// Last tick the pair touched at all.
    last_tick: u32,
    /// Which car of the pair already took this contact's bump.
    counted: [bool; 2],
}

/// Counts a felt bump once per contact: a pair that keeps touching (one
/// car pushing the other, resting contact trickling impulses) is one
/// contact until they have been apart for [`BUMP_REARM_TICKS`].
#[derive(Debug, Default)]
struct BumpTracker {
    pairs: HashMap<(PlayerId, PlayerId), PairContact>,
}

impl BumpTracker {
    /// The pair touched on `tick`, `felt` per car; returns which cars take
    /// a new bump.
    fn on_contact(&mut self, pair: (PlayerId, PlayerId), tick: u32, felt: [bool; 2]) -> [bool; 2] {
        let entry = self.pairs.entry(pair).or_insert(PairContact {
            last_tick: tick,
            counted: [false; 2],
        });
        if tick.wrapping_sub(entry.last_tick) > BUMP_REARM_TICKS {
            entry.counted = [false; 2];
        }
        entry.last_tick = tick;
        let mut new = [false; 2];
        for k in 0..2 {
            if felt[k] && !entry.counted[k] {
                entry.counted[k] = true;
                new[k] = true;
            }
        }
        new
    }

    /// Forget pairs apart for longer than [`BUMP_REARM_TICKS`].
    fn prune(&mut self, tick: u32) {
        self.pairs
            .retain(|_, c| tick.wrapping_sub(c.last_tick) <= BUMP_REARM_TICKS);
    }
}

/// Pairwise car-car contact across joined players, in player-id order so the
/// result does not depend on `HashMap` iteration. Re-wraps moved cars.
/// Called after every physics quantum; `trace` (the tick's last quantum)
/// gates the per-tick proximity trace. A felt bump counts in
/// `ServerAgent::bumps` once per contact (`bump_tracker`).
fn collide_players(
    players: &mut HashMap<PlayerId, ServerAgent>,
    size: (i32, i32),
    common: &config::common::Common,
    tick: u32,
    trace: bool,
    bump_tracker: &mut BumpTracker,
) {
    let mut joined: Vec<(PlayerId, &mut ServerAgent)> = players
        .iter_mut()
        .filter(|(_, a)| a.joined)
        .map(|(id, a)| (*id, a))
        .collect();
    if joined.len() < 2 {
        return;
    }
    joined.sort_unstable_by_key(|(id, _)| *id);
    let wrap = (size.0 as f32, size.1 as f32);
    for i in 0..joined.len() {
        let (head, tail) = joined.split_at_mut(i + 1);
        let (id_a, ref mut a) = head[i];
        for (id_b, b) in tail.iter_mut() {
            let contact = physics::collide_cars(
                physics::CarBody {
                    dynamo: &mut a.dynamo,
                    transform: &mut a.transform,
                    car: &a.phys_data,
                },
                physics::CarBody {
                    dynamo: &mut b.dynamo,
                    transform: &mut b.transform,
                    car: &b.phys_data,
                },
                Some(wrap),
                common,
            );
            if let Some(c) = contact {
                let felt = c.delta_v.map(|dv| dv >= physics::FELT_BUMP_DV);
                let new = bump_tracker.on_contact((id_a, *id_b), tick, felt);
                // Wrapping: clients only compare for a change.
                for (k, agent, id, other) in
                    [(0, &mut **a, id_a, *id_b), (1, &mut **b, *id_b, id_a)]
                {
                    if new[k] {
                        agent.bumps = agent.bumps.wrapping_add(1);
                        info!(
                            "Car bump tick {} p{} from p{} dv={:.2} bumps={}",
                            tick, id, other, c.delta_v[k], agent.bumps
                        );
                    }
                }
            }
            if trace {
                log_proximity(tick, (id_a, &**a), (*id_b, &**b), wrap, contact.as_ref());
            }
            // Resting contact trickles tiny impulses every tick; only
            // the original's "strong" collisions (sound / ram damage
            // there) are worth an info line.
            if let Some(c) = contact.filter(|c| c.impulse > 0.0) {
                let level = if c.impulse >= common.contact.strong_double_collision_threshold {
                    log::Level::Info
                } else {
                    log::Level::Debug
                };
                log::log!(
                    level,
                    "Car contact tick {} p{}<->p{} at ({:.1},{:.1}) impulse={:.3} depth={:.2}",
                    tick,
                    id_a,
                    id_b,
                    c.point.x,
                    c.point.y,
                    c.impulse,
                    c.depth
                );
            }
        }
    }
    for (_, agent) in joined {
        agent.transform.disp.x = agent.transform.disp.x.rem_euclid(wrap.0);
        agent.transform.disp.y = agent.transform.disp.y.rem_euclid(wrap.1);
    }
}

/// Centre distance under which `collide_players` traces both cars every tick.
const PROXIMITY_LOG_RANGE: f32 = 60.0;

/// Per-tick trace of two players' server poses while they are close, for
/// diagnosing contact live: `RUST_LOG=info,vangers_server::contact=debug`.
fn log_proximity(
    tick: u32,
    (id_a, a): (PlayerId, &ServerAgent),
    (id_b, b): (PlayerId, &ServerAgent),
    wrap: (f32, f32),
    contact: Option<&physics::CarContact>,
) {
    let shortest = |d: f32, size: f32| d - size * (d / size).round();
    let d = b.transform.disp - a.transform.disp;
    let dist = glam::vec2(shortest(d.x, wrap.0), shortest(d.y, wrap.1)).length();
    if dist >= PROXIMITY_LOG_RANGE
        || !log::log_enabled!(target: "vangers_server::contact", log::Level::Debug)
    {
        return;
    }
    let vel = |agent: &ServerAgent| agent.transform.rot * agent.dynamo.linear_velocity;
    let (va, vb) = (vel(a), vel(b));
    let (pa, pb) = (a.transform.disp, b.transform.disp);
    let (depth, impulse) = contact.map_or((0.0, 0.0), |c| (c.depth, c.impulse));
    log::debug!(
        target: "vangers_server::contact",
        "Car proximity tick {} dist={:.1} p{} pos=({:.1},{:.1},{:.1}) vel=({:.2},{:.2},{:.2}) p{} pos=({:.1},{:.1},{:.1}) vel=({:.2},{:.2},{:.2}) contact={} depth={:.2} impulse={:.3}",
        tick, dist,
        id_a, pa.x, pa.y, pa.z, va.x, va.y, va.z,
        id_b, pb.x, pb.y, pb.z, vb.x, vb.y, vb.z,
        contact.is_some(), depth, impulse
    );
}

/// Physics sub-step size: the clients' `game.physics.max_quant`.
fn physics_max_quant(settings: Option<&config::Settings>) -> f32 {
    settings
        .map(|s| s.game.physics.max_quant)
        .filter(|q| q.is_finite() && *q > 0.0)
        .unwrap_or(DEFAULT_MAX_QUANT)
}

/// Resolve accept-time conn_id to the effective player_id (after name reclaim).
fn effective_player_id(
    conn_to_player: &HashMap<PlayerId, PlayerId>,
    conn_id: PlayerId,
) -> PlayerId {
    *conn_to_player.get(&conn_id).unwrap_or(&conn_id)
}

/// On Join, reuse a prior player_id when this `--name` is free to reclaim.
///
/// Connection tasks keep speaking `conn_id`; `conn_to_player` remaps them so
/// Welcome / WorldState / remotes see a stable identity across reconnects.
fn reclaim_player_id(
    conn_id: PlayerId,
    player_name: &str,
    players: &mut HashMap<PlayerId, ServerAgent>,
    name_ids: &mut HashMap<String, PlayerId>,
    conn_to_player: &mut HashMap<PlayerId, PlayerId>,
) -> PlayerId {
    // Empty names are not identity keys (avoid every anonymous client colliding).
    if player_name.is_empty() {
        return conn_id;
    }

    let effective = match name_ids.get(player_name).copied() {
        Some(old_id) if old_id != conn_id => {
            let live = players.get(&old_id).is_some_and(|a| a.joined);
            if live {
                // Another connected client already holds this name; keep provisional id.
                conn_id
            } else {
                old_id
            }
        }
        Some(old_id) => old_id,
        None => conn_id,
    };

    if effective != conn_id {
        if let Some(agent) = players.remove(&conn_id) {
            players.insert(effective, agent);
        }
        conn_to_player.insert(conn_id, effective);
        info!(
            "Reclaimed player_id={} for name {:?} (conn_id={})",
            effective, player_name, conn_id
        );
    }

    // Bind name → id unless a different live player already owns the binding.
    let steal = match name_ids.get(player_name).copied() {
        Some(bound) if bound != effective => players.get(&bound).is_some_and(|a| a.joined),
        _ => false,
    };
    if !steal {
        name_ids.insert(player_name.to_string(), effective);
    }

    effective
}

fn purge_expired_poses(last_poses: &mut HashMap<PlayerId, LastPose>) {
    last_poses.retain(|_, pose| pose.disconnected_at.elapsed() <= RECONNECT_POSE_GRACE);
}

/// Record this tick's car positions, then fly every shot one tick on and
/// apply its hit, if any: each shot tests the other joined cars where they
/// were `rewind` ticks ago (`combat::rewind_ticks`), as its shooter saw
/// them. A hit takes `combat::bay_damage` off the target's armour, down
/// to 0, and ends the shot. The terrain stops shots. Wrecks are not hit
/// and their shots are dropped (`combat::fly_shots`).
///
/// A car whose armour this empties is a wreck from now: returns each as
/// `(player, killer, where)` for `ServerMessage::Died`.
fn step_shots(
    shots: &mut Vec<combat::ServerShot<PlayerId>>,
    players: &mut HashMap<PlayerId, ServerAgent>,
    level: &level::Level,
    tick: u32,
    dt: f32,
) -> Vec<(PlayerId, Option<PlayerId>, Vec3)> {
    let period = (level.size.0 as f32, level.size.1 as f32);
    let keep = (combat::HISTORY_SECS / dt).ceil() as usize + 1;
    for agent in players.values_mut().filter(|a| a.joined) {
        agent.history.record(tick, agent.transform.disp, keep);
    }
    let hits = {
        let players = &*players;
        combat::fly_shots(
            shots,
            dt,
            period,
            Some(level),
            |rewind| {
                let at = tick as f32 - rewind;
                players
                    .iter()
                    .filter(|(_, a)| a.joined)
                    .map(|(&id, a)| combat::Target {
                        id,
                        center: a.history.at(at, period).unwrap_or(a.transform.disp),
                        radius: a.phys_data.bbox.radius * a.transform.scale,
                    })
                    .collect()
            },
            |id| players.get(&id).is_some_and(|a| a.dead.is_some()),
        )
    };
    let mut deaths = Vec::new();
    for (shooter, bay, target) in hits {
        let shooter_name = players
            .get(&shooter)
            .map_or_else(|| format!("p{}", shooter), |a| a.name.clone());
        let killer = players.contains_key(&shooter).then_some(shooter);
        let Some(agent) = players.get_mut(&target) else {
            continue;
        };
        if agent.dead.is_some() {
            continue; // killed by an earlier shot this tick
        }
        let damage = combat::bay_damage(bay);
        let before = agent.armor;
        agent.armor = agent.armor.saturating_sub(damage);
        debug!(
            "Hit: p{} ({}) by {} bay {} for {}: armour {}/{}",
            target, agent.name, shooter_name, bay, damage, agent.armor, agent.max_armor
        );
        let armor = (agent.armor, agent.max_armor);
        let emptied = before > 0 && agent.armor == 0;
        agent
            .hit_log
            .note((target, &agent.name), &shooter_name, damage, armor, emptied);
        if agent.armor == 0 {
            let wreck = combat::Wreck {
                at: agent.transform.disp,
                killer,
                respawn: combat::respawn_tick(tick, dt),
            };
            info!(
                "Died: p{} ({}) killed by {} at {:?}; respawn in {}s",
                target,
                agent.name,
                shooter_name,
                wreck.at,
                combat::RESPAWN_SECS
            );
            agent.dead = Some(wreck);
            deaths.push((target, killer, wreck.at));
        }
    }
    deaths
}

/// Put back every wreck whose `combat::Wreck::respawn` tick has come:
/// on clear ground (`clear_ground_near`, away from the other cars) by the
/// escave entrance nearest to where it died, or on the escave itself when
/// none is clear; with no escaves, at a fresh-spawn point. It stands
/// still and undamaged, facing -Y, and the shots of before do not rewind
/// to its old place, and its inputs are numbered afresh
/// (`InputQueue::restart`). Returns each one's new pose and first input
/// quantum for `ServerMessage::Respawned`.
fn respawn_wrecks(
    players: &mut HashMap<PlayerId, ServerAgent>,
    level: &level::Level,
    escaves: &[(i32, i32)],
    tick: u32,
) -> Vec<(PlayerId, NetTransform, u32)> {
    let due: Vec<PlayerId> = players
        .iter()
        .filter(|(_, a)| {
            a.joined
                && a.dead
                    .is_some_and(|wreck| combat::respawn_due(tick, wreck.respawn))
        })
        .map(|(&id, _)| id)
        .collect();
    let mut respawned = Vec::new();
    for player_id in due {
        let occupied: Vec<Vec2> = players
            .iter()
            .filter(|&(&id, a)| id != player_id && a.joined)
            .map(|(_, a)| a.transform.disp.truncate())
            .collect();
        let Some(agent) = players.get_mut(&player_id) else {
            continue;
        };
        let Some(wreck) = agent.dead.take() else {
            continue;
        };
        let period = (level.size.0 as f32, level.size.1 as f32);
        let coords = match combat::nearest_escave(wreck.at, escaves, period) {
            Some(escave) => {
                let (x, y) = clear_ground_near(level, escave, &occupied).unwrap_or(escave);
                (x.rem_euclid(level.size.0), y.rem_euclid(level.size.1))
            }
            None => find_spawn_point(level, 0, &occupied),
        };
        agent.transform = space::Transform {
            scale: agent.phys_data.scale,
            disp: Vec3::new(
                coords.0 as f32,
                coords.1 as f32,
                level.get(coords).high() + 5.0,
            ),
            rot: glam::Quat::from_rotation_z(std::f32::consts::PI),
        };
        agent.dynamo = Dynamo::default();
        agent.armor = agent.max_armor;
        agent.history.clear();
        agent.respawned = Some(tick);
        // The client numbers its inputs on from here; what the wreck
        // still has in flight falls below it and is dropped.
        let next_quant = agent.inputs.restart();
        info!(
            "Respawned: p{} ({}) at ({},{}), armour {}, inputs from quantum {}",
            player_id, agent.name, coords.0, coords.1, agent.armor, next_quant
        );
        let t = &agent.transform;
        respawned.push((
            player_id,
            NetTransform {
                position: t.disp.into(),
                rotation: [t.rot.x, t.rot.y, t.rot.z, t.rot.w],
                scale: t.scale,
            },
            next_quant,
        ));
    }
    respawned
}

fn remove_player(
    players: &mut HashMap<PlayerId, ServerAgent>,
    conn_to_player: &mut HashMap<PlayerId, PlayerId>,
    last_poses: &mut HashMap<PlayerId, LastPose>,
    player_id: PlayerId,
) {
    conn_to_player.retain(|_, mapped| *mapped != player_id);
    if let Some(removed) = players.remove(&player_id) {
        // Keep name→id (reclaim) and last pose/cirtainer/spiral (grace-window restore).
        if removed.joined {
            last_poses.insert(
                player_id,
                LastPose {
                    transform: removed.transform,
                    dynamo: removed.dynamo,
                    cirtainer: removed.cirtainer,
                    spiral_charge: removed.spiral_charge,
                    armor: removed.armor,
                    disconnected_at: Instant::now(),
                },
            );
        }
        info!("Removed player {} ({})", player_id, removed.name);
        let msg = encode(&ServerMessage::PlayerLeft { player_id });
        for agent in players.values() {
            let _ = agent.sender.send(msg.clone());
        }
    }
}

/// `--sim-latency-ms` / `--sim-jitter-ms`: a delay line on each
/// connection's inbound events and outbound messages.
#[derive(Clone, Copy, Debug)]
struct SimLag {
    latency: Duration,
    jitter: Duration,
}

impl SimLag {
    fn is_on(&self) -> bool {
        !self.latency.is_zero() || !self.jitter.is_zero()
    }

    /// A sender that hands each item on to `out` `latency` (plus jitter)
    /// after it was sent, in order. Off: `out` itself.
    fn delay<T: Send + 'static>(
        self,
        out: mpsc::UnboundedSender<T>,
        seed: u32,
    ) -> mpsc::UnboundedSender<T> {
        if !self.is_on() {
            return out;
        }
        let (tx, mut rx) = mpsc::unbounded_channel::<T>();
        let (stamped_tx, mut stamped_rx) = mpsc::unbounded_channel::<(tokio::time::Instant, T)>();
        // Stamp on arrival (never blocks), then release in order.
        tokio::spawn(async move {
            while let Some(item) = rx.recv().await {
                if stamped_tx
                    .send((tokio::time::Instant::now(), item))
                    .is_err()
                {
                    break;
                }
            }
        });
        tokio::spawn(async move {
            let mut rng = seed.wrapping_mul(2_654_435_761) | 1;
            let mut release = tokio::time::Instant::now();
            let jitter_ms = self.jitter.as_millis() as u32;
            while let Some((sent, item)) = stamped_rx.recv().await {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                let extra = Duration::from_millis(if jitter_ms > 0 {
                    (rng % (jitter_ms + 1)) as u64
                } else {
                    0
                });
                release = release.max(sent + self.latency + extra);
                tokio::time::sleep_until(release).await;
                if out.send(item).is_err() {
                    break;
                }
            }
        });
        tx
    }
}

async fn handle_tcp_connection(
    stream: TcpStream,
    player_id: PlayerId,
    event_tx: mpsc::UnboundedSender<SessionEvent>,
    sim_lag: SimLag,
) {
    let event_tx = sim_lag.delay(event_tx, player_id.wrapping_mul(2));
    let (reader, mut writer) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(reader);

    // Channel for outbound messages
    let (send_tx, mut send_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    // Register connection
    let _ = event_tx.send(SessionEvent::Connected {
        conn_id: player_id,
        sender: sim_lag.delay(send_tx, player_id.wrapping_mul(2) + 1),
    });

    // Spawn writer task
    let write_handle = tokio::spawn(async move {
        while let Some(data) = send_rx.recv().await {
            if writer.write_all(&data).await.is_err() {
                break;
            }
        }
    });

    // Reader loop
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];

    loop {
        match reader.read(&mut tmp).await {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                while let Some((msg, consumed)) = decode::<ClientMessage>(&buf) {
                    let _ = event_tx.send(SessionEvent::Message {
                        conn_id: player_id,
                        msg,
                    });
                    buf.drain(..consumed);
                }
            }
            Err(e) => {
                warn!("Read error for player {}: {}", player_id, e);
                break;
            }
        }
    }

    let _ = event_tx.send(SessionEvent::Disconnected { conn_id: player_id });
    write_handle.abort();
}

async fn handle_ws_connection(
    stream: TcpStream,
    player_id: PlayerId,
    event_tx: mpsc::UnboundedSender<SessionEvent>,
    sim_lag: SimLag,
) {
    let event_tx = sim_lag.delay(event_tx, player_id.wrapping_mul(2));
    let ws_stream = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            warn!("WebSocket handshake failed for player {}: {}", player_id, e);
            return;
        }
    };

    let (mut ws_writer, mut ws_reader) = ws_stream.split();

    // Channel for outbound messages
    let (send_tx, mut send_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    // Register connection
    let _ = event_tx.send(SessionEvent::Connected {
        conn_id: player_id,
        sender: sim_lag.delay(send_tx, player_id.wrapping_mul(2) + 1),
    });

    // Writer task: send binary WebSocket frames
    let write_handle = tokio::spawn(async move {
        while let Some(data) = send_rx.recv().await {
            if ws_writer
                .send(tungstenite::Message::Binary(data.into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    // Reader loop: receive binary WebSocket frames
    let mut buf = Vec::with_capacity(4096);

    while let Some(result) = ws_reader.next().await {
        match result {
            Ok(tungstenite::Message::Binary(data)) => {
                buf.extend_from_slice(&data);
                while let Some((msg, consumed)) = decode::<ClientMessage>(&buf) {
                    let _ = event_tx.send(SessionEvent::Message {
                        conn_id: player_id,
                        msg,
                    });
                    buf.drain(..consumed);
                }
            }
            Ok(tungstenite::Message::Close(_)) => break,
            Err(e) => {
                warn!("WS read error for player {}: {}", player_id, e);
                break;
            }
            _ => {} // Ignore ping/pong/text
        }
    }

    let _ = event_tx.send(SessionEvent::Disconnected { conn_id: player_id });
    write_handle.abort();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_max_quant_matches_settings_template() {
        let template = include_str!("../../../config/settings.template.ron");
        let line = template
            .lines()
            .find(|l| l.trim_start().starts_with("max_quant:"))
            .expect("template has max_quant");
        let value: f32 = line
            .split(':')
            .nth(1)
            .unwrap()
            .trim()
            .trim_end_matches(',')
            .parse()
            .unwrap();
        assert_eq!(DEFAULT_MAX_QUANT, value);
        assert_eq!(physics_max_quant(None), value);
    }

    #[test]
    fn input_batch_maps_every_control() {
        let input = quant_input(&NetControl {
            motor: 1.0,
            rudder: -0.5,
            roll: 1.0,
            brake: true,
            turbo: true,
            jump: Some(4.0),
        });
        assert_eq!(
            input,
            QuantInput {
                motor: 1.0,
                rudder: -0.5,
                brake: true,
                turbo: true,
                jump: Some(4.0),
                roll: 1.0,
            }
        );
    }

    #[test]
    fn sustained_contact_is_one_bump() {
        let mut t = BumpTracker::default();
        let pair = (1, 2);
        let mut bumps = [0u32; 2];
        let count = |new: [bool; 2], bumps: &mut [u32; 2]| {
            for k in 0..2 {
                bumps[k] += u32::from(new[k]);
            }
        };
        // Five seconds of pushing: touching every tick, a felt impulse on
        // the pushed car about once a second, on the pusher at the start.
        for tick in 100..200 {
            let felt = [tick == 100, tick % 20 == 0];
            count(t.on_contact(pair, tick, felt), &mut bumps);
            t.prune(tick);
        }
        assert_eq!(bumps, [1, 1]);
        // A short gap is still the same contact.
        count(
            t.on_contact(pair, 199 + BUMP_REARM_TICKS, [true, true]),
            &mut bumps,
        );
        assert_eq!(bumps, [1, 1]);
        // Apart for longer: the next ram is a new bump.
        let later = 199 + 3 * BUMP_REARM_TICKS;
        t.prune(later);
        assert!(t.pairs.is_empty());
        count(t.on_contact(pair, later, [true, false]), &mut bumps);
        assert_eq!(bumps, [2, 1]);
    }

    fn test_level() -> level::Level {
        level::load(&level::LevelConfig::new_test(), &Default::default())
    }

    fn forward(yaw: f32) -> Vec3 {
        glam::Quat::from_rotation_z(yaw) * Vec3::Y
    }

    #[test]
    fn duel_spawn_faces_each_other() {
        let level = test_level();
        let center = (100, 120);
        let (a, yaw_a) = spawn_pose(&level, 0, Some(center), None, &[]);
        let (b, yaw_b) = spawn_pose(&level, 1, Some(center), None, &[]);
        assert_eq!(b.0 - a.0, DUEL_SPACING);
        assert_eq!((a.1, b.1), (center.1, center.1));
        let a_to_b = Vec3::X;
        assert!(forward(yaw_a).dot(a_to_b) > 0.999, "west car faces east");
        assert!(forward(yaw_b).dot(-a_to_b) > 0.999, "east car faces west");
        // A third joiner falls back to the regular grid.
        assert_eq!(
            spawn_pose(&level, 2, Some(center), None, &[]),
            (find_spawn_point(&level, 2, &[]), std::f32::consts::PI)
        );
    }

    #[test]
    fn spawn_at_places_only_the_first_joiner() {
        let level = test_level();
        let at = Some((100, 120));
        assert_eq!(
            spawn_pose(&level, 0, None, at, &[]),
            ((100, 120), std::f32::consts::PI)
        );
        // Wrapped onto the map like any other spawn.
        assert_eq!(
            spawn_pose(&level, 0, None, Some((-1, level.size.1)), &[]).0,
            (level.size.0 - 1, 0)
        );
        // It wins over the duel's first car; the second keeps its place.
        let center = Some((300, 120));
        assert_eq!(spawn_pose(&level, 0, center, at, &[]).0, (100, 120));
        assert_eq!(
            spawn_pose(&level, 1, center, at, &[]),
            spawn_pose(&level, 1, center, None, &[])
        );
        assert_eq!(
            spawn_pose(&level, 1, None, at, &[]),
            (find_spawn_point(&level, 1, &[]), std::f32::consts::PI)
        );
    }

    #[test]
    fn default_spawn_unchanged() {
        let level = test_level();
        for index in 0..5 {
            assert_eq!(
                spawn_pose(&level, index, None, None, &[]),
                (find_spawn_point(&level, index, &[]), std::f32::consts::PI)
            );
        }
    }

    /// The test level made flat and single-layer at stored height `ground`.
    fn flat_level(ground: u8) -> level::Level {
        let mut level = test_level();
        level.height.fill(ground);
        level.meta.fill(0);
        level
    }

    #[test]
    fn spawn_keeps_its_grid_point_on_clear_ground() {
        let level = flat_level(100);
        let base = (level.size.0 / 4, level.size.1 / 4);
        assert_eq!(find_spawn_point(&level, 0, &[]), base);
        assert_eq!(
            find_spawn_point(&level, 1, &[]),
            (base.0 + SPAWN_SPACING, base.1)
        );
    }

    #[test]
    fn spawn_moves_off_a_ledge() {
        // Khox at a quarter of the map: a 40u ridge 6..9 texels off the
        // grid point, diagonally under the car.
        let mut level = flat_level(123);
        let base = (level.size.0 / 4, level.size.1 / 4);
        for y in 0..level.size.1 {
            for x in 0..level.size.0 {
                let d = ((x - base.0) - (y - base.1)).rem_euclid(level.size.0);
                if (6..9).contains(&d) {
                    let i = level.wrap((x, y));
                    level.height[i] = 163;
                }
            }
        }
        assert!(!spawn_ground_clear(&level, base));
        let at = find_spawn_point(&level, 0, &[]);
        assert!(spawn_ground_clear(&level, at), "spawn {at:?} on the ridge");
        let (dx, dy) = (at.0 - base.0, at.1 - base.1);
        assert!(
            dx * dx + dy * dy <= 64 * 64,
            "spawn {at:?} strayed from {base:?}"
        );
    }

    #[test]
    fn spawn_moves_off_a_slab() {
        // The test level's double-level slab sits at its center.
        let mut level = test_level();
        let center = (level.size.0 / 2, level.size.1 / 2);
        let flat = flat_level(100);
        let slab: Vec<usize> = (0..level.meta.len())
            .filter(|&i| level.meta[i] & level::DOUBLE_LEVEL != 0)
            .collect();
        level.height.copy_from_slice(&flat.height);
        let meta = level.meta.clone();
        level.meta.fill(0);
        for i in slab {
            level.meta[i] = meta[i];
        }
        assert!(!spawn_ground_clear(&level, center));
        let at = clear_ground_near(&level, center, &[]).expect("clear ground");
        assert!(spawn_ground_clear(&level, at));
    }

    #[test]
    fn spawn_keeps_clear_of_cars() {
        let level = flat_level(100);
        let base = (level.size.0 / 4, level.size.1 / 4);
        let car = Vec2::new(base.0 as f32 + 5.0, base.1 as f32);
        let at = find_spawn_point(&level, 0, &[car]);
        let gap = Vec2::new(at.0 as f32, at.1 as f32) - car;
        assert!(
            gap.length() >= SPAWN_SPACING as f32,
            "spawn {at:?} on the car"
        );
    }

    #[test]
    fn parse_xy_accepts_pairs() {
        assert_eq!(parse_xy("590,4151"), Ok((590, 4151)));
        assert_eq!(parse_xy(" 1 , -2 "), Ok((1, -2)));
        assert!(parse_xy("590").is_err());
        assert!(parse_xy("a,1").is_err());
    }
}
