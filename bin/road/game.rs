use crate::boilerplate::Application;
use crate::net::{NetEvent, NetworkClient};
use m3d::Mesh;
use vangers::{
    config, level, model,
    physics::{self, CarPhysicsData},
    render::{
        debug::LineBuffer, object::BodyColor, Batcher, GraphicsContext, Render, ScreenTargets,
    },
    space,
};
use vangers_net::PlayerId;

use glam::Vec3;

use std::collections::HashMap;

#[derive(Debug, PartialEq)]
struct Ai {
    last_transform: space::Transform,
    roll_time: f32,
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
        }
    }

    fn change_car(&mut self, car: &config::car::CarInfo, car_name: String) {
        self.phys_data = CarPhysicsData::from_car_info(car);
        self.car = car.clone();
        self.car_name = car_name;
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
        );

        if let Some(focus) = focus_point {
            let wrap = glam::Vec2::new(level.size.0 as f32, (level.size.1 >> 1) as f32);
            let offset = transform.disp - focus;
            transform.disp = focus
                + Vec3::new(
                    (offset.x + 0.5 * wrap.x).rem_euclid(wrap.x) - 0.5 * wrap.x,
                    (offset.y + 0.5 * wrap.y).rem_euclid(wrap.y) - 0.5 * wrap.y,
                    offset.z,
                );
        }
    }

    fn ai_behavior(&mut self, delta: f32) {
        let ai = match self.spirit {
            Spirit::Player => return,
            Spirit::Other(ref mut ai) => ai,
        };
        self.control.motor = 1.0; //full on

        let transform = match self.physics {
            Physics::Cpu { ref transform, .. } => transform,
        };

        if ai.roll_time > 0.0 {
            ai.roll_time -= delta;
            if ai.roll_time <= 0.0 {
                self.control.roll = 0.0;
            }
        } else if ai.last_transform.disp == transform.disp {
            ai.roll_time = 0.5;
            let x_axis = transform.rot * Vec3::X;
            self.control.roll = x_axis.z.signum();
        }

        ai.last_transform = *transform;
    }

    fn position(&self) -> Vec3 {
        match self.physics {
            Physics::Cpu { ref transform, .. } => transform.disp,
        }
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
    _bunches: Vec<config::bunches::Bunch>,
    cars: HashMap<String, config::car::CarInfo>,
    common: config::common::Common,
    _escaves: Vec<config::escaves::Escave>,
    game: config::game::Registry,
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
    jump: Option<f32>,
    roll: Option<Roll>,
    tick: Option<f32>,
}

pub struct Game {
    db: DataBase,
    render: Render,
    batcher: Batcher,
    line_buffer: LineBuffer,
    level: level::Level,
    agents: Vec<Agent>,
    remote_agents: HashMap<PlayerId, RemoteAgent>,
    net: Option<NetworkClient>,
    mp_state: MultiplayerState,
    input_seq: u32,
    ui: config::settings::Ui,
    cam: space::Camera,
    cam_style: CameraStyle,
    max_quant: f32,
    input: Input,
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
        let mut escaves = config::escaves::load(settings.open_relative("escaves.prm"));
        let mut escaves_secondary = config::escaves::load(settings.open_relative("spots.prm"));
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

            let worlds = config::worlds::load(settings.open_relative("wrlds.dat"));
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
                    let pf = space::PerspectiveParams {
                        fovy: 45.0f32.to_radians(),
                        aspect: settings.window.size[0] as f32 / settings.window.size[1] as f32,
                        near: depth.0,
                        far: depth.1,
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
            let game = config::game::Registry::load(settings);
            DataBase {
                _bunches: config::bunches::load(settings.open_relative("bunches.prm")),
                cars: config::car::load_registry(settings, &game, &gfx.device, &render.object),
                common: config::common::load(settings.open_relative("common.prm")),
                _escaves: escaves,
                game,
            }
        };

        log::info!("Loading the level");
        let level = level::load(&level_config, &settings.game.geometry);

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
        for (ms, sid) in player_agent
            .car
            .model
            .slots
            .iter_mut()
            .zip(settings.car.slots.iter())
        {
            let info = &db.game.model_infos[sid];
            let raw = Mesh::load(&mut settings.open_relative(&info.path));
            ms.mesh = Some(model::load_c3d(raw, &gfx.device));
            ms.scale = info.scale;
        }

        let mut agents = vec![player_agent];
        // populate with random agents
        for i in 0..settings.game.other.count {
            use rand::{prelude::SliceRandom, Rng};
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
            NetworkClient::connect(
                addr,
                &player_name,
                &player.car_name,
                player.color as u8,
            )
        });

        Game {
            db,
            render,
            batcher: Batcher::new(),
            line_buffer: LineBuffer::new(),
            level,
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
            ui: settings.ui,
            cam,
            cam_style: CameraStyle::new(&settings.game.camera),
            max_quant: settings.game.physics.max_quant,
            input: Input::default(),
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
            let mut target = match player.physics {
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

                return;
            }

            player.control.rudder = self.input.spin_hor;
            player.control.motor = 1.0 * self.input.spin_ver;
            player.control.turbo = self.input.turbo;
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
                }
            }
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
                let mut dt = physics_dt;
                a.cpu_apply_control(input_factor, common);

                // only go through the full iteration on visible objects
                if !clipper.clip(&a.position()) {
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

                a.ai_behavior(delta);
            });
        }

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
                    NetEvent::Welcome { player_id, level_name } => {
                        log::info!(
                            "Connected as player {} on level '{}'",
                            player_id,
                            level_name,
                        );
                        self.mp_state.status = format!(
                            "Connected (player {}, level '{}')",
                            player_id, level_name
                        );
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
                                // Client-side prediction: soft-correct toward server
                                let player = self
                                    .agents
                                    .iter_mut()
                                    .find(|a| a.spirit == Spirit::Player);
                                if let Some(player) = player {
                                    if let Physics::Cpu {
                                        ref mut transform, ..
                                    } = player.physics
                                    {
                                        // Client-side prediction: blend toward server
                                        // Low rate = trust local physics more (less jitter)
                                        // High rate = trust server more (less desync)
                                        const BLEND: f32 = 0.3;
                                        transform.disp = transform.disp.lerp(
                                            server_transform.disp,
                                            BLEND,
                                        );
                                        transform.rot = transform.rot.slerp(
                                            server_transform.rot,
                                            BLEND,
                                        );
                                        transform.scale = server_transform.scale;
                                    }
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
                disp: remote.prev_transform.disp.lerp(remote.target_transform.disp, t),
                rot: remote.prev_transform.rot.slerp(remote.target_transform.rot, t),
                scale: remote.prev_transform.scale
                    + (remote.target_transform.scale - remote.prev_transform.scale) * t,
            };
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

        let player = self
            .agents
            .iter_mut()
            .find(|agent| agent.spirit == Spirit::Player)
            .unwrap();
        let mut selected_car = &player.car_name;

        #[allow(deprecated)]
        egui::SidePanel::right("Tweaks").show(context, |ui| {
            ui.group(|ui| {
                ui.label("Player:");
                egui::ComboBox::from_label("Mechous")
                    .selected_text(&player.car_name)
                    .show_ui(ui, |ui| {
                        for car_name in self.db.cars.keys() {
                            ui.selectable_value(&mut selected_car, car_name, car_name);
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
            ui.group(|ui| {
                ui.label("Camera:");
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
            ui.group(|ui| {
                ui.label("Level:");
                self.level.draw_ui(ui);
            });
            ui.group(|ui| {
                ui.label("Renderer:");
                self.render.draw_ui(ui);
            });
        });

        if selected_car != &player.car_name {
            let name = selected_car.clone();
            player.change_car(&self.db.cars[&name], name);
        }

        // Multiplayer panel
        egui::Window::new("Multiplayer")
            .default_open(false)
            .show(context, |ui| {
                if self.mp_state.connected {
                    ui.label(format!("Connected to {}", self.mp_state.server_addr));
                    if let Some(ref net) = self.net {
                        if let Some(id) = net.player_id {
                            ui.label(format!("Player ID: {}", id));
                        }
                    }
                    ui.label(format!(
                        "Remote players: {}",
                        self.remote_agents.len()
                    ));
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

    fn draw(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, targets: ScreenTargets) -> wgpu::CommandBuffer {
        let clipper = Clipper::new(&self.cam);
        self.batcher.clear();

        for agent in self.agents.iter() {
            let transform = match agent.physics {
                Physics::Cpu { ref transform, .. } => {
                    if clipper.clip(&transform.disp) {
                        continue;
                    }
                    transform
                }
            };
            let debug_shape_scale = match agent.spirit {
                Spirit::Player => Some(agent.car.physics.scale_bound),
                Spirit::Other { .. } => None,
            };
            self.batcher
                .add_model(&agent.car.model, transform, debug_shape_scale, agent.color);
        }

        // Render remote agents from the network (using interpolated transform)
        for remote in self.remote_agents.values() {
            if clipper.clip(&remote.render_transform.disp) {
                continue;
            }
            self.batcher
                .add_model(&remote.car.model, &remote.render_transform, None, remote.color);
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("World"),
        });

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

        /*
        self.render.debug.draw_lines(
            &self.line_buffer,
            self.cam.get_view_proj().into(),
            encoder,
        );*/

        encoder.finish()
    }
}
