extern crate env_logger;
extern crate vangers;
//#[macro_use]
//extern crate log;
extern crate cgmath;
extern crate getopts;
extern crate gfx;
extern crate gfx_window_glutin;
extern crate glutin;
extern crate time;

mod app;

use vangers::{config, render};

fn main() {
    env_logger::init().unwrap();

    let settings = config::Settings::load("config/settings.toml");

    let win_builder = glutin::WindowBuilder::new()
        .with_title(settings.window.title.clone())
        .with_dimensions(settings.window.size[0], settings.window.size[1]);
    let context_build = glutin::ContextBuilder::new()
        .with_gl_profile(glutin::GlProfile::Core)
        .with_vsync(true);
    let mut event_loop = glutin::EventsLoop::new();
    let (window, mut device, mut factory, main_color, main_depth) =
        gfx_window_glutin::init::<render::ColorFormat, render::DepthFormat>(
            win_builder,
            context_build,
            &event_loop,
        );

    let mut app = app::LevelView::new(&settings, main_color, main_depth, &mut factory);

    let mut encoder = gfx::Encoder::from(factory.create_command_buffer());
    let mut last_time = time::precise_time_s() as f32;
    let mut running = true;

    while running {
        use gfx::Device;
        use glutin::GlContext;

        event_loop.poll_events(|event| {
            if let glutin::Event::WindowEvent { event, .. } = event {
                if !app.react(event, &mut factory) {
                    running = false;
                }
            }
        });

        let delta = time::precise_time_s() as f32 - last_time;
        app.update(delta);
        app.draw(&mut encoder);

        encoder.flush(&mut device);
        window.swap_buffers().unwrap();
        device.cleanup();
        last_time += delta;
    }
}
