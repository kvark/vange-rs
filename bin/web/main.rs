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
    vfs::Vfs,
};

#[cfg(target_arch = "wasm32")]
use vangers::data;

/// Default level to try loading from the release if neither the URL
/// hash nor the level picker have selected one. If the release asset
/// is missing (404) we fall back to the procedural test level.
const DEFAULT_LEVEL: &str = "fostral";

/// INI path inside the per-level zip. Each `<id>.zip` stores the level
/// files at the archive root (no `<id>/` prefix), so the INI key is
/// just `"world.ini"`.
fn level_ini_path(_level_id: &str) -> String {
    "world.ini".to_string()
}

/// JS bridge for loading-screen UI. The HTML defines these on `window`;
/// they update a progress bar and status text. All four are no-ops on
/// pages that don't define them (we wrap calls in a catch_unwind-style
/// closure-or-noop pattern via a wasm_bindgen `catch` attribute).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = vangeProgress, catch)]
    fn js_progress(label: &str, loaded: f64, total: f64) -> Result<(), JsValue>;

    #[wasm_bindgen(js_namespace = window, js_name = vangeProgressDone, catch)]
    fn js_progress_done() -> Result<(), JsValue>;

    #[wasm_bindgen(js_namespace = window, js_name = vangeProgressError, catch)]
    fn js_progress_error(message: &str) -> Result<(), JsValue>;

    #[wasm_bindgen(js_namespace = window, js_name = vangeSelectedLevel, catch)]
    fn js_selected_level() -> Result<JsValue, JsValue>;
}

/// Read the selected level id from JS (set by the level picker UI),
/// falling back to the URL fragment `#level=<id>`, then to
/// [`DEFAULT_LEVEL`].
#[cfg(target_arch = "wasm32")]
fn selected_level_id() -> String {
    if let Ok(val) = js_selected_level() {
        if let Some(s) = val.as_string() {
            if !s.is_empty() {
                return s;
            }
        }
    }
    if let Some(window) = web_sys::window() {
        if let Ok(hash) = window.location().hash() {
            for pair in hash.trim_start_matches('#').split('&') {
                if let Some(rest) = pair.strip_prefix("level=") {
                    if !rest.is_empty() {
                        return rest.to_string();
                    }
                }
            }
        }
    }
    DEFAULT_LEVEL.to_string()
}

/// Fetch `common.zip` and `<level_id>.zip` from the release and mount
/// both into a VFS, reporting download progress to the JS UI. Returns
/// `None` on any error; the caller falls back to a procedural test level.
#[cfg(target_arch = "wasm32")]
async fn fetch_release_level(level_id: &str) -> Option<(Vfs, String)> {
    let mut vfs = Vfs::new();

    let mut report = |label: &str, loaded: u64, total: Option<u64>| {
        let total_f = total.map_or(-1.0, |v| v as f64);
        let _ = js_progress(label, loaded as f64, total_f);
    };

    // common.zip holds cross-level assets. A level-only run is fine
    // without it, so we log+continue on failure.
    if let Err(e) = data::fetch_and_mount(&mut vfs, data::COMMON_ARCHIVE, &mut report).await {
        log::warn!("Couldn't fetch {}: {}", data::COMMON_ARCHIVE, e);
    }

    let archive = data::level_archive_name(level_id);
    if let Err(e) = data::fetch_and_mount(&mut vfs, &archive, &mut report).await {
        log::warn!(
            "Couldn't fetch {}: {}. Falling back to test level.",
            archive,
            e
        );
        let _ = js_progress_error(&format!("{}: {}", archive, e));
        return None;
    }

    let ini_path = level_ini_path(level_id);
    if !vfs.contains(&ini_path) {
        log::warn!(
            "{} did not contain {}. Falling back to test level.",
            archive,
            ini_path
        );
        let _ = js_progress_error(&format!("{} missing {}", archive, ini_path));
        return None;
    }

    log::info!(
        "Loaded release level '{}' from VFS ({} entries)",
        level_id,
        vfs.len()
    );
    Some((vfs, ini_path))
}

use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::{self, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
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
    /// Build the app with a procedural test level. Used as a fallback
    /// when the release data can't be fetched (404, offline, etc.).
    fn new(gfx: &GraphicsContext) -> Self {
        let level_config = level::LevelConfig::new_test();
        let geometry = settings::Geometry::default();
        let level = level::load(&level_config, &geometry);
        Self::build(gfx, level_config, level, geometry)
    }

    /// Build the app from a real level in a [`Vfs`]. `ini_path` is the
    /// VFS key of the world INI (e.g. `"fostral/world.ini"`).
    fn new_from_vfs(gfx: &GraphicsContext, vfs: &Vfs, ini_path: &str) -> Self {
        let level_config = level::LevelConfig::load_from_vfs(vfs, ini_path);
        let geometry = settings::Geometry::default();
        let level = level::load_from_vfs(vfs, &level_config, &geometry);
        Self::build(gfx, level_config, level, geometry)
    }

    fn build(
        gfx: &GraphicsContext,
        level_config: level::LevelConfig,
        level: level::Level,
        geometry: settings::Geometry,
    ) -> Self {
        let objects_palette = [[0xFF; 4]; 0x100];

        let cam = space::Camera {
            loc: glam::vec3(128.0, 128.0, 400.0),
            rot: glam::Quat::IDENTITY,
            scale: glam::vec3(1.0, -1.0, 1.0),
            proj: space::Projection::Perspective(space::PerspectiveParams {
                fovy: 45.0f32.to_radians(),
                aspect: gfx.screen_size.width as f32 / gfx.screen_size.height.max(1) as f32,
                near: 10.0,
                far: 2000.0,
            }),
        };

        // RayTraced terrain does a per-pixel height lookup in the fragment
        // shader. It doesn't need compute shaders or vertex storage, so it
        // works on WebGL2 and produces proper 3D relief with lighting.
        let render_settings = settings::Render {
            terrain: settings::Terrain::RayTraced,
            ..settings::Render::default()
        };
        let render = Render::new(
            gfx,
            &level_config,
            &objects_palette,
            &render_settings,
            &geometry,
            cam.front_face(),
        );

        WebApp {
            render,
            level,
            cam,
            batcher: Batcher::new(),
        }
    }

    fn resize(&mut self, extent: wgpu::Extent3d, device: &wgpu::Device) {
        self.render.resize(extent, device);
        if let space::Projection::Perspective(ref mut p) = self.cam.proj {
            p.aspect = extent.width as f32 / extent.height.max(1) as f32;
        }
    }

    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        targets: ScreenTargets,
    ) -> wgpu::CommandBuffer {
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
    use vangers_net::{decode, encode, ClientMessage, PlayerId, ServerMessage};
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

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

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
        // On native, use ControlFlow::Poll for continuous rendering.
        // On web, rendering is driven by requestAnimationFrame via request_redraw().
        #[cfg(not(target_arch = "wasm32"))]
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
        if self.window.is_some() {
            return;
        }

        let mut _init_width = 800u32;
        let mut _init_height = 600u32;

        let attrs = Window::default_attributes().with_title("Vangers Web");

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

        #[cfg(not(target_arch = "wasm32"))]
        let init_future = {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let surface = instance
                .create_surface(window.clone())
                .expect("Unable to create surface");
            let window_clone = window.clone();
            async move {
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
            }
        };

        #[cfg(target_arch = "wasm32")]
        let init_future = {
            let window_clone = window.clone();
            async move {
                // Use WebGL2 backend on WASM. WebGPU's GPUCanvasContext
                // fails dyn_into type checks on Firefox (wgpu 29 bug).
                // RayTraced terrain doesn't need compute shaders, so
                // WebGL2 is sufficient and more widely compatible.
                let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                    backends: wgpu::Backends::GL,
                    ..wgpu::InstanceDescriptor::new_without_display_handle()
                });

                let surface = instance
                    .create_surface(window_clone.clone())
                    .expect("Unable to create surface");

                let adapter = instance
                    .request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::HighPerformance,
                        compatible_surface: Some(&surface),
                        force_fallback_adapter: false,
                    })
                    .await
                    .expect("No GPU adapter found");

                let adapter_limits = adapter.limits();
                let (device, queue) = adapter
                    .request_device(&wgpu::DeviceDescriptor {
                        label: None,
                        required_features: wgpu::Features::empty(),
                        required_limits: wgpu::Limits {
                            max_texture_dimension_2d: adapter_limits.max_texture_dimension_2d,
                            ..wgpu::Limits::downlevel_webgl2_defaults()
                        },
                        memory_hints: wgpu::MemoryHints::default(),
                        trace: wgpu::Trace::Off,
                        experimental_features: Default::default(),
                    })
                    .await
                    .expect("Failed to create device");

                (instance, surface, adapter, device, queue, window_clone)
            }
        };

        let screen_size = self.screen_size;

        // Build GPU state from the async init results. `vfs_level` is
        // `Some((vfs, ini_path))` when real level data has been fetched;
        // `None` falls back to the procedural test level.
        let build_gpu_state = move |instance: wgpu::Instance,
                                    surface: wgpu::Surface<'static>,
                                    adapter: &wgpu::Adapter,
                                    device: wgpu::Device,
                                    queue: wgpu::Queue,
                                    screen_size: wgpu::Extent3d,
                                    vfs_level: Option<(Vfs, String)>|
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
            let app = match vfs_level {
                Some((vfs, ini_path)) => WebApp::new_from_vfs(&gfx, &vfs, &ini_path),
                None => WebApp::new(&gfx),
            };

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
                // Start GPU init. Data fetch is sequential for now (both
                // are I/O-bound but `join` would need the futures crate).
                let (instance, surface, adapter, device, queue, window) = init_future.await;
                log::info!("GPU initialized on web!");

                // Best-effort fetch of release data. On any failure we
                // fall back to the procedural test level.
                let level_id = selected_level_id();
                log::info!("Selected level: {}", level_id);
                let vfs_level = fetch_release_level(&level_id).await;
                let _ = js_progress_done();

                let state = build_gpu_state(
                    instance,
                    surface,
                    &adapter,
                    device,
                    queue,
                    screen_size,
                    vfs_level,
                );
                *pending.borrow_mut() = Some(state);
                // Wake the event loop — without this, ControlFlow::Wait
                // keeps the loop sleeping and gpu_pending is never picked up.
                window.request_redraw();
            });
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let (instance, surface, adapter, device, queue, _) = pollster::block_on(init_future);
            self.gpu = Some(build_gpu_state(
                instance,
                surface,
                &adapter,
                device,
                queue,
                screen_size,
                None,
            ));
            self.window = Some(window.clone());
            window.request_redraw();
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
                    gpu.depth_view = gpu
                        .device
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
                    gpu.app.resize(self.screen_size, &gpu.device);
                }
            }
            WindowEvent::RedrawRequested => {
                // Check if async GPU init completed (WASM)
                #[cfg(target_arch = "wasm32")]
                if self.gpu.is_none() {
                    if let Some(state) = self.gpu_pending.borrow_mut().take() {
                        self.gpu = Some(state);
                    }
                }

                self.render();

                // Schedule the next frame. On web this calls
                // requestAnimationFrame; on native with ControlFlow::Poll
                // this is redundant but harmless.
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // On native, ControlFlow::Poll drives this continuously.
        // Trigger a redraw each iteration so RedrawRequested fires.
        if let Some(ref window) = self.window {
            window.request_redraw();
        }
    }
}

impl WebHandler {
    fn render(&mut self) {
        let Some(ref mut gpu) = self.gpu else {
            return;
        };

        // If the screen was resized while GPU was initializing asynchronously,
        // the depth texture and surface config still have the old size.
        // Reconfigure now before the first render.
        if gpu.config.width != self.screen_size.width
            || gpu.config.height != self.screen_size.height
        {
            gpu.config.width = self.screen_size.width;
            gpu.config.height = self.screen_size.height;
            gpu.surface.configure(&gpu.device, &gpu.config);
            gpu.depth_view = gpu
                .device
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
            gpu.app.resize(self.screen_size, &gpu.device);
        }

        // Compute delta time
        let now = Instant::now();
        let dt = match self.last_frame {
            Some(prev) => (now - prev).as_secs_f32(),
            None => 1.0 / 60.0,
        };
        self.last_frame = Some(now);
        let dt = dt.min(0.1);
        let mut _motor = 0.0f32;
        let mut _rudder = 0.0f32;

        if self.keys_pressed.contains(&KeyCode::KeyW) {
            _motor = 1.0;
        }
        if self.keys_pressed.contains(&KeyCode::KeyS) {
            _motor = -1.0;
        }
        if self.keys_pressed.contains(&KeyCode::KeyA) {
            _rudder = 1.0;
        }
        if self.keys_pressed.contains(&KeyCode::KeyD) {
            _rudder = -1.0;
        }

        // When not connected to a server, allow direct camera control
        let connected = {
            #[cfg(target_arch = "wasm32")]
            {
                self.ws_client.is_some()
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                false
            }
        };
        if !connected {
            let move_speed = 100.0;
            let rotation_speed = 1.0;
            if _motor != 0.0 {
                let mut dir = gpu.app.cam.rot * glam::Vec3::Y;
                dir.z = 0.0;
                if dir.length_squared() > 0.0 {
                    gpu.app.cam.loc += move_speed * dt * _motor * dir.normalize();
                }
            }
            if _rudder != 0.0 {
                let mut dir = gpu.app.cam.rot * glam::Vec3::X;
                dir.z = 0.0;
                if dir.length_squared() > 0.0 {
                    gpu.app.cam.loc -= move_speed * dt * _rudder * dir.normalize();
                }
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
                    vangers_net::ServerMessage::Welcome {
                        player_id,
                        level_name,
                        ..
                    } => {
                        self.mp_status =
                            format!("Connected (player {}, level '{}')", player_id, level_name);
                        ws.player_id = Some(player_id);
                        log::info!("{}", self.mp_status);
                    }
                    vangers_net::ServerMessage::PlayerJoined {
                        player_id,
                        player_name,
                        ..
                    } => {
                        log::info!("Player {} ({}) joined", player_id, player_name);
                    }
                    vangers_net::ServerMessage::PlayerLeft { player_id } => {
                        log::info!("Player {} left", player_id);
                    }
                    vangers_net::ServerMessage::WorldState { agents, .. } => {
                        // Move camera to follow our agent
                        if let Some(my_id) = ws.player_id {
                            if let Some(me) = agents.iter().find(|a| a.player_id == my_id) {
                                let pos = glam::Vec3::from(me.transform.position);
                                gpu.app.cam.loc = glam::vec3(pos.x, pos.y, pos.z + 200.0);
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
    let handler = WebHandler::new();

    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        event_loop.spawn_app(handler);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut handler = handler;
        event_loop.run_app(&mut handler).unwrap();
    }
}

fn main() {
    web_main();
}
