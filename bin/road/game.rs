use crate::boilerplate::Application;
use crate::net::{NetEvent, NetworkClient};
use m3d::Mesh;
use vangers::{
    config, creature, escave, level, life, model,
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
        } else if (ai.last_transform.disp - transform.disp).length() < 0.05 {
            ai.roll_time = 0.5;
            let x_axis = transform.rot * Vec3::X;
            self.control.roll = x_axis.z.signum();
            self.control.motor = -0.4;
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
    car: config::car::CarInfo,
    color: BodyColor,
    /// Previous snapshot transform (interpolation source).
    prev_transform: space::Transform,
    /// Target snapshot transform (interpolation target).
    target_transform: space::Transform,
    /// Current interpolated transform used for rendering.
    render_transform: space::Transform,
    /// Interpolation progress [0..1], advances each frame.
    interp_t: f32,
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
    input_seq: u32,
    /// Set after the first WorldState snap from the server.
    /// Before this, the client's player position may differ from the
    /// server's spawn point, so we need to hard-snap the camera.
    server_synced: bool,
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
    visit: Option<escave::Visit>,
    data_path: PathBuf,
    /// Original Bug `.a3d` frames, if `game.lst` has one.
    bug: Option<(Vec<Arc<model::Mesh>>, f32)>,
    fauna_meshes: FaunaMeshes,
    /// Space at an escave: fly in, then open the inner visit.
    approach: Option<Approach>,
    /// Secret tunnel: interpolate the car to the other station.
    ride: Option<Ride>,
    shots: Vec<LiveShot>,
}

struct LiveShot {
    shot: weapon::Shot,
    age: f32,
}

struct Approach {
    name: String,
    t: f32,
    start: Vec3,
    dest: Vec3,
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
            let ini_name = match worlds.get(&settings.game.level) {
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
        let mut inventory = escave::Inventory::default();
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
        for good in shop.stock() {
            if good.is_weapon() {
                load_gun(&good.id);
            }
        }
        for sid in &settings.car.slots {
            load_gun(sid);
        }
        hang_weapons(&mut player_agent, &inventory, &weapon_meshes, &db);

        let mut agents = vec![player_agent];
        // populate with random agents
        for i in 0..settings.game.other.count {
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
            input_seq: 0,
            server_synced: false,
            ui: settings.ui,
            ui_expanded: true,
            cam,
            cam_style: CameraStyle::new(&settings.game.camera),
            max_quant: settings.game.physics.max_quant,
            input: Input::default(),
            life,
            inventory,
            shop,
            weapon_meshes,
            visit: None,
            data_path: settings.data_path.clone(),
            bug,
            fauna_meshes,
            shots: Vec::new(),
            approach: None,
            ride: None,
        }
    }

    /// Advances the moving land and hands the touched rectangles to the
    /// renderer. Same [`level::moving::MovingWorld::step`] the web build uses.
    fn step_moving_land(&mut self, delta: f32) {
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

    /// Runs the world's story cycles: every car gathers cirt from whichever
    /// dolly it is near, hands it over on reaching the escave, and once a
    /// stage has had its fill the whole world fades to the next one's
    /// colours.
    fn step_cycle(&mut self, delta: f32) {
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
            for agent in self.agents.iter_mut() {
                let pos = agent.position();
                let coord = (pos.x as i32, pos.y as i32);
                bunch.gather(coord, &mut agent.cirtainer);
                bunch.deliver(coord, &mut agent.cirtainer);
            }
            let step = bunch.quant(&mut self.level);
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

    /// Cuts the stretches the wheels have covered since the last frame into
    /// the level, and hands the touched rectangles to the renderer.
    ///
    /// The wheels record where they have been while the physics runs, which
    /// is over an immutable level and in parallel across the agents. Doing
    /// the cutting here, afterwards, is what lets both of those stand.
    fn step_tracks(&mut self) {
        self.track_regions.clear();
        for agent in self.agents.iter_mut() {
            let reach = (agent.phys_data.bbox.radius * agent.car.scale).max(8.0) as i32;
            let treads = level::terraform::apply_vehicle(
                &mut self.level,
                &self.terraform,
                &mut agent.tracks,
                reach,
                &mut self.track_regions,
            );
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
        let shots: Vec<Vec3> = self.shots.iter().map(|s| s.shot.pos).collect();
        let nibble = self.life.step(&self.level, delta, contact, &shots);
        if nibble != 0 {
            let player = self
                .agents
                .iter_mut()
                .find(|a| a.spirit == Spirit::Player)
                .unwrap();
            player.armor = player.armor.saturating_sub(nibble);
        }
    }

    fn enter_escave(&mut self, name: &str) {
        let mut visit = escave::Visit::enter(name, &self.data_path);
        if let Some(ref mut session) = visit.session {
            session.next_phrase();
        }
        self.visit = Some(visit);
    }

    fn leave_escave(&mut self) {
        self.visit = None;
        self.sync_weapon_slots();
    }

    /// Fold camera and every agent onto the home tile so coordinates
    /// cannot run off to infinity, and a car west of the seam is the
    /// same car you meet again.
    fn rebase_torus(&mut self) {
        let span = self.level.period();
        if span.x <= 0.0 || span.y <= 0.0 {
            return;
        }
        let shift = Vec3::new(
            self.cam.loc.x.div_euclid(span.x) * span.x,
            self.cam.loc.y.div_euclid(span.y) * span.y,
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
        }
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

    fn collect_entrances(&self) -> Vec<escave::Entrance> {
        let mut list: Vec<escave::Entrance> = self
            .db
            .escaves
            .iter()
            .map(|e| escave::Entrance {
                name: e.name.clone(),
                pos: e.coordinates,
                reach: 128,
            })
            .collect();
        for sensor in self.moving.triggers.sensors.iter() {
            let kind = sensor.kind;
            // Passages and secret tunnels are moving-land doors, not
            // dialog rooms. Only escaves and spots open a visit.
            if kind != level::vlc::sensor_kind::ESCAVE && kind != level::vlc::sensor_kind::SPOT {
                continue;
            }
            let name = if sensor.name.is_empty() {
                match kind {
                    level::vlc::sensor_kind::PASSAGE => "Passage".to_string(),
                    level::vlc::sensor_kind::SPOT => "Spot".to_string(),
                    _ => "Escave".to_string(),
                }
            } else {
                sensor.name.clone()
            };
            list.push(escave::Entrance {
                name,
                pos: (sensor.pos.0, sensor.pos.1),
                reach: sensor.radius.max(48) + 48,
            });
        }
        list
    }

    /// Space: poke nearby door sensors so a secret tunnel opens, and if
    /// standing on an escave/spot, fly in then open the inner visit.
    fn try_use_entrance(&mut self) {
        if self.visit.is_some() {
            self.leave_escave();
            return;
        }
        if self.approach.is_some() {
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
        // Space only opens the door. The ride starts once the car has
        // actually fallen into the hole, not from the surface.
        let at = (pos.x as i32, pos.y as i32);
        let pads = self.collect_entrances();
        if let Some(pad) = escave::nearest_entrance(&pads, at) {
            let dest = Vec3::new(pad.pos.0 as f32, pad.pos.1 as f32, pos.z - 24.0);
            self.approach = Some(Approach {
                name: pad.name.clone(),
                t: 0.0,
                start: self.cam.loc,
                dest,
            });
        }
    }

    fn step_approach(&mut self, delta: f32) -> bool {
        let Some(ref mut approach) = self.approach else {
            return false;
        };
        approach.t = (approach.t + delta / 1.2).min(1.0);
        let t = approach.t;
        let s = t * t * (3.0 - 2.0 * t);
        self.cam.loc = approach.start.lerp(approach.dest, s);
        if t >= 1.0 {
            let name = approach.name.clone();
            self.approach = None;
            self.enter_escave(&name);
        }
        true
    }

    /// `EXTERNAL_MODE_MOVE` of a `TrainEngine`: slide the car from this
    /// station to the other over `ActiveTime`.
    fn begin_train_ride(&mut self) -> bool {
        if self.ride.is_some() || self.approach.is_some() {
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
                KeyCode::AltLeft => player.jump = self.input.jump.take(),
                _ => (),
            },
        }

        true
    }

    fn update(&mut self, _device: &wgpu::Device, _queue: &wgpu::Queue, delta: f32) {
        profiling::scope!("Update");

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
        if self.step_approach(delta) {
            return;
        }
        let mut riding = self.step_ride(delta);

        if let Some(bay) = self.input.fire_bay.take()
            && self.visit.is_none()
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
                self.shots.push(LiveShot { shot, age: 0.0 });
            }
        }
        for live in &mut self.shots {
            live.shot.step(delta);
            live.age += delta;
        }
        self.shots.retain(|live| live.age < 1.5);

        self.step_moving_land(delta);
        if !riding {
            riding = self.begin_train_ride();
        }

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

        {
            use rayon::prelude::*;

            let clipper = Clipper::new(&self.cam);
            let max_quant = self.max_quant;
            let common = &self.db.common;
            let level = &self.level;

            self.agents.par_iter_mut().for_each(|a| {
                if riding && a.spirit == Spirit::Player {
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

        self.step_tracks();
        self.step_world_life(delta);

        // Networking: send input and process server events
        if let Some(ref mut net) = self.net {
            // Send local player's control to the server
            let player = self
                .agents
                .iter()
                .find(|a| a.spirit == Spirit::Player)
                .unwrap();
            self.input_seq += 1;
            net.send_input(
                self.input_seq,
                &vangers_net::NetControl {
                    motor: player.control.motor,
                    rudder: player.control.rudder,
                    roll: player.control.roll,
                    brake: player.control.brake,
                    turbo: player.control.turbo,
                    jump: player.jump,
                },
            );

            // Process events from the server
            let my_id = net.player_id;
            for event in net.poll() {
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
                            self.remote_agents.insert(
                                player_id,
                                RemoteAgent {
                                    car: car.clone(),
                                    color: body_color,
                                    prev_transform: space::Transform::IDENTITY,
                                    target_transform: space::Transform::IDENTITY,
                                    render_transform: space::Transform::IDENTITY,
                                    interp_t: 1.0,
                                },
                            );
                        }
                    }
                    NetEvent::PlayerLeft { player_id } => {
                        log::info!("Remote player {} left", player_id);
                        self.remote_agents.remove(&player_id);
                    }
                    NetEvent::WorldState { agents, .. } => {
                        for agent_state in &agents {
                            let server_transform = space::Transform {
                                disp: Vec3::from(agent_state.transform.position),
                                rot: glam::Quat::from_xyzw(
                                    agent_state.transform.rotation[0],
                                    agent_state.transform.rotation[1],
                                    agent_state.transform.rotation[2],
                                    agent_state.transform.rotation[3],
                                ),
                                scale: agent_state.transform.scale,
                            };

                            if Some(agent_state.player_id) == my_id {
                                // Sync local player with server state.
                                // Both client and server run physics independently,
                                // so just snap to keep them consistent.
                                let player =
                                    self.agents.iter_mut().find(|a| a.spirit == Spirit::Player);
                                if let Some(player) = player
                                    && let Physics::Cpu {
                                        ref mut transform,
                                        ref mut dynamo,
                                    } = player.physics
                                {
                                    if !self.server_synced {
                                        // First sync: hard-snap camera to
                                        // avoid slow chase from old spawn.
                                        self.server_synced = true;
                                        self.cam.focus_on(&server_transform);
                                    }
                                    *transform = server_transform;
                                    dynamo.linear_velocity =
                                        Vec3::from(agent_state.dynamo.linear_velocity);
                                    dynamo.angular_velocity =
                                        Vec3::from(agent_state.dynamo.angular_velocity);
                                    dynamo.traction = agent_state.dynamo.traction;
                                    dynamo.rudder = agent_state.dynamo.rudder;
                                }
                            } else if let Some(remote) =
                                self.remote_agents.get_mut(&agent_state.player_id)
                            {
                                // Push current target to prev, set new target
                                remote.prev_transform = remote.target_transform;
                                remote.target_transform = server_transform;
                                remote.interp_t = 0.0;
                            }
                        }
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

        // Advance remote agent interpolation
        // Server tick rate is ~20 Hz, so each snapshot lasts ~0.05s
        let interp_speed = delta * 20.0; // normalize to server tick rate
        for remote in self.remote_agents.values_mut() {
            remote.interp_t = (remote.interp_t + interp_speed).min(1.0);
            let t = remote.interp_t;
            remote.render_transform = space::Transform {
                disp: remote
                    .prev_transform
                    .disp
                    .lerp(remote.target_transform.disp, t),
                rot: remote
                    .prev_transform
                    .rot
                    .slerp(remote.target_transform.rot, t),
                scale: remote.prev_transform.scale
                    + (remote.target_transform.scale - remote.prev_transform.scale) * t,
            };
        }

        // Camera follow runs last so it sees the post-physics,
        // post-network-correction transform — no oscillation.
        {
            let player = self
                .agents
                .iter()
                .find(|a| a.spirit == Spirit::Player)
                .unwrap();
            let mut target = match player.physics {
                Physics::Cpu { ref transform, .. } => *transform,
            };

            if self.visit.is_none() {
                match self.cam_style {
                    CameraStyle::Simple(ref dir) => {
                        self.cam.look_by(&target, dir);
                    }
                    CameraStyle::Follow {
                        ref follow,
                        ground_anchor,
                    } => {
                        if ground_anchor {
                            target.disp.z = self
                                .level
                                .get((target.disp.x as i32, target.disp.y as i32))
                                .high();
                        }
                        self.cam.follow(&target, delta, follow);
                        self.cam.keep_above_ground(&self.level, CAMERA_CLEARANCE);
                    }
                }
            }
            self.rebase_torus();
        }
    }

    fn resize(&mut self, device: &wgpu::Device, extent: wgpu::Extent3d) {
        self.cam
            .proj
            .update(extent.width as u16, extent.height as u16);
        self.render.resize(extent, device);
    }

    fn reload(&mut self, device: &wgpu::Device) {
        self.render.reload(device);
    }

    fn draw_ui(&mut self, context: &egui::Context) {
        if !self.ui.enabled {
            return;
        }
        if !crate::boilerplate::tweaks::expanded(context, &mut self.ui_expanded) {
            return;
        }

        let mut selected_car;
        let mut enter_name: Option<String> = None;
        let mut leave_escave = false;
        let mut sync_slots = false;
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
                                ui.add(
                                    egui::DragValue::new(&mut transform.disp.x)
                                        .speed(1.0)
                                        .prefix("x:"),
                                );
                                ui.add(
                                    egui::DragValue::new(&mut transform.disp.y)
                                        .speed(1.0)
                                        .prefix("y:"),
                                );
                            });
                        }
                    });
                egui::CollapsingHeader::new("Camera")
                    .default_open(false)
                    .show(ui, |ui| {
                        self.cam.draw_ui(ui);
                        if let CameraStyle::Follow {
                            ref mut follow,
                            ref mut ground_anchor,
                        } = self.cam_style
                        {
                            let mut angle_deg = follow.angle_x.to_degrees();
                            ui.add(egui::Slider::new(&mut angle_deg, -105.0..=0.0).text("Angle"));
                            follow.angle_x = angle_deg.to_radians();
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::DragValue::new(&mut follow.offset.x)
                                        .speed(1.0)
                                        .prefix("x:"),
                                );
                                ui.add(
                                    egui::DragValue::new(&mut follow.offset.y)
                                        .speed(1.0)
                                        .prefix("y:"),
                                );
                                ui.add(
                                    egui::DragValue::new(&mut follow.offset.z)
                                        .speed(1.0)
                                        .prefix("z:"),
                                );
                            });
                            ui.add(egui::Slider::new(&mut follow.speed, 0.1..=10.0).text("Speed"));
                            ui.checkbox(ground_anchor, "Ground anchor");
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
                        ui.label("Space: enter escave / open tunnel");
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
                            if self.visit.is_none() && ui.button(format!("Enter {name}")).clicked()
                            {
                                enter_name = Some(name);
                            }
                        }
                        if self.visit.is_some() && ui.button("Leave escave").clicked() {
                            leave_escave = true;
                        }
                    });
            });

            if let Some(ref mut visit) = self.visit {
                let mut leave = false;
                let mut buy: Option<String> = None;
                let mut sell: Option<usize> = None;
                let mut equip: Option<(usize, usize)> = None;
                let mut unequip: Option<usize> = None;
                let mut ask: Option<String> = None;
                let mut next_phrase = false;
                egui::Window::new(format!("Escave: {}", visit.name)).show(context, |ui| {
                    if ui.button("Leave").clicked() {
                        leave = true;
                    }
                    ui.separator();
                    ui.label(format!("Beebs: {}", self.life.beebs));
                    if let Some(ref mut session) = visit.session {
                        if let Some(phrase) = session.last_phrase() {
                            ui.label(phrase);
                        }
                        if !session.ended() && ui.button("Next phrase").clicked() {
                            next_phrase = true;
                        }
                        ui.separator();
                        ui.label("Ask:");
                        for q in session.queries() {
                            if ui.button(q).clicked() {
                                ask = Some(q.clone());
                            }
                        }
                    } else {
                        ui.label("(no dialog data on this path)");
                    }
                    ui.separator();
                    ui.label("Shop");
                    for good in self.shop.stock() {
                        let kind = if good.is_weapon() { "gun" } else { "ware" };
                        if ui
                            .button(format!("Buy {kind} {} ({} beebs)", good.id, good.buy_price))
                            .clicked()
                        {
                            buy = Some(good.id.clone());
                        }
                    }
                    ui.label("Cargo");
                    for (i, good) in self.inventory.items().iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui
                                .button(format!("Sell {} (+{})", good.id, good.sell_price))
                                .clicked()
                            {
                                sell = Some(i);
                            }
                            if good.is_weapon() {
                                if ui.button("Bay 0").clicked() {
                                    equip = Some((i, 0));
                                }
                                if ui.button("Bay 1").clicked() {
                                    equip = Some((i, 1));
                                }
                            }
                        });
                    }
                    ui.label("Bays");
                    for (i, slot) in self.inventory.bays().iter().enumerate() {
                        match slot {
                            Some(good) => {
                                if ui.button(format!("Unequip bay {i}: {}", good.id)).clicked() {
                                    unequip = Some(i);
                                }
                            }
                            None => {
                                ui.label(format!("Bay {i}: empty"));
                            }
                        }
                    }
                });
                if next_phrase && let Some(ref mut session) = visit.session {
                    session.next_phrase();
                }
                if let Some(q) = ask
                    && let Some(ref mut session) = visit.session
                {
                    let _ = session.answer(&q);
                }
                if let Some(id) = buy {
                    let _ = self
                        .shop
                        .buy(&id, &mut self.inventory, &mut self.life.beebs);
                }
                if let Some(i) = sell {
                    let _ = self.shop.sell(i, &mut self.inventory, &mut self.life.beebs);
                }
                if let Some((cargo, bay)) = equip {
                    let _ = self.inventory.equip(cargo, bay);
                    sync_slots = true;
                }
                if let Some(bay) = unequip {
                    let _ = self.inventory.unequip(bay);
                    sync_slots = true;
                }
                if leave {
                    leave_escave = true;
                }
            }

            if selected_car != player.car_name {
                player.change_car(&self.db.cars[&selected_car], selected_car);
            }
        }
        if sync_slots {
            self.sync_weapon_slots();
        }
        if let Some(name) = enter_name.take() {
            self.enter_escave(&name);
        }
        if leave_escave {
            self.leave_escave();
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
        }

        let eye = self.cam.loc;
        for agent in self.agents.iter() {
            let mut drawn = match agent.physics {
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
            let mut drawn = remote.render_transform;
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
        const SHOT_COLOR: u32 = 0xFF44_EEFF;
        for live in &self.shots {
            let dir = if live.shot.vel.length_squared() > 1e-6 {
                live.shot.vel.normalize()
            } else {
                Vec3::Y
            };
            let from = live.shot.pos;
            let to = from + dir * 18.0;
            self.line_buffer
                .add(from.to_array(), to.to_array(), SHOT_COLOR);
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
