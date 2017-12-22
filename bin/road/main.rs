extern crate cgmath;
extern crate getopts;
extern crate gfx;
extern crate glutin;
#[macro_use]
extern crate log;
extern crate time;
extern crate vangers;

mod game;
#[path = "../boilerplate.rs"]
mod boilerplate;

fn main() {
    use std::env;

    let (settings, mut events_loop, window, mut device, mut factory, main_targets) = boilerplate::init();

    info!("Parsing command line");
    let args: Vec<_> = env::args().collect();
    let mut options = getopts::Options::new();
    options
        .parsing_style(getopts::ParsingStyle::StopAtFirstFree)
        .optflag("h", "help", "print this help menu");

    let matches = options.parse(&args[1 ..]).unwrap();
    if matches.opt_present("h") || !matches.free.is_empty() {
        println!("Vangers game prototype");
        let brief = format!("Usage: {} [options]", args[0]);
        println!("{}", options.usage(&brief));
        return;
    }

    let mut game = game::Game::new(&settings, main_targets, &mut factory);

    let mut encoder = gfx::Encoder::from(factory.create_command_buffer());
    let mut last_time = time::precise_time_s() as f32;
    let mut running = true;

    while running {
        use gfx::Device;
        use glutin::GlContext;

        events_loop.poll_events(|event| match event {
            glutin::Event::WindowEvent { window_id, ref event } if window_id == window.id() => {
                match *event {
                    glutin::WindowEvent::Resized(_, _) => {
                        let new_targets = boilerplate::resize(&window);
                        game.resize(new_targets, &mut factory);
                    }
                    glutin::WindowEvent::Closed => {
                        running = false;
                    }
                    glutin::WindowEvent::KeyboardInput { input, .. } => {
                        if !game.on_key(input, &mut factory) {
                            running = false;
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        });

        let delta = time::precise_time_s() as f32 - last_time;
        game.update(delta);
        game.draw(&mut encoder);

        encoder.flush(&mut device);
        window.swap_buffers().unwrap();
        device.cleanup();
        last_time += delta;
    }
}
