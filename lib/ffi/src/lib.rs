/*! Rusty Vangers FFI bindings.

Matches "lib/renderer/src/renderer/scene/rust/vange_rs.h"
See https://github.com/KranX/Vangers/pull/517

Changelog:
  1. Add `rv_resize()`.
  0. Basic version.
!*/

use futures::executor::LocalPool;
use std::{ffi::CString, os::raw, ptr};

// Update this whenever C header changes
#[no_mangle]
pub static rv_api_1: i32 = 1;

#[repr(C)]
#[derive(Default)]
pub struct Vector3 {
    x: f32,
    y: f32,
    z: f32,
}

#[repr(C)]
#[derive(Default)]
pub struct Quaternion {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

#[repr(C)]
#[derive(Default)]
pub struct Transform {
    position: Vector3,
    rotation: Quaternion,
}

#[repr(C)]
pub struct Rect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl Rect {
    fn to_native(&self) -> vangers::render::Rect {
        vangers::render::Rect {
            x: self.x as u16,
            y: self.y as u16,
            w: self.width as u16,
            h: self.height as u16,
        }
    }
}

#[repr(C)]
#[derive(Default)]
pub struct CameraDescription {
    fov: f32,
    aspect: f32,
    near: f32,
    far: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MapDescription {
    width: i32,
    height: i32,
    lines: *const *const u8,
    material_begin_offsets: *const u8,
    material_end_offsets: *const u8,
    material_count: i32,
    palette: *const u8,
}

struct LevelContext {
    desc: MapDescription,
    render: vangers::render::Render,
    level: vangers::level::Level,
}

pub struct Context {
    level: Option<LevelContext>,
    render_config: vangers::config::settings::Render,
    color_format: wgpu::TextureFormat,
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    extent: wgpu::Extent3d,
    queue: wgpu::Queue,
    device: wgpu::Device,
    downlevel_caps: wgpu::DownlevelCapabilities,
    _instance: wgpu::Instance,
    camera: vangers::space::Camera,
}

pub type GlFunctionDiscovery = unsafe extern "C" fn(*const raw::c_char) -> *const raw::c_void;

#[repr(C)]
pub struct InitDescriptor {
    width: u32,
    height: u32,
    gl_functor: GlFunctionDiscovery,
}

fn crate_main_views(
    device: &wgpu::Device,
    extent: wgpu::Extent3d,
    color_format: wgpu::TextureFormat,
) -> (wgpu::TextureView, wgpu::TextureView) {
    let mut texture_desc = wgpu::TextureDescriptor {
        label: None,
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Uint, //dummy
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
    };
    let color_view = {
        let hal_texture_color =
            <hal::api::Gles as hal::Api>::Texture::default_framebuffer(color_format);
        texture_desc.format = color_format;
        let color_texture = unsafe {
            device.create_texture_from_hal::<hal::api::Gles>(hal_texture_color, &texture_desc)
        };
        color_texture.create_view(&wgpu::TextureViewDescriptor::default())
    };
    let depth_view = {
        let hal_texture_depth = <hal::api::Gles as hal::Api>::Texture::default_framebuffer(
            vangers::render::DEPTH_FORMAT,
        );
        texture_desc.format = vangers::render::DEPTH_FORMAT;
        let depth_texture = unsafe {
            device.create_texture_from_hal::<hal::api::Gles>(hal_texture_depth, &texture_desc)
        };
        depth_texture.create_view(&wgpu::TextureViewDescriptor::default())
    };
    (color_view, depth_view)
}

#[no_mangle]
pub extern "C" fn rv_init(desc: InitDescriptor) -> Option<ptr::NonNull<Context>> {
    use cgmath::Zero as _;
    use vangers::config::settings as st;

    #[cfg(feature = "env_logger")]
    let _ = env_logger::try_init();
    let mut task_pool = LocalPool::new();

    let exposed = unsafe {
        <hal::api::Gles as hal::Api>::Adapter::new_external(|name| {
            let cstr = CString::new(name).unwrap();
            (desc.gl_functor)(cstr.as_ptr())
        })
    }
    .expect("GL adapter can't be initialized");

    let instance = wgpu::Instance::new(wgpu::Backends::empty());
    let adapter = unsafe { instance.create_adapter_from_hal(exposed) };

    let limits = {
        let adapter_limits = adapter.limits();
        let desired_height = 16 << 10;
        wgpu::Limits {
            max_texture_dimension_2d: if adapter_limits.max_texture_dimension_2d < desired_height {
                println!(
                    "Adapter only supports {} texutre size, main levels are not compatible",
                    adapter_limits.max_texture_dimension_2d
                );
                adapter_limits.max_texture_dimension_2d
            } else {
                desired_height
            },
            ..wgpu::Limits::downlevel_webgl2_defaults()
        }
    };

    let (device, queue) = task_pool
        .run_until(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                features: wgpu::Features::empty(),
                limits,
            },
            None,
        ))
        .ok()?;

    let extent = wgpu::Extent3d {
        width: desc.width,
        height: desc.height,
        depth_or_array_layers: 1,
    };
    let color_format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let (color_view, depth_view) = crate_main_views(&device, extent, color_format);

    let ctx = Context {
        level: None,
        render_config: st::Render {
            wgpu_trace_path: String::new(),
            light: st::Light {
                pos: [1.0, 2.0, 4.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
                shadow: st::Shadow {
                    size: 1024,
                    terrain: st::ShadowTerrain::RayTraced,
                },
            },
            terrain: st::Terrain::RayTraced,
            water: st::Water {},
            fog: st::Fog {
                color: [0.1, 0.2, 0.3, 1.0],
                depth: 50.0,
            },
            debug: st::DebugRender::default(),
        },
        color_format,
        color_view,
        depth_view,
        extent,
        queue,
        device,
        downlevel_caps: adapter.get_downlevel_properties(),
        _instance: instance,
        camera: vangers::space::Camera {
            loc: cgmath::Vector3::zero(),
            rot: cgmath::Quaternion::zero(),
            scale: cgmath::vec3(1.0, 1.0, 1.0),
            proj: vangers::space::Projection::Perspective(cgmath::PerspectiveFov {
                aspect: 1.0,
                near: 1.0,
                far: 100.0,
                fovy: cgmath::Deg(45.0).into(),
            }),
        },
    };
    let ptr = Box::into_raw(Box::new(ctx));
    ptr::NonNull::new(ptr)
}

#[no_mangle]
pub unsafe extern "C" fn rv_exit(ctx: *mut Context) {
    let _ctx = Box::from_raw(ctx);
}

#[no_mangle]
pub extern "C" fn rv_resize(ctx: &mut Context, width: u32, height: u32) {
    ctx.extent = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let (color_view, depth_view) = crate_main_views(&ctx.device, ctx.extent, ctx.color_format);
    ctx.color_view = color_view;
    ctx.depth_view = depth_view;
}

#[no_mangle]
pub extern "C" fn rv_camera_init(ctx: &mut Context, desc: CameraDescription) {
    ctx.camera.proj = vangers::space::Projection::Perspective(cgmath::PerspectiveFov {
        aspect: desc.aspect,
        near: desc.near,
        far: desc.far,
        fovy: cgmath::Deg(desc.fov).into(),
    });
}

#[no_mangle]
pub extern "C" fn rv_camera_set_transform(ctx: &mut Context, t: Transform) {
    ctx.camera.loc = cgmath::vec3(t.position.x, t.position.y, t.position.z);
    ctx.camera.rot =
        cgmath::Quaternion::new(t.rotation.w, t.rotation.x, t.rotation.y, t.rotation.z);
}

#[no_mangle]
pub extern "C" fn rv_map_init(ctx: &mut Context, desc: MapDescription) {
    let terrains = (0..desc.material_count)
        .map(|i| unsafe {
            let mut tc = vangers::level::TerrainConfig::default();
            tc.colors.start = *desc.material_begin_offsets.offset(i as isize);
            tc.colors.end = *desc.material_end_offsets.offset(i as isize);
            tc
        })
        .collect::<Box<[_]>>();

    let total = (desc.width * desc.height) as usize;
    let level = vangers::level::Level {
        size: (desc.width, desc.height),
        flood_map: vec![0; 128].into_boxed_slice(),
        flood_section_power: 7, //TODO
        height: vec![0; total].into_boxed_slice(),
        meta: vec![0; total].into_boxed_slice(),
        palette: [[0; 4]; 0x100],
        terrains,
    };

    let render = vangers::render::Render::new(
        &ctx.device,
        &ctx.queue,
        &ctx.downlevel_caps,
        &level,
        &[[0; 4]; 0x100], //TODO: objects palette
        &ctx.render_config,
        ctx.color_format,
        ctx.extent, //Note: needs update on window resize
        ctx.camera.front_face(),
    );
    ctx.level = Some(LevelContext {
        desc,
        render,
        level,
    });
}

#[no_mangle]
pub extern "C" fn rv_map_exit(ctx: &mut Context) {
    ctx.level = None;
}

#[no_mangle]
pub unsafe extern "C" fn rv_map_update_data(ctx: &mut Context, region: Rect) {
    let lc = ctx.level.as_mut().unwrap();
    let line_width = lc.level.size.0 as usize;

    for y in region.y..region.y + region.height {
        // In the source data, each line contains N height values followed by N meta values.
        // We copy them into separate height and meta data arrays.
        let dst_offset = y as usize * line_width + region.x as usize;
        let line = *lc.desc.lines.add(y as usize);
        if line.is_null() {
            continue;
        }
        ptr::copy_nonoverlapping(
            line.add(region.x as usize),
            lc.level.height[dst_offset..].as_mut_ptr(),
            region.width as usize,
        );
        ptr::copy_nonoverlapping(
            line.add(region.x as usize + line_width),
            lc.level.meta[dst_offset..].as_mut_ptr(),
            region.width as usize,
        );
    }

    lc.render.terrain.dirty_rects.push(region.to_native());
}

#[no_mangle]
pub unsafe extern "C" fn rv_map_update_palette(
    ctx: &mut Context,
    first_entry: u32,
    entries_count: u32,
    palette: *const u8,
) {
    let lc = ctx.level.as_mut().unwrap();
    let end = first_entry + entries_count;

    for (i, color) in lc.level.palette[first_entry as usize..end as usize]
        .iter_mut()
        .enumerate()
    {
        ptr::copy_nonoverlapping(palette.add(i * 3), color.first_mut().unwrap(), 3);
    }

    let dp = lc.render.terrain.dirty_palette.clone();
    lc.render.terrain.dirty_palette = if dp != (0..0) {
        dp.start.min(first_entry)..dp.end.max(end)
    } else {
        first_entry..end
    };
}

#[no_mangle]
pub extern "C" fn rv_render(ctx: &mut Context, viewport: Rect) {
    let lc = ctx.level.as_mut().unwrap();
    let targets = vangers::render::ScreenTargets {
        extent: ctx.extent,
        depth: &ctx.depth_view,
        color: &ctx.color_view,
    };
    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let mut batcher = vangers::render::Batcher::new();

    lc.render.draw_world(
        &mut encoder,
        &mut batcher,
        &lc.level,
        &ctx.camera,
        targets,
        Some(viewport.to_native()),
        &ctx.device,
    );

    ctx.queue.submit(Some(encoder.finish()));
}
