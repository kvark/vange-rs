use std::collections::HashMap;
use cgmath;
use glutin::Event;
use gfx;
use {config, level, render};


#[derive(Eq, PartialEq)]
enum Control {
    Player,
    //Artificial,
}

struct Dynamo {
    thrust: f32,
    steer: cgmath::Rad<f32>,
}

impl Dynamo {
    fn step(&mut self, target: &Dynamo, delta: f32) {
        // thrust
        let (accel_fw, accel_back, deaccel) = (80.0, 40.0, 120.0);
        let time_stop = self.thrust.max(0.0) / deaccel;
        if target.thrust > self.thrust {
            self.thrust = (self.thrust + accel_fw * delta).min(target.thrust);
        }else if target.thrust >= 0.0 || time_stop >= delta {
            self.thrust = (self.thrust - deaccel * delta).max(target.thrust);
        }else {
            self.thrust = (self.thrust - deaccel * time_stop - accel_back * (delta - time_stop)).max(target.thrust);
        }
        // steer
        let steer_accel = 0.2;
        let angle = if target.steer > self.steer {
            (self.steer.s + steer_accel * delta).min(target.steer.s)
        }else {
            (self.steer.s - steer_accel * delta).max(target.steer.s)
        };
        self.steer = cgmath::Rad::new(angle);
    }
}

pub struct Agent<R: gfx::Resources> {
    control: Control,
    pub transform: super::Transform,
    pub car: config::car::CarInfo<R>,
    dynamo: Dynamo,
}

impl<R: gfx::Resources> Agent<R> {
    fn step(&mut self, delta: f32, level: &level::Level) {
        use cgmath::{Rotation, Rotation3};
        // move forward
        let wheel_rot = cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), -self.dynamo.steer);
        let forward_local = wheel_rot.rotate_vector(cgmath::Vector3::unit_y());
        let forward_world = self.transform.rot.rotate_vector(forward_local);
        self.transform.disp += forward_world * self.dynamo.thrust * delta;
        // height adjust
        let coord = ((self.transform.disp.x + 0.5) as i32, (self.transform.disp.y + 0.5) as i32);
        let texel = level.get(coord);
        let height_scale = (level::HEIGHT_SCALE as f32) / 256.0;
        let vehicle_base = 5.0;
        self.transform.disp.z = (texel.low as f32 + 0.5) * height_scale + vehicle_base;
        // rotate
        let rot_speed = 0.1;
        let rot_angle = cgmath::Rad::new(rot_speed * self.dynamo.thrust * self.dynamo.steer.s * delta);
        self.transform.rot = self.transform.rot * cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), -rot_angle);
    }
}

struct DataBase<R: gfx::Resources> {
    cars: HashMap<String, config::car::CarInfo<R>>,
    _common: config::common::Common,
    _game: config::game::Registry,
}

pub struct Game<R: gfx::Resources> {
    _db: DataBase<R>,
    render: render::Render<R>,
    level: level::Level,
    agents: Vec<Agent<R>>,
    cam: super::Camera,
    dyn_target: Dynamo,
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
                _common: config::common::load(settings.open("common.prm")),
                _game: game,
            }
        };
        let lev_config = settings.get_level();
        let level = level::load(&lev_config);
        let pal_data = level::load_palette(&settings.get_object_palette_path());

        let agent = Agent {
            control: Control::Player,
            transform: cgmath::Decomposed {
                scale: 1.0,
                disp: cgmath::vec3(0.0, 0.0, 40.0),
                rot: cgmath::One::one(),
            },
            car: db.cars[&settings.car.id].clone(),
            dynamo: Dynamo {
                thrust: 0.0,
                steer: cgmath::Zero::zero(),
            },
        };

        Game {
            _db: db,
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
            dyn_target: Dynamo {
                thrust: 0.0,
                steer: cgmath::Zero::zero(),
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

        //let angle = cgmath::rad(delta * 2.0);
        //let step = delta * 400.0;
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
                Event::KeyboardInput(Pressed, _, Some(Key::W)) => self.dyn_target.thrust = 100.0,
                Event::KeyboardInput(Pressed, _, Some(Key::S)) => self.dyn_target.thrust = -40.0,
                Event::KeyboardInput(Released, _, Some(Key::W)) | Event::KeyboardInput(Released, _, Some(Key::S)) =>
                    self.dyn_target.thrust = 0.0,
                Event::KeyboardInput(Pressed, _, Some(Key::A)) => self.dyn_target.steer = cgmath::Rad::new(-0.2),
                Event::KeyboardInput(Pressed, _, Some(Key::D)) => self.dyn_target.steer = cgmath::Rad::new(0.2),
                Event::KeyboardInput(Released, _, Some(Key::A)) | Event::KeyboardInput(Released, _, Some(Key::D)) =>
                    self.dyn_target.steer = cgmath::Rad::new(0.0),
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

        if let Some(p) = self.agents.iter_mut().find(|a| a.control == Control::Player) {
            p.dynamo.step(&self.dyn_target, delta);
            self.cam.follow(&p.transform, delta, &super::Follow {
                transform: cgmath::Decomposed {
                    disp: cgmath::vec3(0.0, -200.0, 100.0),
                    rot: cgmath::Rotation3::from_axis_angle(cgmath::Vector3::unit_x(), cgmath::Rad::new(1.1)),
                    scale: 1.0,
                },
                speed: 4.0,
            });
        }

        for a in self.agents.iter_mut() {
            a.step(delta, &self.level);
        }

        true
    }

    fn draw<C: gfx::CommandBuffer<R>>(&mut self, enc: &mut gfx::Encoder<R, C>) {
        self.render.draw_world(enc, &self.agents, &self.cam);
    }
}