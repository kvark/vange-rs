extern crate env_logger;
extern crate gfx;
extern crate gfx_device_gl;
extern crate gfx_window_glutin;
extern crate glutin;
extern crate vangers;

use vangers::{config, render};

pub use self::glutin::{ElementState, KeyboardInput, ModifiersState, VirtualKeyCode as Key, MouseScrollDelta, MouseButton};


pub trait Application<R: gfx::Resources> {
    fn on_key(&mut self, KeyboardInput) -> bool;
    fn on_mouse_wheel(&mut self, _delta: MouseScrollDelta) {}
    fn on_cursor_move(&mut self, _position: (f64, f64)) {}
    fn on_mouse_button(&mut self, _state: ElementState, _button: MouseButton) {}
    fn gpu_update<F: gfx::Factory<R>>(
        &mut self, &mut F, resize: Option<render::MainTargets<R>>, reload: bool
    );
    fn update(&mut self, delta: f32);
    fn draw<C: gfx::CommandBuffer<R>>(
        &mut self, &mut gfx::Encoder<R, C>
    );
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
        let settings = config::Settings::load("config/settings.ron");

        info!("Creating the window with GL context");
        #[cfg(target_os = "linux")]
        let events_loop = <glutin::EventsLoop as glutin::os::unix::EventsLoopExt>::new_x11()
            .unwrap();
        #[cfg(not(target_os = "linux"))]
        let events_loop = glutin::EventsLoop::new();

        let win_builder = glutin::WindowBuilder::new()
            .with_title(settings.window.title.clone())
            .with_dimensions(glutin::dpi::LogicalSize::from_physical(
                (settings.window.size[0], settings.window.size[1]),
                events_loop.get_primary_monitor().get_hidpi_factor(),
            ));
        let context_build = glutin::ContextBuilder::new()
            .with_gl_profile(glutin::GlProfile::Core)
            .with_vsync(true);

        let (window, device, factory, color, depth) = gfx_window_glutin::init(
            win_builder,
            context_build,
            &events_loop,
        ).unwrap();

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

            let win = &self.window;
            let mut resized_targets = None;
            let mut reload_shaders = false;
            self.events_loop.poll_events(|event| match event {
                glutin::Event::WindowEvent { window_id, ref event } if window_id == win.id() => {
                    match *event {
                        glutin::WindowEvent::Resized(size) => {
                            info!("Resizing to {:?}", size);
                            let (color, depth) = gfx_window_glutin::new_views(win);
                            resized_targets = Some(render::MainTargets { color, depth });
                        }
                        glutin::WindowEvent::Focused(true) => {
                            info!("Reloading shaders");
                            reload_shaders = true;
                        }
                        glutin::WindowEvent::CloseRequested => {
                            running = false;
                        }
                        glutin::WindowEvent::KeyboardInput { input, .. } => {
                            if !app.on_key(input) {
                                running = false;
                            }
                        }
                        glutin::WindowEvent::MouseWheel {delta, ..} => {
                            app.on_mouse_wheel(delta)
                        }
                        glutin::WindowEvent::CursorMoved {position, ..} => {
                            app.on_cursor_move(position.into())
                        }
                        glutin::WindowEvent::MouseInput {state, button, ..} => {
                            app.on_mouse_button(state, button)
                        }
                        _ => {}
                    }
                }
                _ => {}
            });

            app.gpu_update(&mut self.factory, resized_targets, reload_shaders);

            let duration = time::Instant::now() - last_time;
            last_time += duration;
            let delta = duration.as_secs() as f32 +
                duration.subsec_nanos() as f32 * 1.0e-9;

            app.update(delta);
            app.draw(&mut encoder);

            encoder.flush(&mut self.device);
            self.window.swap_buffers().unwrap();
            self.device.cleanup();
        }
    }
}
