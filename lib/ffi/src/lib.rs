//! Rusty Vangers FFI bindings.
//! Matches "lib/renderer/src/renderer/scene/rust/vange_rs.h"
//! See https://github.com/KranX/Vangers/pull/517

use futures::executor::LocalPool;
use std::ptr;

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
    queue: wgpu::Queue,
    device: wgpu::Device,
    downlevel_caps: wgpu::DownlevelCapabilities,
    _instance: wgpu::Instance,
    camera: Camera,
}

#[no_mangle]
pub extern "C" fn rv_init() -> Option<ptr::NonNull<Context>> {
    use vangers::config::settings as st;

    let mut task_pool = LocalPool::new();

    let instance = wgpu::Instance::new(wgpu::Backends::all());
    let adapter = task_pool.run_until(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None, //TODO
        force_fallback_adapter: false,
    }))?;
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
        color_format: wgpu::TextureFormat::Rgba8UnormSrgb,
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
