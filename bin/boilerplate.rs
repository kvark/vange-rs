#![allow(clippy::single_match)]
use vangers::{
    config::{settings::Terrain, Settings},
    render::{GraphicsContext, ScreenTargets, UiData, DEPTH_FORMAT},
};

use egui_winit_platform as egui_wp;
use futures::executor::LocalPool;
use log::info;
use winit::{
    event,
    event_loop::{ControlFlow, EventLoop},
    window::{Window, WindowBuilder},
};

pub trait Application {
    fn on_key(&mut self, input: event::KeyboardInput) -> bool;
    fn on_mouse_wheel(&mut self, _delta: event::MouseScrollDelta) {}
    fn on_cursor_move(&mut self, _position: (f64, f64)) {}
    fn on_mouse_button(&mut self, _state: event::ElementState, _button: event::MouseButton) {}
    fn resize(&mut self, _device: &wgpu::Device, _extent: wgpu::Extent3d) {}
    fn reload(&mut self, device: &wgpu::Device);
    fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, delta: f32);
    fn draw_ui(&mut self, context: &egui::Context);
    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        targets: ScreenTargets,
        ui_data: UiData,
    );
}

struct WindowContext {
    window: Window,
    task_pool: LocalPool,
    surface: wgpu::Surface,
    present_mode: wgpu::PresentMode,
    reload_on_focus: bool,
    egui_platform: egui_wp::Platform,
    depth_target: wgpu::TextureView,
}

pub struct Harness {
    event_loop: EventLoop<()>,
    window_ctx: WindowContext,
    pub graphics_ctx: GraphicsContext,
}

pub struct HarnessOptions {
    pub title: &'static str,
    pub uses_level: bool,
}

impl Harness {
    pub fn init(options: HarnessOptions) -> (Self, Settings) {
        env_logger::init();
        let mut task_pool = LocalPool::new();

        info!("Loading the settings");
        let settings = Settings::load("config/settings.ron");
        let extent = wgpu::Extent3d {
            width: settings.window.size[0],
            height: settings.window.size[1],
            depth_or_array_layers: 1,
        };

        info!("Initializing the window");
        let instance = wgpu::Instance::new(settings.backend.to_wgpu());
        let event_loop = EventLoop::new();
        let window = WindowBuilder::new()
            .with_title(options.title)
            .with_inner_size(winit::dpi::PhysicalSize::new(extent.width, extent.height))
            .with_resizable(true)
            .build(&event_loop)
            .unwrap();
        let surface = unsafe { instance.create_surface(&window) };

        info!("Initializing the device");
        let adapter = task_pool
            .run_until(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            }))
            .expect("Unable to initialize GPU via the selected backend.");

        let downlevel_caps = adapter.get_downlevel_capabilities();
        let adapter_limits = adapter.limits();

        let limits = match settings.render.terrain {
            Terrain::RayTraced { .. }
            | Terrain::RayMipTraced { .. }
            | Terrain::Sliced { .. }
            | Terrain::Painted { .. } => wgpu::Limits {
                max_texture_dimension_2d: adapter_limits.max_texture_dimension_2d,
                max_storage_buffers_per_shader_stage: 1,
                max_storage_buffer_binding_size: 1 << 26,
                ..wgpu::Limits::downlevel_webgl2_defaults()
            },
            Terrain::Scattered { .. } => wgpu::Limits::default(),
        };

        let (device, queue) = task_pool
            .run_until(adapter.request_device(
                &wgpu::DeviceDescriptor {
                    label: None,
                    features: wgpu::Features::empty(),
                    limits,
                },
                if settings.render.wgpu_trace_path.is_empty() {
                    None
                } else {
                    Some(std::path::Path::new(&settings.render.wgpu_trace_path))
                },
            ))
            .unwrap();

        let surface_formats = surface.get_supported_formats(&adapter);
        let surface_modes = surface.get_supported_modes(&adapter);
        let present_mode = if surface_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            log::warn!(
                "Mailbox present is not supported, defaulting to {:?}",
                surface_modes[0]
            );
            surface_modes[0]
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_formats[0],
            width: extent.width,
            height: extent.height,
            present_mode,
        };
        surface.configure(&device, &config);

        let egui_platform = egui_wp::Platform::new(egui_wp::PlatformDescriptor {
            physical_width: extent.width,
            physical_height: extent.height,
            scale_factor: window.scale_factor(),
            font_definitions: egui::FontDefinitions::default(),
            style: Default::default(),
        });

        let depth_target = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Depth"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            })
            .create_view(&wgpu::TextureViewDescriptor::default());

        let harness = Harness {
            event_loop,
            window_ctx: WindowContext {
                window,
                task_pool,
                surface,
                present_mode,
                reload_on_focus: settings.window.reload_on_focus,
                egui_platform,
                depth_target,
            },
            graphics_ctx: GraphicsContext {
                device,
                downlevel_caps,
                queue,
                color_format: config.format,
                screen_size: extent,
            },
        };

        (harness, settings)
    }

    pub fn main_loop<A: 'static + Application>(self, mut app: A) {
        use std::time;

        let start_time = time::Instant::now();
        let mut last_time = time::Instant::now();
        let mut needs_reload = false;
        let Harness {
            event_loop,
            window_ctx: mut win,
            graphics_ctx: mut gfx,
        } = self;

        event_loop.run(move |event, _, control_flow| {
            let _ = win.window;
            *control_flow = ControlFlow::Poll;
            win.task_pool.run_until_stalled();

            win.egui_platform.handle_event(&event);

            match event {
                event::Event::WindowEvent {
                    event: event::WindowEvent::Resized(size),
                    ..
                } => {
                    info!("Resizing to {:?}", size);
                    gfx.screen_size = wgpu::Extent3d {
                        width: size.width,
                        height: size.height,
                        depth_or_array_layers: 1,
                    };
                    let config = wgpu::SurfaceConfiguration {
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        format: gfx.color_format,
                        width: size.width,
                        height: size.height,
                        present_mode: win.present_mode,
                    };
                    win.surface.configure(&gfx.device, &config);
                    win.depth_target = gfx
                        .device
                        .create_texture(&wgpu::TextureDescriptor {
                            label: Some("Depth"),
                            size: gfx.screen_size,
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: DEPTH_FORMAT,
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        })
                        .create_view(&wgpu::TextureViewDescriptor::default());
                    app.resize(&gfx.device, gfx.screen_size);
                }
                event::Event::WindowEvent { event, .. } => match event {
                    event::WindowEvent::Focused(false) => {
                        needs_reload = win.reload_on_focus;
                    }
                    event::WindowEvent::Focused(true) if needs_reload => {
                        info!("Reloading shaders");
                        app.reload(&gfx.device);
                        needs_reload = false;
                    }
                    event::WindowEvent::CloseRequested => {
                        *control_flow = ControlFlow::Exit;
                    }
                    event::WindowEvent::KeyboardInput { input, .. } => {
                        if !app.on_key(input) {
                            *control_flow = ControlFlow::Exit;
                        }
                    }
                    event::WindowEvent::MouseWheel { delta, .. } => app.on_mouse_wheel(delta),
                    event::WindowEvent::CursorMoved { position, .. } => {
                        app.on_cursor_move(position.into())
                    }
                    event::WindowEvent::MouseInput { state, button, .. } => {
                        app.on_mouse_button(state, button)
                    }
                    _ => {}
                },
                event::Event::MainEventsCleared => {
                    let duration = time::Instant::now() - last_time;
                    last_time += duration;
                    let delta = duration.as_secs() as f32 + duration.subsec_nanos() as f32 * 1.0e-9;

                    app.update(&gfx.device, &gfx.queue, delta);

                    match win.surface.get_current_texture() {
                        Ok(frame) => {
                            win.egui_platform
                                .update_time(start_time.elapsed().as_secs_f64());
                            win.egui_platform.begin_frame();
                            app.draw_ui(&win.egui_platform.context());
                            let egui_output = win.egui_platform.end_frame(Some(&win.window));
                            let ui_data = UiData {
                                scale_factor: win.window.scale_factor() as f32,
                                textures_delta: egui_output.textures_delta,
                                primitives: win
                                    .egui_platform
                                    .context()
                                    .tessellate(egui_output.shapes),
                            };

                            let view = frame
                                .texture
                                .create_view(&wgpu::TextureViewDescriptor::default());
                            let targets = ScreenTargets {
                                extent: gfx.screen_size,
                                color: &view,
                                depth: &win.depth_target,
                            };
                            app.draw(&gfx.device, &gfx.queue, targets, ui_data);
                            frame.present();
                        }
                        Err(_) => {}
                    };

                    profiling::finish_frame!();
                }
                _ => (),
            }
        });
    }
}
