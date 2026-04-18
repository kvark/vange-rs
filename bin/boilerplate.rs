#![allow(clippy::single_match)]
use vangers::{
    config::Settings,
    render::{DEPTH_FORMAT, GraphicsContext, ScreenTargets},
};

use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::{self, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::Window,
};

pub trait Application {
    fn on_key(&mut self, key: KeyCode, state: event::ElementState) -> bool;
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
    ) -> wgpu::CommandBuffer;
}

struct WindowContext {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    present_mode: wgpu::PresentMode,
    reload_on_focus: bool,
    egui_state: egui_winit::State,
    depth_target: wgpu::TextureView,
}

pub struct Harness {
    event_loop: EventLoop<()>,
    window_ctx: WindowContext,
    egui_renderer: egui_wgpu::Renderer,
    pub graphics_ctx: GraphicsContext,
}

pub struct HarnessOptions {
    pub title: &'static str,
}

impl Harness {
    pub fn init(options: HarnessOptions) -> (Self, Settings) {
        env_logger::init();

        log::info!("Loading the settings");
        let settings = Settings::load("config/settings.ron");
        let extent = wgpu::Extent3d {
            width: settings.window.size[0],
            height: settings.window.size[1],
            depth_or_array_layers: 1,
        };

        log::info!("Initializing the window");
        let mut instance_desc = wgpu::InstanceDescriptor::new_without_display_handle();
        instance_desc.backends = settings.backend.to_wgpu();
        let instance = wgpu::Instance::new(instance_desc);
        let event_loop = EventLoop::new().unwrap();
        let window = Arc::new({
            let attrs = Window::default_attributes()
                .with_title(options.title)
                .with_inner_size(winit::dpi::PhysicalSize::new(extent.width, extent.height))
                .with_resizable(true);
            #[allow(deprecated)]
            event_loop.create_window(attrs).unwrap()
        });

        let surface = instance
            .create_surface(window.clone())
            .expect("Unable to create surface.");

        log::info!("Initializing the device");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("Unable to initialize GPU via the selected backend.");

        let downlevel_caps = adapter.get_downlevel_capabilities();
        let limits = settings
            .render
            .get_device_limits(&adapter.limits(), settings.game.geometry.height);

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: None,
            required_features: wgpu::Features::empty(),
            required_limits: limits,
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
            experimental_features: Default::default(),
        }))
        .unwrap();

        let surface_caps = surface.get_capabilities(&adapter);
        log::info!("Supported surface formats: {:?}", surface_caps.formats);
        let present_mode = if surface_caps
            .present_modes
            .contains(&wgpu::PresentMode::Mailbox)
        {
            wgpu::PresentMode::Mailbox
        } else {
            log::warn!("Mailbox present is not supported");
            if settings.render.allow_tearing {
                wgpu::PresentMode::Immediate
            } else {
                wgpu::PresentMode::Fifo
            }
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_caps.formats[0],
            width: extent.width,
            height: extent.height,
            present_mode,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: Vec::new(),
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let egui_ctx = egui::Context::default();
        let egui_state =
            egui_winit::State::new(egui_ctx, egui::ViewportId::ROOT, &window, None, None, None);
        let egui_renderer = egui_wgpu::Renderer::new(
            &device,
            config.format,
            egui_wgpu::RendererOptions {
                depth_stencil_format: None,
                ..Default::default()
            },
        );

        let depth_target = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Depth"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());

        let harness = Harness {
            event_loop,
            egui_renderer,
            window_ctx: WindowContext {
                window,
                surface,
                present_mode,
                reload_on_focus: settings.window.reload_on_focus,
                egui_state,
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

    pub fn main_loop<A: 'static + Application>(self, app: A) {
        let Harness {
            event_loop,
            window_ctx: win,
            egui_renderer,
            graphics_ctx: gfx,
        } = self;

        let mut handler = HarnessHandler {
            win,
            gfx,
            egui_renderer,
            app,
            last_time: std::time::Instant::now(),
            needs_reload: false,
        };

        event_loop.set_control_flow(ControlFlow::Poll);
        event_loop.run_app(&mut handler).unwrap();
    }
}

struct HarnessHandler<A> {
    win: WindowContext,
    gfx: GraphicsContext,
    egui_renderer: egui_wgpu::Renderer,
    app: A,
    last_time: std::time::Instant,
    needs_reload: bool,
}

impl<A: Application> ApplicationHandler for HarnessHandler<A> {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let response = self
            .win
            .egui_state
            .on_window_event(&self.win.window, &event);
        if response.consumed {
            return;
        }

        match event {
            WindowEvent::Resized(size) if size.width != u32::MAX => {
                log::info!("Resizing to {:?}", size);
                self.gfx.screen_size = wgpu::Extent3d {
                    width: size.width,
                    height: size.height,
                    depth_or_array_layers: 1,
                };
                let config = wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format: self.gfx.color_format,
                    width: size.width,
                    height: size.height,
                    present_mode: self.win.present_mode,
                    alpha_mode: wgpu::CompositeAlphaMode::Auto,
                    view_formats: Vec::new(),
                    desired_maximum_frame_latency: 2,
                };
                self.win.surface.configure(&self.gfx.device, &config);
                self.win.depth_target = self
                    .gfx
                    .device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("Depth"),
                        size: self.gfx.screen_size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: DEPTH_FORMAT,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    })
                    .create_view(&wgpu::TextureViewDescriptor::default());
                self.app.resize(&self.gfx.device, self.gfx.screen_size);
            }
            WindowEvent::Resized(size) => {
                log::warn!("Ignoring invalid resize request: {:?}", size)
            }
            WindowEvent::Focused(false) => {
                self.needs_reload = self.win.reload_on_focus;
            }
            WindowEvent::Focused(true) if self.needs_reload => {
                log::info!("Reloading shaders");
                self.app.reload(&self.gfx.device);
                self.needs_reload = false;
            }
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::KeyboardInput {
                event:
                    event::KeyEvent {
                        physical_key: PhysicalKey::Code(key),
                        state,
                        ..
                    },
                ..
            } => {
                if !self.app.on_key(key, state) {
                    event_loop.exit();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => self.app.on_mouse_wheel(delta),
            WindowEvent::CursorMoved { position, .. } => self.app.on_cursor_move(position.into()),
            WindowEvent::MouseInput { state, button, .. } => {
                self.app.on_mouse_button(state, button)
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        let duration = std::time::Instant::now() - self.last_time;
        self.last_time += duration;

        self.app
            .update(&self.gfx.device, &self.gfx.queue, duration.as_secs_f32());

        let frame = match self.win.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            _ => return,
        };

        let raw_input = self.win.egui_state.take_egui_input(&self.win.window);
        let full_output = self.win.egui_state.egui_ctx().run_ui(raw_input, |ctx| {
            self.app.draw_ui(ctx);
        });
        self.win
            .egui_state
            .handle_platform_output(&self.win.window, full_output.platform_output);

        let paint_jobs = self
            .win
            .egui_state
            .egui_ctx()
            .tessellate(full_output.shapes, full_output.pixels_per_point);
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.gfx.screen_size.width, self.gfx.screen_size.height],
            pixels_per_point: self.win.window.scale_factor() as f32,
        };

        for (id, delta) in &full_output.textures_delta.set {
            self.egui_renderer
                .update_texture(&self.gfx.device, &self.gfx.queue, *id, delta);
        }
        self.egui_renderer.update_buffers(
            &self.gfx.device,
            &self.gfx.queue,
            &mut self
                .gfx
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("egui upload"),
                }),
            &paint_jobs,
            &screen_descriptor,
        );

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let targets = ScreenTargets {
            extent: self.gfx.screen_size,
            color: &view,
            depth: &self.win.depth_target,
        };
        let command_buffer = self.app.draw(&self.gfx.device, &self.gfx.queue, targets);

        //Note: we can't run this in the main render pass since it has
        // a depth texture, and `egui` doesn't expect that.
        let mut egui_encoder = self
            .gfx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("UI") });
        {
            let mut pass = egui_encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    ..Default::default()
                })
                .forget_lifetime();
            self.egui_renderer
                .render(&mut pass, &paint_jobs, &screen_descriptor);
        }

        self.gfx
            .queue
            .submit(vec![command_buffer, egui_encoder.finish()]);
        for &id in &full_output.textures_delta.free {
            self.egui_renderer.free_texture(&id);
        }

        frame.present();
        profiling::finish_frame!();
    }
}
