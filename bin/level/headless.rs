use vangers::{
    config::settings,
    level,
    render::{Batcher, GraphicsContext, Render, ScreenTargets, DEPTH_FORMAT},
    space,
};

use log::info;
use std::path::Path;

pub fn render_snapshot(
    output_path: &str,
    level_path: Option<&str>,
    terrain: settings::Terrain,
) {
    let width = 800u32;
    let height = 600u32;
    let extent = wgpu::Extent3d {
        width,
        height,
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

    let render_settings = settings::Render {
        terrain,
        ..Default::default()
    };

    let geometry = settings::Geometry::default();

    let limits = render_settings.get_device_limits(&adapter.limits(), geometry.height);
    let downlevel_caps = adapter.get_downlevel_capabilities();

    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
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

    // Set up level config
    let level_config = if let Some(path) = level_path {
        info!("Loading level from {}", path);
        level::LevelConfig::load(Path::new(path))
    } else {
        info!("Using test level");
        level::LevelConfig::new_test()
    };

    // Create a white objects palette
    let objects_palette: Vec<[u8; 4]> = (0..256).map(|_| [255u8, 255, 255, 255]).collect();

    // Flat ortho view looking straight down - shows palette-colored terrain
    let cam = space::Camera {
        loc: glam::vec3(128.0, 128.0, 400.0),
        rot: glam::Quat::IDENTITY,
        scale: glam::vec3(1.0, -1.0, 1.0),
        proj: space::Projection::Ortho {
            p: space::OrthoParams {
                left: 0.0,
                right: 256.0,
                bottom: 0.0,
                top: 256.0,
                near: 10.0,
                far: 1024.0,
            },
            original: (width as u16, height as u16),
        },
    };

    let mut render = Render::new(
        &gfx,
        &level_config,
        &objects_palette,
        &render_settings,
        &geometry,
        cam.front_face(),
    );
    render.resize(extent, &gfx.device);

    let level = level::load(&level_config, &geometry);

    // Create offscreen color texture
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

    // Create depth texture
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

    // Render one frame
    info!("Rendering snapshot frame");
    let targets = ScreenTargets {
        extent,
        color: &color_view,
        depth: &depth_view,
    };

    let mut encoder = gfx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("snapshot"),
        });

    render.draw_world(
        &mut encoder,
        &mut Batcher::new(),
        &level,
        &cam,
        targets,
        None,
        &gfx.device,
        &gfx.queue,
    );

    // Copy rendered texture to staging buffer
    let bytes_per_pixel = 4u32;
    let unpadded_bytes_per_row = bytes_per_pixel * width;
    let align = 256u32;
    let padded_bytes_per_row = (unpadded_bytes_per_row + align - 1) & !(align - 1);

    let staging_buf = gfx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("snapshot-staging"),
        size: (padded_bytes_per_row * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    encoder.copy_texture_to_buffer(
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

    gfx.queue.submit(Some(encoder.finish()));

    // Map the buffer and read back
    let slice = staging_buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        tx.send(result).unwrap();
    });
    gfx.device.poll(wgpu::PollType::Wait {
        timeout: Some(std::time::Duration::from_secs(5)),
        submission_index: Default::default(),
    }).unwrap();
    rx.recv().unwrap().expect("Failed to map staging buffer");

    let data = slice.get_mapped_range();

    // Remove row padding and collect into a contiguous buffer
    let mut pixels = Vec::with_capacity((unpadded_bytes_per_row * height) as usize);
    for row in 0..height as usize {
        let start = row * padded_bytes_per_row as usize;
        let end = start + unpadded_bytes_per_row as usize;
        pixels.extend_from_slice(&data[start..end]);
    }
    drop(data);
    staging_buf.unmap();

    // Save as PNG
    info!("Saving snapshot to {}", output_path);
    if let Some(parent) = std::path::Path::new(output_path).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let file = std::fs::File::create(output_path).expect("Failed to create output PNG file");
    let w = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(w, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::RelativeColorimetric);
    let mut writer = encoder.write_header().expect("Failed to write PNG header");
    writer
        .write_image_data(&pixels)
        .expect("Failed to write PNG data");

    info!("Snapshot saved successfully");
}
