extern crate env_logger;
extern crate gfx;
extern crate gfx_device_gl;
extern crate gfx_window_glutin;
extern crate glutin;
extern crate vangers;

use vangers::{config, render};

pub use self::glutin::{ElementState, KeyboardInput, ModifiersState, VirtualKeyCode as Key, MouseScrollDelta, MouseButton};


pub trait Application<R: gfx::Resources> {
    fn on_resize<F: gfx::Factory<R>>(&mut self, render::MainTargets<R>, &mut F);
    fn on_key(&mut self, KeyboardInput) -> bool;
    fn on_mouse_wheel(&mut self, MouseScrollDelta);
    fn on_mouse_move(&mut self, delta_x: f32, delta_y: f32, alt: bool);
    fn update(&mut self, delta: f32);
    fn draw<C: gfx::CommandBuffer<R>>(&mut self, &mut gfx::Encoder<R, C>);
    fn reload_shaders<F: gfx::Factory<R>>(&mut self, &mut F);
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
        let mut last_mouse_x: f32 = -1.0;
        let mut last_mouse_y: f32 = -1.0;
        let mut mouse_pressed: bool = false;
        let mut alt_pressed: bool = false;

        while running {
            use gfx::Device;
            use self::glutin::GlContext;

            let (win, factory) = (&self.window, &mut self.factory);
            self.events_loop.poll_events(|event| match event {
                glutin::Event::WindowEvent { window_id, ref event } if window_id == win.id() => {
                    match *event {
                        glutin::WindowEvent::Resized(width, height) => {
                            info!("Resizing to {}x{}", width, height);
                            let (color, depth) = gfx_window_glutin::new_views(win);
                            let new_targets = render::MainTargets { color, depth };
                            app.on_resize(new_targets, factory);
                        }
                        glutin::WindowEvent::Focused(true) => {
                            info!("Reloading shaders");
                            app.reload_shaders(factory);
                        }
                        glutin::WindowEvent::Closed => {
                            running = false;
                        }
                        glutin::WindowEvent::KeyboardInput { input, .. } => {
                            if !app.on_key(input) {
                                running = false;
                            }
                            info!("alt_pressed: {:?}", input);
                            match input.virtual_keycode {
                                Some(Key::LControl) => {
                                    alt_pressed = input.state == ElementState::Pressed;
                                }
                                _ => {}
                            }

                        }
                        glutin::WindowEvent::MouseWheel {delta, ..} => {
                            app.on_mouse_wheel(delta)
                        }
                        glutin::WindowEvent::CursorMoved {position, ..} => {
                            if mouse_pressed {
                                match position {
                                    (x, y) => {
                                        if last_mouse_x >= 0.0 {
                                            app.on_mouse_move(
                                                x as f32 - last_mouse_x,
                                                y as f32- last_mouse_y ,
                                                alt_pressed,
                                            );
                                        }
                                        last_mouse_x = x as f32;
                                        last_mouse_y = y as f32;
                                    }
                                }
                            }
                        }
                        glutin::WindowEvent::MouseInput {state, button, ..} => {
                            if button == MouseButton::Left {
                                if state == ElementState::Released {
                                    mouse_pressed = false;
                                    last_mouse_x = -1.0;
                                } else {
                                    mouse_pressed = true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            });

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
