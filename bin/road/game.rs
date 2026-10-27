use crate::boilerplate::Application;
use crate::net::{NetEvent, NetworkClient};
use m3d::Mesh;
use vangers::{
    config, creature, escave, level, life, minimap, model,
    physics::{self, CarPhysicsData},
    render::{
        Batcher, GraphicsContext, Render, ScreenTargets,
        debug::LineBuffer,
        object::{BodyColor, Instance},
    },
    space, weapon,
};
use vangers_net::PlayerId;

use glam::Vec3;

fn body_mark(color: BodyColor) -> egui::Color32 {
    match color {
        BodyColor::Green => egui::Color32::from_rgb(80, 220, 80),
        BodyColor::Red => egui::Color32::from_rgb(230, 70, 70),
        BodyColor::Blue => egui::Color32::from_rgb(70, 130, 240),
        BodyColor::Yellow => egui::Color32::from_rgb(240, 210, 50),
        BodyColor::Gray => egui::Color32::from_rgb(180, 180, 180),
        BodyColor::Dummy => egui::Color32::from_rgb(220, 220, 220),
    }
}

use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, PartialEq)]
struct Ai {
    last_transform: space::Transform,
    roll_time: f32,
    target: Vec3,
    retarget: f32,
}

#[derive(Debug, PartialEq)]
enum Spirit {
    Player,
    Other(Ai),
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Control {
    motor: f32,
    rudder: f32,
    roll: f32,
    brake: bool,
    turbo: bool,
    /// Whether the player is asking to be underground.
    mole: bool,
}

enum Physics {
    Cpu {
        transform: space::Transform,
        dynamo: physics::Dynamo,
    },
}

enum SimulationStep<'a> {
    Intermediate,
    Final {
        focus_point: &'a Vec3,
        line_buffer: Option<&'a mut LineBuffer>,
    },
}

impl SimulationStep<'_> {
    /// The original only marks the ground on the last sub-step of a frame,
    /// so a fast car lays one track per frame rather than one per sub-step.
    fn is_final(&self) -> bool {
        matches!(*self, SimulationStep::Final { .. })
    }
}

pub struct Agent {
    _name: String,
    spirit: Spirit,
    car: config::car::CarInfo,
    phys_data: CarPhysicsData,
    car_name: String,
    color: BodyColor,
    control: Control,
    jump: Option<f32>,
    physics: Physics,
    /// Where this car's wheels have been, for the tracks they leave.
    tracks: level::terraform::Tracks,
    /// The cirt it has gathered and not yet handed in.
    cirtainer: level::cycle::Cirtainer,
    /// Remaining armour; smoke comes off when this is below `max_armor`.
    armor: u16,
    max_armor: u16,
}

impl Agent {
    fn spawn(
        name: String,
        car: &config::car::CarInfo,
        car_name: String,
        color: BodyColor,
        coords: (i32, i32),
        orientation: f32,
        level: &level::Level,
    ) -> Self {
        let height = level.get(coords).high() + 5.; //center offset
        let transform = space::Transform {
            scale: car.scale,
            disp: Vec3::new(coords.0 as f32, coords.1 as f32, height),
            rot: glam::Quat::from_rotation_z(orientation),
        };

        Agent {
            _name: name,
            spirit: Spirit::Other(Ai {
                last_transform: transform,
                roll_time: 0.0,
                target: transform.disp,
                retarget: 0.0,
            }),
            phys_data: CarPhysicsData::from_car_info(car),
            car: car.clone(),
            car_name,
            color,
            control: Control::default(),
            jump: None,
            physics: Physics::Cpu {
                transform,
                dynamo: physics::Dynamo::default(),
            },
            tracks: level::terraform::Tracks::default(),
            cirtainer: level::cycle::Cirtainer::default(),
            armor: car.stats.max_armor as u16,
            max_armor: car.stats.max_armor as u16,
        }
    }

    fn change_car(&mut self, car: &config::car::CarInfo, car_name: String) {
        self.phys_data = CarPhysicsData::from_car_info(car);
        self.car = car.clone();
        self.car_name = car_name;
        self.max_armor = car.stats.max_armor as u16;
        self.armor = self.armor.min(self.max_armor);
        match self.physics {
            Physics::Cpu {
                ref mut transform,
                dynamo: _,
            } => {
                transform.scale = car.scale;
            }
        }
    }

    fn cpu_apply_control(&mut self, dt: f32, common: &config::common::Common) {
        let dynamo = match self.physics {
            Physics::Cpu { ref mut dynamo, .. } => dynamo,
        };
        // `CONTROLS::MOLE_DOWN` / `MOLE_UP`: asking to go under starts the
        // burrow, and letting go starts it climbing back out. It ends
        // itself once the car is clear of the ground.
        dynamo.mole = match (self.control.mole, dynamo.mole) {
            (true, physics::Mole::Off | physics::Mole::Under) => physics::Mole::Under,
            (true, other) => other,
            (false, physics::Mole::Under) => physics::Mole::Surfacing,
            (false, other) => other,
        };
        if self.control.rudder != 0.0 {
            let angle = dynamo.rudder + common.car.rudder_step * 2.0 * dt * self.control.rudder;
            dynamo.rudder = angle.min(common.car.rudder_max).max(-common.car.rudder_max);
        }
        if self.control.motor != 0.0 {
            dynamo.change_traction(self.control.motor * dt * common.car.traction_incr);
        }
        if self.control.brake && dynamo.traction != 0.0 {
            dynamo.traction *= (-dt).exp2();
        }
    }

    fn cpu_step(
        &mut self,
        dt: f32,
        level: &level::Level,
        common: &config::common::Common,
        sim_step: SimulationStep,
    ) {
        let (dynamo, transform) = match self.physics {
            Physics::Cpu {
                ref mut transform,
                ref mut dynamo,
            } => (dynamo, transform),
        };
        let last = sim_step.is_final();
        let (jump, roll, focus_point, line_buffer) = match sim_step {
            SimulationStep::Intermediate => (None, 0.0, None, None),
            SimulationStep::Final {
                focus_point,
                line_buffer,
            } => (
                self.jump.take(),
                self.control.roll,
                Some(*focus_point),
                line_buffer,
            ),
        };
        let tracks = last.then_some(&mut self.tracks);
        physics::step(
            dynamo,
            transform,
            dt,
            &self.phys_data,
            level,
            common,
            if self.control.turbo {
                common.global.k_traction_turbo
            } else {
                1.0
            },
            if self.control.brake {
                common.global.f_brake_max
            } else {
                0.0
            },
            jump,
            roll,
            line_buffer,
            tracks,
        );

        // Nearby replica, not a fold onto `[0, size)`. wrap_pos at x=0
        // teleports the car a world-width away and the chase cam follows.
        if let Some(focus) = focus_point {
            transform.disp = level.display_pos(transform.disp, focus);
        }
    }

    fn ai_behavior(&mut self, delta: f32, level: &level::Level) {
        let ai = match self.spirit {
            Spirit::Player => return,
            Spirit::Other(ref mut ai) => ai,
        };
        let transform = match self.physics {
            Physics::Cpu { ref transform, .. } => *transform,
        };

        // `ActionUnit`: steer toward a target, turbo if the heading is
        // close. Stuck in a hole: side impulse, same as before.
        ai.retarget -= delta;
        let to = level.shortest_xy(transform.disp, ai.target);
        if ai.retarget <= 0.0 || to.length() < 80.0 {
            let span = level.period();
            ai.target = Vec3::new(
                (transform.disp.x + (self.control.rudder + 0.37).sin() * 600.0).rem_euclid(span.x),
                (transform.disp.y + (self.control.motor + 0.71).cos() * 600.0).rem_euclid(span.y),
                transform.disp.z,
            );
            ai.retarget = 4.0 + (transform.disp.x.abs() % 3.0);
        }
        let to = level.shortest_xy(transform.disp, ai.target);
        let forward = transform.rot * Vec3::Y;
        let right = transform.rot * Vec3::X;
        let aim = Vec3::new(to.x, to.y, 0.0);
        let aligned = forward
            .truncate()
            .normalize_or_zero()
            .dot(aim.truncate().normalize_or_zero());
        self.control.rudder = right.truncate().dot(aim.truncate()).signum();
        self.control.motor = if aligned > 0.3 { 1.0 } else { 0.35 };
        self.control.turbo = aligned > 0.85;

        if ai.roll_time > 0.0 {
            ai.roll_time -= delta;
            if ai.roll_time <= 0.0 {
                self.control.roll = 0.0;
            }
        } else {
            // Wrap-aware: after a torus rebase the raw Euclidean gap can look
            // huge even when the car barely moved on the seam.
            let dxy = level.shortest_xy(ai.last_transform.disp, transform.disp);
            let dz = transform.disp.z - ai.last_transform.disp.z;
            if Vec3::new(dxy.x, dxy.y, dz).length() < 0.05 {
                ai.roll_time = 0.5;
                let x_axis = transform.rot * Vec3::X;
                self.control.roll = x_axis.z.signum();
                self.control.motor = -0.4;
            }
        }

        ai.last_transform = transform;
    }

    fn position(&self) -> Vec3 {
        match self.physics {
            Physics::Cpu { ref transform, .. } => transform.disp,
        }
    }

    /// Bounding radius in level texels, which is what the sensors measure in.
    fn touch_radius(&self) -> i32 {
        (self.phys_data.bbox.radius * self.car.scale) as i32
    }
}

/// A remote player received via network, rendered locally with interpolation.
struct RemoteAgent {
    name: String,
    car: config::car::CarInfo,
    color: BodyColor,
    /// Last spiral charge from WorldState (for change logs / HUD).
    spiral_charge: u8,
    /// Server state, drawn pose and contact ghost source
    /// (shared with the web client).
    net: physics::client::RemoteCar,
    /// Its treads, from the drawn pose, for the cosmetic track overlay.
    treads: level::overlay::GhostTreads,
}

impl std::borrow::Borrow<physics::client::RemoteCar> for RemoteAgent {
    fn borrow(&self) -> &physics::client::RemoteCar {
        &self.net
    }
}

/// Multiplayer connection state for the lobby UI.
struct MultiplayerState {
    /// Address input field.
    server_addr: String,
    /// Player name input field.
    player_name: String,
    /// Status message shown in the UI.
    status: String,
    /// Whether we're currently connected.
    connected: bool,
}

struct DataBase {
    bunches: Vec<config::bunches::Bunch>,
    cars: HashMap<String, config::car::CarInfo>,
    common: config::common::Common,
    escaves: Vec<config::escaves::Escave>,
    game: config::game::Registry,
}

fn hang_weapons(
    player: &mut Agent,
    inventory: &escave::Inventory,
    meshes: &HashMap<String, Arc<model::Mesh>>,
    db: &DataBase,
) {
    let ids = escave::equipped_slot_ids(inventory);
    let mounted = escave::mounted_meshes(inventory, |id| meshes.get(id).cloned());
    for (i, (slot, mesh)) in player.car.model.slots.iter_mut().zip(mounted).enumerate() {
        slot.mesh = mesh;
        if let Some(id) = ids[i]
            && let Some(info) = db.game.model_infos.get(id)
        {
            slot.scale = info.scale;
        }
    }
}

enum CameraStyle {
    Simple(space::Direction),
    Follow {
        follow: space::Follow,
        // always track the ground level to make the jumps bearable
        ground_anchor: bool,
    },
}

impl CameraStyle {
    fn new(config: &config::settings::Camera) -> Self {
        // the new angle is relative to the surface perpendicular
        let angle = (config.angle as f32).to_radians() - std::f32::consts::FRAC_PI_2;
        if config.speed > 0.0 {
            CameraStyle::Follow {
                follow: space::Follow {
                    angle_x: angle,
                    offset: Vec3::new(0.0, config.offset, config.height),
                    speed: config.speed,
                    look_ahead: config.look_ahead,
                },
                ground_anchor: angle > 15.0f32.to_radians(),
            }
        } else {
            //Note: this appears to be broken ATM
            CameraStyle::Simple(space::Direction {
                view: Vec3::new(0.0, angle.sin(), -angle.cos()),
                height: config.height,
            })
        }
    }
}

/// How far above the ground the follow camera is held, so it does not
/// end up inside a hillside looking at terrain backfaces.
const CAMERA_CLEARANCE: f32 = 4.0;

struct Clipper {
    mx_vp: glam::Mat4,
    threshold: f32,
}

impl Clipper {
    fn new(cam: &space::Camera) -> Self {
        Clipper {
            mx_vp: cam.get_view_proj(),
            threshold: 1.1,
        }
    }

    fn clip(&self, pos: &Vec3) -> bool {
        let p = self.mx_vp * glam::Vec4::from((*pos, 1.0));
        let w = p.w * self.threshold;
        p.x < -w || p.x > w || p.y < -w || p.y > w
    }
}

struct Roll {
    dir: f32,
    time: f32,
}

/// Debug-only scripted controls (`road --autodrive`): a comma-separated
/// list of steps fed into the same `Input` fields the keyboard drives, so a
/// headless session can play a reproducible loop. A step is either a
/// `seconds:motor:rudder` drive segment or one of the action tokens below.
#[derive(Clone, Debug, PartialEq)]
enum AutoStep {
    /// `seconds:motor:rudder`, motor and rudder in -1..=1.
    Drive {
        secs: f32,
        motor: f32,
        rudder: f32,
    },
    /// `goto:x:y[:timeout]`: steer at full motor towards a world point until
    /// within [`AutoDrive::GOTO_RADIUS`] or the timeout (default 30 s).
    Goto {
        x: f32,
        y: f32,
        timeout: f32,
    },
    /// `use`: Space - enter an escave, open a tunnel, use a passage, leave.
    Use,
    /// `fire0`/`fire1`: F/G - fire a weapon bay.
    Fire(usize),
    /// `jump[:secs]`: hold Alt for `secs` (default 0.5) and release.
    Jump(f32),
    /// `reset`: R - level the car (uploaded to the server online).
    Reset,
    /// `tp:x:y[:z]`: the Tweaks position edit, uploaded to the server.
    Teleport {
        x: f32,
        y: f32,
        z: Option<f32>,
    },
    /// `enter`/`leave`: the Tweaks "Enter <escave>" / "Leave escave" buttons.
    Enter,
    Leave,
    /// `armor:N`: the Tweaks "Armour" slider.
    Armor(u32),
    /// `mark:NAME`: an "Autodrive mark NAME" log line, to sync screenshots.
    Mark(String),
}

struct AutoDrive {
    steps: Vec<AutoStep>,
    /// Total script time, for the periodic pose line.
    time: f32,
    /// Time spent in the current step.
    step_time: f32,
    index: usize,
    logged: Option<usize>,
}

impl AutoDrive {
    const GOTO_RADIUS: f32 = 24.0;

    fn parse_step(seg: &str) -> Result<AutoStep, String> {
        let mut parts = seg.split(':').map(str::trim);
        let head = parts.next().unwrap_or_default();
        let rest = parts.collect::<Vec<_>>();
        let nums = |min: usize, max: usize| -> Result<Vec<f32>, String> {
            if rest.len() < min || rest.len() > max {
                return Err(format!(
                    "{seg:?}: expected {min}..={max} numbers after {head:?}"
                ));
            }
            rest.iter()
                .map(|v| v.parse::<f32>().map_err(|e| format!("{seg:?}: {e}")))
                .collect()
        };
        Ok(match head {
            "use" => AutoStep::Use,
            "fire0" => AutoStep::Fire(0),
            "fire1" => AutoStep::Fire(1),
            "reset" => AutoStep::Reset,
            "enter" => AutoStep::Enter,
            "leave" => AutoStep::Leave,
            "jump" => AutoStep::Jump(nums(0, 1)?.first().copied().unwrap_or(0.5)),
            "armor" => match rest[..] {
                [n] => AutoStep::Armor(n.parse().map_err(|e| format!("{seg:?}: {e}"))?),
                _ => return Err(format!("{seg:?}: expected armor:N")),
            },
            "mark" => match rest[..] {
                [name] if !name.is_empty() => AutoStep::Mark(name.to_string()),
                _ => return Err(format!("{seg:?}: expected mark:NAME")),
            },
            "goto" => {
                let v = nums(2, 3)?;
                AutoStep::Goto {
                    x: v[0],
                    y: v[1],
                    timeout: v.get(2).copied().unwrap_or(30.0),
                }
            }
            "tp" => {
                let v = nums(2, 3)?;
                AutoStep::Teleport {
                    x: v[0],
                    y: v[1],
                    z: v.get(2).copied(),
                }
            }
            _ => {
                let v = seg
                    .split(':')
                    .map(|v| v.trim().parse::<f32>().map_err(|e| format!("{seg:?}: {e}")))
                    .collect::<Result<Vec<_>, _>>()?;
                match v[..] {
                    [secs, motor, rudder] => AutoStep::Drive {
                        secs,
                        motor,
                        rudder,
                    },
                    _ => return Err(format!("{seg:?}: expected seconds:motor:rudder")),
                }
            }
        })
    }

    fn parse(script: &str) -> Result<Self, String> {
        let steps = script
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(Self::parse_step)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(AutoDrive {
            steps,
            time: 0.0,
            step_time: 0.0,
            index: 0,
            logged: None,
        })
    }

    /// Rudder towards `target` from a car at `pos` facing `forward`: the
    /// sign of the cross product, scaled by how far off the heading is.
    fn steer(pos: Vec3, forward: Vec3, target: (f32, f32), wrap: (f32, f32)) -> (f32, f32) {
        let wrapped = |d: f32, size: f32| {
            if size > 0.0 {
                d - size * (d / size).round()
            } else {
                d
            }
        };
        let dx = wrapped(target.0 - pos.x, wrap.0);
        let dy = wrapped(target.1 - pos.y, wrap.1);
        let dist = (dx * dx + dy * dy).sqrt();
        let (fx, fy) = (forward.x, forward.y);
        let flen = (fx * fx + fy * fy).sqrt().max(1e-3);
        let cross = (fx * dy - fy * dx) / (flen * dist.max(1e-3));
        let dot = (fx * dx + fy * dy) / (flen * dist.max(1e-3));
        let angle = cross.atan2(dot);
        // Positive rudder turns right (clockwise seen from above).
        let rudder = (-angle * 2.0).clamp(-1.0, 1.0);
        (dist, rudder)
    }
}

#[derive(Default)]
struct Input {
    is_paused: bool,
    spin_hor: f32,
    spin_ver: f32,
    turbo: bool,
    /// Held while the player wants to be underground.
    mole: bool,
    jump: Option<f32>,
    roll: Option<Roll>,
    tick: Option<f32>,
    /// Space: enter a nearby escave/passage, or leave one.
    use_entrance: bool,
    /// Pressed fire bay, consumed on the next update.
    fire_bay: Option<usize>,
}

pub struct Game {
    autodrive: Option<AutoDrive>,
    db: DataBase,
    render: Render,
    batcher: Batcher,
    line_buffer: LineBuffer,
    level: level::Level,
    moving: level::moving::MovingWorld,
    /// The breathing of the terrain colour ramps.
    palette: level::palette::Animation,
    /// The tide, and where it has taken the water.
    flood: level::flood::Flood,
    /// The world's story cycles, and the colours they paint it in.
    cycle: Option<level::cycle::Bunch>,
    /// Time carried over towards the next cycle quant.
    cycle_time: f32,
    /// How deep a tread the wheels cut into the ground.
    terraform: level::terraform::Config,
    /// Scratch buffer for the rectangles this frame's tracks touched.
    track_regions: Vec<level::Region>,
    agents: Vec<Agent>,
    remote_agents: HashMap<PlayerId, RemoteAgent>,
    net: Option<NetworkClient>,
    mp_state: MultiplayerState,
    /// Multiplayer: client prediction of the local car (quantum clock,
    /// numbered history, remote ghosts, render offset), shared with the
    /// web client. `sync.synced` is set by the first WorldState snap.
    sync: physics::client::ClientSync,
    /// Debug: seconds remaining to ignore server transform for the local
    /// player while Tweaks Position DragValue is focused / recently edited.
    /// Prevents WorldState from snapping the drag back mid-edit.
    tweaks_pos_hold: f32,
    /// Debug: local Tweaks Position changed and not yet uploaded via SetPose.
    tweaks_pose_dirty: bool,
    ui: config::settings::Ui,
    /// Whether the tweaks panel is showing. See `boilerplate::tweaks`.
    ui_expanded: bool,
    cam: space::Camera,
    cam_style: CameraStyle,
    max_quant: f32,
    input: Input,
    life: life::World,
    inventory: escave::Inventory,
    shop: escave::Shop,
    /// Body meshes for shop weapons, keyed by `game.lst` NameID.
    weapon_meshes: HashMap<String, Arc<model::Mesh>>,
    screen: escave::Screen,
    data_path: PathBuf,
    /// Original Bug `.a3d` frames, if `game.lst` has one.
    bug: Option<(Vec<Arc<model::Mesh>>, f32)>,
    fauna_meshes: FaunaMeshes,
    /// Chase-camera and pad while the gates are closing on the way in.
    approach_cam: Option<(Vec3, Vec3)>,
    /// Last shop line shown in the interior ("Bought Nymbos", "Not enough beebs").
    escave_note: Option<String>,
    /// Warehouse id currently shown in the shop preview.
    escave_selected: Option<String>,
    /// CPU turntable meshes for shop goods.
    spin_meshes: HashMap<String, escave::SpinMesh>,
    cave_boot: CaveBoot,
    cave: Option<CaveView>,
    /// Secret tunnel: interpolate the car to the other station.
    ride: Option<Ride>,
    shots: Vec<LiveShot>,
    world: String,
    minimap: minimap::Minimap,
    /// All passages from `passages.prm` (needed for reverse arrival pads).
    all_passages: Vec<config::passages::Passage>,
    /// World-to-world portals that leave the current level (`passages.prm`).
    /// Triggered via `PASSAGE` sensors when `location.lst` links them.
    passages: Vec<config::passages::Passage>,
    /// Charges at an escave or outdoor KEY_UPDATE station; spent to open a passage.
    spiral: level::spiral::Spiral,
    /// HUD line: charge, proximity prompt, arrival, or missing-data note.
    passage_note: Option<String>,
    /// Newest `WorldState` tick applied and when: the view a `Fire` was
    /// aimed in, for the server's lag compensation.
    world_view: Option<(u32, std::time::Instant)>,
    /// Space queued a portal hop; applied on the next `update` with GPU handles.
    pending_portal: Option<PendingPortal>,
    /// After a world hop: hold XY on the reverse-passage pad while physics settles.
    portal_pad_lock: Option<PortalPadLock>,
}

struct CaveBoot {
    /// Outdoor / world render settings (not the cave-forced RayTraced copy).
    world_render: config::settings::Render,
    render: config::settings::Render,
    geometry: config::settings::Geometry,
    downlevel_caps: wgpu::DownlevelCapabilities,
    color_format: wgpu::TextureFormat,
    front_face: wgpu::FrontFace,
    screen_size: wgpu::Extent3d,
}

struct PendingPortal {
    id: String,
    dest: String,
}

/// Keep the player on the reverse-passage pad for a short settle window.
struct PortalPadLock {
    xy: (f32, f32),
    remaining: f32,
}

/// How long to reassert pad XY / zero dynamo after a portal hop.
const PORTAL_PAD_LOCK_SECS: f32 = 1.5;

struct CaveView {
    name: String,
    level: level::Level,
    render: Render,
    cam: space::Camera,
    target: Vec3,
    distance: f32,
    yaw: f32,
}

struct LiveShot {
    shot: weapon::Shot,
    age: f32,
    /// Fired by another player (`ServerMessage::Shot`), drawn in its colour.
    remote: bool,
}

struct Ride {
    t: f32,
    duration: f32,
    start: Vec3,
    dest: Vec3,
}

struct FaunaMeshes {
    farmer: Option<(Arc<model::Mesh>, f32)>,
    fish: Option<(Arc<model::Mesh>, f32)>,
    clef: Option<(Arc<model::Mesh>, f32)>,
}

impl Game {
    pub fn new(
        settings: &config::Settings,
        gfx: &GraphicsContext,
        server_addr: Option<String>,
        player_name: String,
    ) -> Self {
        let mut rng = rand::thread_rng();
        log::info!("Loading world parameters");
        let mut escaves = config::escaves::load_optional(settings, "escaves.prm");
        let mut escaves_secondary = config::escaves::load_optional(settings, "spots.prm");
        escaves.append(&mut escaves_secondary);

        let (level_config, default_coords) = if settings.game.level.is_empty() {
            log::info!("Using test level");
            (level::LevelConfig::new_test(), (0, 0))
        } else {
            use rand::seq::SliceRandom as _;

            let local_escave_coords = escaves
                .iter()
                .filter(|e| e.world == settings.game.level)
                .map(|e| e.coordinates)
                .collect::<Vec<_>>();
            let coordinates = match local_escave_coords.choose(&mut rng) {
                Some(coords) => *coords,
                None => (0, 0),
            };

            let worlds = config::worlds::load_from_settings(settings);
            let ini_name = match config::worlds::resolve(&worlds, &settings.game.level) {
                Some(name) => name,
                None => panic!(
                    "Unknown level '{}', valid names are: {:?}",
                    settings.game.level,
                    worlds.keys().collect::<Vec<_>>()
                ),
            };
            let ini_path = settings.data_path.join(ini_name);
            log::info!("Using level {}", ini_name);

            (level::LevelConfig::load(&ini_path), coordinates)
        };
        let coords = settings.car.pos.unwrap_or(default_coords);

        let depth = settings.game.camera.depth_range;
        let cam = space::Camera {
            chase_yaw: None,
            under_slab: 0.0,
            loc: Vec3::new(coords.0 as f32, coords.1 as f32, 200.0),
            rot: glam::Quat::IDENTITY,
            scale: Vec3::new(1.0, -1.0, 1.0),
            proj: match settings.game.camera.projection {
                config::settings::Projection::Perspective => {
                    let h = settings.window.size[1] as f32;
                    let focal = space::DEFAULT_FOCAL_PX;
                    let pf = space::PerspectiveParams {
                        fovy: space::PerspectiveParams::fov_from_focal_px(focal, h),
                        aspect: settings.window.size[0] as f32 / h,
                        near: depth.0,
                        far: depth.1,
                        focal_px: Some(focal),
                    };
                    space::Projection::Perspective(pf)
                }
                config::settings::Projection::Flat => space::Projection::ortho(
                    settings.window.size[0] as u16,
                    settings.window.size[1] as u16,
                    depth.0..depth.1,
                ),
            },
        };

        log::info!("Initializing the render");
        let pal_data = level::read_palette(settings.open_palette(), Some(&level_config.terrains));
        let render = Render::new(
            gfx,
            &level_config,
            &pal_data,
            &settings.render,
            &settings.game.geometry,
            cam.front_face(),
        );

        log::info!("Loading world database");
        let db = {
            if !settings.check_path("car.prm") {
                panic!(
                    "Need original-game files (car.prm, resource/m3d) at {:?}. \
                     Fostral terrain is in the Vangers source tree, but mechous are not.",
                    settings.data_path
                );
            }
            let game = config::game::Registry::load(settings);
            DataBase {
                bunches: config::bunches::load(settings.open_relative("bunches.prm")),
                cars: config::car::load_registry(settings, &game, &gfx.device, &render.object),
                common: config::common::load(settings.open_relative("common.prm")),
                escaves,
                game,
            }
        };

        log::info!("Loading the level");
        let level = level::load(&level_config, &settings.game.geometry);

        let moving = level::moving::MovingWorld::load(&level_config, None);
        let palette = level::palette::Animation::new(&level, &level_config.dynamic_palette);
        let flood = level::flood::Flood::new(&level, &settings.game.level);
        // The world's story cycles, if it has an escave that runs any.
        let mut level = level;
        let cycle = level::cycle::Bunch::load(
            &settings.game.level,
            &level,
            &db.bunches,
            &db.escaves,
            |path| {
                settings
                    .check_path(path)
                    .then(|| std::fs::read(settings.data_path.join(path)).ok())
                    .flatten()
            },
        );
        let mut palette = palette;
        if let Some(ref bunch) = cycle {
            // A world opens in the colours of the cycle it is on.
            level.palette = *bunch.settled_palette();
            palette.rebase(&level.palette);
        }

        log::info!("Spawning agents");
        let car_names = db.cars.keys().cloned().collect::<Vec<_>>();
        let mut player_agent = Agent::spawn(
            "Player".to_string(),
            match db.cars.get(&settings.car.id) {
                Some(info) => info,
                None => panic!(
                    "Unknown car '{}', valid names are: {:?}",
                    settings.car.id, car_names
                ),
            },
            settings.car.id.clone(),
            settings.car.color,
            coords,
            std::f32::consts::PI,
            &level,
        );
        player_agent.spirit = Spirit::Player;

        let shop = escave::Shop::fostral();
        let boards = escave::Catalog::load(&settings.data_path);
        let mut inventory = escave::Inventory::for_car(&settings.car.id, &boards);
        for (i, sid) in settings
            .car
            .slots
            .iter()
            .enumerate()
            .take(escave::BAY_COUNT)
        {
            let _ = inventory.load_bay(i, escave::Good::weapon(sid.clone(), 0, 0));
        }
        let mut weapon_meshes = HashMap::new();
        let mut spin_meshes = HashMap::new();
        let mut load_gun = |id: &str| {
            if weapon_meshes.contains_key(id) {
                return;
            }
            let Some(info) = db.game.model_infos.get(id) else {
                return;
            };
            let Ok(mut file) = File::open(settings.data_path.join(&info.path)) else {
                return;
            };
            let mesh = model::load_c3d(Mesh::load(&mut file), &gfx.device);
            weapon_meshes.insert(id.to_string(), mesh);
        };
        let mut load_spin = |id: &str, model_id: &str| {
            if spin_meshes.contains_key(id) {
                return;
            }
            let Some(info) = db.game.model_infos.get(model_id) else {
                return;
            };
            let Some(spin) = escave::SpinMesh::load_path(&settings.data_path.join(&info.path))
            else {
                return;
            };
            spin_meshes.insert(id.to_string(), spin);
        };
        for good in shop.stock() {
            if good.is_weapon() {
                load_gun(&good.id);
            }
            load_spin(&good.id, good.mesh_id());
        }
        for sid in &settings.car.slots {
            load_gun(sid);
            load_spin(sid, sid);
        }
        hang_weapons(&mut player_agent, &inventory, &weapon_meshes, &db);
        let front_face = cam.front_face();

        let mut agents = vec![player_agent];
        // Local NPCs look like other players in MP and were misread as
        // stale remotes during joint playtests. Skip them when joining a
        // server from the CLI; the Multiplayer Connect button clears any
        // that were spawned for offline play.
        let npc_count = if server_addr.is_some() {
            0
        } else {
            settings.game.other.count
        };
        // populate with random agents
        for i in 0..npc_count {
            use rand::{Rng, prelude::SliceRandom};
            let color = match rng.gen_range(0..3) {
                0 => BodyColor::Green,
                1 => BodyColor::Red,
                2 => BodyColor::Blue,
                _ => unreachable!(),
            };
            let car_id = car_names.choose(&mut rng).unwrap();
            let (x, y) = match settings.game.other.spawn_at {
                config::settings::SpawnAt::Player => coords,
                // Independent of the player — same as other drivers already
                // out on the torus, not clustered around you.
                config::settings::SpawnAt::Random => (
                    rng.gen_range(0..level.size.0),
                    rng.gen_range(0..level.size.1),
                ),
            };
            let agent = Agent::spawn(
                format!("Other-{}", i),
                &db.cars[car_id],
                car_id.clone(),
                color,
                (x, y),
                rng.gen_range(0.0..std::f32::consts::TAU),
                &level,
            );
            agents.push(agent);
        }

        // Connect to server if requested via CLI
        let connected = server_addr.is_some();
        let net = server_addr.as_ref().map(|addr| {
            let player = agents.iter().find(|a| a.spirit == Spirit::Player).unwrap();
            NetworkClient::connect(addr, &player_name, &player.car_name, player.color as u8)
        });

        let mut life = life::World::spawn(&settings.game.level, &level, &settings.data_path);
        life.beebs = 500;

        let bug = db.game.model_infos.get("Bug").and_then(|info| {
            let path = settings.data_path.join(&info.path);
            File::open(&path).ok().map(|file| {
                let frames = if path.extension().and_then(|e| e.to_str()) == Some("a3d") {
                    model::load_a3d_frames(file, &gfx.device)
                } else {
                    vec![model::load_listed_body(
                        &path,
                        file,
                        &gfx.device,
                        &render.object,
                        settings.game.physics.shape_sampling,
                    )]
                };
                // game.lst Size/MaxSize is ~0.1; we used to floor at 0.2.
                (frames, info.scale.max(0.2) / 3.0)
            })
        });
        if bug.is_none() {
            log::info!("No Bug model in game.lst; beebs draw as ticks");
        }

        let load_listed = |name: &str| {
            db.game.model_infos.get(name).and_then(|info| {
                let path = settings.data_path.join(&info.path);
                File::open(&path).ok().map(|file| {
                    (
                        model::load_listed_body(
                            &path,
                            file,
                            &gfx.device,
                            &render.object,
                            settings.game.physics.shape_sampling,
                        ),
                        info.scale.max(0.15),
                    )
                })
            })
        };
        let fauna_meshes = FaunaMeshes {
            farmer: load_listed("SkyFarmer"),
            fish: load_listed("FishWarrior"),
            clef: load_listed("WorldLocker"),
        };

        let all_passages = config::passages::load_optional(settings);
        let passages: Vec<_> = config::passages::from_world(&all_passages, &settings.game.level)
            .into_iter()
            .cloned()
            .collect();
        let spiral = level::spiral::Spiral::with_capacity(
            db.cars
                .get(&settings.car.id)
                .map(|c| c.stats.max_teleport)
                .filter(|&t| t > 0)
                .unwrap_or(level::spiral::DEFAULT_CAPACITY),
        );
        log::info!(
            "Spiral {} slots; {} passage(s) from {}",
            spiral.capacity,
            passages.len(),
            settings.game.level
        );

        Game {
            db,
            render,
            batcher: Batcher::new(),
            line_buffer: LineBuffer::new(),
            level,
            moving,
            palette,
            flood,
            cycle,
            cycle_time: 0.0,
            terraform: level::terraform::Config::default(),
            track_regions: Vec::new(),
            agents,
            remote_agents: HashMap::new(),
            net,
            mp_state: MultiplayerState {
                server_addr: server_addr.unwrap_or_else(|| "127.0.0.1:7800".to_string()),
                player_name,
                status: if connected {
                    "Connecting...".to_string()
                } else {
                    String::new()
                },
                connected,
            },
            sync: Default::default(),
            tweaks_pos_hold: 0.0,
            tweaks_pose_dirty: false,
            ui: settings.ui,
            ui_expanded: true,
            cam,
            cam_style: CameraStyle::new(&settings.game.camera),
            max_quant: settings.game.physics.max_quant,
            input: Input::default(),
            autodrive: None,
            life,
            inventory,
            shop,
            weapon_meshes,
            spin_meshes,
            screen: escave::Screen::new(),
            data_path: settings.data_path.clone(),
            bug,
            fauna_meshes,
            shots: Vec::new(),
            approach_cam: None,
            escave_note: None,
            escave_selected: None,
            cave_boot: CaveBoot {
                world_render: settings.render.clone(),
                render: escave::cave::render_settings(&settings.render),
                geometry: settings.game.geometry,
                downlevel_caps: gfx.downlevel_caps.clone(),
                color_format: gfx.color_format,
                front_face,
                screen_size: gfx.screen_size,
            },
            cave: None,
            ride: None,
            world: settings.game.level.clone(),
            minimap: minimap::Minimap::new(),
            all_passages,
            passages,
            spiral,
            passage_note: None,
            world_view: None,
            pending_portal: None,
            portal_pad_lock: None,
        }
    }

    /// Advances the moving land and hands the touched rectangles to the
    /// renderer. Same [`level::moving::MovingWorld::step`] the web build uses.
    fn step_moving_land(&mut self, delta: f32) {
        // Multiplayer: server owns playback via `MovingLandState` on WorldState.
        if self.mp_state.connected {
            return;
        }
        let height = self.level.geometry.height as u16;
        let touches = self.agents.iter().map(|a| {
            let pos = a.position();
            level::moving::Touch {
                pos: (pos.x as i32, pos.y as i32, pos.z as i32),
                radius: a.touch_radius(),
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

    /// Runs the world's story cycles: every car gathers cirt from whichever
    /// dolly it is near, hands it over on reaching the escave, and once a
    /// stage has had its fill the whole world fades to the next one's
    /// colours.
    ///
    /// When connected to a multiplayer server, gather / deliver / bank
    /// progression are server-authoritative (`CycleState` on WorldState).
    /// This path only advances the local palette fade for visuals.
    fn step_cycle(&mut self, delta: f32) {
        let networked = self.mp_state.connected;
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
                for agent in self.agents.iter_mut() {
                    let pos = agent.position();
                    let coord = (pos.x as i32, pos.y as i32);
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
            // A fade rewrites the palette wholesale, so the animation has
            // to start again from the colours it left behind.
            if !bunch.is_fading() {
                self.palette.rebase(&self.level.palette);
            }
        }
    }

    /// Apply a server `CycleState` to the local bunch and player cirtainers.
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
            Some((range, bunch.light(), bunch.is_fading()))
        } else {
            None
        };
        if let Some((range, light, fading)) = applied {
            if range.start != range.end {
                self.render.dirty_palette(range);
                self.render.set_light_modulation(light);
                if !fading {
                    self.palette.rebase(&self.level.palette);
                }
            } else {
                self.render.set_light_modulation(state.light);
            }
        }

        let my_id = self.net.as_ref().and_then(|n| n.player_id);
        for pc in &state.players {
            if Some(pc.player_id) == my_id
                && let Some(player) = self.agents.iter_mut().find(|a| a.spirit == Spirit::Player)
            {
                player.cirtainer.set_held(&pc.held);
            }
        }
    }

    /// Push local spiral charge to the server after charge/discharge.
    fn report_spiral_to_server(&self) {
        if let Some(net) = self.net.as_ref()
            && net.player_id.is_some()
        {
            net.send_set_spiral(self.spiral.charge);
        }
    }

    /// Online the server owns our armour (it resolves the shots): take it
    /// from the `WorldState`.
    fn apply_armor_from_world(
        &mut self,
        agents: &[vangers_net::AgentState],
        my_id: Option<PlayerId>,
    ) {
        let Some(state) = agents.iter().find(|a| Some(a.player_id) == my_id) else {
            return;
        };
        let Some(player) = self.agents.iter_mut().find(|a| a.spirit == Spirit::Player) else {
            return;
        };
        if (player.armor, player.max_armor) != (state.armor, state.max_armor) {
            if state.armor == 0 && player.armor > 0 {
                log::info!("Armour from server: 0/{}", state.max_armor);
            } else {
                log::debug!("Armour from server: {}/{}", state.armor, state.max_armor);
            }
            player.armor = state.armor;
            player.max_armor = state.max_armor;
        }
    }

    /// Apply server spiral for the local player; log remote charge changes.
    fn apply_spiral_from_world(
        &mut self,
        agents: &[vangers_net::AgentState],
        my_id: Option<PlayerId>,
    ) {
        for agent_state in agents {
            if Some(agent_state.player_id) == my_id {
                let prev = self.spiral.charge;
                let next = agent_state.spiral_charge.min(self.spiral.capacity);
                if next != prev {
                    self.spiral.charge = next;
                    if next > prev && next == self.spiral.capacity {
                        self.passage_note = Some("Spiral charged.".to_string());
                    }
                    log::info!(
                        "Spiral synced from server: {}/{}",
                        self.spiral.charge,
                        self.spiral.capacity
                    );
                } else {
                    self.spiral.charge = next;
                }
            } else if let Some(remote) = self.remote_agents.get_mut(&agent_state.player_id)
                && remote.spiral_charge != agent_state.spiral_charge
            {
                log::info!(
                    "Remote {} spiral {} -> {}",
                    remote.name,
                    remote.spiral_charge,
                    agent_state.spiral_charge
                );
                remote.spiral_charge = agent_state.spiral_charge;
            }
        }
    }

    /// Cuts the stretches the wheels have covered since the last frame into
    /// the level, and hands the touched rectangles to the renderer.
    ///
    /// The wheels record where they have been while the physics runs, which
    /// is over an immutable level and in parallel across the agents. Doing
    /// the cutting here, afterwards, is what lets both of those stand.
    ///
    /// Online the level is left alone: the server never edits its copy,
    /// and the car has to be predicted over the same ground (see
    /// `terraform::drain_vehicle`). The treads are cut into the renderer's
    /// cosmetic track overlay instead (`level::overlay`), and the dust
    /// still flies.
    fn step_tracks(&mut self) {
        self.track_regions.clear();
        let online = self.net.is_some();
        if !online {
            // Back offline the tracks are real again; the overlay's are not.
            self.render.clear_tracks(&self.level);
        }
        for agent in self.agents.iter_mut() {
            let reach = (agent.phys_data.bbox.radius * agent.car.scale).max(8.0) as i32;
            let treads = if online {
                self.render
                    .cut_tracks(&mut self.level, &self.terraform, &mut agent.tracks)
            } else {
                level::terraform::apply_vehicle(
                    &mut self.level,
                    &self.terraform,
                    &mut agent.tracks,
                    reach,
                    &mut self.track_regions,
                )
            };
            for track in treads {
                self.life.particles.from_track(&track, &self.level);
            }
        }
        if self.track_regions.is_empty() {
            return;
        }
        self.track_regions.sort_unstable();
        self.track_regions.dedup();
        let height = self.level.geometry.height as u16;
        self.render.dirty_terrain(&self.track_regions, height);
    }

    /// Multiplayer: the treads of the remote cars, sampled from the pose
    /// each is drawn at (`level::overlay::GhostTreads`) and cut into the
    /// same cosmetic overlay as our own, so each player sees the others'
    /// tracks. The level and every physics state are left alone.
    fn step_remote_tracks(&mut self, delta: f32) {
        for remote in self.remote_agents.values_mut() {
            if !remote.net.pose_ready {
                continue;
            }
            let wheels = remote
                .net
                .phys_data
                .wheels
                .iter()
                .map(|w| Vec3::from(w.pos));
            if !remote
                .treads
                .record(&self.level, &remote.net.render_transform, wheels, delta)
            {
                continue;
            }
            let treads = self.render.cut_tracks(
                &mut self.level,
                &self.terraform,
                remote.treads.tracks_mut(),
            );
            for track in treads {
                self.life.particles.from_track(&track, &self.level);
            }
        }
    }

    /// Multiplayer: step the local car `quanta` whole `max_quant` quanta,
    /// the way `vangers-server` steps every car on its tick: control, then
    /// `physics::step` (jump and roll on the last quantum of the frame),
    /// then contact against the remote ghosts. Every quantum goes into the
    /// prediction history. Returns the frame's input batch for the server:
    /// its first quantum index and the input (held controls, with the
    /// jump and roll of the last quantum).
    fn step_player_quanta(
        &mut self,
        quanta: u32,
        focus_point: &Vec3,
    ) -> Option<(u32, physics::prediction::QuantInput)> {
        let max_quant = self.max_quant;
        let quant_input_factor = physics::QuantClock::input_factor(max_quant, &self.db.common);
        let first = self.sync.prediction.next();
        let mut batch = None;
        for i in 0..quanta {
            let Some(player) = self.agents.iter_mut().find(|a| a.spirit == Spirit::Player) else {
                return batch.map(|input| (first, input));
            };
            let last = i + 1 == quanta;
            let held = physics::prediction::QuantInput {
                motor: player.control.motor,
                rudder: player.control.rudder,
                brake: player.control.brake,
                turbo: player.control.turbo,
                jump: player.jump,
                roll: player.control.roll,
            };
            let input = physics::client::ClientSync::frame_input(&held, i, quanta);
            player.cpu_apply_control(quant_input_factor, &self.db.common);
            let step = if last {
                SimulationStep::Final {
                    focus_point,
                    line_buffer: None,
                }
            } else {
                SimulationStep::Intermediate
            };
            player.cpu_step(max_quant, &self.level, &self.db.common, step);
            // Fold onto the home tile where the server does: after the
            // step and again after the contacts.
            self.rebase_torus();
            self.predict_remote_contacts();
            self.rebase_torus();
            let player = self
                .agents
                .iter()
                .find(|a| a.spirit == Spirit::Player)
                .unwrap();
            let Physics::Cpu {
                ref transform,
                ref dynamo,
            } = player.physics;
            self.sync.prediction.push(input, transform, dynamo);
            batch = Some(input);
        }
        batch.map(|input| (first, input))
    }

    /// Multiplayer: bump the local car off remote ghosts right away, with
    /// the same `physics::collide_cars` the server runs authoritatively.
    /// Only the local car takes the result; the ghost keeps following
    /// WorldState. Without this the local car drives through the ghost
    /// until the reconcile pulls it back.
    ///
    /// Runs after every physics quantum, as the server's `collide_players`
    /// does (and again for every replayed quantum of a reconcile): the
    /// push-apart moves half the overlap per call, so once per render frame
    /// took a different path at every frame rate. The ghost starts at its
    /// latest server pose (not the interpolated render pose a tick behind
    /// it) and is carried along with the local car, see
    /// `physics::prediction::Ghost`.
    fn predict_remote_contacts(&mut self) {
        if self.net.is_none() {
            return;
        }
        let Some(player) = self.agents.iter_mut().find(|a| a.spirit == Spirit::Player) else {
            return;
        };
        let Physics::Cpu {
            ref mut transform,
            ref mut dynamo,
        } = player.physics;
        self.sync.contacts(
            physics::client::LocalCar {
                transform,
                dynamo,
                phys: &player.phys_data,
            },
            &self.remote_agents,
            physics::client::Sim {
                level: &self.level,
                common: &self.db.common,
                max_quant: self.max_quant,
            },
        );
    }

    fn step_world_life(&mut self, delta: f32) {
        let player = self
            .agents
            .iter()
            .find(|a| a.spirit == Spirit::Player)
            .unwrap();
        let transform = match player.physics {
            Physics::Cpu { ref transform, .. } => *transform,
        };
        let wheels = player.phys_data.wheel_points(&transform);
        let contact = life::Contact {
            pos: player.position(),
            wheels: &wheels,
            radius: player.touch_radius() as f32,
            armor: player.armor,
            max_armor: player.max_armor,
        };
        // Our own shots only: another player's hits are the server's call.
        let shots: Vec<Vec3> = self
            .shots
            .iter()
            .filter(|s| !s.remote)
            .map(|s| s.shot.pos)
            .collect();
        let nibble = self.life.step(&self.level, delta, contact, &shots);
        // Online the server owns the armour: wildlife bites stay cosmetic.
        if nibble != 0 && self.net.is_none() {
            let player = self
                .agents
                .iter_mut()
                .find(|a| a.spirit == Spirit::Player)
                .unwrap();
            player.armor = player.armor.saturating_sub(nibble);
        }
    }

    fn start_enter(&mut self, name: String, dest: Vec3) {
        if self.screen.blocks_drive() {
            return;
        }
        self.approach_cam = Some((self.cam.loc, dest));
        self.escave_note = None;
        self.escave_selected = None;
        if self.spiral.charge_full() {
            self.passage_note = Some("Spiral charged.".to_string());
            self.report_spiral_to_server();
        }
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
            self.sync_weapon_slots();
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
            escave::cave::load(&self.data_path, name, &self.cave_boot.geometry, situation)
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

    fn update_follow_camera(&mut self, delta: f32) {
        let player = self
            .agents
            .iter()
            .find(|a| a.spirit == Spirit::Player)
            .unwrap();
        // The drawn pose: physics plus the decaying reconcile offset.
        let target = match player.physics {
            Physics::Cpu { ref transform, .. } => self.sync.drawn(transform),
        };
        match self.cam_style {
            CameraStyle::Simple(ref dir) => {
                self.cam.look_by(&target, dir);
            }
            CameraStyle::Follow {
                ref follow,
                ground_anchor,
            } => {
                // The surface under the car: the cave floor beneath a
                // slab, not the slab top, and the floor its hull rides
                // on rather than a pit under its centre.
                let ground = if ground_anchor {
                    space::Camera::ground_under(&self.level, target.disp)
                } else {
                    target.disp.z
                };
                // Follow, pull out of hills toward the car, re-aim the
                // look-ahead, and drop under overhanging slabs.
                let subject = space::Subject {
                    ground,
                    reach: space::Subject::reach_of(&player.phys_data.bbox, player.car.scale),
                };
                self.cam.chase_anchored(
                    &self.level,
                    &target,
                    subject,
                    delta,
                    follow,
                    CAMERA_CLEARANCE,
                );
            }
        }
        self.rebase_torus();
    }

    /// Fold camera and every agent onto the home tile so coordinates
    /// cannot run off to infinity, and a car west of the seam is the
    /// same car you meet again.
    ///
    /// Multiplayer folds by the local car instead of the camera, after
    /// every quantum: the server folds every car onto the home tile after
    /// each quantum, and the predicted car has to be stepped on the same
    /// coordinates, bit for bit, or float rounding (a car at -20 is not
    /// the car at 236) makes the prediction drift from the server.
    fn rebase_torus(&mut self) {
        let span = self.level.period();
        if span.x <= 0.0 || span.y <= 0.0 {
            return;
        }
        let player_disp = self
            .agents
            .iter()
            .find(|a| a.spirit == Spirit::Player)
            .map(|a| {
                let Physics::Cpu { ref transform, .. } = a.physics;
                transform.disp
            });
        let reference = match player_disp {
            Some(disp) if self.net.is_some() => disp,
            _ => self.cam.loc,
        };
        let shift = Vec3::new(
            reference.x.div_euclid(span.x) * span.x,
            reference.y.div_euclid(span.y) * span.y,
            0.0,
        );
        if shift.x == 0.0 && shift.y == 0.0 {
            return;
        }
        self.cam.loc -= shift;
        for agent in self.agents.iter_mut() {
            if let Physics::Cpu {
                ref mut transform, ..
            } = agent.physics
            {
                transform.disp -= shift;
            }
            if let Spirit::Other(ref mut ai) = agent.spirit {
                ai.target -= shift;
                ai.last_transform.disp -= shift;
            }
        }
        for remote in self.remote_agents.values_mut() {
            remote.net.shift(shift);
        }
        self.sync.shift(shift);
        self.life.shift(shift);
        for live in self.shots.iter_mut() {
            live.shot.pos -= shift;
        }
        if let Some(ref mut ride) = self.ride {
            ride.start -= shift;
            ride.dest -= shift;
        }
    }

    fn sync_weapon_slots(&mut self) {
        let Some(player) = self.agents.iter_mut().find(|a| a.spirit == Spirit::Player) else {
            return;
        };
        hang_weapons(player, &self.inventory, &self.weapon_meshes, &self.db);
    }

    /// Space: poke nearby door sensors so a secret tunnel or escave hatch
    /// opens. The visit starts once the car has fallen into the hole.
    fn try_use_entrance(&mut self) {
        if self.screen.visit().is_some() {
            self.screen.begin_leave();
            return;
        }
        if self.screen.blocks_drive() {
            return;
        }
        if self.try_use_passage() {
            return;
        }
        let pos = match self.agents.iter().find(|a| a.spirit == Spirit::Player) {
            Some(player) => player.position(),
            None => return,
        };
        let radius = self
            .agents
            .iter()
            .find(|a| a.spirit == Spirit::Player)
            .map(|p| p.touch_radius() + 96)
            .unwrap_or(128);
        let at3 = (pos.x as i32, pos.y as i32, pos.z as i32);
        self.moving.use_at(at3, radius, self.level.size);
        self.moving.triggers.touch(at3, radius, self.level.size);
    }

    /// `EXTERNAL_MODE_ESCAVE_IN` after the fall: the hatch is already open.
    fn try_begin_escave_visit(&mut self) {
        if !self.screen.is_world() || self.ride.is_some() {
            return;
        }
        let Some(player) = self.agents.iter().find(|a| a.spirit == Spirit::Player) else {
            return;
        };
        if let Physics::Cpu { ref dynamo, .. } = player.physics
            && dynamo.linear_velocity.z > 10.0
        {
            return;
        }
        let pos = player.position();
        let radius = player.touch_radius();
        let Some(arrival) = self.moving.triggers.escave_arrival_at(
            (pos.x as i32, pos.y as i32, pos.z as i32),
            radius,
            self.level.size,
        ) else {
            return;
        };
        let name = self.resolve_escave_name(&arrival.name, (arrival.pos.0, arrival.pos.1));
        self.start_enter(
            name,
            Vec3::new(arrival.pos.0 as f32, arrival.pos.1 as f32, pos.z),
        );
    }

    fn resolve_escave_name(&self, sensor: &str, pos: (i32, i32)) -> String {
        let pads: Vec<escave::cave::Pad> = self
            .db
            .escaves
            .iter()
            .map(|e| escave::cave::Pad {
                name: e.name.clone(),
                pos: e.coordinates,
            })
            .collect();
        escave::cave::resolve_name(sensor, pos, &pads)
    }

    /// Close the hatch and kick the car out, `ImpulseEscave::Active`.
    fn eject_from_escave(&mut self) {
        let Some(player) = self.agents.iter_mut().find(|a| a.spirit == Spirit::Player) else {
            return;
        };
        let pos = player.position();
        let radius = player.touch_radius() + 96;
        let at = (pos.x as i32, pos.y as i32, pos.z as i32);
        let size = self.level.size;
        self.moving.close_doors_at(at, radius, size);
        let kick = self.moving.triggers.impulse_at(at, radius, size);
        let vel = match kick {
            Some(k) => {
                let dir = Vec3::new(
                    k.direction.0 as f32,
                    k.direction.1 as f32,
                    k.direction.2 as f32,
                );
                let n = dir.length();
                let dir = if n < 1e-3 { Vec3::Z } else { dir / n };
                dir * (k.power as f32).max(24.0)
            }
            None => Vec3::new(0.0, 24.0, 80.0),
        };
        if let Physics::Cpu { ref mut dynamo, .. } = player.physics {
            dynamo.linear_velocity += vel;
        }
    }

    /// `EXTERNAL_MODE_MOVE` of a `TrainEngine`: slide the car from this
    /// station to the other over `ActiveTime`.
    fn begin_train_ride(&mut self) -> bool {
        if self.ride.is_some() || self.screen.blocks_drive() {
            return false;
        }
        let Some(player) = self.agents.iter().find(|a| a.spirit == Spirit::Player) else {
            return false;
        };
        let pos = player.position();
        let radius = player.touch_radius();
        let Some(ride) = self.moving.triggers.train_ride_at(
            (pos.x as i32, pos.y as i32, pos.z as i32),
            radius,
            self.level.size,
        ) else {
            return false;
        };
        let dest_z = self.level.get((ride.dest.0, ride.dest.1)).high() + 8.0;
        self.ride = Some(Ride {
            t: 0.0,
            duration: (ride.quants as f32 * config::common::MAIN_LOOP_TIME).max(0.6),
            start: pos,
            dest: Vec3::new(ride.dest.0 as f32, ride.dest.1 as f32, dest_z),
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
        if let Some(player) = self
            .agents
            .iter_mut()
            .find(|agent| agent.spirit == Spirit::Player)
            && let Physics::Cpu {
                ref mut transform,
                ref mut dynamo,
            } = player.physics
        {
            transform.disp = pos;
            dynamo.linear_velocity = Vec3::ZERO;
            dynamo.angular_velocity = Vec3::ZERO;
            dynamo.traction = 0.0;
        }
        if done {
            self.ride = None;
        }
        true
    }

    /// Hold the player on the reverse-passage pad while ground contact settles.
    /// Mirrors `step_ride`: reassert XY + height, zero dynamo, skip physics.
    fn step_portal_pad_lock(&mut self, delta: f32) -> bool {
        let Some(ref mut lock) = self.portal_pad_lock else {
            return false;
        };
        lock.remaining -= delta;
        let xy = lock.xy;
        let done = lock.remaining <= 0.0;
        if let Some(player) = self
            .agents
            .iter_mut()
            .find(|agent| agent.spirit == Spirit::Player)
        {
            player.control = Control::default();
            player.jump = None;
            if let Physics::Cpu {
                ref mut transform,
                ref mut dynamo,
            } = player.physics
            {
                let height = self.level.get((xy.0 as i32, xy.1 as i32)).high() + 5.;
                transform.disp.x = xy.0;
                transform.disp.y = xy.1;
                transform.disp.z = height;
                *dynamo = physics::Dynamo::default();
            }
        }
        if done {
            self.portal_pad_lock = None;
            log::info!("Portal pad lock released at ({:.0},{:.0})", xy.0, xy.1);
        }
        true
    }

    /// Which cycle the world is on, how much cirt each stage has towards
    /// its turn, and where the dollies that cirt comes from are. Returns a
    /// cycle to jump straight to, if one was clicked.
    fn draw_cycle_ui(bunch: &level::cycle::Bunch, ui: &mut egui::Ui) -> Option<usize> {
        let mut jump = None;
        ui.label(format!("{} at {:?}", bunch.escave, bunch.escave_pos));
        for (i, stage) in bunch.stages.iter().enumerate() {
            let banked = bunch.banked()[i];
            let mark = if i == bunch.current() { "*" } else { " " };
            ui.horizontal(|ui| {
                if ui
                    .button(format!(
                        "{}{} {}/{}",
                        mark, stage.name, banked, stage.cirt_max
                    ))
                    .on_hover_text(format!("cirt from {:?}", stage.dolly))
                    .clicked()
                {
                    jump = Some(i);
                }
            });
        }
        if bunch.is_fading() {
            ui.label("changing...");
        }
        jump
    }

    /// The tread the wheels cut. Turning the depth up makes a few laps
    /// enough to see the ground give way, which is otherwise a slow effect
    /// to watch.
    fn draw_terraform_ui(config: &mut level::terraform::Config, ui: &mut egui::Ui) {
        let tread = &mut config.tread;
        ui.checkbox(&mut tread.enabled, "Tyre tracks");
        ui.add_enabled_ui(tread.enabled, |ui| {
            ui.add(egui::Slider::new(&mut tread.depth, 0..=8).text("Depth"));
            ui.add(egui::Slider::new(&mut tread.period, 1..=8).text("Tread period"));
            ui.add(egui::Slider::new(&mut tread.bar, 1..=8).text("Bar stamps"));
            ui.add(egui::Slider::new(&mut tread.spacing, 0.5..=4.0).text("Bar spacing"));
        });
        ui.separator();
        let press = &mut config.press;
        ui.checkbox(&mut press.enabled, "Hull pressing");
        ui.add_enabled_ui(press.enabled, |ui| {
            ui.add(egui::Slider::new(&mut press.clearance, 0..=32).text("Clearance"));
        });
        ui.separator();
        let molehills = &mut config.molehills;
        ui.checkbox(&mut molehills.enabled, "Mole mounds");
        ui.add_enabled_ui(molehills.enabled, |ui| {
            ui.add(egui::Slider::new(&mut molehills.radius, 1..=20).text("Mound radius"));
            ui.add(egui::Slider::new(&mut molehills.height, 1..=40).text("Mound height"));
        });
        ui.separator();
        let grader = &mut config.grader;
        ui.checkbox(&mut grader.enabled, "Grader blade");
        ui.add_enabled_ui(grader.enabled, |ui| {
            ui.add(egui::Slider::new(&mut grader.lift, 1..=60).text("Drop rate"));
            ui.add(egui::Slider::new(&mut grader.spread, 1..=24).text("Spread"));
            ui.add(egui::Slider::new(&mut grader.reach, 1..=24).text("Berm reach"));
        });
    }

    /// Lists each location with its playback state, and each engine with the
    /// location it drives - enough to tell a stuck bridge from an idle one.
    fn draw_moving_land_ui(
        land: &level::moving::MovingLand,
        triggers: &level::trigger::Triggers,
        ui: &mut egui::Ui,
    ) {
        for location in land.locations.iter() {
            let go = location.go_phase();
            ui.label(format!(
                "{}: frame {} phase {}{}",
                location.source.name,
                location.current_frame(),
                location.current_phase(),
                if go == level::moving::FREE_RUNNING {
                    String::new()
                } else if location.is_go_finish() {
                    format!(" (parked at {go})")
                } else {
                    format!(" (heading to {go})")
                },
            ));
        }
        for engine in triggers.engines.iter() {
            let name = match engine.location {
                Some(index) => land.locations[index].source.name.as_str(),
                None => "<unlinked>",
            };
            ui.label(format!(
                "engine on {}: {} sensors, {} touching{}",
                name,
                engine.sensors().len(),
                engine.touch_count(),
                if engine.is_open() { ", open" } else { "" },
            ));
        }
    }

    fn _move_cam(&mut self, step: f32) {
        let mut back = self.cam.rot * Vec3::Z;
        back.z = 0.0;
        self.cam.loc -= back.normalize() * step;
    }

    fn draw_minimap(&mut self, context: &egui::Context) {
        let player = self.agents.iter().find(|a| a.spirit == Spirit::Player);
        let (center, heading) = if let Some(p) = player {
            let pos = p.position();
            let h = match p.physics {
                Physics::Cpu { ref transform, .. } => {
                    let mut f = transform.rot * Vec3::Y;
                    f.z = 0.0;
                    f.truncate()
                }
            };
            (pos.truncate(), h)
        } else {
            (
                self.cam.loc.truncate(),
                self.cam.ground_forward().truncate(),
            )
        };
        let mut marks = Vec::new();
        for agent in &self.agents {
            if agent.spirit == Spirit::Player {
                continue;
            }
            marks.push(minimap::Mark {
                pos: agent.position().truncate(),
                color: body_mark(agent.color),
                large: false,
            });
        }
        for remote in self.remote_agents.values() {
            marks.push(minimap::Mark {
                pos: remote.net.render_transform.disp.truncate(),
                color: body_mark(remote.color),
                large: false,
            });
        }
        for pad in self
            .db
            .escaves
            .iter()
            .filter(|e| e.world.eq_ignore_ascii_case(&self.world))
        {
            marks.push(minimap::Mark {
                pos: glam::Vec2::new(pad.coordinates.0 as f32, pad.coordinates.1 as f32),
                color: egui::Color32::from_rgb(220, 160, 40),
                large: true,
            });
        }
        for passage in &self.passages {
            marks.push(minimap::Mark {
                pos: glam::Vec2::new(passage.coordinates.0 as f32, passage.coordinates.1 as f32),
                color: egui::Color32::from_rgb(80, 220, 220),
                large: true,
            });
        }
        for sensor in &self.moving.triggers.sensors {
            let (color, large) = if sensor.kind == level::vlc::sensor_kind::KEY_UPDATE {
                (egui::Color32::from_rgb(220, 80, 200), true)
            } else if sensor.kind == level::vlc::sensor_kind::PASSAGE {
                (egui::Color32::from_rgb(40, 180, 220), false)
            } else {
                continue;
            };
            marks.push(minimap::Mark {
                pos: glam::Vec2::new(sensor.pos.0 as f32, sensor.pos.1 as f32),
                color,
                large,
            });
        }
        self.minimap
            .show(context, &self.level, center, heading, &marks);
    }

    /// Outdoor spiral station: `KEY_UPDATE` pads in `snstable.vlc` (KranX
    /// `VangerUnit::TouchSensor`). Fills the spiral without entering an escave.
    fn try_charge_spiral_station(&mut self) {
        if !self.screen.is_world() {
            return;
        }
        let Some(player) = self.agents.iter().find(|a| a.spirit == Spirit::Player) else {
            return;
        };
        let pos = player.position();
        let radius = player.touch_radius();
        let Some(station) = self.moving.triggers.spiral_station_at(
            (pos.x as i32, pos.y as i32, pos.z as i32),
            radius,
            self.level.size,
        ) else {
            return;
        };
        if self.spiral.charge_full() {
            self.passage_note = Some("Spiral charged.".to_string());
            self.report_spiral_to_server();
            log::info!(
                "Spiral charged at station {} ({},{})",
                station.name,
                station.pos.0,
                station.pos.1
            );
        }
    }

    /// Near a world passage: HUD prompt. Prefers original `PASSAGE` sensors
    /// (`snstable.vlc` + `PassageEngine`); falls back to `passages.prm`
    /// proximity when this world has no passage engines. Keeps arrival /
    /// missing notes until we leave reach.
    fn update_passage_proximity(&mut self) {
        if !self.screen.is_world() {
            return;
        }
        match self.passage_in_reach() {
            Some(passage) => {
                if self
                    .passage_note
                    .as_deref()
                    .is_some_and(Self::sticky_passage_note)
                {
                    return;
                }
                self.passage_note = Some(level::spiral::passage_prompt(
                    passage,
                    &self.spiral,
                    self.net.is_some(),
                ));
            }
            None => {
                if self.passage_note.as_deref().is_some_and(|n| {
                    n.contains("in sight")
                        || n.contains("Passage closed")
                        || Self::sticky_passage_note(n)
                }) {
                    self.passage_note = None;
                }
            }
        }
    }

    fn sticky_passage_note(note: &str) -> bool {
        note.starts_with("Arrived on ")
            || note.contains("data not found")
            || note.contains("stubbed for this slice")
    }

    /// Active portal under the player: `PASSAGE` sensor first (KranX), else
    /// prm proximity when no `PassageEngine`s are loaded for this world.
    fn passage_in_reach(&self) -> Option<&config::passages::Passage> {
        let player = self.agents.iter().find(|a| a.spirit == Spirit::Player)?;
        let pos = player.position();
        let radius = player.touch_radius();
        let at3 = (pos.x as i32, pos.y as i32, pos.z as i32);
        if let Some(hit) = self
            .moving
            .triggers
            .passage_at(at3, radius, self.level.size)
        {
            return config::passages::by_id(&self.passages, &hit.passage_id);
        }
        if self
            .moving
            .triggers
            .engines
            .iter()
            .any(|e| matches!(e.kind, level::trigger::Kind::Passage { .. }))
        {
            return None;
        }
        let at = (pos.x as i32, pos.y as i32);
        level::spiral::nearest_passage(
            &self.passages,
            at,
            self.level.size,
            config::passages::Passage::DEFAULT_REACH,
        )
    }

    /// Space at a charged passage: queue a dest-world hop (discharged on success).
    /// Online the passage stays shut (`spiral::passage_open`): Space is taken
    /// with a note, and the spiral keeps its charge.
    fn try_use_passage(&mut self) -> bool {
        if !self.screen.is_world() || self.pending_portal.is_some() {
            return false;
        }
        let online = self.net.is_some();
        let Some((id, dest, note)) = self.passage_in_reach().map(|p| {
            (
                p.id.clone(),
                p.to_world.clone(),
                level::spiral::passage_prompt(p, &self.spiral, online),
            )
        }) else {
            return false;
        };
        if online {
            log::info!("Passage {id} to {dest}: closed in multiplayer");
            self.passage_note = Some(note);
            return true;
        }
        if !level::spiral::passage_open(&self.spiral, online) {
            return false;
        }
        log::info!("Passage {id} to {dest}: queueing dest load");
        self.pending_portal = Some(PendingPortal { id, dest });
        true
    }

    /// Apply a queued portal hop once GPU handles are available.
    fn apply_pending_portal(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let Some(pending) = self.pending_portal.take() else {
            return;
        };
        let worlds = config::worlds::load_from_path(&self.data_path);
        let Some(ini_path) =
            config::worlds::ini_path_if_present(&self.data_path, &worlds, &pending.dest)
        else {
            self.passage_note = Some(format!(
                "{} data not found. Install thechain/{{world}} from data-0.",
                pending.dest
            ));
            log::warn!(
                "Passage {} to {}: dest world.ini missing under {:?}",
                pending.id,
                pending.dest,
                self.data_path
            );
            return;
        };

        let old_world = self.world.clone();
        let arrival =
            config::passages::arrival_coords(&self.all_passages, &pending.dest, &old_world)
                .unwrap_or_else(|| {
                    log::warn!(
                        "No reverse passage for {} <- {}; spawning at origin",
                        pending.dest,
                        old_world
                    );
                    (0, 0)
                });

        log::info!(
            "Loading dest world {} from {:?} -> arrival {:?}",
            pending.dest,
            ini_path,
            arrival
        );

        let level_config = level::LevelConfig::load(&ini_path);
        let mut level = level::load(&level_config, &self.cave_boot.geometry);
        let gfx = GraphicsContext {
            device: device.clone(),
            queue: queue.clone(),
            downlevel_caps: self.cave_boot.downlevel_caps.clone(),
            color_format: self.cave_boot.color_format,
            screen_size: self.cave_boot.screen_size,
        };
        let pal = level.palette;
        let render = Render::new(
            &gfx,
            &level_config,
            &pal,
            &self.cave_boot.world_render,
            &self.cave_boot.geometry,
            self.cave_boot.front_face,
        );

        let moving = level::moving::MovingWorld::load(&level_config, None);
        let mut palette = level::palette::Animation::new(&level, &level_config.dynamic_palette);
        let flood = level::flood::Flood::new(&level, &pending.dest);
        let cycle = level::cycle::Bunch::load(
            &pending.dest,
            &level,
            &self.db.bunches,
            &self.db.escaves,
            |path| {
                let full = self.data_path.join(path);
                full.is_file().then(|| std::fs::read(&full).ok()).flatten()
            },
        );
        if let Some(ref bunch) = cycle {
            level.palette = *bunch.settled_palette();
            palette.rebase(&level.palette);
        }

        let beebs = self.life.beebs;
        let mut life = life::World::spawn(&pending.dest, &level, &self.data_path);
        life.beebs = beebs;

        let passages: Vec<_> = config::passages::from_world(&self.all_passages, &pending.dest)
            .into_iter()
            .cloned()
            .collect();

        let other_count = self
            .agents
            .iter()
            .filter(|a| matches!(a.spirit, Spirit::Other(_)))
            .count();
        let car_names: Vec<_> = self.db.cars.keys().cloned().collect();

        // Discharge only after the dest world is fully built.
        if !self.spiral.discharge() {
            self.passage_note = Some("Spiral discharged. Passage closed!".to_string());
            return;
        }
        self.report_spiral_to_server();

        self.level = level;
        self.render = render;
        self.moving = moving;
        self.palette = palette;
        self.flood = flood;
        self.cycle = cycle;
        self.cycle_time = 0.0;
        self.life = life;
        self.passages = passages;
        self.world = pending.dest.clone();
        self.cave = None;
        self.ride = None;
        self.shots.clear();
        self.approach_cam = None;
        self.track_regions.clear();
        self.escave_note = None;
        self.escave_selected = None;

        {
            let Some(player) = self.agents.iter_mut().find(|a| a.spirit == Spirit::Player) else {
                return;
            };
            let height = self.level.get(arrival).high() + 5.;
            match player.physics {
                Physics::Cpu {
                    ref mut transform,
                    ref mut dynamo,
                } => {
                    // Level the chassis (spawn-style Z yaw only). Leftover pitch/roll
                    // from the origin world digs wheels into Glorx slopes and slides.
                    let fwd = transform.rot * Vec3::Y;
                    let yaw = (-fwd.x).atan2(fwd.y);
                    transform.rot = glam::Quat::from_rotation_z(yaw);
                    transform.disp = Vec3::new(arrival.0 as f32, arrival.1 as f32, height);
                    *dynamo = physics::Dynamo::default();
                }
            }
            player.jump = None;
            player.control = Control::default();
            player.tracks.reset();
            self.cam.loc = Vec3::new(arrival.0 as f32, arrival.1 as f32, height + 200.0);
            self.portal_pad_lock = Some(PortalPadLock {
                xy: (arrival.0 as f32, arrival.1 as f32),
                remaining: PORTAL_PAD_LOCK_SECS,
            });
        }

        // Respawn local NPCs on the new torus; keep the player agent.
        self.agents.retain(|a| a.spirit == Spirit::Player);
        {
            use rand::{Rng, prelude::SliceRandom};
            let mut rng = rand::thread_rng();
            for i in 0..other_count {
                let color = match rng.gen_range(0..3) {
                    0 => BodyColor::Green,
                    1 => BodyColor::Red,
                    2 => BodyColor::Blue,
                    _ => unreachable!(),
                };
                let car_id = car_names.choose(&mut rng).unwrap();
                let (x, y) = (
                    rng.gen_range(0..self.level.size.0),
                    rng.gen_range(0..self.level.size.1),
                );
                let agent = Agent::spawn(
                    format!("Other-{}", i),
                    &self.db.cars[car_id],
                    car_id.clone(),
                    color,
                    (x, y),
                    rng.gen_range(0.0..std::f32::consts::TAU),
                    &self.level,
                );
                self.agents.push(agent);
            }
        }

        self.passage_note = Some(format!("Arrived on {}", pending.dest));
        log::info!(
            "Passage {} hop complete: {} -> {} at {:?}; spiral {}/{}",
            pending.id,
            old_world,
            pending.dest,
            arrival,
            self.spiral.charge,
            self.spiral.capacity
        );
    }

    /// MP: push the local pose to the server (Tweaks Position edit, R reset).
    /// The car was moved outside its inputs, so the prediction history is
    /// dropped and the server told which quantum the pose holds from.
    fn send_tweaks_pose(&mut self) {
        if self.net.is_none() {
            return;
        }
        self.sync.reset();
        let next_quant = self.sync.prediction.next();
        let Some(net) = self.net.as_ref() else {
            return;
        };
        let Some(player) = self.agents.iter().find(|a| a.spirit == Spirit::Player) else {
            return;
        };
        let Physics::Cpu { ref transform, .. } = player.physics else {
            return;
        };
        let nt = vangers_net::NetTransform {
            position: transform.disp.into(),
            rotation: [
                transform.rot.x,
                transform.rot.y,
                transform.rot.z,
                transform.rot.w,
            ],
            scale: transform.scale,
        };
        net.send_set_pose(&nt, next_quant);
    }

    fn draw_spiral_hud(&self, context: &egui::Context) {
        egui::Area::new(egui::Id::new("spiral-hud"))
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(12.0, 120.0))
            .order(egui::Order::Foreground)
            .show(context, |ui| {
                if let Some(player) = self.agents.iter().find(|a| a.spirit == Spirit::Player) {
                    ui.label(
                        egui::RichText::new(format!(
                            "Armour {}/{}",
                            player.armor, player.max_armor
                        ))
                        .strong(),
                    );
                }
                ui.label(
                    egui::RichText::new(format!(
                        "Spiral {}/{}",
                        self.spiral.charge, self.spiral.capacity
                    ))
                    .strong(),
                );
                if let Some(note) = self.passage_note.as_deref() {
                    ui.label(note);
                }
            });
    }
}

impl Game {
    /// Debug: drive the local car from a `seconds:motor:rudder,...` script.
    pub fn set_autodrive(&mut self, script: &str) {
        match AutoDrive::parse(script) {
            Ok(ad) => {
                log::info!("Autodrive: {} step(s)", ad.steps.len());
                self.autodrive = Some(ad);
            }
            Err(e) => log::error!("Autodrive script rejected: {e}"),
        }
    }

    fn step_autodrive(&mut self, delta: f32) {
        if self.autodrive.is_none() {
            return;
        }
        let overlay = self.render.tracks();
        let tiles = (overlay.len(), overlay.bytes());
        let player_pose = || {
            self.agents
                .iter()
                .find(|a| a.spirit == Spirit::Player)
                .map(|a| match a.physics {
                    Physics::Cpu { ref transform, .. } => *transform,
                })
        };
        let pose = player_pose();
        let wrap = (self.level.size.0 as f32, self.level.size.1 as f32);
        let Some(ad) = self.autodrive.as_mut() else {
            return;
        };
        // Instant actions run back to back; a drive or goto step holds the
        // controls for this frame.
        let mut actions = Vec::new();
        let mut controls = (0.0, 0.0);
        while let Some(step) = ad.steps.get(ad.index).cloned() {
            if ad.logged != Some(ad.index) {
                ad.logged = Some(ad.index);
                log::info!(
                    "Autodrive step {}/{} at t={:.1}s {:?} pos={:?} track overlay: {} tiles, {} KiB",
                    ad.index,
                    ad.steps.len(),
                    ad.time,
                    step,
                    pose.map(|t| t.disp),
                    tiles.0,
                    tiles.1 / 1024,
                );
                if let AutoStep::Goto { .. } | AutoStep::Jump(_) = step {
                    // Open-ended steps time themselves from their start.
                    ad.step_time = 0.0;
                    if let AutoStep::Jump(_) = step {
                        actions.push(AutoStep::Jump(0.0));
                    }
                }
            }
            let done = match step {
                AutoStep::Drive {
                    secs,
                    motor,
                    rudder,
                } => {
                    controls = (motor, rudder);
                    ad.step_time >= secs
                }
                AutoStep::Goto { x, y, timeout } => match pose {
                    Some(t) => {
                        let (dist, rudder) =
                            AutoDrive::steer(t.disp, t.rot * Vec3::Y, (x, y), wrap);
                        let arrived = dist <= AutoDrive::GOTO_RADIUS;
                        if arrived || ad.step_time >= timeout {
                            log::info!(
                                "Autodrive goto ({x},{y}) {} after {:.1}s, dist={dist:.0}",
                                if arrived { "arrived" } else { "timed out" },
                                ad.step_time
                            );
                            true
                        } else {
                            controls = ((dist / 60.0).clamp(0.4, 1.0), rudder);
                            false
                        }
                    }
                    None => true,
                },
                AutoStep::Jump(secs) => ad.step_time >= secs,
                ref other => {
                    actions.push(other.clone());
                    true
                }
            };
            if !done {
                break;
            }
            if let AutoStep::Jump(_) = step {
                actions.push(AutoStep::Jump(-1.0));
            }
            ad.index += 1;
            // Keep a drive step's overshoot (and pass it through instant
            // actions), so a long script stays on its absolute timeline at
            // a low frame rate and two clients' scripts stay in step.
            ad.step_time = match step {
                AutoStep::Drive { secs, .. } => (ad.step_time - secs).max(0.0),
                AutoStep::Goto { .. } | AutoStep::Jump(_) => 0.0,
                _ => ad.step_time,
            };
        }
        if ad.index < ad.steps.len() {
            ad.step_time += delta;
        }
        // A pose line every two seconds, to follow the drive in the log.
        let periodic = (ad.time / 2.0).floor() != ((ad.time + delta) / 2.0).floor();
        if periodic {
            let armor = self
                .agents
                .iter()
                .find(|a| a.spirit == Spirit::Player)
                .map(|a| (a.armor, a.max_armor));
            log::info!(
                "Autodrive t={:.1}s pos={:?} armor={:?} spiral={}/{} shots={} remote_shots={} world={} remotes={}",
                ad.time + delta,
                pose.map(|t| t.disp),
                armor,
                self.spiral.charge,
                self.spiral.capacity,
                self.shots.iter().filter(|l| !l.remote).count(),
                self.shots.iter().filter(|l| l.remote).count(),
                self.screen.is_world(),
                self.remote_agents.len(),
            );
        }
        ad.time += delta;
        self.input.spin_ver = controls.0;
        self.input.spin_hor = controls.1;
        for action in actions {
            self.run_auto_action(action);
        }
    }

    fn run_auto_action(&mut self, action: AutoStep) {
        use winit::{event::ElementState, keyboard::KeyCode};
        let key = |game: &mut Self, key| {
            game.on_key(key, ElementState::Pressed);
            game.on_key(key, ElementState::Released);
        };
        match action {
            AutoStep::Use => self.input.use_entrance = true,
            AutoStep::Fire(bay) => self.input.fire_bay = Some(bay),
            AutoStep::Reset => key(self, KeyCode::KeyR),
            AutoStep::Jump(t) if t >= 0.0 => {
                self.on_key(KeyCode::AltLeft, ElementState::Pressed);
            }
            AutoStep::Jump(_) => {
                self.on_key(KeyCode::AltLeft, ElementState::Released);
            }
            AutoStep::Teleport { x, y, z } => {
                let z = z.unwrap_or_else(|| {
                    level::terraform::surface_height(&self.level, (x as i32, y as i32)) + 8.0
                });
                if let Some(player) = self.agents.iter_mut().find(|a| a.spirit == Spirit::Player) {
                    let Physics::Cpu {
                        ref mut transform,
                        ref mut dynamo,
                    } = player.physics;
                    transform.disp = Vec3::new(x, y, z);
                    transform.rot = glam::Quat::IDENTITY;
                    dynamo.linear_velocity = Vec3::ZERO;
                    dynamo.angular_velocity = Vec3::ZERO;
                    player.tracks.reset();
                }
                if self.net.is_some() {
                    self.send_tweaks_pose();
                    self.tweaks_pos_hold = self.tweaks_pos_hold.max(0.75);
                }
            }
            AutoStep::Enter => {
                let Some(pos) = self
                    .agents
                    .iter()
                    .find(|a| a.spirit == Spirit::Player)
                    .map(|a| a.position())
                else {
                    return;
                };
                let reach = (level::cycle::DELIVERY_RADIUS * 2).max(96);
                let near = self.db.escaves.iter().find(|e| {
                    let dx = pos.x as i32 - e.coordinates.0;
                    let dy = pos.y as i32 - e.coordinates.1;
                    dx * dx + dy * dy <= reach * reach
                });
                match near {
                    Some(e) if self.screen.is_world() => {
                        let dest =
                            Vec3::new(e.coordinates.0 as f32, e.coordinates.1 as f32, pos.z - 24.0);
                        let name = e.name.clone();
                        log::info!("Autodrive enter {name}");
                        self.start_enter(name, dest);
                    }
                    _ => log::warn!("Autodrive enter: no escave in reach of {pos:?}"),
                }
            }
            AutoStep::Leave => {
                if self.screen.visit().is_some() {
                    self.sync_weapon_slots();
                    self.screen.begin_leave();
                } else {
                    log::warn!("Autodrive leave: not in an escave");
                }
            }
            AutoStep::Armor(n) => {
                if let Some(player) = self.agents.iter_mut().find(|a| a.spirit == Spirit::Player) {
                    player.armor = n.min(player.max_armor as u32) as u16;
                    log::info!("Autodrive armor {}/{}", player.armor, player.max_armor);
                }
            }
            AutoStep::Mark(name) => log::info!("Autodrive mark {name}"),
            AutoStep::Drive { .. } | AutoStep::Goto { .. } => {}
        }
    }
}

impl Application for Game {
    fn on_key(&mut self, key: winit::keyboard::KeyCode, state: winit::event::ElementState) -> bool {
        use winit::{event::ElementState, keyboard::KeyCode};

        let player = match self.agents.iter_mut().find(|a| a.spirit == Spirit::Player) {
            Some(agent) => agent,
            None => return false,
        };

        match state {
            ElementState::Pressed => match key {
                KeyCode::Escape => return false,
                KeyCode::KeyP => {
                    let center = match player.physics {
                        Physics::Cpu { ref transform, .. } => *transform,
                    };
                    self.input.tick = None;
                    if self.input.is_paused {
                        self.input.is_paused = false;
                        self.cam.loc = center.disp + Vec3::new(0.0, 0.0, 200.0);
                        self.cam.rot = glam::Quat::IDENTITY;
                    } else {
                        self.input.is_paused = true;
                        self.cam.focus_on(&center);
                    }
                }
                KeyCode::Comma => self.input.tick = Some(-1.0),
                KeyCode::Period => self.input.tick = Some(1.0),
                KeyCode::ShiftLeft => self.input.turbo = true,
                KeyCode::KeyM => self.input.mole = true,
                KeyCode::KeyC => {
                    if let Physics::Cpu { ref mut dynamo, .. } = player.physics {
                        dynamo.flotator = !dynamo.flotator;
                        log::info!("cutterig {}", if dynamo.flotator { "on" } else { "off" });
                    }
                }
                KeyCode::Space => self.input.use_entrance = true,
                KeyCode::KeyF => self.input.fire_bay = Some(0),
                KeyCode::KeyG => self.input.fire_bay = Some(1),
                KeyCode::AltLeft => self.input.jump = Some(0.0),
                KeyCode::KeyW => self.input.spin_ver = self.cam.scale.x,
                KeyCode::KeyS => self.input.spin_ver = -self.cam.scale.x,
                KeyCode::KeyR => {
                    if let Physics::Cpu {
                        ref mut transform,
                        ref mut dynamo,
                    } = player.physics
                    {
                        transform.rot = glam::Quat::IDENTITY;
                        dynamo.linear_velocity = Vec3::ZERO;
                        dynamo.angular_velocity = Vec3::ZERO;
                    }
                    player.tracks.reset();
                    // Multiplayer: the server owns the pose, so a local-only
                    // reset is snapped straight back by the next WorldState
                    // (rotation error > SNAP_ROT). Upload it, and skip the
                    // reconcile briefly so in-flight snapshots don't undo it.
                    if self.net.is_some() {
                        self.send_tweaks_pose();
                        self.tweaks_pos_hold = self.tweaks_pos_hold.max(0.25);
                    }
                }
                KeyCode::KeyA => self.input.spin_hor = -self.cam.scale.y,
                KeyCode::KeyD => self.input.spin_hor = self.cam.scale.y,
                KeyCode::KeyQ => {
                    self.input.roll = Some(Roll {
                        dir: -self.cam.scale.y,
                        time: 0.0,
                    })
                }
                KeyCode::KeyE => {
                    self.input.roll = Some(Roll {
                        dir: self.cam.scale.y,
                        time: 0.0,
                    })
                }
                _ => (),
            },
            ElementState::Released => match key {
                KeyCode::KeyW | KeyCode::KeyS => self.input.spin_ver = 0.0,
                KeyCode::KeyA | KeyCode::KeyD => self.input.spin_hor = 0.0,
                KeyCode::KeyQ | KeyCode::KeyE => self.input.roll = None,
                KeyCode::ShiftLeft => self.input.turbo = false,
                KeyCode::KeyM => self.input.mole = false,
                KeyCode::AltLeft => {
                    player.jump = self.input.jump.take();
                }
                _ => (),
            },
        }

        true
    }

    fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, delta: f32) {
        profiling::scope!("Update");

        self.apply_pending_portal(device, queue);
        self.step_autodrive(delta);

        let focus_point = self
            .cam
            .intersect_height(self.level.geometry.height as f32 * 0.3);

        if let Some(ref mut jump) = self.input.jump {
            let power = delta * (self.db.common.speed.standard_frame_rate as f32);
            *jump = (*jump + power).min(self.db.common.force.max_jump_power);
        }

        {
            let player = self
                .agents
                .iter_mut()
                .find(|a| a.spirit == Spirit::Player)
                .unwrap();
            let target = match player.physics {
                Physics::Cpu { ref transform, .. } => *transform,
            };

            if self.input.is_paused {
                if let Some(tick) = self.input.tick.take() {
                    self.line_buffer.clear();
                    player.control.roll = 0.0;

                    player.cpu_step(
                        tick * self.max_quant,
                        &self.level,
                        &self.db.common,
                        SimulationStep::Final {
                            focus_point: &focus_point,
                            line_buffer: Some(&mut self.line_buffer),
                        },
                    );
                }

                self.cam.rotate_focus(
                    &target,
                    2.0 * delta * self.input.spin_hor,
                    delta * self.input.spin_ver,
                );

                // Stepping frame by frame still lays the track down, which
                // is the only way to watch the tread take shape.
                self.step_tracks();
                return;
            }

            player.control.rudder = self.input.spin_hor;
            player.control.motor = 1.0 * self.input.spin_ver;
            player.control.turbo = self.input.turbo;
            player.control.mole = self.input.mole;
            player.control.roll = match self.input.roll {
                Some(ref mut roll) => {
                    let roll_count = (roll.time * self.db.common.speed.standard_frame_rate as f32)
                        .min(100.0) as u8;
                    roll.time += delta;
                    if roll_count > self.db.common.force.side_impulse_delay {
                        roll.time = 0.0;
                    }
                    if roll_count < self.db.common.force.side_impulse_duration {
                        roll.dir
                    } else {
                        0.0
                    }
                }
                None => 0.0,
            };
        }

        if self.input.use_entrance {
            self.input.use_entrance = false;
            self.try_use_entrance();
        }
        if !self.screen.is_world() {
            self.step_screen(delta);
        }
        if self.screen.blocks_drive() {
            if self.screen.follows_camera() {
                self.update_follow_camera(delta);
            }
            return;
        }
        let mut riding = self.step_ride(delta);
        let pad_locked = self.step_portal_pad_lock(delta);

        if let Some(bay) = self.input.fire_bay.take()
            && !self.screen.blocks_drive()
        {
            let player = self
                .agents
                .iter()
                .find(|a| a.spirit == Spirit::Player)
                .unwrap();
            let transform = match player.physics {
                Physics::Cpu { ref transform, .. } => *transform,
            };
            let forward = transform.rot * Vec3::Y;
            if let Some(shot) = weapon::fire(&self.inventory, bay, transform.disp, forward) {
                log::debug!("Fire bay {bay} from {:?} dir {:?}", transform.disp, forward);
                if let Some(ref net) = self.net {
                    // Lined up with our inputs: the shot happens before
                    // the next quantum we send.
                    let dir = shot.vel.normalize();
                    let view = self.world_view.map_or((0, 0), |(tick, at)| {
                        (tick, at.elapsed().as_millis().min(u16::MAX as u128) as u16)
                    });
                    net.send_fire(
                        self.sync.prediction.next(),
                        bay as u8,
                        shot.pos.to_array(),
                        dir.to_array(),
                        view,
                    );
                }
                self.shots.push(LiveShot {
                    shot,
                    age: 0.0,
                    remote: false,
                });
            } else {
                log::debug!("Fire bay {bay}: no weapon");
            }
        }
        for live in &mut self.shots {
            live.shot.step(delta);
            live.age += delta;
        }
        self.shots.retain(|live| live.age < weapon::SHOT_LIFE);

        self.step_moving_land(delta);
        if !riding && !pad_locked {
            riding = self.begin_train_ride();
        }
        if !riding && !pad_locked {
            self.try_begin_escave_visit();
        }
        // Train ride and post-hop pad lock both freeze local player physics.
        let hold_player = riding || pad_locked;
        self.update_passage_proximity();
        self.try_charge_spiral_station();

        if self.flood.step(&mut self.level, delta) {
            self.render.terrain.dirty_flood = true;
        }

        self.step_cycle(delta);
        if !self.cycle.as_ref().is_some_and(|b| b.is_fading()) {
            let range = self.palette.step(&mut self.level, delta);
            self.render.dirty_palette(range);
        }

        const TIME_HACK: f32 = 1.0;
        // Note: the equations below make the game absolutely match the original
        // in terms of time scale for both input and physics.
        // However! the game feels much faster, presumably because of the lack
        // of collision/drag forces that slow you down.
        let input_factor = TIME_HACK * delta / config::common::MAIN_LOOP_TIME;
        let physics_dt = TIME_HACK * delta * {
            let n = &self.db.common.nature;
            let fps = self.db.common.speed.standard_frame_rate as f32;
            fps * n.time_delta0 * n.num_calls_analysis as f32
        };

        // Multiplayer: the server steps the car in whole `max_quant` quanta
        // and applies the jump on the last one of its tick. Do the same for
        // the local car (jump on the last quantum of the frame) so the
        // prediction follows the server's path instead of one that depends
        // on the frame rate. Single player keeps the per-frame split.
        // While the player is held (train ride, pad lock) the quanta still
        // go to the server, with no control, so its copy of the car stays
        // put too instead of running on the last held input.
        let player_quanta = self
            .net
            .is_some()
            .then(|| self.sync.due_quanta(physics_dt, self.max_quant));

        {
            use rayon::prelude::*;

            let clipper = Clipper::new(&self.cam);
            let max_quant = self.max_quant;
            let common = &self.db.common;
            let level = &self.level;

            self.agents.par_iter_mut().for_each(|a| {
                if hold_player && a.spirit == Spirit::Player {
                    return;
                }
                if a.spirit == Spirit::Player && player_quanta.is_some() {
                    // Stepped below, quantum by quantum with its contacts.
                    return;
                }
                let mut dt = physics_dt;
                a.cpu_apply_control(input_factor, common);

                // only go through the full iteration on visible objects
                if !clipper.clip(&level.display_pos(a.position(), focus_point)) {
                    while dt > max_quant {
                        a.cpu_step(max_quant, level, common, SimulationStep::Intermediate);
                        dt -= max_quant;
                    }
                }

                a.cpu_step(
                    dt,
                    level,
                    common,
                    SimulationStep::Final {
                        focus_point: &focus_point,
                        line_buffer: None,
                    },
                );

                a.ai_behavior(delta, level);
            });
        }

        let input_batch = match player_quanta {
            Some(quanta) if hold_player => {
                // The car moves outside its inputs: no history to replay.
                let first = self.sync.skip(quanta);
                (quanta > 0).then(|| (first, quanta, Default::default()))
            }
            Some(quanta) => self
                .step_player_quanta(quanta, &focus_point)
                .map(|(first, input)| (first, quanta, input)),
            None => None,
        };
        self.step_tracks();
        self.step_world_life(delta);

        // Networking: send input and process server events.
        // Send + poll under a short `net` borrow, then handle events with
        // full `&mut self` so cycle sync can touch render/level/agents.
        if self.net.is_some() {
            let (events, my_id) = {
                let net = self.net.as_mut().unwrap();
                if let Some((first, quanta, input)) = input_batch {
                    let input: physics::prediction::QuantInput = input;
                    net.send_input(
                        first,
                        quanta.min(u16::MAX as u32) as u16,
                        &physics::client::net_control(&input),
                    );
                }
                (net.poll(), net.player_id)
            };
            // A slow frame drains several 20 Hz snapshots at once; the
            // newest has the latest ack and is the only one reconciled.
            let newest_world = events
                .iter()
                .rposition(|e| matches!(e, NetEvent::WorldState { .. }));
            for (event_index, event) in events.into_iter().enumerate() {
                match event {
                    NetEvent::Welcome {
                        player_id,
                        level_name,
                    } => {
                        log::info!(
                            "Connected as player {} on level '{}'",
                            player_id,
                            level_name,
                        );
                        self.mp_state.status =
                            format!("Connected (player {}, level '{}')", player_id, level_name);
                        self.mp_state.connected = true;
                    }
                    NetEvent::PlayerJoined {
                        player_id,
                        player_name,
                        car_name,
                        color,
                    } => {
                        if Some(player_id) == my_id {
                            continue;
                        }
                        log::info!(
                            "Remote player {} ({}) joined with car={}",
                            player_id,
                            player_name,
                            car_name,
                        );
                        // Look up the car model in our local database
                        let car_info = self
                            .db
                            .cars
                            .get(&car_name)
                            .or_else(|| self.db.cars.values().next());
                        if let Some(car) = car_info {
                            let body_color = BodyColor::from_value(color);
                            // Keep an existing pose if WorldState already
                            // arrived (or this is a duplicate Joined).
                            self.remote_agents
                                .entry(player_id)
                                .and_modify(|remote| {
                                    remote.name = player_name.clone();
                                    remote.car = car.clone();
                                    remote.net.phys_data = CarPhysicsData::from_car_info(car);
                                    remote.color = body_color;
                                    remote.treads.reset();
                                })
                                .or_insert_with(|| RemoteAgent {
                                    name: player_name.clone(),
                                    car: car.clone(),
                                    color: body_color,
                                    spiral_charge: 0,
                                    net: physics::client::RemoteCar::new(
                                        CarPhysicsData::from_car_info(car),
                                    ),
                                    treads: Default::default(),
                                });
                        }
                    }
                    NetEvent::PlayerLeft { player_id } => {
                        log::info!("Remote player {} left", player_id);
                        self.remote_agents.remove(&player_id);
                    }
                    NetEvent::WorldState {
                        tick,
                        agents,
                        cycle,
                        moving,
                    } => {
                        if let Some(ref cycle_state) = cycle {
                            self.apply_cycle_state(cycle_state);
                        }
                        if let Some(ref moving_state) = moving {
                            self.apply_moving_land_state(moving_state);
                        }
                        self.apply_spiral_from_world(&agents, my_id);
                        self.apply_armor_from_world(&agents, my_id);
                        self.world_view = Some((tick, std::time::Instant::now()));
                        // Remote cars first, so a replay of our inputs
                        // bumps against the ghosts of this same snapshot.
                        let mut ordered: Vec<_> = agents.iter().collect();
                        ordered.sort_by_key(|a| Some(a.player_id) == my_id);
                        for agent_state in ordered {
                            if Some(agent_state.player_id) == my_id {
                                if Some(event_index) != newest_world {
                                    continue;
                                }
                                // Client-side prediction: compare the
                                // server's state with what we predicted
                                // at its ack, and on a mismatch rewind
                                // there and replay our inputs since
                                // (`physics::client`, shared with the web
                                // build). The replay's move is drawn as a
                                // decaying offset. Skipped while the car
                                // is held or the Tweaks Position is being
                                // edited.
                                let player =
                                    self.agents.iter_mut().find(|a| a.spirit == Spirit::Player);
                                if let Some(player) = player
                                    && let Physics::Cpu {
                                        ref mut transform,
                                        ref mut dynamo,
                                    } = player.physics
                                {
                                    let snap =
                                        physics::client::LocalSnapshot::from_agent(agent_state);
                                    let (event, bumped) = self.sync.on_snapshot(
                                        &snap,
                                        physics::client::LocalCar {
                                            transform,
                                            dynamo,
                                            phys: &player.phys_data,
                                        },
                                        &self.remote_agents,
                                        physics::client::Sim {
                                            level: &self.level,
                                            common: &self.db.common,
                                            max_quant: self.max_quant,
                                        },
                                        self.tweaks_pos_hold > 0.0 || hold_player,
                                    );
                                    match event {
                                        physics::client::SyncEvent::FirstSync => {
                                            // First sync: hard-snap camera to
                                            // avoid slow chase from old spawn
                                            // (onto the car, which replayed
                                            // its quanta since the snapshot).
                                            self.cam.focus_on(transform);
                                        }
                                        physics::client::SyncEvent::Held => {}
                                        physics::client::SyncEvent::Reconciled {
                                            outcome,
                                            moved,
                                            shown_at_once,
                                            contact,
                                        } => {
                                            if shown_at_once {
                                                log::warn!(
                                                    "Server correction {:.1}u: shown at once",
                                                    moved
                                                );
                                            } else if moved > physics::client::ClientSync::FADE_LOG
                                            {
                                                log::info!(
                                                    "Server correction {:.1}u{}: faded",
                                                    moved,
                                                    if contact { " in contact" } else { "" }
                                                );
                                            } else if matches!(
                                                outcome,
                                                physics::prediction::Reconciled::Replayed(_)
                                                    | physics::prediction::Reconciled::Ahead
                                            ) {
                                                log::debug!(
                                                    "Reconcile {:?} at ack {}: moved {:.2}u",
                                                    outcome,
                                                    agent_state.ack,
                                                    moved
                                                );
                                            }
                                            if bumped {
                                                let v = transform.rot * dynamo.linear_velocity;
                                                log::info!(
                                                    "Server bump: {:?} vel=({:.2},{:.2},{:.2}) bumps={}",
                                                    outcome,
                                                    v.x,
                                                    v.y,
                                                    v.z,
                                                    agent_state.bumps
                                                );
                                            }
                                        }
                                    }
                                }
                            } else if let Some(remote) =
                                self.remote_agents.get_mut(&agent_state.player_id)
                            {
                                let quanta_per_sec = {
                                    let c = &self.db.common;
                                    c.speed.standard_frame_rate as f32
                                        * c.nature.time_delta0
                                        * c.nature.num_calls_analysis as f32
                                        / self.max_quant
                                };
                                remote.net.on_agent(
                                    agent_state,
                                    tick,
                                    quanta_per_sec,
                                    Some(self.level.period()),
                                );
                            }
                        }
                    }
                    NetEvent::Shot {
                        shooter,
                        bay,
                        origin,
                        dir,
                    } => {
                        let remote = self.remote_agents.get(&shooter);
                        let drawn = remote
                            .filter(|r| r.net.pose_ready)
                            .map(|r| r.net.render_transform.disp);
                        let span = self.level.period();
                        let shot = weapon::remote_shot(
                            Vec3::from(origin),
                            Vec3::from(dir),
                            drawn,
                            (span.x, span.y),
                        );
                        log::debug!(
                            "Remote shot: {} bay {bay} from {:?} dir {:?} (reported {:?})",
                            remote.map_or("?", |r| r.name.as_str()),
                            shot.pos,
                            Vec3::from(dir),
                            Vec3::from(origin),
                        );
                        self.shots.push(LiveShot {
                            shot,
                            age: 0.0,
                            remote: true,
                        });
                    }
                    NetEvent::Disconnected => {
                        log::warn!("Disconnected from server");
                        self.remote_agents.clear();
                        self.mp_state.connected = false;
                        self.mp_state.status = "Disconnected".to_string();
                    }
                }
            }
        }

        if self.tweaks_pos_hold > 0.0 {
            self.tweaks_pos_hold = (self.tweaks_pos_hold - delta).max(0.0);
            // Hold ended without an earlier apply (e.g. panel collapsed):
            // upload so WorldState keeps the edited pose for all clients.
            if self.tweaks_pos_hold <= 0.0 && self.tweaks_pose_dirty {
                self.send_tweaks_pose();
                self.tweaks_pose_dirty = false;
            }
        }

        // Advance remote agent interpolation
        // Server tick rate is ~20 Hz, so each snapshot lasts ~0.05s
        for remote in self.remote_agents.values_mut() {
            remote.net.interpolate(delta);
        }
        if self.net.is_some() {
            self.step_remote_tracks(delta);
            self.sync.decay(delta);
        }

        // Camera follow runs last so it sees the post-physics,
        // post-network-correction transform — no oscillation.
        self.update_follow_camera(delta);
    }

    fn resize(&mut self, device: &wgpu::Device, extent: wgpu::Extent3d) {
        self.cam
            .proj
            .update(extent.width as u16, extent.height as u16);
        self.cave_boot.screen_size = extent;
        self.render.resize(extent, device);
        if let Some(ref mut cave) = self.cave {
            cave.cam
                .proj
                .update(extent.width as u16, extent.height as u16);
            cave.render.resize(extent, device);
        }
    }

    fn reload(&mut self, device: &wgpu::Device) {
        self.render.reload(device);
    }

    fn draw_ui(&mut self, context: &egui::Context) {
        let mut leave_escave = false;
        let mut sync_slots = false;
        if let Some(visit) = self.screen.visit() {
            let see_through = self.cave.is_some();
            let action = escave::draw_interior(
                context,
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
                let (note, slots) = action.apply(
                    visit,
                    &mut self.shop,
                    &mut self.inventory,
                    &mut self.life.beebs,
                );
                if let Some(note) = note {
                    self.escave_note = Some(note);
                }
                sync_slots = slots;
            }
            if action.leave {
                leave_escave = true;
                self.escave_selected = None;
            }
        }
        let shutter = self.screen.shutter();
        if shutter > 0.0 {
            escave::draw_shutters(context, shutter);
        }

        if self.ui.enabled && self.screen.is_world() {
            self.draw_minimap(context);
        }
        if self.ui.enabled {
            self.draw_spiral_hud(context);
        }

        if !self.ui.enabled || !self.screen.is_world() {
            if sync_slots || leave_escave {
                self.sync_weapon_slots();
            }
            if leave_escave {
                self.screen.begin_leave();
            }
            return;
        }
        if !crate::boilerplate::tweaks::expanded(context, &mut self.ui_expanded) {
            if sync_slots || leave_escave {
                self.sync_weapon_slots();
            }
            if leave_escave {
                self.screen.begin_leave();
            }
            return;
        }

        let mut selected_car;
        let mut enter_name: Option<String> = None;
        let mut upload_tweaks_pose = false;
        {
            let player = self
                .agents
                .iter_mut()
                .find(|agent| agent.spirit == Spirit::Player)
                .unwrap();
            selected_car = player.car_name.clone();

            #[allow(deprecated)]
            egui::SidePanel::right("Tweaks").show(context, |ui| {
                ui.horizontal(|ui| {
                    crate::boilerplate::tweaks::collapse_button(ui, &mut self.ui_expanded);
                    ui.label("Tweaks");
                });
                egui::CollapsingHeader::new("Player")
                    .default_open(true)
                    .show(ui, |ui| {
                        egui::ComboBox::from_label("Mechous")
                            .selected_text(&player.car_name)
                            .show_ui(ui, |ui| {
                                for car_name in self.db.cars.keys() {
                                    ui.selectable_value(
                                        &mut selected_car,
                                        car_name.clone(),
                                        car_name,
                                    );
                                }
                            });
                        egui::ComboBox::from_label("Color")
                            .selected_text(player.color.name())
                            .show_ui(ui, |ui| {
                                for &color in &[
                                    BodyColor::Green,
                                    BodyColor::Red,
                                    BodyColor::Blue,
                                    BodyColor::Yellow,
                                    BodyColor::Gray,
                                ] {
                                    ui.selectable_value(&mut player.color, color, color.name());
                                }
                            });
                        if let Physics::Cpu {
                            ref mut transform,
                            dynamo: _,
                        } = player.physics
                        {
                            ui.horizontal(|ui| {
                                ui.label("Position");
                                let rx = ui.add(
                                    egui::DragValue::new(&mut transform.disp.x)
                                        .speed(1.0)
                                        .prefix("x:"),
                                );
                                let ry = ui.add(
                                    egui::DragValue::new(&mut transform.disp.y)
                                        .speed(1.0)
                                        .prefix("y:"),
                                );
                                // Debug MP: keep local edit while DragValue
                                // is focused / dragged, plus a short hold
                                // after release. When the edit session ends,
                                // upload via SetPose so WorldState keeps it.
                                let editing = rx.changed()
                                    || ry.changed()
                                    || rx.dragged()
                                    || ry.dragged()
                                    || rx.has_focus()
                                    || ry.has_focus();
                                if editing {
                                    self.tweaks_pos_hold = 0.75;
                                    self.tweaks_pose_dirty = true;
                                } else if self.tweaks_pose_dirty {
                                    // Defer send: `player` still borrows self.
                                    upload_tweaks_pose = true;
                                    self.tweaks_pose_dirty = false;
                                    // Keep hold briefly so the next snap
                                    // already reflects the uploaded pose.
                                    self.tweaks_pos_hold = 0.75;
                                }
                            });
                        }
                    });
                egui::CollapsingHeader::new("Camera")
                    .default_open(false)
                    .show(ui, |ui| {
                        if let CameraStyle::Follow {
                            ref mut follow,
                            ref mut ground_anchor,
                        } = self.cam_style
                        {
                            self.cam
                                .draw_controls(ui, Some(follow), Some(ground_anchor));
                        } else {
                            self.cam.draw_controls(ui, None, None);
                        }
                    });
                egui::CollapsingHeader::new("Level")
                    .default_open(false)
                    .show(ui, |ui| {
                        self.level.draw_ui(ui);
                    });
                if !self.moving.is_empty() {
                    egui::CollapsingHeader::new("Moving land")
                        .default_open(false)
                        .show(ui, |ui| {
                            Self::draw_moving_land_ui(&self.moving.land, &self.moving.triggers, ui);
                        });
                }
                egui::CollapsingHeader::new("Terrain")
                    .default_open(false)
                    .show(ui, |ui| {
                        Self::draw_terraform_ui(&mut self.terraform, ui);
                    });
                if !self.palette.is_empty() {
                    egui::CollapsingHeader::new("Palette")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.checkbox(&mut self.palette.enabled, "Animate");
                        });
                }
                if self.flood.is_dynamic() {
                    egui::CollapsingHeader::new("Tide")
                        .default_open(false)
                        .show(ui, |ui| {
                            let moved = ui.checkbox(&mut self.flood.enabled, "Drift").changed();
                            ui.add(
                                egui::Slider::new(&mut self.flood.seconds_per_day, 5.0..=600.0)
                                    .text("Seconds per day"),
                            );
                            ui.label(format!(
                                "level {} ({:+.0}%)",
                                self.level.flood_map[0],
                                (self.flood.scale() - 1.0) * 100.0
                            ));
                            if moved {
                                self.flood.apply(&mut self.level);
                                self.render.terrain.dirty_flood = true;
                            }
                        });
                }
                if self.cycle.is_some() {
                    egui::CollapsingHeader::new("Cycle")
                        .default_open(false)
                        .show(ui, |ui| {
                            if let Some(jump) =
                                Self::draw_cycle_ui(self.cycle.as_ref().unwrap(), ui)
                            {
                                let bunch = self.cycle.as_mut().unwrap();
                                let range = bunch.set_cycle(jump, &mut self.level);
                                self.render.dirty_palette(range);
                                self.render.set_light_modulation(bunch.light());
                                self.palette.rebase(&self.level.palette);
                            }
                        });
                }
                egui::CollapsingHeader::new("Renderer")
                    .default_open(false)
                    .show(ui, |ui| {
                        self.render.draw_ui(ui);
                    });
                egui::CollapsingHeader::new("World life")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.label(format!("Beebs: {}", self.life.beebs));
                        ui.label(format!(
                            "Particles: {}  Beebs on the ground: {}",
                            self.life.particles.particles().len(),
                            self.life.swarm.insects().len()
                        ));
                        ui.label(format!(
                            "Spiral {}/{}",
                            self.spiral.charge, self.spiral.capacity
                        ));
                        if let Some(note) = self.passage_note.as_deref() {
                            ui.label(note);
                        }
                        ui.label("Space: enter escave / open tunnel / use passage");
                        ui.label("F/G: fire bays 0/1");
                        ui.add(
                            egui::Slider::new(&mut player.armor, 0..=player.max_armor.max(1))
                                .text("Armour"),
                        );
                        let pos = player.position();
                        let reach = (level::cycle::DELIVERY_RADIUS * 2).max(96);
                        let near = self.db.escaves.iter().find(|e| {
                            let dx = pos.x as i32 - e.coordinates.0;
                            let dy = pos.y as i32 - e.coordinates.1;
                            dx * dx + dy * dy <= reach * reach
                        });
                        if let Some(escave) = near {
                            let name = escave.name.clone();
                            ui.label(format!("Near {name}"));
                            if self.screen.is_world()
                                && ui.button(format!("Enter {name}")).clicked()
                            {
                                enter_name = Some(name);
                            }
                        }
                        if self.screen.visit().is_some() && ui.button("Leave escave").clicked() {
                            leave_escave = true;
                        }
                    });
            });

            if selected_car != player.car_name {
                player.change_car(&self.db.cars[&selected_car], selected_car);
            }
        }
        if upload_tweaks_pose {
            self.send_tweaks_pose();
        }
        if sync_slots || leave_escave {
            self.sync_weapon_slots();
        }
        if let Some(name) = enter_name.take() {
            let dest = self
                .db
                .escaves
                .iter()
                .find(|e| e.name == name)
                .map(|e| {
                    let z = self
                        .agents
                        .iter()
                        .find(|a| a.spirit == Spirit::Player)
                        .map(|a| a.position().z)
                        .unwrap_or(0.0);
                    Vec3::new(e.coordinates.0 as f32, e.coordinates.1 as f32, z - 24.0)
                })
                .unwrap_or(self.cam.loc);
            self.start_enter(name, dest);
        }
        if leave_escave {
            self.screen.begin_leave();
        }

        // Multiplayer panel
        egui::Window::new("Multiplayer")
            .default_open(false)
            .show(context, |ui| {
                if self.mp_state.connected {
                    ui.label(format!("Connected to {}", self.mp_state.server_addr));
                    if let Some(ref net) = self.net
                        && let Some(id) = net.player_id
                    {
                        ui.label(format!("Player ID: {}", id));
                    }
                    ui.label(format!("Remote players: {}", self.remote_agents.len()));
                    ui.label(format!(
                        "Spiral (you): {}/{}",
                        self.spiral.charge, self.spiral.capacity
                    ));
                    for remote in self.remote_agents.values() {
                        let r = remote.net.render_transform.disp;
                        let t = remote.net.target_transform.disp;
                        ui.label(format!(
                            "{} render XY: ({:.2}, {:.2})",
                            remote.name, r.x, r.y
                        ));
                        ui.label(format!(
                            "{} target XY: ({:.2}, {:.2}){}",
                            remote.name,
                            t.x,
                            t.y,
                            if remote.net.pose_ready {
                                ""
                            } else {
                                " (waiting)"
                            }
                        ));
                        ui.label(format!(
                            "Spiral ({}): {}",
                            remote.name, remote.spiral_charge
                        ));
                    }
                    if ui.button("Disconnect").clicked() {
                        self.net = None;
                        self.remote_agents.clear();
                        self.mp_state.connected = false;
                        self.mp_state.status = "Disconnected".to_string();
                    }
                } else {
                    ui.horizontal(|ui| {
                        ui.label("Server:");
                        ui.text_edit_singleline(&mut self.mp_state.server_addr);
                    });
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut self.mp_state.player_name);
                    });
                    if ui.button("Connect").clicked() {
                        let player = self
                            .agents
                            .iter()
                            .find(|a| a.spirit == Spirit::Player)
                            .unwrap();
                        self.net = Some(NetworkClient::connect(
                            &self.mp_state.server_addr,
                            &self.mp_state.player_name,
                            &player.car_name,
                            player.color as u8,
                        ));
                        // Drop offline NPCs so they cannot be mistaken for
                        // remote players (joint playtest false positive).
                        self.agents.retain(|a| a.spirit == Spirit::Player);
                        self.mp_state.connected = true;
                        self.mp_state.status = "Connecting...".to_string();
                    }
                }
                if !self.mp_state.status.is_empty() {
                    ui.label(&self.mp_state.status);
                }
            });
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

        let clipper = Clipper::new(&self.cam);
        self.batcher.clear();
        {
            let eye = self.cam.loc;
            let mut best = (f32::MAX, eye);
            for agent in self.agents.iter() {
                let p = self.level.display_pos(agent.position(), eye);
                let d = (p - eye).length_squared();
                if d < best.0 {
                    best = (d, p);
                }
            }
            self.render
                .set_local_light(best.1, 420.0, [1.0, 0.85, 0.55]);
            if let Some(player) = self.agents.iter().find(|a| a.spirit == Spirit::Player) {
                let p = self.level.display_pos(player.position(), eye);
                let r = (player.phys_data.bbox.radius * player.car.scale).max(8.0);
                // Only punch the see-through cone when terrain still blocks
                // the car after camera avoidance — otherwise solid hills.
                if space::Camera::terrain_blocks_view(&self.level, eye, p, CAMERA_CLEARANCE) {
                    self.render.set_focus(p, r * 1.15);
                } else {
                    self.render.clear_focus();
                }
            } else {
                self.render.clear_focus();
            }
        }

        let eye = self.cam.loc;
        for agent in self.agents.iter() {
            let mut drawn = match agent.physics {
                Physics::Cpu { ref transform, .. } if agent.spirit == Spirit::Player => {
                    self.sync.drawn(transform)
                }
                Physics::Cpu { ref transform, .. } => *transform,
            };
            drawn.disp = self.level.display_pos(drawn.disp, eye);
            if clipper.clip(&drawn.disp) {
                continue;
            }
            let debug_shape_scale = match agent.spirit {
                Spirit::Player => Some(agent.car.physics.scale_bound),
                Spirit::Other { .. } => None,
            };
            self.batcher
                .add_model(&agent.car.model, &drawn, debug_shape_scale, agent.color);
        }

        // Render remote agents from the network (using interpolated transform)
        for remote in self.remote_agents.values() {
            let mut drawn = remote.net.render_transform;
            drawn.disp = self.level.display_pos(drawn.disp, eye);
            if clipper.clip(&drawn.disp) {
                continue;
            }
            self.batcher
                .add_model(&remote.car.model, &drawn, None, remote.color);
        }

        if let Some((ref frames, scale)) = self.bug {
            for insect in self.life.swarm.near(eye, creature::ACTIVE_RADIUS) {
                let disp = self.level.display_pos(insect.pos, eye);
                if clipper.clip(&disp) {
                    continue;
                }
                let mesh = &frames[insect.frame(frames.len())];
                let transform = space::Transform {
                    scale,
                    disp,
                    rot: insect.rotation(),
                };
                // `InsectUnit::CreateInsect`: MATERIAL_1/2/4.
                let color = match insect.tier {
                    0 => m3d::ColorId::Custom1 as u8,
                    1 => m3d::ColorId::Custom2 as u8,
                    _ => m3d::ColorId::Custom4 as u8,
                };
                self.batcher
                    .add_mesh(mesh, Instance::new(&transform, 0.0, color));
            }
        }

        let level = &self.level;
        let add_body = |batcher: &mut Batcher,
                        mesh: &Arc<model::Mesh>,
                        scale: f32,
                        pos: Vec3,
                        heading: f32,
                        color: u8| {
            let disp = level.display_pos(pos, eye);
            if clipper.clip(&disp) {
                return;
            }
            let transform = space::Transform {
                scale,
                disp,
                rot: glam::Quat::from_rotation_z(heading),
            };
            batcher.add_mesh(mesh, Instance::new(&transform, 0.0, color));
        };
        if let Some((ref mesh, scale)) = self.fauna_meshes.farmer {
            for (pos, heading, kernoboo) in self.life.fauna.farmers() {
                let color = if kernoboo {
                    m3d::ColorId::SkyFarmerKernboo as u8
                } else {
                    m3d::ColorId::SkyFarmerPipetka as u8
                };
                add_body(&mut self.batcher, mesh, scale, pos, heading, color);
            }
        }
        if let Some((ref mesh, scale)) = self.fauna_meshes.fish {
            for (pos, heading) in self.life.fauna.fish() {
                add_body(
                    &mut self.batcher,
                    mesh,
                    scale,
                    pos,
                    heading,
                    m3d::ColorId::Body as u8,
                );
            }
        }
        if let Some((ref mesh, scale)) = self.fauna_meshes.clef {
            for pos in self.life.fauna.clefs() {
                add_body(
                    &mut self.batcher,
                    mesh,
                    scale,
                    pos,
                    0.0,
                    m3d::ColorId::Custom5 as u8,
                );
            }
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("World"),
        });

        self.life
            .draw_fx(&mut self.line_buffer, eye, self.bug.is_none());
        for live in &self.shots {
            // The replica next to the camera, as the cars are drawn.
            let (from, to) = live.shot.streak();
            let shown = self.level.display_pos(from, eye);
            let (from, to) = (shown, to - from + shown);
            let color = if live.remote {
                weapon::REMOTE_SHOT_COLOR
            } else {
                weapon::SHOT_COLOR
            };
            self.line_buffer.add(from.to_array(), to.to_array(), color);
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
            Some(&self.line_buffer),
        );

        /*
        self.render.debug.draw_lines(
            &self.line_buffer,
            self.cam.get_view_proj().into(),
            encoder,
        );*/

        encoder.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{AutoDrive, AutoStep};
    use glam::Vec3;

    #[test]
    fn autodrive_parses_segments_and_actions() {
        let ad = AutoDrive::parse(
            "2:0:0, goto:1500:15700, goto:1:2:5, use, fire0, fire1, jump, jump:0.2, \
             reset, tp:590:4140, tp:1:2:90, enter, leave, armor:0, mark:hit",
        )
        .unwrap();
        assert_eq!(
            ad.steps,
            vec![
                AutoStep::Drive {
                    secs: 2.0,
                    motor: 0.0,
                    rudder: 0.0
                },
                AutoStep::Goto {
                    x: 1500.0,
                    y: 15700.0,
                    timeout: 30.0
                },
                AutoStep::Goto {
                    x: 1.0,
                    y: 2.0,
                    timeout: 5.0
                },
                AutoStep::Use,
                AutoStep::Fire(0),
                AutoStep::Fire(1),
                AutoStep::Jump(0.5),
                AutoStep::Jump(0.2),
                AutoStep::Reset,
                AutoStep::Teleport {
                    x: 590.0,
                    y: 4140.0,
                    z: None
                },
                AutoStep::Teleport {
                    x: 1.0,
                    y: 2.0,
                    z: Some(90.0)
                },
                AutoStep::Enter,
                AutoStep::Leave,
                AutoStep::Armor(0),
                AutoStep::Mark("hit".to_string()),
            ]
        );
        for bad in ["1:2", "goto:1", "tp:1:2:3:4", "mark:", "armor:x", "fly:1:2"] {
            assert!(AutoDrive::parse(bad).is_err(), "{bad:?} parsed");
        }
    }

    #[test]
    fn autodrive_steers_towards_the_target() {
        let size = (2048.0, 16384.0);
        // Facing +Y: a target straight ahead needs no rudder.
        let (dist, rudder) = AutoDrive::steer(Vec3::ZERO, Vec3::Y, (0.0, 100.0), size);
        assert!((dist - 100.0).abs() < 1e-3 && rudder.abs() < 1e-3);
        // Targets either side get opposite rudder, the full amount when
        // abeam.
        let (_, right) = AutoDrive::steer(Vec3::ZERO, Vec3::Y, (100.0, 0.0), size);
        let (_, left) = AutoDrive::steer(Vec3::ZERO, Vec3::Y, (-100.0, 0.0), size);
        assert_eq!((right, left), (1.0, -1.0));
        // Across the seam the short way round wins.
        let (dist, _) = AutoDrive::steer(Vec3::new(2040.0, 0.0, 0.0), Vec3::Y, (8.0, 0.0), size);
        assert!((dist - 16.0).abs() < 1e-3, "{dist}");
    }
}
