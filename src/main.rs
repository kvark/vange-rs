extern crate byteorder;
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


#[derive(RustcDecodable)]
struct Window {
    title: String,
    size: [u32; 2],
}

#[derive(RustcDecodable)]
struct Settings {
    game_path: String,
    level: String,
    window: Window,
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

    let builder = glutin::WindowBuilder::new()
        .with_title(settings.window.title)
        .with_dimensions(settings.window.size[0], settings.window.size[1])
        .with_vsync();
    let (window, mut device, mut factory, main_color, _main_depth) =
        gfx_window_glutin::init::<render::ColorFormat, render::DepthFormat>(builder);

    let config = level::Config {
        path_vpr: format!("{}/thechain/{}/output.vpr", settings.game_path, settings.level),
        path_vmc: format!("{}/thechain/{}/output.vmc", settings.game_path, settings.level),
        name: settings.level,
        size: (Power(11), Power(14)),
        geo: Power(5),
        section: Power(7),
    };
    let lev = level::load(&config);

    let mut encoder: gfx::Encoder<_, _> = factory.create_command_buffer().into();
    let render = render::init(&mut factory, main_color, lev.size, &lev.height, &lev.meta);

    'main: loop {
        use gfx::Device;
        // loop over events
        for event in window.poll_events() {
            match event {
                glutin::Event::KeyboardInput(_, _, Some(glutin::VirtualKeyCode::Escape)) |
                glutin::Event::Closed => break 'main,
                _ => {},
            }
        }
        // draw a frame
        render.draw(&mut encoder);
        encoder.flush(&mut device);
        window.swap_buffers().unwrap();
        device.cleanup();
    }
}
