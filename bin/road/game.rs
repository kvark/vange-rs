use crate::{boilerplate::Application, physics};
use m3d::Mesh;
use vangers::{
    config, level, model,
    render::{
        debug::LineBuffer, object::BodyColor, Batcher, GraphicsContext, Render, ScreenTargets,
    },
    space,
};

use cgmath::prelude::*;

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
        focus_point: &'a cgmath::Point3<f32>,
        line_buffer: Option<&'a mut LineBuffer>,
    },
}

pub struct Agent {
    _name: String,
    spirit: Spirit,
    car: config::car::CarInfo,
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
        orientation: cgmath::Rad<f32>,
        level: &level::Level,
    ) -> Self {
        let height = level.get(coords).top() + 5.; //center offset
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
        }
    }
}

#[derive(Default)]
struct Stats {
    frame_deltas: Vec<f32>,
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
        let angle = cgmath::Deg(config.angle as f32) - cgmath::Deg::turn_div_4();
        if config.speed > 0.0 {
            CameraStyle::Follow {
                follow: space::Follow {
                    angle_x: angle,
                    offset: cgmath::vec3(0.0, config.offset, config.height),
                    speed: config.speed,
                },
                ground_anchor: angle > cgmath::Deg(15.0),
            }
        } else {
            //Note: this appears to be broken ATM
            CameraStyle::Simple(space::Direction {
                view: cgmath::vec3(0.0, angle.sin(), -angle.cos()),
                height: config.height,
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
            threshold: 1.1,
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
    stats: Stats,
    ui: config::settings::Ui,
    cam: space::Camera,
    cam_style: CameraStyle,
    max_quant: f32,
    input: Input,
}

impl Game {
    pub fn new(settings: &config::Settings, gfx: &GraphicsContext) -> Self {
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
            loc: cgmath::vec3(coords.0 as f32, coords.1 as f32, 200.0),
            rot: cgmath::One::one(),
            scale: cgmath::vec3(1.0, -1.0, 1.0),
            proj: match settings.game.camera.projection {
                config::settings::Projection::Perspective => {
                    let pf = cgmath::PerspectiveFov {
                        fovy: cgmath::Deg(45.0).into(),
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
            cgmath::Rad::turn_div_2(),
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
                rng.gen(),
                &level,
            );
            agents.push(agent);
        }

        Game {
            db,
            render,
            batcher: Batcher::new(),
            line_buffer: LineBuffer::new(),
            level,
            agents,
            stats: Stats::default(),
            ui: settings.ui.clone(),
            cam,
            cam_style: CameraStyle::new(&settings.game.camera),
            max_quant: settings.game.physics.max_quant,
            input: Input::default(),
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
                        Physics::Cpu { ref transform, .. } => *transform,
                    };
                    self.input.tick = None;
                    if self.input.is_paused {
                        self.input.is_paused = false;
                        self.cam.loc = center.disp + cgmath::vec3(0.0, 0.0, 200.0);
                        self.cam.rot = cgmath::Quaternion::new(1.0, 0.0, 0.0, 0.0);
                    } else {
                        self.input.is_paused = true;
                        self.cam.focus_on(&center);
                    }
                }
                Key::Comma => self.input.tick = Some(-1.0),
                Key::Period => self.input.tick = Some(1.0),
                Key::LShift => self.input.turbo = true,
                Key::LAlt => self.input.jump = Some(0.0),
                Key::W => self.input.spin_ver = self.cam.scale.x,
                Key::S => self.input.spin_ver = -self.cam.scale.x,
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
                Key::A => self.input.spin_hor = -self.cam.scale.y,
                Key::D => self.input.spin_hor = self.cam.scale.y,
                Key::Q => {
                    self.input.roll = Some(Roll {
                        dir: -self.cam.scale.y,
                        time: 0.0,
                    })
                }
                Key::E => {
                    self.input.roll = Some(Roll {
                        dir: self.cam.scale.y,
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
                Key::W | Key::S => self.input.spin_ver = 0.0,
                Key::A | Key::D => self.input.spin_hor = 0.0,
                Key::Q | Key::E => self.input.roll = None,
                Key::LShift => self.input.turbo = false,
                Key::LAlt => player.jump = self.input.jump.take(),
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

    fn update(&mut self, _device: &wgpu::Device, _queue: &wgpu::Queue, delta: f32) {
        profiling::scope!("Update");

        self.stats.frame_deltas.push(delta * 1000.0);
        if self.stats.frame_deltas.len() > self.ui.frame_history {
            self.stats.frame_deltas.remove(0);
        }

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
                    cgmath::Rad(2.0 * delta * self.input.spin_hor),
                    cgmath::Rad(delta * self.input.spin_ver),
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
                            .top();
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

        let fd_points = egui::plot::PlotPoints::from_ys_f32(&self.stats.frame_deltas);
        let fd_line = egui::plot::Line::new(fd_points).name("last");

        let player = self
            .agents
            .iter_mut()
            .find(|agent| agent.spirit == Spirit::Player)
            .unwrap();
        let mut selected_car = &player.car_name;

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
                    ui.add(egui::Slider::new(&mut follow.angle_x.0, -105.0..=0.0).text("Angle"));
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
            egui::plot::Plot::new("Frame time")
                .allow_zoom(false)
                .allow_scroll(false)
                .allow_drag(false)
                .show_x(false)
                .include_y(0.0)
                .show_axes([false, true])
                .show(ui, |plot_ui| {
                    plot_ui.line(fd_line);
                    plot_ui.hline(egui::plot::HLine::new(1000.0 / 60.0).name("smooth"));
                });
        });

        if selected_car != &player.car_name {
            let name = selected_car.clone();
            player.change_car(&self.db.cars[&name], name);
        }
    }

    fn draw(&mut self, device: &wgpu::Device, targets: ScreenTargets) -> wgpu::CommandBuffer {
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
