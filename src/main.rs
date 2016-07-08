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
extern crate time;
extern crate toml;

mod app;
mod config;
mod level;
mod model;
mod render;
mod splay;


enum RoadApp<R: gfx::Resources> {
    Car(app::CarView<R>),
    Game(app::Game<R>),
    View(app::ResourceView<R>),
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
    let (window, mut device, mut factory, main_color, main_depth) =
        gfx_window_glutin::init::<render::ColorFormat, render::DepthFormat>(builder);

    info!("Parsing command line");
    let args: Vec<_> = env::args().collect();
    let program = args[0].clone();
    let mut options = getopts::Options::new();
    options
        .parsing_style(getopts::ParsingStyle::StopAtFirstFree)
        .optflag("h", "help", "print this help menu")
        .optopt("v", "view", "view a particular game resource", "")
        .optflag("m", "model", "view a only the car model");
    let matches = options.parse(&args[1..]).unwrap();
    if matches.opt_present("h") || !matches.free.is_empty() {
        let brief = format!("Usage: {} [options]", program);
        println!("{}", options.usage(&brief));
        return;
    }

    let mut app = if matches.opt_present("m") {
        RoadApp::Car(app::CarView::new(&settings, main_color, main_depth, &mut factory))
    } else {
        match matches.opt_str("v") {
            Some(path) => RoadApp::View(app::ResourceView::new(&path, &settings, main_color, main_depth, &mut factory)),
            _ => RoadApp::Game(app::Game::new(&settings, main_color, main_depth, &mut factory)),
        }
    };

    let mut encoder: gfx::Encoder<_, _> = factory.create_command_buffer().into();
    let mut last_time = time::precise_time_s() as f32;
    loop {
        use gfx::Device;
        use app::App;
        let events = window.poll_events();
        let delta = time::precise_time_s() as f32 - last_time;
        match app {
            RoadApp::Car(ref mut a) => {
                if !a.update(events, delta, &mut factory) {
                    break
                }
                a.draw(&mut encoder);
            },
            RoadApp::Game(ref mut a) => {
                if !a.update(events, delta, &mut factory) {
                    break
                }
                a.draw(&mut encoder);
            },
            RoadApp::View(ref mut a) => {
                if !a.update(events, delta, &mut factory) {
                    break
                }
                a.draw(&mut encoder);
            },
        }
        encoder.flush(&mut device);
        window.swap_buffers().unwrap();
        device.cleanup();
        last_time += delta;
    }
}
