use crate::{
    boilerplate::Application,
    physics,
};
use m3d::Mesh;
use vangers::{
    config, level, model, space,
    render::{
        Render, RenderModel, ScreenTargets,
        instantiate_visual_model,
        body::{GpuBody, GpuStore, GpuStoreInit},
        collision::{GpuCollider, GpuEpoch},
        debug::LineBuffer,
    },
};

use cgmath::prelude::*;
use futures::executor::LocalSpawner;

use std::{
    collections::HashMap,
};


#[derive(Eq, PartialEq)]
enum Spirit {
    Player,
    Other,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Control {
    motor: f32,
    rudder: f32,
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

pub struct Agent {
    _name: String,
    spirit: Spirit,
    car: config::car::CarInfo,
    instance_buf: wgpu::Buffer,
    dirty_uniforms: bool,
    control: Control,
    jump: Option<f32>,
    physics: Physics,
}

impl Agent {
    fn spawn(
        name: String,
        car: &config::car::CarInfo,
        coords: (i32, i32),
        orientation: cgmath::Rad<f32>,
        level: &level::Level,
        device: &wgpu::Device,
        gpu_store: Option<&mut GpuStore>,
    ) -> Self {
        let height = physics::get_height(level.get(coords).top()) + 5.; //center offset
        let transform = cgmath::Decomposed {
            scale: car.scale,
            disp: cgmath::vec3(coords.0 as f32, coords.1 as f32, height),
            rot: cgmath::Quaternion::from_angle_z(orientation),
        };
        let instance_buf = instantiate_visual_model(&car.model, device);

        Agent {
            _name: name,
            spirit: Spirit::Other,
            car: car.clone(),
            instance_buf,
            dirty_uniforms: true,
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
            let angle = dynamo.rudder.0 +
                common.car.rudder_step * 2.0 * dt * self.control.rudder;
            dynamo.rudder.0 = angle
                .min(common.car.rudder_max)
                .max(-common.car.rudder_max);
        }
        if self.control.motor != 0.0 {
            dynamo
                .change_traction(self.control.motor * dt * common.car.traction_incr);
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
        line_buffer: Option<&mut LineBuffer>,
    ) {
        let (dynamo, transform) = match self.physics {
            Physics::Cpu { ref mut transform, ref mut dynamo } => (dynamo, transform),
            Physics::Gpu { .. } => return,
        };
        self.dirty_uniforms = true;
        physics::step(
            dynamo,
            transform,
            dt,
            &self.car,
            level,
            common,
            if self.control.turbo { common.global.k_traction_turbo } else { 1.0 },
            if self.control.brake { common.global.f_brake_max } else { 0.0 },
            self.jump.take(),
            line_buffer,
        )
    }

    fn to_render_model(&self) -> RenderModel {
        let (gpu_body, transform) = match self.physics {
            Physics::Cpu { ref transform, .. } => (&GpuBody::ZERO, transform.clone()),
            Physics::Gpu { ref body, .. } => (body, space::Transform::one()),
        };
        RenderModel {
            model: &self.car.model,
            gpu_body,
            instance_buf: &self.instance_buf,
            transform,
            debug_shape_scale: match self.spirit {
                Spirit::Player => Some(self.car.physics.scale_bound),
                Spirit::Other => None,
            },
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
        let angle = cgmath::Deg(config.angle as f32);
        if config.speed > 0.0 {
            CameraStyle::Follow(space::Follow {
                transform: cgmath::Decomposed {
                    disp: cgmath::vec3(
                        0.0,
                        -angle.cos() * config.height,
                        config.height + config.target_height_offset,
                    ),
                    rot: cgmath::Quaternion::from_angle_x(cgmath::Deg::turn_div_4() - angle),
                    scale: 1.0,
                },
                speed: config.speed,
                fix_z: true,
            })
        } else {
            CameraStyle::Simple(space::Direction {
                view: cgmath::vec3(0.0, angle.cos(), -angle.sin()),
                height: config.height,
            })
        }
    }
}

pub struct Game {
    db: DataBase,
    render: Render,
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
    is_paused: bool,
    tick: Option<f32>,
}


impl Game {
    pub fn new(
        settings: &config::Settings,
        screen_extent: wgpu::Extent3d,
        device: &wgpu::Device,
        queue: &mut wgpu::Queue,
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
            let ini_name = &worlds[&settings.game.level];
            let ini_path = settings.data_path.join(ini_name);
            log::info!("Using level {}", ini_name);

            let config = level::LevelConfig::load(&ini_path);
            let level = level::load(&config);

            (level, coordinates)
        };

        log::info!("Initializing the render");
        let depth = 10f32 .. 10000f32;
        let pal_data = level::read_palette(settings.open_palette(), Some(&level.terrains));
        let store_init = match settings.game.physics.gpu_collision {
            Some(ref gc) => GpuStoreInit::new(device, gc),
            None => GpuStoreInit::new_dummy(device),
        };
        let render = Render::new(device, queue, &level, &pal_data, &settings.render, screen_extent, store_init.resource());

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
            let collider = GpuCollider::new(device, gc, &db.common, &render.object, &render.terrain, store_init.resource());
            let store = GpuStore::new(device, &db.common, store_init, collider.collision_buffer());
            Gpu {
                store,
                collider,
            }
        });

        log::info!("Spawning agents");
        let mut player_agent = Agent::spawn(
            "Player".to_string(),
            &db.cars[&settings.car.id],
            coords,
            cgmath::Rad::turn_div_2(),
            &level,
            device,
            gpu.as_mut().map(|Gpu { ref mut store, .. }| store),
        );
        player_agent.spirit = Spirit::Player;
        let slot_locals_id = player_agent.car.model.mesh_count() - player_agent.car.model.slots.len();
        for (i, (ms, sid)) in player_agent.car.model.slots
            .iter_mut()
            .zip(settings.car.slots.iter())
            .enumerate()
        {
            let info = &db.game.model_infos[sid];
            let raw = Mesh::load(&mut settings.open_relative(&info.path));
            ms.mesh = Some(model::load_c3d(
                raw,
                device,
                slot_locals_id + i,
            ));
            ms.scale = info.scale;
        }

        let mut agents = vec![player_agent];
        let mut rng = rand::thread_rng();
        let car_names = db.cars.keys().cloned().collect::<Vec<_>>();
        // populate with random agents
        for i in 0 .. settings.game.other.count {
            use rand::{Rng, prelude::SliceRandom};
            let car_id = car_names.choose(&mut rng).unwrap();
            let x = rng.gen_range(0, level.size.0);
            let y = rng.gen_range(0, level.size.1);
            let mut agent = Agent::spawn(
                format!("Other-{}", i),
                &db.cars[car_id],
                (x, y),
                cgmath::Rad(rng.gen()),
                &level,
                device,
                gpu.as_mut().map(|Gpu { ref mut store, .. }| store),
            );
            agent.control.motor = 1.0; //full on
            agents.push(agent);
        }

        Game {
            db,
            render,
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
                            near: depth.start,
                            far: depth.end,
                        };
                        space::Projection::Perspective(pf)
                    }
                    config::settings::View::Flat => {
                        space::Projection::ortho(
                            settings.window.size[0] as u16,
                            settings.window.size[1] as u16,
                            depth,
                        )
                    }
                },
            },
            cam_style: CameraStyle::new(&settings.game.camera),
            max_quant: settings.game.physics.max_quant,
            //debug_collision_map: settings.render.debug.collision_map,
            spin_hor: 0.0,
            spin_ver: 0.0,
            turbo: false,
            jump: None,
            is_paused: false,
            tick: None,
        }
    }

    fn _move_cam(
        &mut self,
        step: f32,
    ) {
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
                        Physics::Gpu { ref body, .. } => {
                            self.gpu.as_ref().unwrap()
                                .store.cpu_mirror()
                                .get(body).unwrap().clone()
                        },
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
                    if let Physics::Cpu { ref mut transform ,ref mut dynamo } = player.physics {
                        player.dirty_uniforms = true;
                        transform.rot = cgmath::One::one();
                        dynamo.linear_velocity = cgmath::Vector3::zero();
                        dynamo.angular_velocity = cgmath::Vector3::zero();
                    }
                }
                Key::A => self.spin_hor = -1.0,
                Key::D => self.spin_hor = 1.0,
                _ => (),
            }
            KeyboardInput {
                state: ElementState::Released,
                virtual_keycode: Some(key),
                ..
            } => match key {
                Key::W | Key::S => self.spin_ver = 0.0,
                Key::A | Key::D => self.spin_hor = 0.0,
                Key::LShift => self.turbo = false,
                Key::LAlt => player.jump = self.jump.take(),
                _ => (),
            }
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
        if let Some(ref mut jump) = self.jump {
            let power = delta * (self.db.common.speed.standard_frame_rate as f32);
            *jump = (*jump + power).min(self.db.common.force.max_jump_power);
        }

        {
            let player = self.agents
                .iter_mut()
                .find(|a| a.spirit == Spirit::Player)
                .unwrap();
            let target = match player.physics {
                Physics::Cpu { ref transform, .. } => transform.clone(),
                Physics::Gpu { ref body, .. } => {
                    self.gpu.as_ref().unwrap()
                        .store.cpu_mirror().get(body).cloned()
                        .unwrap_or(space::Transform::one())
                },
            };

            if self.is_paused {
                if let Some(tick) = self.tick.take() {
                    self.line_buffer.clear();
                    player.cpu_step(
                        tick * self.max_quant,
                        &self.level,
                        &self.db.common,
                        Some(&mut self.line_buffer),
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
                todo: 0,
            });
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                todo: 0,
            });

            // initialize new entries, update
            for agent in self.agents.iter_mut() {
                if agent.dirty_uniforms {
                    agent.dirty_uniforms = false;
                    agent.to_render_model().prepare(&mut prep_encoder, device);
                }

                if let Physics::Gpu { ref body, ref mut last_control, .. } = agent.physics {
                    if *last_control != agent.control {
                        *last_control = agent.control.clone();
                        let glob = &self.db.common.global;
                        let c = [
                            agent.control.rudder,
                            agent.control.motor,
                            if agent.control.turbo { glob.k_traction_turbo } else { 1.0 },
                            if agent.control.brake { glob.f_brake_max } else { 0.0 },
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
                let mut session = gpu.collider.begin(
                    &mut encoder,
                    &self.render.terrain,
                    spawner,
                );
                for agent in &mut self.agents {
                    if let Physics::Gpu { ref body, .. } = agent.physics {
                        session.add(&agent.car.model.shape, body.index());
                    }
                }
                let ranges = session.finish(&mut prep_encoder, device);

                gpu.store.step(device, &mut encoder, self.max_quant, ranges);
                physics_dt -= self.max_quant;
            }

            let mut session = gpu.collider.begin(&mut encoder, &self.render.terrain, spawner);
            for agent in &mut self.agents {
                if let Physics::Gpu { ref body, ref mut collision_epochs, .. } = agent.physics {
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
            for a in self.agents.iter_mut() {
                a.cpu_apply_control(
                    input_factor,
                    &self.db.common,
                );
            }

            while physics_dt > self.max_quant {
                for a in self.agents.iter_mut() {
                    a.cpu_step(
                        self.max_quant,
                        &self.level,
                        &self.db.common,
                        None,
                    );
                }
                physics_dt -= self.max_quant;
            }

            self.line_buffer.clear();

            for a in self.agents.iter_mut() {
                let lbuf = match a.spirit {
                    Spirit::Player => Some(&mut self.line_buffer),
                    Spirit::Other => None,
                };
                a.cpu_step(
                    physics_dt,
                    &self.level,
                    &self.db.common,
                    lbuf,
                );
            }

            Vec::new()
        }
    }

    fn resize(&mut self, device: &wgpu::Device, extent: wgpu::Extent3d) {
        self.cam.proj.update(extent.width as u16, extent.height as u16);
        self.render.resize(extent, device);
    }

    fn reload(&mut self, device: &wgpu::Device) {
        self.render.reload(device);
        if let Some(Gpu{ ref mut store, ref mut collider, .. }) = self.gpu {
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

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            todo: 0,
        });

        let models = self.agents
            .iter()
            .map(Agent::to_render_model)
            .collect::<Vec<_>>();
        for (rm, agent) in models.iter().zip(self.agents.iter()) {
            if agent.dirty_uniforms {
                rm.prepare(&mut encoder, device);
            }
        }

        self.render.draw_world(
            &mut encoder,
            &models,
            &self.cam,
            targets,
            device,
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
