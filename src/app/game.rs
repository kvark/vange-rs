use std::collections::HashMap;
use cgmath;
use glutin::Event;
use gfx;
use {config, level, render};


#[derive(Eq, PartialEq)]
enum Spirit {
    Player,
    //Computer,
}

use config::common::{Traction};
const MAX_TRACTION: Traction = 4.0;

struct Dynamo {
    traction: Traction,
    steer: cgmath::Rad<f32>,
    linear_velocity: cgmath::Vector3<f32>,
    angular_velocity: cgmath::Vector3<f32>,
}

impl Dynamo {
    fn change_traction(&mut self, delta: Traction) {
        let old = self.traction;
        self.traction = (old + delta).min(MAX_TRACTION).max(-MAX_TRACTION);
        if old * self.traction < 0.0 {
            self.traction = 0.0; // full stop
        }
    }
}

struct Control {
    motor: i8,
    hand_break: bool,
    turbo: bool,
}

pub struct Agent<R: gfx::Resources> {
    spirit: Spirit,
    pub transform: super::Transform,
    pub car: config::car::CarInfo<R>,
    dynamo: Dynamo,
    control: Control,
}

impl<R: gfx::Resources> Agent<R> {
    fn step(&mut self, dt: f32, level: &level::Level, common: &config::common::Common) {
        if self.control.motor != 0 {
            self.dynamo.change_traction(self.control.motor as f32 * dt * common.car.traction_incr);
        }
        if self.control.hand_break && self.dynamo.traction != 0.0 {
            self.dynamo.traction *= (config::common::ORIGINAL_FPS as f32 * -dt).exp2();
        }
        let _f_global = cgmath::vec3(0.0, 0.0, -common.nature.gravity);
        let _k_global = cgmath::vec3(0.0, 0.0, 0.0);
        for _ in 0 .. common.nature.num_calls_analysis {
            use cgmath::{InnerSpace};
            let f_traction_per_wheel =
                self.car.physics.mobility_factor * common.global.mobility_factor *
                if self.control.hand_break { common.global.k_traction_turbo } else { 1.0 } *
                self.dynamo.traction / (self.car.model.wheels.len() as f32);
            let v_drag = common.drag.free.v * common.drag.speed.v.powf(self.dynamo.linear_velocity.magnitude());
            let w_drag = common.drag.free.w * common.drag.speed.w.powf(self.dynamo.angular_velocity.magnitude2());
            if self.dynamo.linear_velocity.magnitude() * v_drag > common.drag.abs_stop.v ||
               self.dynamo.angular_velocity.magnitude() *w_drag > common.drag.abs_stop.w {
                self.transform.disp += self.dynamo.linear_velocity * dt;
            }
            self.dynamo.linear_velocity *= v_drag.powf(config::common::SPEED_CORRECTION_FACTOR);
            self.dynamo.angular_velocity *= w_drag.powf(config::common::SPEED_CORRECTION_FACTOR);
        }
        // height adjust
        let coord = ((self.transform.disp.x + 0.5) as i32, (self.transform.disp.y + 0.5) as i32);
        let texel = level.get(coord);
        let height_scale = (level::HEIGHT_SCALE as f32) / 256.0;
        let vehicle_base = 5.0;
        self.transform.disp.z = (texel.low as f32 + 0.5) * height_scale + vehicle_base;
        // slow down
        let sign = self.dynamo.traction.signum();
        self.dynamo.change_traction(-sign * dt * common.car.traction_decr);
    }
}

struct DataBase<R: gfx::Resources> {
    cars: HashMap<String, config::car::CarInfo<R>>,
    common: config::common::Common,
    _game: config::game::Registry,
}

pub struct Game<R: gfx::Resources> {
    db: DataBase<R>,
    render: render::Render<R>,
    level: level::Level,
    agents: Vec<Agent<R>>,
    cam: super::Camera,
}

impl<R: gfx::Resources> Game<R> {
    pub fn new<F: gfx::Factory<R>>(settings: &config::Settings,
           out_color: gfx::handle::RenderTargetView<R, render::ColorFormat>,
           out_depth: gfx::handle::DepthStencilView<R, render::DepthFormat>,
           factory: &mut F) -> Game<R>
    {
        info!("Loading world parameters");
        let db = {
            let game = config::game::Registry::load(settings);
            DataBase {
                cars: config::car::load_registry(settings, &game, factory),
                common: config::common::load(settings.open("common.prm")),
                _game: game,
            }
        };
        let lev_config = settings.get_level();
        let level = level::load(&lev_config);
        let pal_data = level::load_palette(&settings.get_object_palette_path());

        let agent = Agent {
            spirit: Spirit::Player,
            transform: cgmath::Decomposed {
                scale: 1.0,
                disp: cgmath::vec3(0.0, 0.0, 40.0),
                rot: cgmath::One::one(),
            },
            car: db.cars[&settings.car.id].clone(),
            dynamo: Dynamo {
                traction: 0.0,
                steer: cgmath::Zero::zero(),
                linear_velocity: cgmath::vec3(0.0, 0.0, 0.0),
                angular_velocity: cgmath::vec3(0.0, 0.0, 0.0),
            },
            control: Control {
                motor: 0,
                hand_break: false,
                turbo: false,
            },
        };

        Game {
            db: db,
            render: render::init(factory, out_color, out_depth, &level, &pal_data),
            level: level,
            agents: vec![agent],
            cam: super::Camera {
                loc: cgmath::vec3(0.0, 0.0, 200.0),
                rot: cgmath::Quaternion::new(1.0, 0.0, 0.0, 0.0),
                proj: cgmath::PerspectiveFov {
                    fovy: cgmath::deg(45.0).into(),
                    aspect: settings.get_screen_aspect(),
                    near: 10.0,
                    far: 10000.0,
                },
            },
        }
    }

    fn _move_cam(&mut self, step: f32) {
        use cgmath::{InnerSpace, Rotation};
        let mut back = self.cam.rot.rotate_vector(cgmath::Vector3::unit_z());
        back.z = 0.0;
        self.cam.loc -= back.normalize() * step;
    }
}

impl<R: gfx::Resources> super::App<R> for Game<R> {
    fn update<I: Iterator<Item=Event>, F: gfx::Factory<R>>(&mut self,
              events: I, delta: f32, factory: &mut F) -> bool {
        use glutin::VirtualKeyCode as Key;
        use glutin::ElementState::*;

        let pid = match self.agents.iter().position(|a| a.spirit == Spirit::Player) {
            Some(pos) => pos,
            None => return false,
        };
        for event in events {
            match event {
                Event::KeyboardInput(Pressed, _, Some(Key::Escape)) |
                Event::Closed => return false,
                Event::KeyboardInput(Pressed, _, Some(Key::L)) => self.render.reload(factory),
                /*
                Event::KeyboardInput(_, _, Some(Key::R)) =>
                    self.cam.rot = self.cam.rot * cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_x(), angle),
                Event::KeyboardInput(_, _, Some(Key::F)) =>
                    self.cam.rot = self.cam.rot * cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_x(), -angle),
                */
                Event::KeyboardInput(Pressed, _, Some(Key::W)) => self.agents[pid].control.motor = 1,
                Event::KeyboardInput(Pressed, _, Some(Key::S)) => self.agents[pid].control.motor = -1,
                Event::KeyboardInput(Released, _, Some(Key::W)) | Event::KeyboardInput(Released, _, Some(Key::S)) =>
                    self.agents[pid].control.motor = 0,
                /*
                Event::KeyboardInput(Pressed, _, Some(Key::A)) => self.dyn_target.steer = cgmath::Rad::new(-0.2),
                Event::KeyboardInput(Pressed, _, Some(Key::D)) => self.dyn_target.steer = cgmath::Rad::new(0.2),
                Event::KeyboardInput(Released, _, Some(Key::A)) | Event::KeyboardInput(Released, _, Some(Key::D)) =>
                    self.dyn_target.steer = cgmath::Rad::new(0.0),
                */
                /*
                Event::KeyboardInput(_, _, Some(Key::W)) => self.move_cam(step),
                Event::KeyboardInput(_, _, Some(Key::S)) => self.move_cam(-step),
                Event::KeyboardInput(_, _, Some(Key::A)) =>
                    self.cam.rot = cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), angle) * self.cam.rot,
                Event::KeyboardInput(_, _, Some(Key::D)) =>
                    self.cam.rot = cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), -angle) * self.cam.rot,
                */
                _ => {},
            }
        }

        self.cam.follow(&self.agents[pid].transform, delta, &super::Follow {
            transform: cgmath::Decomposed {
                disp: cgmath::vec3(0.0, -200.0, 100.0),
                rot: cgmath::Rotation3::from_axis_angle(cgmath::Vector3::unit_x(), cgmath::Rad::new(1.1)),
                scale: 1.0,
            },
            speed: 4.0,
        });

        for a in self.agents.iter_mut() {
            a.step(delta * config::common::SPEED_CORRECTION_FACTOR, &self.level, &self.db.common);
        }

        true
    }

    fn draw<C: gfx::CommandBuffer<R>>(&mut self, enc: &mut gfx::Encoder<R, C>) {
        self.render.draw_world(enc, &self.agents, &self.cam);
    }
}