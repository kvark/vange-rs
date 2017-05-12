extern crate vangers;
extern crate env_logger;
#[macro_use]
extern crate log;
extern crate getopts;
extern crate cgmath;
extern crate gfx;
extern crate gfx_window_glutin;
extern crate glutin;
extern crate time;

mod game;

use vangers::{config, render};


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
    let event_loop = glutin::EventsLoop::new();
    let (window, mut device, mut factory, main_color, main_depth) =
        gfx_window_glutin::init::<render::ColorFormat, render::DepthFormat>(builder, &event_loop);

    info!("Parsing command line");
    let args: Vec<_> = env::args().collect();
    let mut options = getopts::Options::new();
    options
        .parsing_style(getopts::ParsingStyle::StopAtFirstFree)
        .optflag("h", "help", "print this help menu");

    let matches = options.parse(&args[1..]).unwrap();
    if matches.opt_present("h") || !matches.free.is_empty() {
        println!("Vangers game prototype");
        let brief = format!("Usage: {} [options]", args[0]);
        println!("{}", options.usage(&brief));
        return;
    }

    let mut game = game::Game::new(&settings, main_color, main_depth, &mut factory);

    let mut encoder = gfx::Encoder::from(factory.create_command_buffer());
    let mut last_time = time::precise_time_s() as f32;
    let mut running = true;

    while running {
        use gfx::Device;

        event_loop.poll_events(|glutin::Event::WindowEvent {event, ..}|
            if !game.react(event, &mut factory) {
                running = false;
            }
        );

        let delta = time::precise_time_s() as f32 - last_time;
        game.update(delta);
        game.draw(&mut encoder);

        encoder.flush(&mut device);
        window.swap_buffers().unwrap();
        device.cleanup();
        last_time += delta;
    }
}
