//! Web entry point for vange-rs level viewer with test level.
//! Compiled with `cargo build --target wasm32-unknown-unknown --features web --bin web`

use wasm_bindgen::prelude::*;

use vangers::{
    config::settings,
    level,
    render::{Batcher, GraphicsContext, Render, ScreenTargets, DEPTH_FORMAT},
    space,
};

use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::{self, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::Window,
};

struct WebApp {
    render: Render,
    level: level::Level,
    cam: space::Camera,
    batcher: Batcher,
}

impl WebApp {
    fn new(gfx: &GraphicsContext) -> Self {
        let level_config = level::LevelConfig::new_test();
        let geometry = settings::Geometry::default();
        let objects_palette = [[0xFF; 4]; 0x100]; // white palette for test level

        let cam = space::Camera {
            loc: glam::vec3(0.0, 0.0, 400.0),
            rot: glam::Quat::IDENTITY,
            scale: glam::vec3(1.0, -1.0, 1.0),
            proj: space::Projection::Perspective(space::PerspectiveParams {
                fovy: 45.0f32.to_radians(),
                aspect: 800.0 / 600.0,
                near: 10.0,
                far: 2000.0,
            }),
        };

        let render = Render::new(
            gfx,
            &level_config,
            &objects_palette,
            &settings::Render::default(),
            &geometry,
            cam.front_face(),
        );
        let level = level::load(&level_config, &geometry);

        WebApp {
            render,
            level,
            cam,
            batcher: Batcher::new(),
        }
    }

    fn draw(&mut self, device: &wgpu::Device, targets: ScreenTargets) -> wgpu::CommandBuffer {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("World"),
        });
        self.render.draw_world(
            &mut encoder,
            &mut self.batcher,
            &self.level,
            &self.cam,
            targets,
            None,
            device,
        );
        encoder.finish()
    }
}

#[cfg(target_arch = "wasm32")]
use web_time::Instant;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

struct WebHandler {
    window: Option<Arc<Window>>,
    surface: Option<wgpu::Surface<'static>>,
    device: Option<wgpu::Device>,
    queue: Option<wgpu::Queue>,
    config: Option<wgpu::SurfaceConfiguration>,
    depth_view: Option<wgpu::TextureView>,
    app: Option<WebApp>,
    screen_size: wgpu::Extent3d,
    keys_pressed: std::collections::HashSet<KeyCode>,
    last_frame: Option<Instant>,
}

impl WebHandler {
    fn new() -> Self {
        WebHandler {
            window: None,
            surface: None,
            device: None,
            queue: None,
            config: None,
            depth_view: None,
            app: None,
            screen_size: wgpu::Extent3d {
                width: 800,
                height: 600,
                depth_or_array_layers: 1,
            },
            keys_pressed: std::collections::HashSet::new(),
            last_frame: None,
        }
    }
}

impl ApplicationHandler for WebHandler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = Window::default_attributes()
            .with_title("Vangers Web")
            .with_inner_size(winit::dpi::PhysicalSize::new(800u32, 600u32));

        #[cfg(target_arch = "wasm32")]
        let attrs = {
            use winit::platform::web::WindowAttributesExtWebSys;
            let document = web_sys::window().unwrap().document().unwrap();
            let canvas = document
                .get_element_by_id("canvas")
                .expect("missing <canvas id='canvas'>")
                .dyn_into::<web_sys::HtmlCanvasElement>()
                .unwrap();
            attrs.with_canvas(Some(canvas))
        };

        let window = Arc::new(event_loop.create_window(attrs).unwrap());
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());

        let surface = instance
            .create_surface(window.clone())
            .expect("Unable to create surface");

        // Async GPU init
        let window_clone = window.clone();
        let init_future = async move {
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: false,
                })
                .await
                .expect("No GPU adapter found");

            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: None,
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
                    memory_hints: wgpu::MemoryHints::default(),
                    trace: wgpu::Trace::Off,
                    experimental_features: Default::default(),
                })
                .await
                .expect("Failed to create device");

            (surface, adapter, device, queue, window_clone)
        };

        // On web, we need wasm_bindgen_futures; on native, use pollster
        #[cfg(target_arch = "wasm32")]
        {
            self.window = Some(window);
            wasm_bindgen_futures::spawn_local(async move {
                let (_surface, _adapter, _device, _queue, _window) = init_future.await;
                // Note: This is a simplified structure. A full implementation
                // would use shared state to feed these back to the handler.
                log::info!("GPU initialized on web!");
            });
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let (surface, _adapter, device, queue, _) = pollster::block_on(init_future);
            let caps = surface.get_capabilities(&_adapter);
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: caps.formats[0],
                width: self.screen_size.width,
                height: self.screen_size.height,
                present_mode: wgpu::PresentMode::Fifo,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: Vec::new(),
                desired_maximum_frame_latency: 2,
            };
            surface.configure(&device, &config);

            let depth_view = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("Depth"),
                    size: self.screen_size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: DEPTH_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default());

            let gfx = GraphicsContext {
                downlevel_caps: _adapter.get_downlevel_capabilities(),
                color_format: config.format,
                screen_size: self.screen_size,
                device,
                queue,
            };
            let app = WebApp::new(&gfx);

            self.window = Some(window);
            self.device = Some(gfx.device);
            self.queue = Some(gfx.queue);
            self.surface = Some(surface);
            self.config = Some(config);
            self.depth_view = Some(depth_view);
            self.app = Some(app);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput {
                event:
                    event::KeyEvent {
                        physical_key: PhysicalKey::Code(key),
                        state,
                        ..
                    },
                ..
            } => {
                match state {
                    event::ElementState::Pressed => {
                        self.keys_pressed.insert(key);
                    }
                    event::ElementState::Released => {
                        self.keys_pressed.remove(&key);
                    }
                }
                if key == KeyCode::Escape && state == event::ElementState::Pressed {
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                self.screen_size = wgpu::Extent3d {
                    width: size.width,
                    height: size.height,
                    depth_or_array_layers: 1,
                };
                if let (Some(device), Some(surface), Some(config)) =
                    (&self.device, &self.surface, &mut self.config)
                {
                    config.width = size.width;
                    config.height = size.height;
                    surface.configure(device, config);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        let (Some(device), Some(queue), Some(surface), Some(depth_view), Some(app)) = (
            &self.device,
            &self.queue,
            &self.surface,
            &self.depth_view,
            &mut self.app,
        ) else {
            return;
        };

        // Compute delta time
        let now = Instant::now();
        let dt = match self.last_frame {
            Some(prev) => (now - prev).as_secs_f32(),
            None => 1.0 / 60.0,
        };
        self.last_frame = Some(now);
        let dt = dt.min(0.1); // clamp to avoid huge jumps

        // Camera movement from keyboard
        let move_speed = 100.0;
        let rotation_speed = 1.0;

        // Forward (W)
        if self.keys_pressed.contains(&KeyCode::KeyW) {
            let mut dir = app.cam.rot * glam::Vec3::Y;
            dir.z = 0.0;
            if dir.length_squared() > 0.0 {
                app.cam.loc += move_speed * dt * dir.normalize();
            }
        }
        // Backward (S)
        if self.keys_pressed.contains(&KeyCode::KeyS) {
            let mut dir = app.cam.rot * glam::Vec3::Y;
            dir.z = 0.0;
            if dir.length_squared() > 0.0 {
                app.cam.loc -= move_speed * dt * dir.normalize();
            }
        }
        // Strafe left (A)
        if self.keys_pressed.contains(&KeyCode::KeyA) {
            let mut dir = app.cam.rot * glam::Vec3::X;
            dir.z = 0.0;
            if dir.length_squared() > 0.0 {
                app.cam.loc -= move_speed * dt * dir.normalize();
            }
        }
        // Strafe right (D)
        if self.keys_pressed.contains(&KeyCode::KeyD) {
            let mut dir = app.cam.rot * glam::Vec3::X;
            dir.z = 0.0;
            if dir.length_squared() > 0.0 {
                app.cam.loc += move_speed * dt * dir.normalize();
            }
        }
        // Up (Z)
        if self.keys_pressed.contains(&KeyCode::KeyZ) {
            app.cam.loc.z += move_speed * dt;
        }
        // Down (X)
        if self.keys_pressed.contains(&KeyCode::KeyX) {
            app.cam.loc.z -= move_speed * dt;
        }
        // Rotate left (Q)
        if self.keys_pressed.contains(&KeyCode::KeyQ) {
            let rotation = glam::Quat::from_rotation_z(rotation_speed * dt);
            app.cam.rot = rotation * app.cam.rot;
        }
        // Rotate right (E)
        if self.keys_pressed.contains(&KeyCode::KeyE) {
            let rotation = glam::Quat::from_rotation_z(-rotation_speed * dt);
            app.cam.rot = rotation * app.cam.rot;
        }

        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            _ => return,
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let targets = ScreenTargets {
            extent: self.screen_size,
            color: &view,
            depth: depth_view,
        };
        let command_buffer = app.draw(device, targets);
        queue.submit(std::iter::once(command_buffer));
        frame.present();
    }
}

#[wasm_bindgen(start)]
pub fn web_main() {
    #[cfg(target_arch = "wasm32")]
    {
        console_error_panic_hook::set_once();
        console_log::init_with_level(log::Level::Info).unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        env_logger::init();
    }

    log::info!("Starting Vangers Web");
    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut handler = WebHandler::new();
    event_loop.run_app(&mut handler).unwrap();
}

fn main() {
    web_main();
}
