extern crate env_logger;
extern crate gfx;
extern crate gfx_device_gl;
extern crate gfx_window_glutin;
extern crate glutin;
extern crate vangers;

use vangers::{config, render};

pub fn init() -> (
    config::Settings,
    glutin::EventsLoop,
    glutin::GlWindow,
    gfx_device_gl::Device,
    gfx_device_gl::Factory,
    render::MainTargets<gfx_device_gl::Resources>,
) {
    env_logger::init().unwrap();
    info!("Loading the settings");
    let settings = config::Settings::load("config/settings.toml");

    info!("Creating the window with GL context");
    let win_builder = glutin::WindowBuilder::new()
        .with_title(settings.window.title.clone())
        .with_dimensions(settings.window.size[0], settings.window.size[1]);
    let context_build = glutin::ContextBuilder::new()
        .with_gl_profile(glutin::GlProfile::Core)
        .with_vsync(true);
    let events_loop = glutin::EventsLoop::new();
    let (window, device, factory, color, depth) = gfx_window_glutin::init(
        win_builder,
        context_build,
        &events_loop,
    );

    let targets = render::MainTargets { color, depth };
    (settings, events_loop, window, device, factory, targets)
}
