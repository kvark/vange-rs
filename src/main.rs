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

mod app;
mod config;
mod level;
mod model;
mod render;
mod splay;


enum RoadApp<R: gfx::Resources> {
    Game(app::Game<R>),
    View(app::ModelView<R>),
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

    let mut app = match matches.opt_str("v") {
        Some(path) => RoadApp::View(app::ModelView::new(&path, &settings, main_color, &mut factory)),
        _ => RoadApp::Game(app::Game::new(&settings, main_color, &mut factory)),
    };

    let mut encoder: gfx::Encoder<_, _> = factory.create_command_buffer().into();
    loop {
        use gfx::Device;
        use app::App;
        let ok = match app {
            RoadApp::Game(ref mut a) => a.do_iter(window.poll_events(), &mut factory, &mut encoder),
            RoadApp::View(ref mut a) => a.do_iter(window.poll_events(), &mut factory, &mut encoder),
        };
        if !ok {
            break
        }
        encoder.flush(&mut device);
        window.swap_buffers().unwrap();
        device.cleanup();
    }
}
