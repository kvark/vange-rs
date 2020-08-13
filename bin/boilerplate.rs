use vangers::{
    config,
    render::{ScreenTargets, COLOR_FORMAT, DEPTH_FORMAT},
};

use env_logger;
use futures::executor::{LocalPool, LocalSpawner};
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
    fn update(
        &mut self,
        device: &wgpu::Device,
        delta: f32,
        spawner: &LocalSpawner,
    ) -> Vec<wgpu::CommandBuffer>;
    fn draw(
        &mut self,
        device: &wgpu::Device,
        targets: ScreenTargets,
        spawner: &LocalSpawner,
    ) -> wgpu::CommandBuffer;
}

pub struct Harness {
    task_pool: LocalPool,
    event_loop: EventLoop<()>,
    window: Window,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    surface: wgpu::Surface,
    swap_chain: wgpu::SwapChain,
    pub extent: wgpu::Extent3d,
    reload_on_focus: bool,
    depth_target: wgpu::TextureView,
}

impl Harness {
    pub fn init(title: &str) -> (Self, config::Settings) {
        env_logger::init();
        let mut task_pool = LocalPool::new();

        info!("Loading the settings");
        let settings = config::Settings::load("config/settings.ron");
        let extent = wgpu::Extent3d {
            width: settings.window.size[0],
            height: settings.window.size[1],
            depth: 1,
        };

        info!("Initializing the window");
        let instance = wgpu::Instance::new(settings.backend.to_wgpu());
        let event_loop = EventLoop::new();
        let window = WindowBuilder::new()
            .with_title(title)
            .with_inner_size(winit::dpi::PhysicalSize::new(extent.width, extent.height))
            .with_resizable(true)
            .build(&event_loop)
            .unwrap();
        let surface = unsafe { instance.create_surface(&window) };

        info!("Initializing the device");
        let adapter = task_pool
            .run_until(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::Default,
                compatible_surface: Some(&surface),
            }))
            .expect("Unable to initialize GPU via the selected backend.");
        let (device, queue) = task_pool
            .run_until(adapter.request_device(
                &wgpu::DeviceDescriptor {
                    features: wgpu::Features::empty(),
                    limits: wgpu::Limits::default(),
                    shader_validation: true,
                },
                None,
            ))
            .unwrap();

        let sc_desc = wgpu::SwapChainDescriptor {
            usage: wgpu::TextureUsage::OUTPUT_ATTACHMENT,
            format: COLOR_FORMAT,
            width: extent.width,
            height: extent.height,
            present_mode: wgpu::PresentMode::Mailbox,
        };
        let swap_chain = device.create_swap_chain(&surface, &sc_desc);
        let depth_target = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Depth"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsage::OUTPUT_ATTACHMENT,
            })
            .create_view(&wgpu::TextureViewDescriptor::default());

        let harness = Harness {
            task_pool,
            event_loop,
            window: window,
            device,
            queue,
            surface,
            swap_chain,
            extent,
            reload_on_focus: settings.window.reload_on_focus,
            depth_target,
        };

        (harness, settings)
    }

    pub fn main_loop<A: 'static + Application>(self, mut app: A) {
        use std::time;

        let mut last_time = time::Instant::now();
        let mut needs_reload = false;
        let Harness {
            mut task_pool,
            event_loop,
            window,
            device,
            queue,
            surface,
            mut swap_chain,
            mut extent,
            reload_on_focus,
            mut depth_target,
        } = self;

        event_loop.run(move |event, _, control_flow| {
            let _ = window;
            *control_flow = ControlFlow::Poll;
            task_pool.run_until_stalled();

            match event {
                event::Event::WindowEvent {
                    event: event::WindowEvent::Resized(size),
                    ..
                } => {
                    info!("Resizing to {:?}", size);
                    extent = wgpu::Extent3d {
                        width: size.width,
                        height: size.height,
                        depth: 1,
                    };
                    let sc_desc = wgpu::SwapChainDescriptor {
                        usage: wgpu::TextureUsage::OUTPUT_ATTACHMENT,
                        format: COLOR_FORMAT,
                        width: size.width,
                        height: size.height,
                        present_mode: wgpu::PresentMode::Mailbox,
                    };
                    swap_chain = device.create_swap_chain(&surface, &sc_desc);
                    depth_target = device
                        .create_texture(&wgpu::TextureDescriptor {
                            label: Some("Depth"),
                            size: extent,
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: DEPTH_FORMAT,
                            usage: wgpu::TextureUsage::OUTPUT_ATTACHMENT,
                        })
                        .create_view(&wgpu::TextureViewDescriptor::default());
                    app.resize(&device, extent);
                }
                event::Event::WindowEvent { event, .. } => match event {
                    event::WindowEvent::Focused(false) => {
                        needs_reload = reload_on_focus;
                    }
                    event::WindowEvent::Focused(true) if needs_reload => {
                        info!("Reloading shaders");
                        app.reload(&device);
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
                    let spawner = task_pool.spawner();
                    let duration = time::Instant::now() - last_time;
                    last_time += duration;
                    let delta = duration.as_secs() as f32 + duration.subsec_nanos() as f32 * 1.0e-9;

                    let update_command_buffers = app.update(&device, delta, &spawner);
                    if !update_command_buffers.is_empty() {
                        queue.submit(update_command_buffers);
                    }

                    match swap_chain.get_current_frame() {
                        Ok(frame) => {
                            let targets = ScreenTargets {
                                extent,
                                color: &frame.output.view,
                                depth: &depth_target,
                            };
                            let render_commane_buffer = app.draw(&device, targets, &spawner);
                            queue.submit(Some(render_commane_buffer));
                        }
                        Err(_) => {}
                    };
                }
                _ => (),
            }
        });
    }
}
