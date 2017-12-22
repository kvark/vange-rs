extern crate env_logger;
extern crate gfx;
extern crate gfx_device_gl;
extern crate gfx_window_glutin;
extern crate glutin;
extern crate vangers;

use vangers::{config, render};


pub trait Application<R: gfx::Resources> {
    fn resize<F: gfx::Factory<R>>(&mut self, render::MainTargets<R>, &mut F);
    fn on_key<F: gfx::Factory<R>>(&mut self, glutin::KeyboardInput, &mut F) -> bool;
    fn update(&mut self, delta: f32);
    fn draw<C: gfx::CommandBuffer<R>>(&mut self, &mut gfx::Encoder<R, C>);
}

type MainTargets = render::MainTargets<gfx_device_gl::Resources>;

pub struct Harness {
    events_loop: glutin::EventsLoop,
    window: glutin::GlWindow,
    device: gfx_device_gl::Device,
    pub factory: gfx_device_gl::Factory,
}

impl Harness {
    pub fn init() -> (Self, config::Settings, MainTargets) {
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
        let harness = Harness {
            events_loop,
            window,
            device,
            factory,
        };

        (harness, settings, targets)
    }

    pub fn main_loop<A>(&mut self, mut app: A)
    where
        A: Application<gfx_device_gl::Resources>,
    {
        use std::time;

        let mut encoder = gfx::Encoder::from(self.factory.create_command_buffer());
        let mut last_time = time::Instant::now();
        let mut running = true;

        while running {
            use gfx::Device;
            use glutin::GlContext;

            let (win, factory) = (&self.window, &mut self.factory);
            self.events_loop.poll_events(|event| match event {
                glutin::Event::WindowEvent { window_id, ref event } if window_id == win.id() => {
                    match *event {
                        glutin::WindowEvent::Resized(_, _) => {
                            let (color, depth) = gfx_window_glutin::new_views(win);
                            let new_targets = render::MainTargets { color, depth };
                            app.resize(new_targets, factory);
                        }
                        glutin::WindowEvent::Closed => {
                            running = false;
                        }
                        glutin::WindowEvent::KeyboardInput { input, .. } => {
                            if !app.on_key(input, factory) {
                                running = false;
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            });

            let duration = time::Instant::now() - last_time;
            let delta = duration.as_secs() as f32 +
                duration.subsec_nanos() as f32 * 1.0e-9;
            app.update(delta);
            app.draw(&mut encoder);

            encoder.flush(&mut self.device);
            self.window.swap_buffers().unwrap();
            self.device.cleanup();
            last_time += duration;
        }
    }
}
