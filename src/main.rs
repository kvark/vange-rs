extern crate byteorder;
extern crate cgmath;
extern crate env_logger;
#[macro_use]
extern crate gfx;
extern crate gfx_window_glutin;
extern crate glutin;
#[macro_use]
extern crate log;
extern crate progressive;
extern crate rustc_serialize;
extern crate toml;

mod level;
mod render;
mod splay;


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

#[derive(RustcDecodable)]
struct WindowSettings {
    title: String,
    size: [u32; 2],
}

#[derive(RustcDecodable)]
struct Settings {
    game_path: String,
    level: String,
    window: WindowSettings,
}

use level::Power;

fn main() {
    env_logger::init().unwrap();
    info!("Loading the settings");

    let settings: Settings = {
        use std::io::{BufReader, Read};
        use std::fs::File;
        let mut string = String::new();
        BufReader::new(File::open("config/settings.toml").unwrap())
            .read_to_string(&mut string).unwrap();
        toml::decode_str(&string).unwrap()
    };

    info!("Creating the window with GL context");
    let builder = glutin::WindowBuilder::new()
        .with_title(settings.window.title)
        .with_dimensions(settings.window.size[0], settings.window.size[1])
        .with_vsync();
    let (window, mut device, mut factory, main_color, _main_depth) =
        gfx_window_glutin::init::<render::ColorFormat, render::DepthFormat>(builder);

    let mut cam = Camera {
        loc: cgmath::vec3(0.0, 0.0, 200.0),
        rot: cgmath::Quaternion::new(1.0, 0.0, 0.0, 0.0),
        proj: cgmath::PerspectiveFov {
            fovy: cgmath::deg(45.0).into(),
            aspect: settings.window.size[0] as f32 / settings.window.size[1] as f32,
            near: 1.0,
            far: 100000.0,
        },
    };

    let config = level::Config {
        path_vpr: format!("{}/thechain/{}/output.vpr", settings.game_path, settings.level),
        path_vmc: format!("{}/thechain/{}/output.vmc", settings.game_path, settings.level),
        path_palette: format!("{}/thechain/{}/harmony.pal", settings.game_path, settings.level),
        name: settings.level,
        size: (Power(11), Power(14)),
        geo: Power(5),
        section: Power(7),
    };
    let lev = level::load(&config);

    let mut encoder: gfx::Encoder<_, _> = factory.create_command_buffer().into();
    let mut render = render::init(&mut factory, main_color, lev.size, &lev.height, &lev.meta, &lev.palette);

    'main: loop {
        use gfx::Device;
        // loop over events
        for event in window.poll_events() {
            use cgmath::Rotation3;
            use glutin::{Event, VirtualKeyCode as Key};
            let delta = cgmath::rad(0.05);
            let step = 10.0;
            match event {
                Event::KeyboardInput(_, _, Some(Key::Escape)) |
                Event::Closed => break 'main,
                Event::KeyboardInput(_, _, Some(Key::L)) => render.reload(&mut factory),
                Event::KeyboardInput(_, _, Some(Key::W)) =>
                    cam.loc = cam.loc + cgmath::vec3(0.0, step, 0.0),
                Event::KeyboardInput(_, _, Some(Key::S)) =>
                    cam.loc = cam.loc - cgmath::vec3(0.0, step, 0.0),
                Event::KeyboardInput(_, _, Some(Key::R)) =>
                    cam.rot = cam.rot * cgmath::Quaternion::from_axis_angle(cgmath::vec3(1.0, 0.0, 0.0), delta),
                Event::KeyboardInput(_, _, Some(Key::F)) =>
                    cam.rot = cam.rot * cgmath::Quaternion::from_axis_angle(cgmath::vec3(1.0, 0.0, 0.0), -delta),
                Event::KeyboardInput(_, _, Some(Key::A)) =>
                    cam.rot = cgmath::Quaternion::from_axis_angle(cgmath::vec3(0.0, 0.0, 1.0), delta) * cam.rot,
                Event::KeyboardInput(_, _, Some(Key::D)) =>
                    cam.rot = cgmath::Quaternion::from_axis_angle(cgmath::vec3(0.0, 0.0, 1.0), -delta) * cam.rot,
                _ => {},
            }
        }
        // draw a frame
        render.draw(&mut encoder, &cam);
        encoder.flush(&mut device);
        window.swap_buffers().unwrap();
        device.cleanup();
    }
}
