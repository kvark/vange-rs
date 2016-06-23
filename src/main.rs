extern crate byteorder;
extern crate cgmath;
extern crate env_logger;
extern crate getopts;
#[macro_use]
extern crate gfx;
extern crate gfx_window_glutin;
extern crate glutin;
#[macro_use]
extern crate log;
extern crate progressive;
extern crate rustc_serialize;
extern crate ini;
extern crate toml;

mod config;
mod level;
mod model;
mod render;
mod splay;

use glutin::Event;


pub struct Camera {
    loc: cgmath::Vector3<f32>,
    rot: cgmath::Quaternion<f32>,
    proj: cgmath::PerspectiveFov<f32>,
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

trait App<R: gfx::Resources> {
    fn on_event<F: gfx::Factory<R>>(&mut self, Event, &mut F) -> bool;
    fn on_frame<C: gfx::CommandBuffer<R>>(&mut self, &mut gfx::Encoder<R, C>);
}

struct GameApp<R: gfx::Resources> {
    render: render::Render<R>,
    cam: Camera,
}

impl<R: gfx::Resources> GameApp<R> {
    fn new<F: gfx::Factory<R>>(settings: &config::Settings,
           output: gfx::handle::RenderTargetView<R, render::ColorFormat>,
           factory: &mut F) -> GameApp<R>
    {
        info!("Loading world parameters");
        let config = settings.get_level();
        let lev = level::load(&config);

        GameApp {
            render: render::init(factory, output, &lev),
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

impl<R: gfx::Resources> App<R> for GameApp<R> {
    fn on_event<F: gfx::Factory<R>>(&mut self, event: Event, factory: &mut F) -> bool {
        use cgmath::Rotation3;
        use glutin::VirtualKeyCode as Key;
        let delta = cgmath::rad(0.05);
        let step = 10.0;
        match event {
            Event::KeyboardInput(_, _, Some(Key::Escape)) |
            Event::Closed => return false,
            Event::KeyboardInput(_, _, Some(Key::L)) => self.render.reload(factory),
            Event::KeyboardInput(_, _, Some(Key::W)) =>
                self.cam.loc = self.cam.loc + cgmath::vec3(0.0, step, 0.0),
            Event::KeyboardInput(_, _, Some(Key::S)) =>
                self.cam.loc = self.cam.loc - cgmath::vec3(0.0, step, 0.0),
            Event::KeyboardInput(_, _, Some(Key::R)) =>
                self.cam.rot = self.cam.rot * cgmath::Quaternion::from_axis_angle(cgmath::vec3(1.0, 0.0, 0.0), delta),
            Event::KeyboardInput(_, _, Some(Key::F)) =>
                self.cam.rot = self.cam.rot * cgmath::Quaternion::from_axis_angle(cgmath::vec3(1.0, 0.0, 0.0), -delta),
            Event::KeyboardInput(_, _, Some(Key::A)) =>
                self.cam.rot = cgmath::Quaternion::from_axis_angle(cgmath::vec3(0.0, 0.0, 1.0), delta) * self.cam.rot,
            Event::KeyboardInput(_, _, Some(Key::D)) =>
                self.cam.rot = cgmath::Quaternion::from_axis_angle(cgmath::vec3(0.0, 0.0, 1.0), -delta) * self.cam.rot,
            _ => {},
        }
        true
    }
    fn on_frame<C: gfx::CommandBuffer<R>>(&mut self, enc: &mut gfx::Encoder<R, C>) {
        self.render.draw(enc, &self.cam);
    }
}


struct ObjectViewApp<R: gfx::Resources> {
    bundle: gfx::Bundle<R, render::object::Data<R>>,
    cam: Camera,
}

impl<R: gfx::Resources> ObjectViewApp<R> {
    fn new<F: gfx::Factory<R>>(path: &str, settings: &config::Settings,
           output: gfx::handle::RenderTargetView<R, render::ColorFormat>,
           factory: &mut F) -> ObjectViewApp<R>
    {
        use std::io::BufReader;
        use std::fs::File;
        use gfx::traits::FactoryExt;

        let pal_data = level::load_palette(&settings.get_object_palette_path());

        let mut file = BufReader::new(File::open(path).unwrap());
        let (vbuf, slice) = model::load_c3d(&mut file, factory);
        let pso = render::Render::create_object_pso(factory);
        let data = render::object::Data {
            vbuf: vbuf,
            locals: factory.create_constant_buffer(1),
            palette: render::Render::create_palette(&pal_data, factory),
            out: output,
        };
        ObjectViewApp {
            bundle: gfx::Bundle::new(slice, pso, data),
            cam: Camera {
                loc: cgmath::vec3(0.0, -100.0, 50.0),
                rot: cgmath::Quaternion::new(0.0, 1.0, 0.0, 0.0),
                proj: cgmath::PerspectiveFov {
                    fovy: cgmath::deg(45.0).into(),
                    aspect: settings.get_screen_aspect(),
                    near: 1.0,
                    far: 300.0,
                },
            },
        }
    }
}


fn main() {
    use std::env;
    env_logger::init().unwrap();

    info!("Loading the settings");
    let settings = config::Settings::load("config/settings.toml");

    info!("Creating the window with GL context");
    let builder = glutin::WindowBuilder::new()
        .with_title(settings.window.title.clone())
        .with_dimensions(settings.window.size[0], settings.window.size[1])
        .with_vsync();
    let (window, mut device, mut factory, main_color, _main_depth) =
        gfx_window_glutin::init::<render::ColorFormat, render::DepthFormat>(builder);

    info!("Parsing command line");
    let args: Vec<_> = env::args().collect();
    let program = args[0].clone();
    let mut options = getopts::Options::new();
    options
        .parsing_style(getopts::ParsingStyle::StopAtFirstFree)
        .optflag("h", "help", "print this help menu")
        .optopt("v", "view", "view a particular game resource", "");
    let matches = options.parse(&args[1..]).unwrap();
    if matches.opt_present("h") || !matches.free.is_empty() {
        let brief = format!("Usage: {} [options]", program);
        println!("{}", options.usage(&brief));
        return;
    }

    //let app = match matches.opt_str("v") {}
    let mut app = Box::new(GameApp::new(&settings, main_color, &mut factory));

    let mut encoder: gfx::Encoder<_, _> = factory.create_command_buffer().into();
    'main: loop {
        use gfx::Device;
        for event in window.poll_events() {
            if !app.on_event(event, &mut factory) {
                break 'main;
            }
        }
        app.on_frame(&mut encoder);
        encoder.flush(&mut device);
        window.swap_buffers().unwrap();
        device.cleanup();
    }
}
