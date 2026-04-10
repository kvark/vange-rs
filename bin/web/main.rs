//! Web entry point for vange-rs level viewer with test level.
//! Compiled with `cargo build --target wasm32-unknown-unknown --features web --bin web`
//!
//! If the `VANGERS_SERVER_WS` environment variable is set at compile time,
//! the viewer will attempt to connect to that WebSocket address on startup.
//! If the connection fails, it continues as a standalone viewer.

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

/// Compile-time server address for multiplayer. Set via:
///   VANGERS_SERVER_WS=ws://host:port cargo build ...
const SERVER_WS: Option<&str> = option_env!("VANGERS_SERVER_WS");

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
        let objects_palette = [[0xFF; 4]; 0x100];

        let cam = space::Camera {
            loc: glam::vec3(0.0, 0.0, 400.0),
            rot: glam::Quat::IDENTITY,
            scale: glam::vec3(1.0, -1.0, 1.0),
            proj: space::Projection::Perspective(space::PerspectiveParams {
                fovy: 45.0f32.to_radians(),
                aspect: gfx.screen_size.width as f32 / gfx.screen_size.height.max(1) as f32,
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

    fn draw(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, targets: ScreenTargets) -> wgpu::CommandBuffer {
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
            queue,
        );
        encoder.finish()
    }
}

// --- Multiplayer WebSocket client (WASM only) ---

#[cfg(target_arch = "wasm32")]
mod net_ws {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use vangers_net::{decode, encode, ClientMessage, ServerMessage, PlayerId};
    use wasm_bindgen::closure::Closure;

    pub struct WsClient {
        ws: web_sys::WebSocket,
        pub received: Rc<RefCell<Vec<ServerMessage>>>,
        pub connected: Rc<RefCell<bool>>,
        pub player_id: Option<PlayerId>,
        _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
        _on_open: Closure<dyn FnMut(JsValue)>,
        _on_error: Closure<dyn FnMut(web_sys::ErrorEvent)>,
        _on_close: Closure<dyn FnMut(web_sys::CloseEvent)>,
    }

    impl WsClient {
        pub fn connect(url: &str) -> Result<Self, JsValue> {
            log::info!("Connecting to WebSocket server: {}", url);
            let ws = web_sys::WebSocket::new(url)?;
            ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

            let received = Rc::new(RefCell::new(Vec::<ServerMessage>::new()));
            let connected = Rc::new(RefCell::new(false));

            // on_message: decode binary frames
            let recv_clone = received.clone();
            let on_message = Closure::wrap(Box::new(move |e: web_sys::MessageEvent| {
                if let Ok(buf) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                    let array = js_sys::Uint8Array::new(&buf);
                    let data = array.to_vec();
                    // Our protocol is length-prefixed; the server sends framed messages
                    let mut offset = 0;
                    while let Some((msg, consumed)) = decode::<ServerMessage>(&data[offset..]) {
                        recv_clone.borrow_mut().push(msg);
                        offset += consumed;
                    }
                }
            }) as Box<dyn FnMut(web_sys::MessageEvent)>);
            ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

            // on_open: send Join
            let ws_clone = ws.clone();
            let conn_clone = connected.clone();
            let on_open = Closure::wrap(Box::new(move |_: JsValue| {
                log::info!("WebSocket connected");
                *conn_clone.borrow_mut() = true;
                let msg = encode(&ClientMessage::Join {
                    player_name: "WebPlayer".into(),
                    car_name: "TestCar".into(),
                    color: 21,
                });
                let _ = ws_clone.send_with_u8_array(&msg);
            }) as Box<dyn FnMut(JsValue)>);
            ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));

            // on_error
            let on_error = Closure::wrap(Box::new(move |_: web_sys::ErrorEvent| {
                log::warn!("WebSocket error");
            }) as Box<dyn FnMut(web_sys::ErrorEvent)>);
            ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));

            // on_close
            let conn_close = connected.clone();
            let on_close = Closure::wrap(Box::new(move |_: web_sys::CloseEvent| {
                log::info!("WebSocket closed");
                *conn_close.borrow_mut() = false;
            }) as Box<dyn FnMut(web_sys::CloseEvent)>);
            ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));

            Ok(WsClient {
                ws,
                received,
                connected,
                player_id: None,
                _on_message: on_message,
                _on_open: on_open,
                _on_error: on_error,
                _on_close: on_close,
            })
        }

        pub fn send_input(&self, motor: f32, rudder: f32) {
            if !*self.connected.borrow() {
                return;
            }
            let msg = encode(&ClientMessage::Input {
                sequence: 0,
                control: vangers_net::NetControl {
                    motor,
                    rudder,
                    roll: 0.0,
                    brake: false,
                    turbo: false,
                    jump: None,
                },
            });
            let _ = self.ws.send_with_u8_array(&msg);
        }

        pub fn poll(&mut self) -> Vec<ServerMessage> {
            let mut msgs = self.received.borrow_mut();
            let result = msgs.drain(..).collect();
            result
        }

        #[allow(dead_code)]
        pub fn is_connected(&self) -> bool {
            *self.connected.borrow()
        }
    }
}

#[cfg(target_arch = "wasm32")]
use web_time::Instant;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

/// GPU resources initialized asynchronously on WASM.
struct GpuState {
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth_view: wgpu::TextureView,
    app: WebApp,
}

struct WebHandler {
    window: Option<Arc<Window>>,
    gpu: Option<GpuState>,
    /// Shared slot for async WASM GPU init to deliver results.
    #[cfg(target_arch = "wasm32")]
    gpu_pending: std::rc::Rc<std::cell::RefCell<Option<GpuState>>>,
    screen_size: wgpu::Extent3d,
    keys_pressed: std::collections::HashSet<KeyCode>,
    last_frame: Option<Instant>,
    #[cfg(target_arch = "wasm32")]
    ws_client: Option<net_ws::WsClient>,
    /// Status text overlay (used in multiplayer logging)
    #[allow(dead_code)]
    mp_status: String,
}

impl WebHandler {
    fn new() -> Self {
        // Try to connect to multiplayer server if configured
        #[cfg(target_arch = "wasm32")]
        let (ws_client, mp_status) = match SERVER_WS {
            Some(url) if !url.is_empty() => {
                // Auto-upgrade ws:// to wss:// when page is served over HTTPS
                let url = {
                    let is_https = web_sys::window()
                        .and_then(|w| w.location().protocol().ok())
                        .map_or(false, |p| p == "https:");
                    if is_https && url.starts_with("ws://") {
                        let upgraded = format!("wss://{}", &url[5..]);
                        log::info!("HTTPS page: upgrading {} to {}", url, upgraded);
                        upgraded
                    } else {
                        url.to_string()
                    }
                };
                match net_ws::WsClient::connect(&url) {
                    Ok(client) => (Some(client), format!("Connecting to {}...", url)),
                    Err(e) => {
                        log::warn!("Failed to connect to {}: {:?}", url, e);
                        (None, "Standalone mode (connection failed)".into())
                    }
                }
            }
            _ => {
                log::info!("No server configured, running standalone");
                (None, String::new())
            }
        };

        #[cfg(not(target_arch = "wasm32"))]
        let mp_status = String::new();

        WebHandler {
            window: None,
            gpu: None,
            #[cfg(target_arch = "wasm32")]
            gpu_pending: std::rc::Rc::new(std::cell::RefCell::new(None)),
            screen_size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            keys_pressed: std::collections::HashSet::new(),
            last_frame: None,
            #[cfg(target_arch = "wasm32")]
            ws_client,
            mp_status,
        }
    }
}

impl ApplicationHandler for WebHandler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let mut _init_width = 800u32;
        let mut _init_height = 600u32;

        let attrs = Window::default_attributes()
            .with_title("Vangers Web");

        #[cfg(target_arch = "wasm32")]
        let attrs = {
            use winit::platform::web::WindowAttributesExtWebSys;
            let document = web_sys::window().unwrap().document().unwrap();
            let canvas = document
                .get_element_by_id("canvas")
                .expect("missing <canvas id='canvas'>")
                .dyn_into::<web_sys::HtmlCanvasElement>()
                .unwrap();

            // Read the CSS layout size and set canvas resolution to match.
            // Cap at 4096 to stay within WebGPU texture limits.
            let dpr = web_sys::window().unwrap().device_pixel_ratio();
            let max_dim = 4096u32;
            let cw = ((canvas.client_width() as f64 * dpr) as u32).min(max_dim);
            let ch = ((canvas.client_height() as f64 * dpr) as u32).min(max_dim);
            if cw > 0 && ch > 0 {
                canvas.set_width(cw);
                canvas.set_height(ch);
                _init_width = cw;
                _init_height = ch;
                log::info!("Canvas size: {}x{} (dpr={:.1})", cw, ch, dpr);
            }

            attrs.with_canvas(Some(canvas))
        };

        #[cfg(not(target_arch = "wasm32"))]
        let attrs = attrs.with_inner_size(winit::dpi::PhysicalSize::new(_init_width, _init_height));

        self.screen_size = wgpu::Extent3d {
            width: _init_width,
            height: _init_height,
            depth_or_array_layers: 1,
        };

        let window = Arc::new(event_loop.create_window(attrs).unwrap());
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());

        let surface = instance
            .create_surface(window.clone())
            .expect("Unable to create surface");

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

            (instance, surface, adapter, device, queue, window_clone)
        };

        let screen_size = self.screen_size;

        // Build GPU state from the async init results
        let build_gpu_state = move |instance: wgpu::Instance,
                                     surface: wgpu::Surface<'static>,
                                     adapter: &wgpu::Adapter,
                                     device: wgpu::Device,
                                     queue: wgpu::Queue,
                                     screen_size: wgpu::Extent3d|
                                     -> GpuState {
            let caps = surface.get_capabilities(adapter);
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: caps.formats[0],
                width: screen_size.width,
                height: screen_size.height,
                present_mode: wgpu::PresentMode::Fifo,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: Vec::new(),
                desired_maximum_frame_latency: 2,
            };
            surface.configure(&device, &config);

            let depth_view = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("Depth"),
                    size: screen_size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: DEPTH_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default());

            let gfx = GraphicsContext {
                downlevel_caps: adapter.get_downlevel_capabilities(),
                color_format: config.format,
                screen_size,
                device,
                queue,
            };
            let app = WebApp::new(&gfx);

            GpuState {
                _instance: instance,
                surface,
                device: gfx.device,
                queue: gfx.queue,
                config,
                depth_view,
                app,
            }
        };

        #[cfg(target_arch = "wasm32")]
        {
            self.window = Some(window);
            let pending = self.gpu_pending.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let (instance, surface, adapter, device, queue, _window) = init_future.await;
                log::info!("GPU initialized on web!");
                let state = build_gpu_state(instance, surface, &adapter, device, queue, screen_size);
                *pending.borrow_mut() = Some(state);
            });
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let (instance, surface, adapter, device, queue, _) = pollster::block_on(init_future);
            self.gpu = Some(build_gpu_state(instance, surface, &adapter, device, queue, screen_size));
            self.window = Some(window);
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
                if let Some(ref mut gpu) = self.gpu {
                    gpu.config.width = size.width;
                    gpu.config.height = size.height;
                    gpu.surface.configure(&gpu.device, &gpu.config);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Check if async GPU init completed (WASM)
        #[cfg(target_arch = "wasm32")]
        if self.gpu.is_none() {
            if let Some(state) = self.gpu_pending.borrow_mut().take() {
                self.gpu = Some(state);
            } else {
                return; // Still waiting for GPU init
            }
        }

        let Some(ref mut gpu) = self.gpu else {
            return;
        };

        // Compute delta time
        let now = Instant::now();
        let dt = match self.last_frame {
            Some(prev) => (now - prev).as_secs_f32(),
            None => 1.0 / 60.0,
        };
        self.last_frame = Some(now);
        let dt = dt.min(0.1);

        // Camera movement from keyboard
        let move_speed = 100.0;
        let rotation_speed = 1.0;

        let mut _motor = 0.0f32;
        let mut _rudder = 0.0f32;

        if self.keys_pressed.contains(&KeyCode::KeyW) {
            let mut dir = gpu.app.cam.rot * glam::Vec3::Y;
            dir.z = 0.0;
            if dir.length_squared() > 0.0 {
                gpu.app.cam.loc += move_speed * dt * dir.normalize();
            }
            _motor = 1.0;
        }
        if self.keys_pressed.contains(&KeyCode::KeyS) {
            let mut dir = gpu.app.cam.rot * glam::Vec3::Y;
            dir.z = 0.0;
            if dir.length_squared() > 0.0 {
                gpu.app.cam.loc -= move_speed * dt * dir.normalize();
            }
            _motor = -1.0;
        }
        if self.keys_pressed.contains(&KeyCode::KeyA) {
            let mut dir = gpu.app.cam.rot * glam::Vec3::X;
            dir.z = 0.0;
            if dir.length_squared() > 0.0 {
                gpu.app.cam.loc -= move_speed * dt * dir.normalize();
            }
            _rudder = 1.0;
        }
        if self.keys_pressed.contains(&KeyCode::KeyD) {
            let mut dir = gpu.app.cam.rot * glam::Vec3::X;
            dir.z = 0.0;
            if dir.length_squared() > 0.0 {
                gpu.app.cam.loc += move_speed * dt * dir.normalize();
            }
            _rudder = -1.0;
        }
        if self.keys_pressed.contains(&KeyCode::KeyZ) {
            gpu.app.cam.loc.z += move_speed * dt;
        }
        if self.keys_pressed.contains(&KeyCode::KeyX) {
            gpu.app.cam.loc.z -= move_speed * dt;
        }
        if self.keys_pressed.contains(&KeyCode::KeyQ) {
            let rotation = glam::Quat::from_rotation_z(rotation_speed * dt);
            gpu.app.cam.rot = rotation * gpu.app.cam.rot;
        }
        if self.keys_pressed.contains(&KeyCode::KeyE) {
            let rotation = glam::Quat::from_rotation_z(-rotation_speed * dt);
            gpu.app.cam.rot = rotation * gpu.app.cam.rot;
        }

        // Process multiplayer messages
        #[cfg(target_arch = "wasm32")]
        if let Some(ref mut ws) = self.ws_client {
            // Send input
            if _motor != 0.0 || _rudder != 0.0 {
                ws.send_input(_motor, _rudder);
            }

            // Process received messages
            for msg in ws.poll() {
                match msg {
                    vangers_net::ServerMessage::Welcome { player_id, level_name, .. } => {
                        self.mp_status = format!("Connected (player {}, level '{}')", player_id, level_name);
                        ws.player_id = Some(player_id);
                        log::info!("{}", self.mp_status);
                    }
                    vangers_net::ServerMessage::PlayerJoined { player_id, player_name, .. } => {
                        log::info!("Player {} ({}) joined", player_id, player_name);
                    }
                    vangers_net::ServerMessage::PlayerLeft { player_id } => {
                        log::info!("Player {} left", player_id);
                    }
                    vangers_net::ServerMessage::WorldState { agents, .. } => {
                        // Move camera to follow our agent if we have one
                        if let Some(my_id) = ws.player_id {
                            if let Some(me) = agents.iter().find(|a| a.player_id == my_id) {
                                let pos = glam::Vec3::from(me.transform.position);
                                // Smoothly follow server position
                                gpu.app.cam.loc = gpu.app.cam.loc.lerp(
                                    glam::vec3(pos.x, pos.y, pos.z + 200.0),
                                    0.1,
                                );
                            }
                        }
                    }
                }
            }
        }

        let frame = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            _ => return,
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let targets = ScreenTargets {
            extent: self.screen_size,
            color: &view,
            depth: &gpu.depth_view,
        };
        let command_buffer = gpu.app.draw(&gpu.device, &gpu.queue, targets);
        gpu.queue.submit(std::iter::once(command_buffer));
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
    if let Some(url) = SERVER_WS {
        log::info!("Multiplayer server: {}", url);
    } else {
        log::info!("Standalone mode (no VANGERS_SERVER_WS set)");
    }

    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut handler = WebHandler::new();
    event_loop.run_app(&mut handler).unwrap();
}

fn main() {
    web_main();
}
