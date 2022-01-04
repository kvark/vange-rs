//! Rusty Vangers FFI bindings.
//! Matches "lib/renderer/src/renderer/scene/rust/vange_rs.h"
//! See https://github.com/KranX/Vangers/pull/517

use futures::executor::LocalPool;
use std::{ffi::CString, os::raw, ptr};

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
}

#[derive(Default)]
struct Camera {
    desc: CameraDescription,
    transform: Transform,
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
    queue: wgpu::Queue,
    device: wgpu::Device,
    downlevel_caps: wgpu::DownlevelCapabilities,
    _instance: wgpu::Instance,
    camera: Camera,
}

pub type GlFunctionDiscovery = unsafe extern "C" fn(*const raw::c_char) -> *const raw::c_void;

#[repr(C)]
pub struct InitDescriptor {
    width: u32,
    height: u32,
    gl_functor: GlFunctionDiscovery,
}

#[no_mangle]
pub extern "C" fn rv_init(desc: InitDescriptor) -> Option<ptr::NonNull<Context>> {
    use vangers::config::settings as st;

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

    let (device, queue) = task_pool
        .run_until(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                features: wgpu::Features::empty(),
                limits: wgpu::Limits::default(),
            },
            None,
        ))
        .ok()?;

    let mut texture_desc = wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: desc.width,
            height: desc.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Uint, //dummy
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
    };
    let color_format = wgpu::TextureFormat::Rgba8UnormSrgb;

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
        let depth_format = wgpu::TextureFormat::Depth24Plus;
        let hal_texture_depth =
            <hal::api::Gles as hal::Api>::Texture::default_framebuffer(depth_format);
        texture_desc.format = depth_format;
        let depth_texture = unsafe {
            device.create_texture_from_hal::<hal::api::Gles>(hal_texture_depth, &texture_desc)
        };
        depth_texture.create_view(&wgpu::TextureViewDescriptor::default())
    };

    let ctx = Context {
        level: None,
        render_config: st::Render {
            wgpu_trace_path: String::new(),
            light: st::Light {
                pos: [1.0, 2.0, 4.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
                shadow: st::Shadow {
                    size: 0,
                    terrain: st::ShadowTerrain::RayTraced,
                },
            },
            terrain: st::Terrain::RayTraced,
            fog: st::Fog {
                color: [0.1, 0.2, 0.3, 1.0],
                depth: 50.0,
            },
            debug: st::DebugRender::default(),
        },
        color_format,
        color_view,
        depth_view,
        queue,
        device,
        downlevel_caps: adapter.get_downlevel_properties(),
        _instance: instance,
        camera: Default::default(),
    };
    let ptr = Box::into_raw(Box::new(ctx));
    ptr::NonNull::new(ptr)
}

#[no_mangle]
pub unsafe extern "C" fn rv_exit(ctx: *mut Context) {
    let _ctx = Box::from_raw(ctx);
}

#[no_mangle]
pub extern "C" fn rv_camera_init(ctx: &mut Context, desc: CameraDescription) {
    ctx.camera.desc = desc;
}

#[no_mangle]
pub extern "C" fn rv_camera_set_transform(ctx: &mut Context, transform: Transform) {
    ctx.camera.transform = transform;
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
        flood_map: vec![].into_boxed_slice(),
        flood_section_power: 0, //TODO
        height: vec![0; total].into_boxed_slice(),
        meta: vec![0; total].into_boxed_slice(),
        palette: [[0; 4]; 0x100], //TODO
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
        // extent only matters for "scatter" style rendering
        wgpu::Extent3d {
            width: 0,
            height: 0,
            depth_or_array_layers: 0,
        },
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
pub extern "C" fn rv_map_request_update(ctx: &mut Context, region: Rect) {
    //TODO
}
