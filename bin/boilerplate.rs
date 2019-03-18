use vangers::{
    config,
    render::{ScreenTargets, DEPTH_FORMAT},
};

use env_logger;
use log::info;
use wgpu;
use wgpu::winit;

const SWAP_CHAIN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

pub trait Application {
    fn on_key(&mut self, input: winit::KeyboardInput) -> bool;
    fn on_mouse_wheel(&mut self, _delta: winit::MouseScrollDelta) {}
    fn on_cursor_move(&mut self, _position: (f64, f64)) {}
    fn on_mouse_button(&mut self, _state: winit::ElementState, _button: winit::MouseButton) {}
    fn resize(&mut self, _device: &wgpu::Device, _extent: wgpu::Extent3d) {}
    fn reload(&mut self, _device: &wgpu::Device);
    fn update(&mut self, delta: f32);
    fn draw(
        &mut self,
        device: &wgpu::Device,
        targets: ScreenTargets,
    ) -> Vec<wgpu::CommandBuffer>;
}

pub struct Harness {
    events_loop: winit::EventsLoop,
    window: winit::Window,
    pub device: wgpu::Device,
    surface: wgpu::Surface,
    swap_chain: wgpu::SwapChain,
    extent: wgpu::Extent3d,
    depth_target: wgpu::TextureView,
}

impl Harness {
    pub fn init(title: &str) -> (Self, config::Settings) {
        info!("Initializing the device");
        env_logger::init();

        let instance = wgpu::Instance::new();
        let adapter = instance.get_adapter(&wgpu::AdapterDescriptor {
            power_preference: wgpu::PowerPreference::LowPower,
        });
        let device = adapter.create_device(&wgpu::DeviceDescriptor {
            extensions: wgpu::Extensions {
                anisotropic_filtering: false,
            },
        });

        info!("Loading the settings");
        let settings = config::Settings::load("config/settings.ron");
        let extent = wgpu::Extent3d {
            width: settings.window.size[0],
            height: settings.window.size[1],
            depth: 1,
        };

        info!("Initializing the window...");
        let events_loop = winit::EventsLoop::new();
        let dpi = events_loop
            .get_primary_monitor()
            .get_hidpi_factor();
        let window = winit::WindowBuilder::new()
            .with_title(title)
            .with_dimensions(
                winit::dpi::LogicalSize::from_physical((extent.width, extent.height), dpi),
            )
            .build(&events_loop)
            .unwrap();

        let surface = instance.create_surface(&window);
        let sc_desc = wgpu::SwapChainDescriptor {
            usage: wgpu::TextureUsageFlags::OUTPUT_ATTACHMENT,
            format: SWAP_CHAIN_FORMAT,
            width: extent.width,
            height: extent.height,
        };
        let swap_chain = device.create_swap_chain(&surface, &sc_desc);
        let depth_target = device
            .create_texture(&wgpu::TextureDescriptor {
                size: extent,
                array_size: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsageFlags::OUTPUT_ATTACHMENT,
            })
            .create_default_view();

        let harness = Harness {
            events_loop,
            window,
            device,
            surface,
            swap_chain,
            extent,
            depth_target,
        };

        (harness, settings)
    }

    pub fn main_loop<A: Application>(&mut self, mut app: A) {
        use std::time;
        use wgpu::winit::{Event, WindowEvent};

        let mut last_time = time::Instant::now();
        loop {
            let win = &self.window;
            let mut running = true;
            let mut resized_extent = None;
            let mut reload_shaders = false;
            self.events_loop.poll_events(|event| match event {
                Event::WindowEvent { window_id, ref event } if window_id == win.id() => {
                    match *event {
                        WindowEvent::Resized(size) => {
                            let physical = size.to_physical(win.get_hidpi_factor());
                            info!("Resizing to {:?}", physical);
                            resized_extent = Some(wgpu::Extent3d {
                                width: physical.width as u32,
                                height: physical.height as u32,
                                depth: 1,
                            });
                        }
                        WindowEvent::Focused(true) => {
                            info!("Reloading shaders");
                            reload_shaders = true;
                        }
                        WindowEvent::CloseRequested => {
                            running = false;
                        }
                        WindowEvent::KeyboardInput { input, .. } => {
                            if !app.on_key(input) {
                                running = false;
                            }
                        }
                        WindowEvent::MouseWheel {delta, ..} => {
                            app.on_mouse_wheel(delta)
                        }
                        WindowEvent::CursorMoved {position, ..} => {
                            let physical = position.to_physical(win.get_hidpi_factor());
                            app.on_cursor_move(physical.into())
                        }
                        WindowEvent::MouseInput {state, button, ..} => {
                            app.on_mouse_button(state, button)
                        }
                        _ => {}
                    }
                }
                _ => {}
            });

            if !running {
                break;
            }
            if let Some(extent) = resized_extent {
                self.extent = extent;
                let sc_desc = wgpu::SwapChainDescriptor {
                    usage: wgpu::TextureUsageFlags::OUTPUT_ATTACHMENT,
                    format: SWAP_CHAIN_FORMAT,
                    width: extent.width,
                    height: extent.height,
                };
                self.swap_chain = self.device.create_swap_chain(&self.surface, &sc_desc);
                self.depth_target = self.device
                    .create_texture(&wgpu::TextureDescriptor {
                        size: extent,
                        array_size: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: DEPTH_FORMAT,
                        usage: wgpu::TextureUsageFlags::OUTPUT_ATTACHMENT,
                    })
                    .create_default_view();
                app.resize(&self.device, extent);
            }
            if reload_shaders {
                app.reload(&self.device);
            }

            let duration = time::Instant::now() - last_time;
            last_time += duration;
            let delta = duration.as_secs() as f32 +
                duration.subsec_nanos() as f32 * 1.0e-9;

            app.update(delta);
            {
                let frame = self.swap_chain.get_next_texture();
                let targets = ScreenTargets {
                    extent: self.extent,
                    color: &frame.view,
                    depth: &self.depth_target,
                };
                let command_buffers = app.draw(&self.device, targets);
                self.device
                    .get_queue()
                    .submit(&command_buffers);
            }
        }
    }
}
