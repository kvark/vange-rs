use cgmath;
use glutin::Event;
use gfx;
use {level, model, render};
use config::Settings;


pub type Transform = cgmath::Decomposed<cgmath::Vector3<f32>, cgmath::Quaternion<f32>>;

pub struct Camera {
    pub loc: cgmath::Vector3<f32>,
    pub rot: cgmath::Quaternion<f32>,
    proj: cgmath::PerspectiveFov<f32>,
}

pub struct Follow {
    transform: Transform,
    move_speed: f32,
    rot_speed: cgmath::Rad<f32>,
}

impl Camera {
    pub fn get_view_proj(&self) -> cgmath::Matrix4<f32> {
        use cgmath::{Decomposed, Matrix4, Transform};
        let view = Decomposed {
            scale: 1.0,
            rot: self.rot,
            disp: self.loc,
        };
        let view_mx: Matrix4<f32> = view.inverse_transform().unwrap().into();
        let proj_mx: Matrix4<f32> = self.proj.into();
        proj_mx * view_mx
    }

    fn follow(&mut self, target: &Transform, follow: &Follow) {
        use cgmath::Transform;
        let result = target.concat(&follow.transform);
        //TODO
        self.loc = result.disp;
        self.rot = result.rot;
    }
}


pub trait App<R: gfx::Resources> {
    fn update<I: Iterator<Item=Event>, F: gfx::Factory<R>>(&mut self, I, f32, &mut F) -> bool;
    fn draw<C: gfx::CommandBuffer<R>>(&mut self, &mut gfx::Encoder<R, C>);
}

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
    pub transform: Transform,
    pub model: model::Model<R>,
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

pub struct Game<R: gfx::Resources> {
    render: render::Render<R>,
    level: level::Level,
    agents: Vec<Agent<R>>,
    cam: Camera,
    dyn_target: Dynamo,
}

impl<R: gfx::Resources> Game<R> {
    pub fn new<F: gfx::Factory<R>>(settings: &Settings,
           out_color: gfx::handle::RenderTargetView<R, render::ColorFormat>,
           out_depth: gfx::handle::DepthStencilView<R, render::DepthFormat>,
           factory: &mut F) -> Game<R>
    {
        use std::io::BufReader;
        use std::fs::File;

        info!("Loading world parameters");
        let config = settings.get_level();
        let lev = level::load(&config);
        let pal_data = level::load_palette(&settings.get_object_palette_path());

        let mut model_file = BufReader::new(File::open(
            settings.get_vehicle_model_path(&settings.game.vehicle)
        ).unwrap());
        let agent = Agent {
            control: Control::Player,
            transform: cgmath::Decomposed {
                scale: 1.0,
                disp: cgmath::vec3(0.0, 0.0, 40.0),
                rot: cgmath::One::one(),
            },
            model: model::load_m3d(&mut model_file, factory),
            dynamo: Dynamo {
                thrust: 0.0,
                steer: cgmath::Zero::zero(),
            },
        };

        Game {
            render: render::init(factory, out_color, out_depth, &lev, &pal_data),
            level: lev,
            agents: vec![agent],
            cam: Camera {
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

impl<R: gfx::Resources> App<R> for Game<R> {
    fn update<I: Iterator<Item=Event>, F: gfx::Factory<R>>(&mut self, events: I, delta: f32, factory: &mut F) -> bool {
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
            self.cam.follow(&p.transform, &Follow {
                transform: cgmath::Decomposed {
                    disp: cgmath::vec3(0.0, -100.0, 50.0),
                    rot: cgmath::Rotation3::from_axis_angle(cgmath::Vector3::unit_x(), cgmath::Angle::turn_div_6()),
                    scale: 1.0,
                },
                move_speed: 5.0,
                rot_speed: cgmath::Rad::new(0.1),
            });
        }

        for a in self.agents.iter_mut() {
            a.step(delta, &self.level);
        }

        true
    }

    fn draw<C: gfx::CommandBuffer<R>>(&mut self, enc: &mut gfx::Encoder<R, C>) {
        self.render.draw(enc, &self.agents, &self.cam);
    }
}


pub struct ModelView<R: gfx::Resources> {
    model: model::Model<R>,
    transform: Transform,
    pso: gfx::PipelineState<R, render::object::Meta>,
    data: render::object::Data<R>,
    cam: Camera,
}

impl<R: gfx::Resources> ModelView<R> {
    pub fn new<F: gfx::Factory<R>>(path: &str, settings: &Settings,
               out_color: gfx::handle::RenderTargetView<R, render::ColorFormat>,
               out_depth: gfx::handle::DepthStencilView<R, render::DepthFormat>,
               factory: &mut F) -> ModelView<R>
    {
        use std::io::BufReader;
        use gfx::traits::FactoryExt;

        let pal_data = level::load_palette(&settings.get_object_palette_path());

        info!("Loading model {}", path);
        let mut file = BufReader::new(settings.open(path));
        let model = model::load_m3d(&mut file, factory);
        let data = render::object::Data {
            vbuf: model.body.buffer.clone(),
            locals: factory.create_constant_buffer(1),
            ctable: render::Render::create_color_table(factory),
            palette: render::Render::create_palette(&pal_data, factory),
            out_color: out_color,
            out_depth: out_depth,
        };

        ModelView {
            model: model,
            transform: cgmath::Decomposed {
                scale: 1.0,
                disp: cgmath::Vector3::unit_z(),
                rot: cgmath::One::one(),
            },
            pso: render::Render::create_object_pso(factory),
            data: data,
            cam: Camera {
                loc: cgmath::vec3(0.0, -40.0, 20.0),
                rot: cgmath::Rotation3::from_axis_angle(cgmath::Vector3::unit_x(), cgmath::Angle::turn_div_6()),
                proj: cgmath::PerspectiveFov {
                    fovy: cgmath::deg(45.0).into(),
                    aspect: settings.get_screen_aspect(),
                    near: 1.0,
                    far: 100.0,
                },
            },
        }
    }

    fn rotate(&mut self, angle: cgmath::Rad<f32>) {
        use cgmath::Transform;
        let other = cgmath::Decomposed {
            scale: 1.0,
            rot: cgmath::Rotation3::from_axis_angle(cgmath::Vector3::unit_z(), angle),
            disp: cgmath::Zero::zero(),
        };
        self.transform = other.concat(&self.transform);
    }
}

impl<R: gfx::Resources> App<R> for ModelView<R> {
    fn update<I: Iterator<Item=Event>, F: gfx::Factory<R>>(&mut self, events: I, delta: f32, factory: &mut F) -> bool {
        use glutin::VirtualKeyCode as Key;
        let angle = cgmath::rad(delta * 2.0);
        for event in events {
            match event {
                Event::KeyboardInput(_, _, Some(Key::Escape)) |
                Event::Closed => return false,
                Event::KeyboardInput(_, _, Some(Key::A)) => self.rotate(-angle),
                Event::KeyboardInput(_, _, Some(Key::D)) => self.rotate(angle),
                Event::KeyboardInput(_, _, Some(Key::L)) =>
                    self.pso = render::Render::create_object_pso(factory),
                _ => {}, //TODO
            }
        }
        true
    }

    fn draw<C: gfx::CommandBuffer<R>>(&mut self, enc: &mut gfx::Encoder<R, C>) {
        let model_trans: cgmath::Matrix4<f32> = self.transform.into();
        let locals = render::ObjectLocals {
            m_mvp: (self.cam.get_view_proj() * model_trans).into(),
        };
        enc.update_constant_buffer(&self.data.locals, &locals);
        enc.clear(&self.data.out_color, [0.1, 0.2, 0.3, 1.0]);
        enc.clear_depth(&self.data.out_depth, 1.0);
        enc.draw(&self.model.body.slice, &self.pso, &self.data);
    }
}
