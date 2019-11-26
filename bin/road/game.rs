use crate::{
    boilerplate::Application,
    physics,
};
use m3d::Mesh;
use vangers::{
    config, level, model, space,
    render::{
        Render, RenderModel, ScreenTargets,
        collision::{GpuCollider, GpuResult},
        debug::LineBuffer,
    },
};

use cgmath::{
    self,
    Angle, Rotation3, Zero,
};

use std::{
    collections::HashMap,
    sync::MutexGuard,
};


#[derive(Eq, PartialEq)]
enum Spirit {
    Player,
    Other,
}

#[derive(Debug, Default)]
struct Control {
    motor: f32,
    rudder: f32,
    brake: bool,
    turbo: bool,
}

pub struct Agent {
    _name: String,
    spirit: Spirit,
    pub transform: space::Transform,
    pub car: config::car::CarInfo,
    dynamo: physics::Dynamo,
    control: Control,
    gpu_physics: physics::GpuLink,
}

impl Agent {
    fn spawn(
        name: String,
        car: &config::car::CarInfo,
        coords: (i32, i32),
        orientation: cgmath::Rad<f32>,
        level: &level::Level,
    ) -> Self {
        let height = physics::get_height(level.get(coords).top()) + 5.; //center offset
        Agent {
            _name: name,
            spirit: Spirit::Other,
            transform: cgmath::Decomposed {
                scale: car.scale,
                disp: cgmath::vec3(coords.0 as f32, coords.1 as f32, height),
                rot: cgmath::Quaternion::from_angle_z(orientation),
            },
            car: car.clone(),
            dynamo: physics::Dynamo::default(),
            control: Control::default(),
            gpu_physics: physics::GpuLink::default(),
        }
    }

    fn apply_control(&mut self, dt: f32, common: &config::common::Common) {
        if self.control.rudder != 0.0 {
            let angle = self.dynamo.rudder.0 +
                common.car.rudder_step * 2.0 * dt * self.control.rudder;
            self.dynamo.rudder.0 = angle
                .min(common.car.rudder_max)
                .max(-common.car.rudder_max);
        }
        if self.control.motor != 0.0 {
            self.dynamo
                .change_traction(self.control.motor * dt * common.car.traction_incr);
        }
        if self.control.brake && self.dynamo.traction != 0.0 {
            self.dynamo.traction *= (-dt).exp2();
        }
    }

    fn step(
        &mut self,
        dt: f32,
        level: &level::Level,
        common: &config::common::Common,
        gpu_latest: Option<&MutexGuard<GpuResult>>,
        line_buffer: Option<&mut LineBuffer>,
    ) {
        physics::step(
            &mut self.dynamo,
            &mut self.transform,
            dt,
            &self.car,
            level,
            common,
            if self.control.turbo { common.global.k_traction_turbo } else { 1.0 },
            if self.control.brake { common.global.f_brake_max } else { 0.0 },
            &mut self.gpu_physics,
            gpu_latest,
            line_buffer,
        )
    }
}

struct DataBase {
    _bunches: Vec<config::bunches::Bunch>,
    cars: HashMap<String, config::car::CarInfo>,
    common: config::common::Common,
    _escaves: Vec<config::escaves::Escave>,
    game: config::game::Registry,
}

pub struct Game {
    db: DataBase,
    render: Render,
    collider: Option<GpuCollider>,
    //debug_collision_map: bool,
    line_buffer: LineBuffer,
    level: level::Level,
    agents: Vec<Agent>,
    cam: space::Camera,
    max_quant: f32,
    spin_hor: f32,
    spin_ver: f32,
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
        let render = Render::new(device, queue, &level, &pal_data, &settings.render, screen_extent);

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

        let collider = settings.game.physics.gpu_collision.as_ref().map(|gc| {
            GpuCollider::new(device, gc, &db.common, &render.object, &render.terrain)
        });

        let mut player_agent = Agent::spawn(
            "Player".to_string(),
            &db.cars[&settings.car.id],
            coords,
            cgmath::Rad::turn_div_2(),
            &level,
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
                &player_agent.car.locals_buf,
                slot_locals_id + i,
                &render.object,
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
            );
            agent.control.motor = 1.0; //full on
            agents.push(agent);
        }

        Game {
            db,
            render,
            collider,
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
            max_quant: settings.game.physics.max_quant,
            //debug_collision_map: settings.render.debug.collision_map,
            spin_hor: 0.0,
            spin_ver: 0.0,
            is_paused: false,
            tick: None,
        }
    }

    fn _move_cam(
        &mut self,
        step: f32,
    ) {
        use cgmath::InnerSpace;
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
                    let center = &player.transform;
                    self.tick = None;
                    if self.is_paused {
                        self.is_paused = false;
                        self.cam.loc = center.disp + cgmath::vec3(0.0, 0.0, 200.0);
                        self.cam.rot = cgmath::Quaternion::new(1.0, 0.0, 0.0, 0.0);
                    } else {
                        self.is_paused = true;
                        self.cam.focus_on(center);
                    }
                }
                Key::Comma => self.tick = Some(-1.0),
                Key::Period => self.tick = Some(1.0),
                Key::W => self.spin_ver = 1.0,
                Key::S => self.spin_ver = -1.0,
                Key::R => {
                    player.transform.rot = cgmath::One::one();
                    player.dynamo.linear_velocity = cgmath::Vector3::zero();
                    player.dynamo.angular_velocity = cgmath::Vector3::zero();
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

    fn update(&mut self, delta: f32) {
        let pid = self.agents
            .iter()
            .position(|a| a.spirit == Spirit::Player)
            .unwrap();

        if self.is_paused {
            let player = &mut self.agents[pid];
            if let Some(tick) = self.tick.take() {
                self.line_buffer.clear();
                player.step(
                    tick * self.max_quant,
                    &self.level,
                    &self.db.common,
                    None,
                    Some(&mut self.line_buffer),
                );
            }
            self.cam.rotate_focus(
                &player.transform,
                cgmath::Rad(2.0 * delta * self.spin_hor),
                cgmath::Rad(delta * self.spin_ver),
            );
        } else {
            self.agents[pid].control.rudder = self.spin_hor;
            self.agents[pid].control.motor = 1.0 * self.spin_ver;

            if true {
                self.cam.follow(
                    &self.agents[pid].transform,
                    delta,
                    &space::Follow {
                        transform: cgmath::Decomposed {
                            disp: cgmath::vec3(0.0, -300.0, 500.0),
                            rot: cgmath::Quaternion::from_angle_x(cgmath::Rad(0.7)),
                            scale: 1.0,
                        },
                        speed: 100.0,
                        fix_z: true,
                    },
                );
            } else {
                self.cam.look_by(
                    &self.agents[pid].transform,
                    &space::Direction {
                        view: cgmath::vec3(0.0, 1.0, -3.0),
                        height: 200.0,
                    },
                );
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

            for a in self.agents.iter_mut() {
                a.apply_control(
                    input_factor,
                    &self.db.common,
                );
            }

            // don't quantify if going through GPU, for now
            if self.collider.is_none() {
                while physics_dt > self.max_quant {
                    for a in self.agents.iter_mut() {
                        a.step(
                            self.max_quant,
                            &self.level,
                            &self.db.common,
                            None,
                            None,
                        );
                    }
                    physics_dt -= self.max_quant;
                }
            }

            self.line_buffer.clear();
            let gpu_result = self.collider
                .as_ref()
                .map(GpuCollider::result);

            for a in self.agents.iter_mut() {
                let lbuf = match a.spirit {
                    Spirit::Player => Some(&mut self.line_buffer),
                    Spirit::Other => None,
                };
                a.step(
                    physics_dt,
                    &self.level,
                    &self.db.common,
                    gpu_result.as_ref(),
                    lbuf,
                );
            }
        }
    }

    fn resize(&mut self, device: &wgpu::Device, extent: wgpu::Extent3d) {
        self.cam.proj.update(extent.width as u16, extent.height as u16);
        self.render.resize(extent, device);
    }

    fn reload(&mut self, device: &wgpu::Device) {
        self.render.reload(device);
        if let Some(ref mut collider) = self.collider {
            collider.reload(device);
        }
    }

    fn draw(
        &mut self,
        device: &wgpu::Device,
        targets: ScreenTargets,
    ) -> Vec<wgpu::CommandBuffer> {
        let mut combs = Vec::new();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            todo: 0,
        });

        let post_comb = match self.collider {
            Some(ref mut collider) => {
                let mut session = collider.begin(&mut encoder, &self.render.terrain);
                for agent in &mut self.agents {
                    let mut transform = agent.transform.clone();
                    transform.scale *= agent.car.physics.scale_bound;
                    let start_index = session.add(&agent.car.model.shape, transform);
                    let old = agent.gpu_physics.insert(session.epoch, start_index);
                    assert_eq!(old, None);
                }
                let (pre, post) = session.finish(device);
                combs.push(pre);
                Some(post)
            }
            None => None,
        };

        let models = self.agents
            .iter()
            .map(|a| RenderModel {
                model: &a.car.model,
                locals_buf: &a.car.locals_buf,
                transform: a.transform.clone(),
                debug_shape_scale: match a.spirit {
                    Spirit::Player => Some(a.car.physics.scale_bound),
                    Spirit::Other => None,
                },
            })
            .collect::<Vec<_>>();
        self.render.draw_world(
            &mut encoder,
            &models,
            &self.cam,
            targets,
            device,
        );
        combs.push(encoder.finish());

        /*
        self.render.debug.draw_lines(
            &self.line_buffer,
            self.cam.get_view_proj().into(),
            encoder,
        );*/

        /*
        if self.compute_gpu_collision {
            let mut collider = self.collider.start(encoder, &self.db.common);
            for agent in &mut self.agents {
                let mut transform = agent.transform.clone();
                transform.scale *= agent.car.physics.scale_bound;
                let shape_id = collider.add(&agent.car.model.shape, transform);
                agent.gpu_momentum = Some(GpuMomentum::Pending(shape_id));
            }

            let debug_blit = if self.debug_collision_map {
                let target = self.render.target_color();
                self.agents
                    .iter()
                    .find(|a| a.spirit == Spirit::Player)
                    .and_then(|a| match a.gpu_momentum {
                        Some(GpuMomentum::Pending(ref shape)) => Some(render::DebugBlit {
                            target,
                            shape: shape.clone(),
                            scale: 4,
                        }),
                        _ => None,
                    })
            } else {
                None
            };

            let results = collider.finish(debug_blit);
            for agent in &mut self.agents {
                if let Some(GpuMomentum::Pending(shape_id)) = agent.gpu_momentum.take() {
                    let rect = results[shape_id];
                    assert_eq!((rect.y, rect.w, rect.h), (0, 1, 1));
                    agent.gpu_momentum = Some(GpuMomentum::Computed(rect.x as usize));
                }
            }
        }*/

        combs.extend(post_comb);
        combs
    }
}
