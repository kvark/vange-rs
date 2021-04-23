use crate::{boilerplate::Application, physics};
use m3d::Mesh;
use vangers::{
    config, level, model,
    render::{
        body::{GpuBody, GpuStore, GpuStoreInit},
        collision::{GpuCollider, GpuEpoch},
        debug::LineBuffer,
        object::BodyColor,
        Batcher, Render, ScreenTargets,
    },
    space,
};

use cgmath::prelude::*;
use futures::executor::LocalSpawner;

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
    Gpu {
        body: GpuBody,
        collision_epochs: HashMap<GpuEpoch, usize>,
        last_control: Control,
    },
}

enum SimulationStep<'a> {
    Intermediate,
    Final {
        focus_point: &'a cgmath::Point3<f32>,
        line_buffer: Option<&'a mut LineBuffer>,
    },
}

pub struct Agent {
    _name: String,
    spirit: Spirit,
    car: config::car::CarInfo,
    color: BodyColor,
    control: Control,
    jump: Option<f32>,
    physics: Physics,
}

impl Agent {
    fn spawn(
        name: String,
        car: &config::car::CarInfo,
        color: BodyColor,
        coords: (i32, i32),
        orientation: cgmath::Rad<f32>,
        level: &level::Level,
        gpu_store: Option<&mut GpuStore>,
    ) -> Self {
        let height = physics::get_height(level.get(coords).top()) + 5.; //center offset
        let transform = cgmath::Decomposed {
            scale: car.scale,
            disp: cgmath::vec3(coords.0 as f32, coords.1 as f32, height),
            rot: cgmath::Quaternion::from_angle_z(orientation),
        };

        Agent {
            _name: name,
            spirit: Spirit::Other(Ai {
                last_transform: transform,
                roll_time: 0.0,
            }),
            car: car.clone(),
            color,
            control: Control::default(),
            jump: None,
            physics: match gpu_store {
                Some(store) => Physics::Gpu {
                    body: store.alloc(&transform, &car.model, &car.physics),
                    collision_epochs: HashMap::default(),
                    last_control: Control::default(),
                },
                None => Physics::Cpu {
                    transform,
                    dynamo: physics::Dynamo::default(),
                },
            },
        }
    }

    fn cpu_apply_control(&mut self, dt: f32, common: &config::common::Common) {
        let dynamo = match self.physics {
            Physics::Cpu { ref mut dynamo, .. } => dynamo,
            Physics::Gpu { .. } => return,
        };
        if self.control.rudder != 0.0 {
            let angle = dynamo.rudder.0 + common.car.rudder_step * 2.0 * dt * self.control.rudder;
            dynamo.rudder.0 = angle.min(common.car.rudder_max).max(-common.car.rudder_max);
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
            Physics::Gpu { .. } => return,
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
            &self.car,
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
            let wrap = cgmath::vec2(level.size.0 as f32, (level.size.1 >> 1) as f32);
            let offset = cgmath::Point3::from_vec(transform.disp) - focus;
            transform.disp = focus.to_vec()
                + cgmath::vec3(
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
            Physics::Gpu { .. } => return, //TODO
        };

        if ai.roll_time > 0.0 {
            ai.roll_time -= delta;
            if ai.roll_time <= 0.0 {
                self.control.roll = 0.0;
            }
        } else if ai.last_transform.disp == transform.disp {
            ai.roll_time = 0.5;
            let x_axis = transform.rot * cgmath::Vector3::unit_x();
            self.control.roll = x_axis.z.signum();
        }

        ai.last_transform = *transform;
    }

    fn position(&self) -> cgmath::Vector3<f32> {
        match self.physics {
            Physics::Cpu { ref transform, .. } => transform.disp,
            Physics::Gpu { .. } => cgmath::Vector3::zero(), //TODO
        }
    }
}

struct DataBase {
    _bunches: Vec<config::bunches::Bunch>,
    cars: HashMap<String, config::car::CarInfo>,
    common: config::common::Common,
    _escaves: Vec<config::escaves::Escave>,
    game: config::game::Registry,
}

struct Gpu {
    store: GpuStore,
    collider: GpuCollider,
}

enum CameraStyle {
    Simple(space::Direction),
    Follow(space::Follow),
}

impl CameraStyle {
    fn new(config: &config::settings::Camera) -> Self {
        // the new angle is relative to the surface perpendicular
        let angle = cgmath::Deg(config.angle as f32) - cgmath::Deg::turn_div_4();
        let z = config.height + config.target_overhead;
        if config.speed > 0.0 {
            CameraStyle::Follow(space::Follow {
                transform: cgmath::Decomposed {
                    disp: cgmath::vec3(0.0, angle.tan() * config.height, z),
                    rot: cgmath::Quaternion::from_angle_x(angle),
                    scale: 1.0,
                },
                speed: config.speed,
                fix_z: true,
            })
        } else {
            //Note: this appears to be broken ATM
            CameraStyle::Simple(space::Direction {
                view: cgmath::vec3(0.0, angle.sin(), -angle.cos()),
                height: z,
            })
        }
    }
}

struct Clipper {
    mx_vp: cgmath::Matrix4<f32>,
    threshold: f32,
}

impl Clipper {
    fn new(cam: &space::Camera) -> Self {
        Clipper {
            mx_vp: cam.get_view_proj(),
            threshold: 1.05,
        }
    }

    fn clip(&self, pos: &cgmath::Vector3<f32>) -> bool {
        let p = self.mx_vp * pos.extend(1.0);
        let w = p.w * self.threshold;
        p.x < -w || p.x > w || p.y < -w || p.y > w
    }
}

struct Roll {
    dir: f32,
    time: f32,
}

pub struct Game {
    db: DataBase,
    render: Render,
    batcher: Batcher,
    gpu: Option<Gpu>,
    //debug_collision_map: bool,
    line_buffer: LineBuffer,
    level: level::Level,
    agents: Vec<Agent>,
    cam: space::Camera,
    cam_style: CameraStyle,
    max_quant: f32,
    spin_hor: f32,
    spin_ver: f32,
    turbo: bool,
    jump: Option<f32>,
    roll: Option<Roll>,
    is_paused: bool,
    tick: Option<f32>,
}

impl Game {
    pub fn new(
        settings: &config::Settings,
        screen_extent: wgpu::Extent3d,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Self {
        log::info!("Loading world parameters");
        let (level, coords) = if settings.game.level.is_empty() {
            log::info!("Using test level");
            (level::Level::new_test(), (0, 0))
        } else {
            let escaves = config::escaves::load(settings.open_relative("escaves.prm"));
            let coordinates = escaves
                .iter()
                .find(|e| e.world == settings.game.level)
                .map_or((0, 0), |e| e.coordinates);

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

            let config = level::LevelConfig::load(&ini_path);
            let level = level::load(&config);

            (level, coordinates)
        };

        log::info!("Initializing the render");
        let depth = settings.game.camera.depth_range;
        let pal_data = level::read_palette(settings.open_palette(), Some(&level.terrains));
        let store_init = match settings.game.physics.gpu_collision {
            Some(ref gc) => GpuStoreInit::new(device, gc),
            None => GpuStoreInit::new_dummy(device),
        };
        let render = Render::new(
            device,
            queue,
            &level,
            &pal_data,
            &settings.render,
            screen_extent,
            store_init.resource(),
        );

        log::info!("Loading world database");
        let db = {
            let game = config::game::Registry::load(settings);
            DataBase {
                _bunches: config::bunches::load(settings.open_relative("bunches.prm")),
                cars: config::car::load_registry(settings, &game, device, &render.object),
                common: config::common::load(settings.open_relative("common.prm")),
                _escaves: config::escaves::load(settings.open_relative("escaves.prm")),
                game,
            }
        };

        let mut gpu = settings.game.physics.gpu_collision.as_ref().map(|gc| {
            log::info!("Initializing the GPU store and collider");
            let collider = GpuCollider::new(
                device,
                gc,
                &db.common,
                &render.object,
                &render.terrain,
                store_init.resource(),
            );
            let store = GpuStore::new(device, &db.common, store_init, collider.collision_buffer());
            Gpu { store, collider }
        });

        log::info!("Spawning agents");
        let car_names = db.cars.keys().cloned().collect::<Vec<_>>();
        let mut player_agent = Agent::spawn(
            "Player".to_string(),
            match db.cars.get(&settings.car.id) {
                Some(name) => name,
                None => panic!(
                    "Unknown car '{}', valid names are: {:?}",
                    settings.car.id, car_names
                ),
            },
            settings.car.color,
            coords,
            cgmath::Rad::turn_div_2(),
            &level,
            gpu.as_mut().map(|Gpu { ref mut store, .. }| store),
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
            ms.mesh = Some(model::load_c3d(raw, device));
            ms.scale = info.scale;
        }

        let mut agents = vec![player_agent];
        let mut rng = rand::thread_rng();
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
                color,
                (x, y),
                rng.gen(),
                &level,
                gpu.as_mut().map(|Gpu { ref mut store, .. }| store),
            );
            agents.push(agent);
        }

        Game {
            db,
            render,
            batcher: Batcher::new(),
            gpu,
            line_buffer: LineBuffer::new(),
            level,
            agents,
            cam: space::Camera {
                loc: cgmath::vec3(coords.0 as f32, coords.1 as f32, 200.0),
                rot: cgmath::Quaternion::new(0.0, 0.0, 1.0, 0.0),
                proj: match settings.game.view {
                    config::settings::View::Perspective => {
                        let pf = cgmath::PerspectiveFov {
                            fovy: cgmath::Deg(45.0).into(),
                            aspect: settings.window.size[0] as f32 / settings.window.size[1] as f32,
                            near: depth.0,
                            far: depth.1,
                        };
                        space::Projection::Perspective(pf)
                    }
                    config::settings::View::Flat => space::Projection::ortho(
                        settings.window.size[0] as u16,
                        settings.window.size[1] as u16,
                        depth.0..depth.1,
                    ),
                },
            },
            cam_style: CameraStyle::new(&settings.game.camera),
            max_quant: settings.game.physics.max_quant,
            //debug_collision_map: settings.render.debug.collision_map,
            spin_hor: 0.0,
            spin_ver: 0.0,
            turbo: false,
            jump: None,
            roll: None,
            is_paused: false,
            tick: None,
        }
    }

    fn _move_cam(&mut self, step: f32) {
        let mut back = self.cam.rot * cgmath::Vector3::unit_z();
        back.z = 0.0;
        self.cam.loc -= back.normalize() * step;
    }
}

impl Application for Game {
    fn on_key(&mut self, input: winit::event::KeyboardInput) -> bool {
        use winit::event::{ElementState, KeyboardInput, VirtualKeyCode as Key};

        let player = match self.agents.iter_mut().find(|a| a.spirit == Spirit::Player) {
            Some(agent) => agent,
            None => return false,
        };

        match input {
            KeyboardInput {
                state: ElementState::Pressed,
                virtual_keycode: Some(key),
                ..
            } => match key {
                Key::Escape => return false,
                Key::P => {
                    let center = match player.physics {
                        Physics::Cpu { ref transform, .. } => transform.clone(),
                        Physics::Gpu { ref body, .. } => self
                            .gpu
                            .as_ref()
                            .unwrap()
                            .store
                            .cpu_mirror()
                            .get(body)
                            .unwrap()
                            .clone(),
                    };
                    self.tick = None;
                    if self.is_paused {
                        self.is_paused = false;
                        self.cam.loc = center.disp + cgmath::vec3(0.0, 0.0, 200.0);
                        self.cam.rot = cgmath::Quaternion::new(1.0, 0.0, 0.0, 0.0);
                    } else {
                        self.is_paused = true;
                        self.cam.focus_on(&center);
                    }
                }
                Key::Comma => self.tick = Some(-1.0),
                Key::Period => self.tick = Some(1.0),
                Key::LShift => self.turbo = true,
                Key::LAlt => self.jump = Some(0.0),
                Key::W => self.spin_ver = 1.0,
                Key::S => self.spin_ver = -1.0,
                Key::R => {
                    if let Physics::Cpu {
                        ref mut transform,
                        ref mut dynamo,
                    } = player.physics
                    {
                        transform.rot = cgmath::One::one();
                        dynamo.linear_velocity = cgmath::Vector3::zero();
                        dynamo.angular_velocity = cgmath::Vector3::zero();
                    }
                }
                Key::A => self.spin_hor = -1.0,
                Key::D => self.spin_hor = 1.0,
                Key::Q => {
                    self.roll = Some(Roll {
                        dir: -1.0,
                        time: 0.0,
                    })
                }
                Key::E => {
                    self.roll = Some(Roll {
                        dir: 1.0,
                        time: 0.0,
                    })
                }
                _ => (),
            },
            KeyboardInput {
                state: ElementState::Released,
                virtual_keycode: Some(key),
                ..
            } => match key {
                Key::W | Key::S => self.spin_ver = 0.0,
                Key::A | Key::D => self.spin_hor = 0.0,
                Key::Q | Key::E => self.roll = None,
                Key::LShift => self.turbo = false,
                Key::LAlt => player.jump = self.jump.take(),
                _ => (),
            },
            /*
            Event::KeyboardInput(_, _, Some(Key::R)) =>
                self.cam.rot = self.cam.rot * cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_x(), angle),
            Event::KeyboardInput(_, _, Some(Key::F)) =>
                self.cam.rot = self.cam.rot * cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_x(), -angle),
            */
            /*
            Event::KeyboardInput(_, _, Some(Key::W)) => self.move_cam(step),
            Event::KeyboardInput(_, _, Some(Key::S)) => self.move_cam(-step),
            Event::KeyboardInput(_, _, Some(Key::A)) =>
                self.cam.rot = cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), angle) * self.cam.rot,
            Event::KeyboardInput(_, _, Some(Key::D)) =>
                self.cam.rot = cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), -angle) * self.cam.rot,
            */
            _ => {}
        }

        true
    }

    fn update(
        &mut self,
        device: &wgpu::Device,
        delta: f32,
        spawner: &LocalSpawner,
    ) -> Vec<wgpu::CommandBuffer> {
        let focus_point = self.cam.intersect_height(level::HEIGHT_SCALE as f32 * 0.3);

        if let Some(ref mut jump) = self.jump {
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
                Physics::Cpu { ref transform, .. } => transform.clone(),
                Physics::Gpu { ref body, .. } => self
                    .gpu
                    .as_ref()
                    .unwrap()
                    .store
                    .cpu_mirror()
                    .get(body)
                    .cloned()
                    .unwrap_or(space::Transform::one()),
            };

            if self.is_paused {
                if let Some(tick) = self.tick.take() {
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
                    cgmath::Rad(2.0 * delta * self.spin_hor),
                    cgmath::Rad(delta * self.spin_ver),
                );

                return Vec::new();
            }

            player.control.rudder = self.spin_hor;
            player.control.motor = 1.0 * self.spin_ver;
            player.control.turbo = self.turbo;
            player.control.roll = match self.roll {
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
                CameraStyle::Follow(ref follow) => {
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
        let mut physics_dt = TIME_HACK * delta * {
            let n = &self.db.common.nature;
            let fps = self.db.common.speed.standard_frame_rate as f32;
            fps * n.time_delta0 * n.num_calls_analysis as f32
        };

        if let Some(ref mut gpu) = self.gpu {
            let mut prep_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Preparation"),
            });
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Update"),
            });

            // initialize new entries, update
            for agent in self.agents.iter_mut() {
                if let Physics::Gpu {
                    ref body,
                    ref mut last_control,
                    ..
                } = agent.physics
                {
                    if *last_control != agent.control {
                        *last_control = agent.control.clone();
                        let glob = &self.db.common.global;
                        let c = [
                            agent.control.rudder,
                            agent.control.motor,
                            if agent.control.turbo {
                                glob.k_traction_turbo
                            } else {
                                1.0
                            },
                            if agent.control.brake {
                                glob.f_brake_max
                            } else {
                                0.0
                            },
                        ];
                        gpu.store.update_control(body, c);
                    }
                    if let Some(power) = agent.jump.take() {
                        gpu.store.add_push(body, physics::jump_dir(power));
                    }
                };
            }
            gpu.store.update_entries(device, &mut encoder);

            while physics_dt > self.max_quant {
                let mut session = gpu
                    .collider
                    .begin(&mut encoder, &self.render.terrain, spawner);
                for agent in &mut self.agents {
                    if let Physics::Gpu { ref body, .. } = agent.physics {
                        session.add(&agent.car.model.shape, body.index());
                    }
                }
                let ranges = session.finish(&mut prep_encoder, device);

                gpu.store.step(device, &mut encoder, self.max_quant, ranges);
                physics_dt -= self.max_quant;
            }

            let mut session = gpu
                .collider
                .begin(&mut encoder, &self.render.terrain, spawner);
            for agent in &mut self.agents {
                if let Physics::Gpu {
                    ref body,
                    ref mut collision_epochs,
                    ..
                } = agent.physics
                {
                    let start_index = session.add(&agent.car.model.shape, body.index());
                    let old = collision_epochs.insert(session.epoch, start_index);
                    assert_eq!(old, None);
                }
            }
            let ranges = session.finish(&mut prep_encoder, device);
            gpu.store.step(device, &mut encoder, physics_dt, ranges);
            gpu.store.produce_gpu_results(device, &mut encoder);

            vec![prep_encoder.finish(), encoder.finish()]
        } else {
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

            Vec::new()
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
        if let Some(Gpu {
            ref mut store,
            ref mut collider,
            ..
        }) = self.gpu
        {
            store.reload(device);
            collider.reload(device);
        }
    }

    fn draw(
        &mut self,
        device: &wgpu::Device,
        targets: ScreenTargets,
        spawner: &LocalSpawner,
    ) -> wgpu::CommandBuffer {
        if let Some(ref mut gpu) = self.gpu {
            //Note: we rely on the fact that updates where submitted separately
            gpu.store.consume_gpu_results(spawner);
        }

        let identity_transform = space::Transform::one();
        let clipper = Clipper::new(&self.cam);
        self.batcher.clear();

        for agent in self.agents.iter() {
            let (gpu_body, transform) = match agent.physics {
                Physics::Cpu { ref transform, .. } => {
                    if clipper.clip(&transform.disp) {
                        continue;
                    }
                    (&GpuBody::ZERO, transform)
                }
                Physics::Gpu { ref body, .. } => (body, &identity_transform),
            };
            let debug_shape_scale = match agent.spirit {
                Spirit::Player => Some(agent.car.physics.scale_bound),
                Spirit::Other { .. } => None,
            };
            self.batcher.add_model(
                &agent.car.model,
                &transform,
                debug_shape_scale,
                gpu_body,
                agent.color,
            );
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Draw"),
        });

        self.render
            .draw_world(&mut encoder, &mut self.batcher, &self.cam, targets, device);

        /*
        self.render.debug.draw_lines(
            &self.line_buffer,
            self.cam.get_view_proj().into(),
            encoder,
        );*/

        encoder.finish()
    }
}
