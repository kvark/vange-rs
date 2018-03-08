use cgmath;
use cgmath::{Angle, Rotation3, Zero};
use gfx;
use rand;
use std::collections::HashMap;

use boilerplate::{Application, KeyboardInput};
use physics;
use vangers::{config, level, render, space};


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

pub struct Agent<R: gfx::Resources> {
    _name: String,
    spirit: Spirit,
    pub transform: space::Transform,
    pub car: config::car::CarInfo<R>,
    dynamo: physics::Dynamo,
    control: Control,
}

impl<R: gfx::Resources> Agent<R> {
    fn spawn(
        name: String,
        car: &config::car::CarInfo<R>,
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
        line_buffer: Option<&mut render::LineBuffer>,
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
            line_buffer,
        )
    }
}

struct DataBase<R: gfx::Resources> {
    _bunches: Vec<config::bunches::Bunch>,
    cars: HashMap<String, config::car::CarInfo<R>>,
    common: config::common::Common,
    _escaves: Vec<config::escaves::Escave>,
    _game: config::game::Registry,
}

pub struct Game<R: gfx::Resources> {
    db: DataBase<R>,
    render: render::Render<R>,
    collider: render::GpuCollider<R>,
    line_buffer: render::LineBuffer,
    level: level::Level,
    agents: Vec<Agent<R>>,
    cam: space::Camera,
    max_quant: f32,
    spin_hor: f32,
    spin_ver: f32,
    is_paused: bool,
    tick: Option<f32>,
}


impl<R: gfx::Resources> Game<R> {
    pub fn new<F: gfx::Factory<R>>(
        settings: &config::Settings,
        targets: render::MainTargets<R>,
        factory: &mut F,
    ) -> Self {
        info!("Loading world parameters");
        let db = {
            let game = config::game::Registry::load(settings);
            DataBase {
                _bunches: config::bunches::load(settings.open_relative("bunches.prm")),
                cars: config::car::load_registry(settings, &game, factory),
                common: config::common::load(settings.open_relative("common.prm")),
                _escaves: config::escaves::load(settings.open_relative("escaves.prm")),
                _game: game,
            }
        };
        let (level, coords) = if settings.game.level.is_empty() {
            info!("Using test level");
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
            info!("Using level {}", ini_name);

            let config = level::LevelConfig::load(&ini_path);
            let level = level::load(&config);

            (level, coordinates)
        };

        let (width, height, _, _) = targets.color.get_dimensions();
        let depth = 10f32 .. 10000f32;
        let pal_data = level::read_palette(settings.open_palette());
        let render = render::init(factory, targets, &level, &pal_data, &settings.render);
        let collider = render::GpuCollider::new(factory,
            (256, 256), 400, 1000,
            render.surface_data(),
        );

        let mut player_agent = Agent::spawn(
            "Player".to_string(),
            &db.cars[&settings.car.id],
            coords,
            cgmath::Rad::turn_div_2(),
            &level,
        );
        player_agent.spirit = Spirit::Player;
        let mut agents = vec![player_agent];
        let mut rng = rand::thread_rng();
        let car_names = db.cars.keys().cloned().collect::<Vec<_>>();
        // populate with random agents
        for i in 0 .. settings.game.other.count {
            use rand::Rng;
            let car_id = rng.choose(&car_names).unwrap();
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
            line_buffer: render::LineBuffer::new(),
            level,
            agents,
            cam: space::Camera {
                loc: cgmath::vec3(coords.0 as f32, coords.1 as f32, 200.0),
                rot: cgmath::Quaternion::new(0.0, 0.0, 1.0, 0.0),
                proj: match settings.game.view {
                    config::settings::View::Perspective => {
                        let pf = cgmath::PerspectiveFov {
                            fovy: cgmath::Deg(45.0).into(),
                            aspect: width as f32 / height as f32,
                            near: depth.start,
                            far: depth.end,
                        };
                        space::Projection::Perspective(pf)
                    }
                    config::settings::View::Flat => {
                        space::Projection::ortho(width, height, depth)
                    }
                },
            },
            max_quant: settings.game.physics.max_quant,
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

impl<R: gfx::Resources> Application<R> for Game<R> {
    fn on_resize<F: gfx::Factory<R>>(
        &mut self, targets: render::MainTargets<R>, _factory: &mut F
    ) {
        let (w, h, _, _) = targets.color.get_dimensions();
        self.cam.proj.update(w, h);
        self.render.resize(targets);
    }

    fn on_key(&mut self, input: KeyboardInput) -> bool {
        use boilerplate::{ElementState, Key};

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

            const TIME_HACK: f32 = 0.5;
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

            self.line_buffer.clear();

            for a in self.agents.iter_mut() {
                a.apply_control(
                    input_factor,
                    &self.db.common,
                );
            }

            while physics_dt > self.max_quant {
                for a in self.agents.iter_mut() {
                    a.step(
                        self.max_quant,
                        &self.level,
                        &self.db.common,
                        None,
                    );
                }
                physics_dt -= self.max_quant;
            }

            for a in self.agents.iter_mut() {
                let lbuf = match a.spirit {
                    Spirit::Player => Some(&mut self.line_buffer),
                    Spirit::Other => None,
                };
                a.step(
                    physics_dt,
                    &self.level,
                    &self.db.common,
                    lbuf,
                );
            }
        }
    }

    fn draw<C: gfx::CommandBuffer<R>>(
        &mut self, encoder: &mut gfx::Encoder<R, C>
    ) {
        let _ = {
            let mut collider = self.collider.start(encoder, &self.db.common);
            for agent in &self.agents {
                let mut transform = agent.transform.clone();
                transform.scale *= agent.car.physics.scale_bound;
                let _ = collider.add(&agent.car.model.shape, transform);
            }
            collider.finish()
        };

        let models = self.agents
            .iter()
            .map(|a| render::RenderModel {
                model: &a.car.model,
                transform: a.transform.clone(),
                debug_shape_scale: match a.spirit {
                    Spirit::Player => Some(a.car.physics.scale_bound),
                    Spirit::Other => None,
                },
            })
            .collect::<Vec<_>>();

        self.render.draw_world(encoder, &models, &self.cam);
        self.render
            .debug
            .draw_lines(&self.line_buffer, self.cam.get_view_proj().into(), encoder);
    }

    fn reload_shaders<F: gfx::Factory<R>>(&mut self, factory: &mut F) {
        self.render.reload(factory);
        self.collider.reload(factory);
    }
}
