//! Headless renderer used for snapshot tests and benchmarking.
//!
//! Two modes:
//!   * `render_snapshot(opts)` — renders `opts.frames` (after `opts.warmup`),
//!     times each frame end-to-end on the CPU side (submit + poll), then
//!     saves the last frame as PNG. Optionally writes a JSON file with the
//!     min/avg/max frame time.
//!
//! Levels can come from three places:
//!   * `--level-zip` + optional `--common-zip`: mounted into a `Vfs` and
//!     loaded via `level::load_from_vfs`. Same code path the web build uses.
//!   * `--level-path` (path to `world.ini`): native filesystem load.
//!   * neither: built-in procedural test level.
//!
//! Camera is parametrised by target world position, distance, and elevation
//! (degrees from horizontal). 90° = top-down, 0° = looking horizontal at the
//! target. A simple look-at with world-up = +Z, falling back to +Y up when
//! the camera is straight overhead.

use vangers::{
    config::settings,
    level,
    render::{Batcher, DEPTH_FORMAT, GraphicsContext, Render, ScreenTargets},
    space,
    vfs::Vfs,
};

use glam::{Mat3, Quat, Vec3};
use log::info;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct SnapshotOptions {
    pub output_path: String,
    pub level_zip: Option<String>,
    pub common_zip: Option<String>,
    pub level_path: Option<String>,
    pub terrain: settings::Terrain,
    pub width: u32,
    pub height: u32,
    pub cam_target: Vec3,
    pub cam_distance: f32,
    pub cam_elev_deg: f32,
    pub frames: u32,
    pub warmup: u32,
    pub bench_out: Option<String>,
    pub shadow_voxel: bool,
    pub shadow_ray: bool,
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        Self {
            output_path: "snapshot.png".into(),
            level_zip: None,
            common_zip: None,
            level_path: None,
            terrain: settings::Terrain::RayTraced,
            width: 800,
            height: 600,
            cam_target: Vec3::new(128.0, 128.0, 0.0),
            cam_distance: 300.0,
            cam_elev_deg: 60.0,
            frames: 1,
            warmup: 0,
            bench_out: None,
            shadow_voxel: false,
            shadow_ray: false,
        }
    }
}

fn make_camera(opts: &SnapshotOptions) -> space::Camera {
    let elev = opts.cam_elev_deg.to_radians();
    let cam_loc = opts.cam_target + opts.cam_distance * Vec3::new(0.0, -elev.cos(), elev.sin());
    let forward = (opts.cam_target - cam_loc).normalize();
    // World-up = +Z, except when forward is parallel (looking straight down).
    let up_ref = if forward.cross(Vec3::Z).length_squared() > 1e-6 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let right = forward.cross(up_ref).normalize();
    let up = right.cross(forward).normalize();
    // Camera-local axes: +X right, +Y up, -Z forward.
    let rot_mat = Mat3::from_cols(right, up, -forward);
    let rot = Quat::from_mat3(&rot_mat);

    space::Camera {
        loc: cam_loc,
        rot,
        scale: Vec3::new(1.0, -1.0, 1.0),
        proj: {
            let h = opts.height.max(1) as f32;
            let focal = space::DEFAULT_FOCAL_PX;
            space::Projection::Perspective(space::PerspectiveParams {
                fovy: space::PerspectiveParams::fov_from_focal_px(focal, h),
                aspect: opts.width as f32 / h,
                near: 10.0,
                far: 4000.0,
                focal_px: Some(focal),
            })
        },
    }
}

fn load_level_via_vfs(level_zip: &str, common_zip: Option<&str>) -> (level::LevelConfig, Vfs) {
    let mut vfs = Vfs::new();
    if let Some(common_zip) = common_zip {
        info!("Mounting common zip: {}", common_zip);
        let bytes = std::fs::read(common_zip).expect("Failed to read common zip");
        vfs.mount_zip(&bytes).expect("Failed to mount common zip");
    }
    info!("Mounting level zip: {}", level_zip);
    let bytes = std::fs::read(level_zip).expect("Failed to read level zip");
    vfs.mount_zip(&bytes).expect("Failed to mount level zip");
    // Per the web build's convention (bin/web/main.rs), level zips have
    // their files at the archive root, so the INI key is just "world.ini".
    let level_config = level::LevelConfig::load_from_vfs(&vfs, "world.ini");
    (level_config, vfs)
}

pub fn render_snapshot(opts: SnapshotOptions) {
    let extent = wgpu::Extent3d {
        width: opts.width,
        height: opts.height,
        depth_or_array_layers: 1,
    };

    info!("Creating headless wgpu instance");
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("No suitable GPU adapter found for headless rendering");

    info!("Adapter: {:?}", adapter.get_info().name);

    let mut render_settings = settings::Render {
        terrain: opts.terrain,
        ..Default::default()
    };
    if opts.shadow_voxel {
        // Default `Render::default()` leaves `shadow.size = 0`, which
        // disables shadow rendering entirely. Mirror the WebGPU/native
        // settings.ron value (1024) so the voxel shadow path is actually
        // exercised. Step counts match the WebGPU build.
        render_settings.light.shadow.size = 1024;
        render_settings.light.shadow.terrain = settings::ShadowTerrain::RayVoxelTraced {
            max_outer_steps: 20,
            max_inner_steps: 20,
        };
    } else if opts.shadow_ray {
        // Mirrors the WebGL2 fallback: 1024² shadow map, height-field
        // ray-traced.
        render_settings.light.shadow.size = 1024;
        render_settings.light.shadow.terrain = settings::ShadowTerrain::RayTraced;
    }

    let geometry = settings::Geometry::default();

    let limits = render_settings.get_device_limits(&adapter.limits(), geometry.height);
    let downlevel_caps = adapter.get_downlevel_capabilities();

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("headless"),
        required_features: wgpu::Features::empty(),
        required_limits: limits,
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
        experimental_features: Default::default(),
    }))
    .expect("Failed to create device");

    let color_format = wgpu::TextureFormat::Rgba8UnormSrgb;

    let gfx = GraphicsContext {
        device,
        queue,
        downlevel_caps,
        color_format,
        screen_size: extent,
    };

    // Resolve the level data source. VFS path matches the web pipeline; native
    // path matches the road/level binaries; otherwise fall back to test level.
    let (level_config, vfs) = match (opts.level_zip.as_deref(), opts.level_path.as_deref()) {
        (Some(zip), _) => {
            let (lc, vfs) = load_level_via_vfs(zip, opts.common_zip.as_deref());
            (lc, Some(vfs))
        }
        (None, Some(path)) => {
            info!("Loading level from {}", path);
            (level::LevelConfig::load(Path::new(path)), None)
        }
        (None, None) => {
            info!("Using procedural test level");
            (level::LevelConfig::new_test(), None)
        }
    };

    // Load level data. VFS path uses an in-memory mount; native uses paths
    // resolved through the LevelConfig, plus relative .pal/.vmc files.
    let lvl = match vfs.as_ref() {
        Some(vfs) => level::load_from_vfs(vfs, &level_config, &geometry),
        None => level::load(&level_config, &geometry),
    };

    // Objects palette: white if we don't have one (test level / no palette
    // wired through here), otherwise the real palette from the VFS.
    let objects_palette: Vec<[u8; 4]> = match vfs
        .as_ref()
        .and_then(|v| v.read("resource/pal/objects.pal"))
    {
        Some(bytes) => level::read_palette_bytes(&bytes, None).to_vec(),
        None => (0..256).map(|_| [255u8, 255, 255, 255]).collect(),
    };

    let cam = make_camera(&opts);

    let mut render = Render::new(
        &gfx,
        &level_config,
        &objects_palette,
        &render_settings,
        &geometry,
        cam.front_face(),
    );
    render.resize(extent, &gfx.device);

    // Offscreen color + depth.
    let color_tex = gfx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("snapshot-color"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: color_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let color_view = color_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let depth_tex = gfx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("snapshot-depth"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth_view = depth_tex.create_view(&wgpu::TextureViewDescriptor::default());

    // Warmup + timed loop. Warmup drains the bake queue so that timed
    // frames measure steady-state cost; without this, the first frame
    // includes the full voxel-grid build for the level.
    info!(
        "Rendering {} warmup + {} timed frame(s)",
        opts.warmup, opts.frames
    );
    let mut frame_times: Vec<Duration> = Vec::with_capacity(opts.frames as usize);
    let total_frames = opts.warmup + opts.frames.max(1);

    for frame_index in 0..total_frames {
        let is_timed = frame_index >= opts.warmup;
        let started = Instant::now();

        let mut encoder = gfx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("snapshot-frame"),
            });

        let targets = ScreenTargets {
            extent,
            color: &color_view,
            depth: &depth_view,
        };
        render.draw_world(
            &mut encoder,
            &mut Batcher::new(),
            &lvl,
            &cam,
            targets,
            None,
            &gfx.device,
            &gfx.queue,
        );

        gfx.queue.submit(Some(encoder.finish()));

        // Wait for GPU completion so the timing reflects actual draw cost.
        gfx.device
            .poll(wgpu::PollType::Wait {
                timeout: Some(Duration::from_secs(30)),
                submission_index: Default::default(),
            })
            .expect("device poll failed");

        if is_timed {
            frame_times.push(started.elapsed());
        }
    }

    if !frame_times.is_empty() {
        let total: Duration = frame_times.iter().sum();
        let avg = total / frame_times.len() as u32;
        let min = *frame_times.iter().min().unwrap();
        let max = *frame_times.iter().max().unwrap();
        info!(
            "Frame times: min={:.3}ms avg={:.3}ms max={:.3}ms (n={})",
            min.as_secs_f64() * 1e3,
            avg.as_secs_f64() * 1e3,
            max.as_secs_f64() * 1e3,
            frame_times.len(),
        );

        if let Some(ref path) = opts.bench_out {
            let body = format!(
                concat!(
                    "{{\n",
                    "  \"adapter\": \"{}\",\n",
                    "  \"width\": {},\n",
                    "  \"height\": {},\n",
                    "  \"frames\": {},\n",
                    "  \"warmup\": {},\n",
                    "  \"cam_elev_deg\": {},\n",
                    "  \"min_ms\": {:.4},\n",
                    "  \"avg_ms\": {:.4},\n",
                    "  \"max_ms\": {:.4}\n",
                    "}}\n"
                ),
                adapter.get_info().name,
                opts.width,
                opts.height,
                opts.frames,
                opts.warmup,
                opts.cam_elev_deg,
                min.as_secs_f64() * 1e3,
                avg.as_secs_f64() * 1e3,
                max.as_secs_f64() * 1e3,
            );
            if let Some(parent) = std::path::Path::new(path).parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::write(path, body).expect("Failed to write bench JSON");
            info!("Bench results written to {}", path);
        }
    }

    // Pull the last frame back to CPU.
    let bytes_per_pixel = 4u32;
    let unpadded_bytes_per_row = bytes_per_pixel * opts.width;
    let align = 256u32;
    let padded_bytes_per_row = (unpadded_bytes_per_row + align - 1) & !(align - 1);

    let staging_buf = gfx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("snapshot-staging"),
        size: (padded_bytes_per_row * opts.height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut readback_encoder = gfx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("snapshot-readback"),
        });
    readback_encoder.copy_texture_to_buffer(
        color_tex.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &staging_buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: None,
            },
        },
        extent,
    );
    gfx.queue.submit(Some(readback_encoder.finish()));

    let slice = staging_buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        tx.send(result).unwrap();
    });
    gfx.device
        .poll(wgpu::PollType::Wait {
            timeout: Some(Duration::from_secs(5)),
            submission_index: Default::default(),
        })
        .unwrap();
    rx.recv().unwrap().expect("Failed to map staging buffer");

    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((unpadded_bytes_per_row * opts.height) as usize);
    for row in 0..opts.height as usize {
        let start = row * padded_bytes_per_row as usize;
        let end = start + unpadded_bytes_per_row as usize;
        pixels.extend_from_slice(&data[start..end]);
    }
    drop(data);
    staging_buf.unmap();

    info!("Saving snapshot to {}", opts.output_path);
    if let Some(parent) = std::path::Path::new(&opts.output_path).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let file = std::fs::File::create(&opts.output_path).expect("Failed to create output PNG file");
    let w = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(w, opts.width, opts.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::RelativeColorimetric);
    let mut writer = encoder.write_header().expect("Failed to write PNG header");
    writer
        .write_image_data(&pixels)
        .expect("Failed to write PNG data");

    info!("Snapshot saved successfully");
}
