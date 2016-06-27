use cgmath;
use glutin::Event;
use gfx;
use {level, model, render};
use config::Settings;


pub struct Camera {
    pub loc: cgmath::Vector3<f32>,
    pub rot: cgmath::Quaternion<f32>,
    pub proj: cgmath::PerspectiveFov<f32>,
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
}


pub trait App<R: gfx::Resources> {
    fn on_event<F: gfx::Factory<R>>(&mut self, Event, delta: f32, &mut F) -> bool;
    fn on_frame<C: gfx::CommandBuffer<R>>(&mut self, &mut gfx::Encoder<R, C>);
    fn do_iter<I, F, C>(&mut self, events: I, delta: f32, factory: &mut F, encoder: &mut gfx::Encoder<R, C>)
               -> bool where
        I: Iterator<Item=Event>,
        F: gfx::Factory<R>,
        C: gfx::CommandBuffer<R>,
    {
        for event in events {
            if !self.on_event(event, delta, factory) {
                return false;
            }
        }
        self.on_frame(encoder);
        true
    }
}


pub struct Game<R: gfx::Resources> {
    render: render::Render<R>,
    cam: Camera,
}

impl<R: gfx::Resources> Game<R> {
    pub fn new<F: gfx::Factory<R>>(settings: &Settings,
           out_color: gfx::handle::RenderTargetView<R, render::ColorFormat>,
           out_depth: gfx::handle::DepthStencilView<R, render::DepthFormat>,
           factory: &mut F) -> Game<R>
    {
        info!("Loading world parameters");
        let config = settings.get_level();
        let lev = level::load(&config);

        Game {
            render: render::init(factory, out_color, out_depth, &lev),
            cam: Camera {
                loc: cgmath::vec3(0.0, 0.0, 200.0),
                rot: cgmath::Quaternion::new(1.0, 0.0, 0.0, 0.0),
                proj: cgmath::PerspectiveFov {
                    fovy: cgmath::deg(45.0).into(),
                    aspect: settings.get_screen_aspect(),
                    near: 1.0,
                    far: 100000.0,
                },
            },
        }
    }
}

impl<R: gfx::Resources> App<R> for Game<R> {
    fn on_event<F: gfx::Factory<R>>(&mut self, event: Event, delta: f32, factory: &mut F) -> bool {
        use cgmath::Rotation3;
        use glutin::VirtualKeyCode as Key;
        let angle = cgmath::rad(delta * 2.0);
        let step = delta * 400.0;
        match event {
            Event::KeyboardInput(_, _, Some(Key::Escape)) |
            Event::Closed => return false,
            Event::KeyboardInput(_, _, Some(Key::L)) => self.render.reload(factory),
            Event::KeyboardInput(_, _, Some(Key::W)) =>
                self.cam.loc = self.cam.loc + cgmath::vec3(0.0, step, 0.0),
            Event::KeyboardInput(_, _, Some(Key::S)) =>
                self.cam.loc = self.cam.loc - cgmath::vec3(0.0, step, 0.0),
            Event::KeyboardInput(_, _, Some(Key::R)) =>
                self.cam.rot = self.cam.rot * cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_x(), angle),
            Event::KeyboardInput(_, _, Some(Key::F)) =>
                self.cam.rot = self.cam.rot * cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_x(), -angle),
            Event::KeyboardInput(_, _, Some(Key::A)) =>
                self.cam.rot = cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), angle) * self.cam.rot,
            Event::KeyboardInput(_, _, Some(Key::D)) =>
                self.cam.rot = cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), -angle) * self.cam.rot,
            _ => {},
        }
        true
    }
    fn on_frame<C: gfx::CommandBuffer<R>>(&mut self, enc: &mut gfx::Encoder<R, C>) {
        self.render.draw(enc, &self.cam);
    }
}


pub struct ModelView<R: gfx::Resources> {
    model: model::Model<R>,
    transform: cgmath::Decomposed<cgmath::Vector3<f32>, cgmath::Quaternion<f32>>,
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
    fn on_event<F: gfx::Factory<R>>(&mut self, event: Event, delta: f32, factory: &mut F) -> bool {
        use glutin::VirtualKeyCode as Key;
        let angle = cgmath::rad(delta * 2.0);
        match event {
            Event::KeyboardInput(_, _, Some(Key::Escape)) |
            Event::Closed => return false,
            Event::KeyboardInput(_, _, Some(Key::A)) => self.rotate(-angle),
            Event::KeyboardInput(_, _, Some(Key::D)) => self.rotate(angle),
            Event::KeyboardInput(_, _, Some(Key::L)) =>
                self.pso = render::Render::create_object_pso(factory),
            _ => {}, //TODO
        }
        true
    }
    fn on_frame<C: gfx::CommandBuffer<R>>(&mut self, enc: &mut gfx::Encoder<R, C>) {
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
