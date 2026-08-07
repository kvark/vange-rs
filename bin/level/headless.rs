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

/// Carve a round pit into the middle of the level and return the rect that
/// changed. Used by `--dig` to exercise incremental terrain updates.
fn dig_crater(level: &mut level::Level) -> vangers::render::Rect {
    let (cx, cy) = (level.size.0 / 2, level.size.1 / 2);
    let r = (level.size.0.min(level.size.1) / 5).max(8);
    for y in (cy - r)..(cy + r) {
        for x in (cx - r)..(cx + r) {
            let (dx, dy) = ((x - cx) as f32, (y - cy) as f32);
            let d2 = dx * dx + dy * dy;
            if d2 > (r * r) as f32 {
                continue;
            }
            let i = (y * level.size.0 + x) as usize;
            let depth = (90.0 * (1.0 - d2 / (r * r) as f32)) as u8;
            level.height[i] = level.height[i].saturating_sub(depth);
            // A crater cuts through the slab, so drop any double level.
            level.meta[i] &= !level::DOUBLE_LEVEL;
        }
    }
    vangers::render::Rect {
        x: (cx - r) as u16,
        y: (cy - r) as u16,
        w: (2 * r) as u16,
        h: (2 * r) as u16,
    }
}

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
    pub mesh_wireframe: bool,
    pub no_cull: bool,
    pub mesh_lod: Option<usize>,
    pub mesh_lod_distance: Option<f32>,
    /// Deform the terrain after the first frame, to exercise the
    /// incremental update path.
    pub dig: bool,
    /// Frame to dig on. 0 means before the mesh is ever built, which
    /// gives a from-scratch reference to compare the incremental path to.
    pub dig_frame: u32,
    /// First-person camera: stand at this XY instead of orbiting a target.
    pub fp: Option<(f32, f32)>,
    /// Eye height above the local ground surface.
    pub fp_height: f32,
    /// Heading in degrees; 0 looks along +Y.
    pub fp_yaw: f32,
    /// Pitch in degrees; 0 is horizontal, positive looks up.
    pub fp_pitch: f32,
    /// Stand on the `low` floor rather than the slab top -- i.e. inside a
    /// double-level region rather than on its roof.
    pub fp_under: bool,
    /// Near clip distance. The default of 10 is fine for an orbit camera
    /// but clips the ground out from under a first-person one.
    pub near: f32,
    /// Far clip distance. The painter emits one instance per visible
    /// ground sample, so an unbounded view distance blows its instance
    /// budget and leaves most of the frame unpainted.
    pub far: f32,
    /// Draw the voxel occupancy grid for LODs `[n, n+1)` instead of the
    /// terrain, to inspect what the ray marcher actually sees.
    pub voxel_debug_lod: Option<usize>,
    /// Probe the voxel occupancy grid at this world position, at every
    /// LOD, and print it alongside the height map's own answer.
    pub voxel_probe: Option<(i32, i32)>,
    /// Also dump the depth buffer as raw little-endian f32, one per pixel.
    /// Lets a comparison score geometry against a reference by distance
    /// rather than by classifying colours.
    pub depth_out: Option<String>,
    pub cull_dump: Option<String>,
    /// Print the raw packed data and the decoded surface for one texel
    /// pair, to compare the CPU query against the shader's decoding.
    pub dump_texel: Option<(i32, i32)>,
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        Self {
            output_path: "snapshot.png".into(),
            level_zip: None,
            common_zip: None,
            level_path: None,
            terrain: settings::Terrain::RayTraced,
            mesh_wireframe: false,
            no_cull: false,
            mesh_lod: None,
            mesh_lod_distance: None,
            dig: false,
            dig_frame: 1,
            fp: None,
            fp_height: 8.0,
            fp_yaw: 0.0,
            fp_pitch: 0.0,
            fp_under: false,
            near: 10.0,
            far: 4000.0,
            voxel_debug_lod: None,
            voxel_probe: None,
            depth_out: None,
            cull_dump: None,
            dump_texel: None,
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

fn make_camera(opts: &SnapshotOptions, lvl: &level::Level) -> space::Camera {
    let (cam_loc, forward) = match opts.fp {
        // First person: stand on the surface and look along a heading. This
        // is the view the orbit camera can't express -- at a low elevation
        // it happily puts the eye inside a hill, and the interesting cases
        // here are exactly the ones grazing the terrain.
        Some((x, y)) => {
            let texel = lvl.get((x as i32, y as i32));
            let ground = if opts.fp_under {
                texel.low()
            } else {
                texel.high()
            };
            let loc = Vec3::new(x, y, ground + opts.fp_height);
            let (yaw, pitch) = (opts.fp_yaw.to_radians(), opts.fp_pitch.to_radians());
            let forward = Vec3::new(
                yaw.sin() * pitch.cos(),
                yaw.cos() * pitch.cos(),
                pitch.sin(),
            );
            (loc, forward.normalize())
        }
        None => {
            let elev = opts.cam_elev_deg.to_radians();
            let loc = opts.cam_target + opts.cam_distance * Vec3::new(0.0, -elev.cos(), elev.sin());
            (loc, (opts.cam_target - loc).normalize())
        }
    };
    // World-up = +Z, except when forward is parallel (looking straight down).
    let up_ref = if forward.cross(Vec3::Z).length_squared() > 1e-6 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let right = forward.cross(up_ref).normalize();
    let up = right.cross(forward).normalize();
    // The engine folds a `scale` of (1, -1, 1) into the view matrix to make
    // the camera left-handed (see `Camera::get_view_proj`), so what ends up
    // pointing up on screen is `-(rot * Y)`, not `rot * Y`. Build the basis
    // accordingly; negating two columns keeps the rotation proper.
    let rot_mat = Mat3::from_cols(-right, -up, -forward);
    let rot = Quat::from_mat3(&rot_mat);

    info!(
        "Camera at ({:.1}, {:.1}, {:.1}) facing ({:.3}, {:.3}, {:.3})",
        cam_loc.x, cam_loc.y, cam_loc.z, forward.x, forward.y, forward.z
    );

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
                near: opts.near.max(0.01),
                far: opts.far.max(opts.near + 1.0),
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
    // `WGPU_BACKEND=gl` picks the GL backend, which is what the web
    // build runs on. Worth being able to reach from here: the wrap-tile
    // instancing the mesh relies on is emulated on GLES and cannot be
    // exercised on the Vulkan path at all.
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY),
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

    // `POLYGON_MODE_LINE` drives the mesh terrain's grid view. It is
    // optional in WebGPU, so take it only when the adapter offers it and
    // let the renderer disable the view otherwise.
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("headless"),
        required_features: adapter.features() & wgpu::Features::POLYGON_MODE_LINE,
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
    let mut lvl = match vfs.as_ref() {
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

    let cam = make_camera(&opts, &lvl);

    let mut render = Render::new(
        &gfx,
        &level_config,
        &objects_palette,
        &render_settings,
        &geometry,
        cam.front_face(),
    );
    render.resize(extent, &gfx.device);
    render.terrain.set_mesh_wireframe(opts.mesh_wireframe);
    render.terrain.set_mesh_culling(!opts.no_cull);
    render
        .terrain
        .set_mesh_lod(opts.mesh_lod, opts.mesh_lod_distance);
    render
        .terrain
        .set_voxel_debug_lods(opts.voxel_debug_lod.map(|n| n..n + 1));

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
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
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
        if opts.dig && frame_index == opts.dig_frame {
            let rect = dig_crater(&mut lvl);
            info!(
                "Dug a crater at {},{} size {}x{}",
                rect.x, rect.y, rect.w, rect.h
            );
            render.terrain.dirty_rects.push(vangers::render::DirtyRect {
                rect,
                z_range: 0..0x100,
                need_upload: true,
            });
        }
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
            // Everything needed to tell one machine's run from another's
            // later, plus the raw frame times so percentiles can be taken
            // after the fact - min/avg/max hides a bimodal distribution,
            // which is exactly what a driver hitch looks like.
            let info = adapter.get_info();
            let times = frame_times
                .iter()
                .map(|d| format!("{:.4}", d.as_secs_f64() * 1e3))
                .collect::<Vec<_>>()
                .join(", ");
            let body = format!(
                concat!(
                    "{{\n",
                    "  \"adapter\": {:?},\n",
                    "  \"backend\": {:?},\n",
                    "  \"device_type\": {:?},\n",
                    "  \"driver\": {:?},\n",
                    "  \"driver_info\": {:?},\n",
                    "  \"width\": {},\n",
                    "  \"height\": {},\n",
                    "  \"frames\": {},\n",
                    "  \"warmup\": {},\n",
                    "  \"cam_elev_deg\": {},\n",
                    "  \"fp_yaw_deg\": {},\n",
                    "  \"fp_pitch_deg\": {},\n",
                    "  \"near\": {},\n",
                    "  \"far\": {},\n",
                    "  \"min_ms\": {:.4},\n",
                    "  \"avg_ms\": {:.4},\n",
                    "  \"max_ms\": {:.4},\n",
                    "  \"frame_ms\": [{}]\n",
                    "}}\n"
                ),
                info.name,
                format!("{:?}", info.backend),
                format!("{:?}", info.device_type),
                info.driver,
                info.driver_info,
                opts.width,
                opts.height,
                opts.frames,
                opts.warmup,
                opts.cam_elev_deg,
                opts.fp_yaw,
                opts.fp_pitch,
                opts.near,
                opts.far,
                min.as_secs_f64() * 1e3,
                avg.as_secs_f64() * 1e3,
                max.as_secs_f64() * 1e3,
                times,
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

    if let Some((px, py)) = opts.dump_texel {
        use vangers::level::{DOUBLE_LEVEL, Texel};
        let w = lvl.size.0 as usize;
        let even = (py as usize * w) + (px as usize & !1);
        let odd = even | 1;
        info!("texel pair at x={} y={} (even index {}):", px, py, px & !1);
        info!(
            "  raw: height[even]={:3} meta[even]=0x{:02X} (DOUBLE={})   height[odd]={:3} meta[odd]=0x{:02X} (DOUBLE={})",
            lvl.height[even],
            lvl.meta[even],
            lvl.meta[even] & DOUBLE_LEVEL != 0,
            lvl.height[odd],
            lvl.meta[odd],
            lvl.meta[odd] & DOUBLE_LEVEL != 0,
        );
        for x in [px & !1, (px & !1) + 1] {
            match lvl.get((x, py)) {
                Texel::Single(p) => info!(
                    "  Level::get(x={}) -> Single   alt={:.1} ty={}",
                    x, p.0, p.1
                ),
                Texel::Dual { low, mid, high } => info!(
                    "  Level::get(x={}) -> Dual     low={:.1} mid={:.1} high={:.1} ty={}/{}",
                    x, low.0, mid, high.0, low.1, high.1
                ),
            }
        }
        // What `get_surface_impl` in surface.inc.wgsl decodes: it always
        // tests the *even* texel's meta for the double-level bit.
        {
            use vangers::level::DOUBLE_LEVEL as DL;
            let (mut mismatch, mut dual_pairs) = (0usize, 0usize);
            for row in lvl.meta.chunks(w) {
                for pair in row.chunks_exact(2) {
                    let (e, o) = (pair[0] & DL != 0, pair[1] & DL != 0);
                    if e || o {
                        dual_pairs += 1;
                    }
                    if e != o {
                        mismatch += 1;
                    }
                }
            }
            info!(
                "  level-wide: {} dual pairs, {} where even/odd DOUBLE_LEVEL disagree ({:.2}%)",
                dual_pairs,
                mismatch,
                100.0 * mismatch as f32 / dual_pairs.max(1) as f32
            );
        }
        info!(
            "  shader would treat both texels as {}",
            if lvl.meta[even] & DOUBLE_LEVEL != 0 {
                "DUAL"
            } else {
                "single"
            }
        );
    }

    if let Some((px, py)) = opts.voxel_probe {
        let texel = lvl.get((px, py));
        let low = texel.low();
        info!(
            "Probing voxel occupancy at x={} y={} (floor {})",
            px, py, low
        );
        let mut col = String::new();
        for zi in 0..128 {
            let z = zi * 2;
            let bits = render
                .terrain
                .debug_voxel_occupancy(&gfx.device, &gfx.queue, [px, py, z]);
            col.push(if bits.first().copied().unwrap_or(false) {
                '#'
            } else {
                '.'
            });
        }
        info!("  LOD0 occupancy up the column (z = 0..256 step 2):");
        info!("  {}", col);
        info!(
            "  height map: solid below z={} ({} steps of 2)",
            low,
            (low / 2.0) as i32
        );
    }

    // Top-down debug data: the frustum, and what the frustum test and the
    // LOD picker actually decided for every chunk of every wrap copy. Read
    // out of the renderer rather than recomputed here, so the picture
    // cannot drift from what was drawn.
    if let Some(ref path) = opts.cull_dump
        && let Some(dbg) = render.terrain.mesh_debug()
    {
        use std::fmt::Write as _;
        // The frustum as eight world-space corners, by pushing the clip
        // cube back through the inverse of the matrix the culling used.
        let inv = cam.get_view_proj().inverse();
        let mut corners = Vec::with_capacity(8);
        for i in 0..8u32 {
            let clip = glam::Vec4::new(
                if i & 1 == 0 { -1.0 } else { 1.0 },
                if i & 2 == 0 { -1.0 } else { 1.0 },
                if i & 4 == 0 { 0.0 } else { 1.0 },
                1.0,
            );
            let p = inv * clip;
            corners.push(p.truncate() / p.w);
        }

        let mut out = String::new();
        out.push_str("{\n");
        let _ = writeln!(
            out,
            "  \"camera\": [{}, {}, {}],",
            cam.loc.x, cam.loc.y, cam.loc.z
        );
        let d = cam.dir();
        let _ = writeln!(out, "  \"dir\": [{}, {}, {}],", d.x, d.y, d.z);
        let _ = writeln!(
            out,
            "  \"level_size\": [{}, {}],",
            dbg.level_size[0], dbg.level_size[1]
        );
        let _ = writeln!(out, "  \"lod_distance\": {},", dbg.lod_distance);
        let _ = writeln!(out, "  \"culling\": {},", dbg.culling);
        out.push_str("  \"frustum\": [");
        for (i, c) in corners.iter().enumerate() {
            let _ = write!(
                out,
                "{}[{}, {}, {}]",
                if i == 0 { "" } else { ", " },
                c.x,
                c.y,
                c.z
            );
        }
        out.push_str("],\n  \"chunks\": [");
        for (i, c) in dbg.chunks.iter().enumerate() {
            let _ = write!(
                out,
                "{}[{}, {}, {}, {}]",
                if i == 0 { "" } else { ", " },
                c.min[0],
                c.min[1],
                c.max[0],
                c.max[1]
            );
        }
        out.push_str("],\n  \"draws\": [");
        for (i, d) in dbg.draws.iter().enumerate() {
            let _ = write!(
                out,
                "{}[{}, {}, {}, {}]",
                if i == 0 { "" } else { ", " },
                d.chunk,
                d.copy,
                d.lod,
                d.distance
            );
        }
        out.push_str("]\n}\n");
        std::fs::write(path, out).expect("failed to write the cull dump");
        info!("Wrote cull dump to {}", path);
    }

    if let Some(ref path) = opts.depth_out {
        // Depth32Float, so 4 bytes per pixel, rows padded to the copy
        // alignment like the colour readback.
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let unpadded = 4 * opts.width;
        let padded = (unpadded + align - 1) & !(align - 1);
        let buf = gfx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("depth readback"),
            size: (padded * opts.height) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = gfx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &depth_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::DepthOnly,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(opts.height),
                },
            },
            extent,
        );
        gfx.queue.submit(Some(enc.finish()));
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = gfx.device.poll(wgpu::PollType::Wait {
            timeout: None,
            submission_index: Default::default(),
        });
        let view = buf.slice(..).get_mapped_range();
        let mut out = Vec::with_capacity((unpadded * opts.height) as usize);
        for row in 0..opts.height as usize {
            let start = row * padded as usize;
            out.extend_from_slice(&view[start..start + unpadded as usize]);
        }
        drop(view);
        buf.unmap();
        std::fs::write(path, &out).expect("failed to write depth dump");
        info!(
            "Wrote {}x{} depth buffer to {}",
            opts.width, opts.height, path
        );
    }

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
