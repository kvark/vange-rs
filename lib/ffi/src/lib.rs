/*! Rusty Vangers FFI bindings.

Matches "lib/renderer/src/renderer/scene/rust/vange_rs.h"
See https://github.com/KranX/Vangers/pull/517

Changelog:
  3.2: Control model visibility.
  3.1: Expose render config path.
  2.1: Transform scales, model APIs.
  1.1: Add `rv_resize()`.
  0.1: Basic version.
!*/

use futures::executor::LocalPool;
use slotmap::{DefaultKey, Key as _, SlotMap};
use std::{
    ffi::{CStr, CString},
    fs::File,
    mem,
    os::raw,
    ptr, slice,
    sync::Arc,
};

// Update this whenever C header changes
#[no_mangle]
pub static rv_api_3: i32 = 2;

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
    scale: f32,
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
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Vertex {
    data: [i8; 3],
    scr: [u8; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Normal {
    data: [i8; 3],
    i: u8,
    n_power: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Polygon {
    vertices: [*const Vertex; 3],
    normals: [*const Normal; 3],
    color_id: u8,
    middle: [i8; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Model {
    num_vert: i32,
    vertices: *const Vertex,
    num_norm: i32,
    normals: *const Normal,
    num_poly: i32,
    polygons: *const Polygon,
    max: [i32; 3],
    min: [i32; 3],
    off: [i32; 3],
    rmax: i32,
    memory_allocation_method: i32,
    volume: f64,
    rcm: [f64; 3],
    jacobian: [f64; 9],
}

struct MeshInstance {
    mesh: Arc<vangers::model::Mesh>,
    transform: vangers::space::Transform,
    color_id: u8,
    visible: bool,
}

struct LevelContext {
    desc: MapDescription,
    render: vangers::render::Render,
    level: vangers::level::Level,
}

pub struct Context {
    level: Option<LevelContext>,
    geometry_config: vangers::config::settings::Geometry,
    render_config: vangers::config::settings::Render,
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    gfx: vangers::render::GraphicsContext,
    _instance: wgpu::Instance,
    camera: vangers::space::Camera,
    objects_palette: [[u8; 4]; 0x100],
    meshes: SlotMap<DefaultKey, Arc<vangers::model::Mesh>>,
    instances: SlotMap<DefaultKey, MeshInstance>,
}

pub type GlFunctionDiscovery = unsafe extern "C" fn(*const raw::c_char) -> *const raw::c_void;

#[repr(C)]
pub struct InitDescriptor {
    width: u32,
    height: u32,
    render_config: *const raw::c_char,
    gl_functor: GlFunctionDiscovery,
}

fn crate_main_views(
    gfx: &vangers::render::GraphicsContext,
) -> (wgpu::TextureView, wgpu::TextureView) {
    let mut texture_desc = wgpu::TextureDescriptor {
        label: None,
        size: gfx.screen_size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Uint, //dummy
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    };
    let color_view = {
        let hal_texture_color =
            <hal::api::Gles as hal::Api>::Texture::default_framebuffer(gfx.color_format);
        texture_desc.format = gfx.color_format;
        let color_texture = unsafe {
            gfx.device
                .create_texture_from_hal::<hal::api::Gles>(hal_texture_color, &texture_desc)
        };
        color_texture.create_view(&wgpu::TextureViewDescriptor::default())
    };
    let depth_view = {
        let hal_texture_depth = <hal::api::Gles as hal::Api>::Texture::default_framebuffer(
            vangers::render::DEPTH_FORMAT,
        );
        texture_desc.format = vangers::render::DEPTH_FORMAT;
        let depth_texture = unsafe {
            gfx.device
                .create_texture_from_hal::<hal::api::Gles>(hal_texture_depth, &texture_desc)
        };
        depth_texture.create_view(&wgpu::TextureViewDescriptor::default())
    };
    (color_view, depth_view)
}

#[no_mangle]
pub extern "C" fn rv_init(desc: InitDescriptor) -> Option<ptr::NonNull<Context>> {
    #[cfg(feature = "env_logger")]
    let _ = env_logger::try_init();

    #[cfg(feature = "android_logger")]
    log_panics::init();

    #[cfg(feature = "android_logger")]
    let _ = android_logger::init_once(
        android_logger::Config::default().with_min_level(log::Level::Warn),
    );

    let geometry_config: vangers::config::settings::Geometry = {
        let file = File::open("res/ffi/geometry.ron").unwrap();
        ron::de::from_reader(file).unwrap()
    };
    let render_config: vangers::config::settings::Render = {
        let config_path = if desc.render_config.is_null() {
            log::info!("Null render config path");
            "res/ffi/render-compat.ron".to_string()
        } else {
            unsafe { CStr::from_ptr(desc.render_config) }
                .to_string_lossy()
                .to_string()
        };
        log::info!("Using render: {}", config_path);
        let file = File::open(&config_path).unwrap();
        ron::de::from_reader(file).unwrap()
    };
    let objects_palette = {
        let file = File::open("res/ffi/objects.pal").unwrap();
        vangers::level::read_palette(file, None)
    };

    let mut task_pool = LocalPool::new();

    let exposed = unsafe {
        <hal::api::Gles as hal::Api>::Adapter::new_external(|name| {
            let cstr = CString::new(name).unwrap();
            (desc.gl_functor)(cstr.as_ptr())
        })
    }
    .expect("GL adapter can't be initialized");

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::empty(),
        ..Default::default()
    });
    let adapter = unsafe { instance.create_adapter_from_hal(exposed) };
    let limits = render_config.get_device_limits(&adapter.limits(), geometry_config.height);

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

    let gfx = vangers::render::GraphicsContext {
        queue,
        device,
        downlevel_caps: adapter.get_downlevel_capabilities(),
        screen_size: wgpu::Extent3d {
            width: desc.width,
            height: desc.height,
            depth_or_array_layers: 1,
        },
        color_format: wgpu::TextureFormat::Rgba8UnormSrgb,
    };
    let (color_view, depth_view) = crate_main_views(&gfx);

    let ctx = Context {
        level: None,
        geometry_config,
        render_config,
        gfx,
        color_view,
        depth_view,
        _instance: instance,
        camera: vangers::space::Camera {
            loc: cgmath::Zero::zero(),
            rot: cgmath::Zero::zero(),
            scale: cgmath::vec3(1.0, 1.0, 1.0),
            proj: vangers::space::Projection::Perspective(cgmath::PerspectiveFov {
                aspect: 1.0,
                near: 1.0,
                far: 100.0,
                fovy: cgmath::Deg(45.0).into(),
            }),
        },
        objects_palette,
        meshes: SlotMap::new(),
        instances: SlotMap::new(),
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
    ctx.gfx.screen_size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let (color_view, depth_view) = crate_main_views(&ctx.gfx);
    ctx.color_view = color_view;
    ctx.depth_view = depth_view;

    //if let vangers::space::Projection::Perspective(ref mut p) = ctx.camera.proj {
    //    p.aspect = width as f32 / height as f32;
    //}
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
    assert_eq!(t.scale, 1.0);
    ctx.camera.loc = cgmath::vec3(t.position.x, t.position.y, t.position.z);
    ctx.camera.rot =
        cgmath::Quaternion::new(t.rotation.w, t.rotation.x, t.rotation.y, t.rotation.z);
}

#[no_mangle]
pub extern "C" fn rv_map_init(ctx: &mut Context, desc: MapDescription) {
    use vangers::level::Power;

    let level_config = vangers::level::LevelConfig {
        path_palette: Default::default(),
        path_data: Default::default(),
        is_compressed: false,
        size: (
            Power::from_value(desc.width as i32),
            Power::from_value(desc.height as i32),
        ),
        geo: Power(0),
        section: Power(8),
        min_square: Power(0),
        terrains: (0..desc.material_count)
            .map(|i| unsafe {
                let mut tc = vangers::level::TerrainConfig::default();
                tc.colors.start = *desc.material_begin_offsets.offset(i as isize);
                tc.colors.end = *desc.material_end_offsets.offset(i as isize);
                tc
            })
            .collect(),
    };

    let render = vangers::render::Render::new(
        &ctx.gfx,
        &level_config,
        &ctx.objects_palette,
        &ctx.render_config,
        &ctx.geometry_config,
        ctx.camera.front_face(),
    );

    let level = vangers::level::load(&level_config, &ctx.geometry_config);
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

    lc.render
        .terrain
        .dirty_rects
        .push(vangers::render::DirtyRect {
            rect: region.to_native(),
            z_range: 0..0x100,
            need_upload: true,
        });
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
        extent: ctx.gfx.screen_size,
        depth: &ctx.depth_view,
        color: &ctx.color_view,
    };

    let mut batcher = vangers::render::Batcher::new();
    for (_, instance) in ctx.instances.iter() {
        if !instance.visible {
            continue;
        }
        batcher.add_mesh(
            &instance.mesh,
            vangers::render::object::Instance::new(&instance.transform, 1.0, instance.color_id),
        );
    }

    let mut encoder = ctx
        .gfx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("World"),
        });

    lc.render.draw_world(
        &mut encoder,
        &mut batcher,
        &lc.level,
        &ctx.camera,
        targets,
        Some(viewport.to_native()),
        &ctx.gfx.device,
    );

    ctx.gfx.queue.submit(Some(encoder.finish()));
}

fn vec_i2f(v: [i32; 3]) -> [f32; 3] {
    [v[0] as f32, v[1] as f32, v[2] as f32]
}

#[no_mangle]
pub extern "C" fn rv_model_create(
    ctx: &mut Context,
    name: *const raw::c_char,
    model: &Model,
) -> u64 {
    use vangers::render::object::Vertex as ObjectVertex;

    let owned_name;
    let label = if name.is_null() {
        "_unknown_mesh_"
    } else {
        owned_name = unsafe { CStr::from_ptr(name) }.to_string_lossy();
        &owned_name
    };

    let num_vertices = model.num_poly as usize * 3;
    log::debug!("\tGot {} GPU vertices...", num_vertices);
    let vertex_size = mem::size_of::<ObjectVertex>();
    let vertex_buf = ctx.gfx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (num_vertices * vertex_size) as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::VERTEX,
        mapped_at_creation: true,
    });
    {
        let polygons = unsafe { slice::from_raw_parts(model.polygons, model.num_poly as usize) };
        let mut mapping = vertex_buf.slice(..).get_mapped_range_mut();
        for (chunk, tri) in mapping.chunks_mut(3 * vertex_size).zip(polygons) {
            let out_vertices =
                unsafe { slice::from_raw_parts_mut(chunk.as_mut_ptr() as *mut ObjectVertex, 3) };
            for ((vo, v_ptr), n_ptr) in out_vertices.iter_mut().zip(tri.vertices).zip(tri.normals) {
                let p = unsafe { &(*v_ptr).data };
                let n = unsafe { &(*n_ptr).data };
                *vo = ObjectVertex {
                    pos: [p[0], p[1], p[2], 1],
                    color: tri.color_id as u32,
                    normal: [n[0], n[1], n[2], 0],
                };
            }
        }
    }
    vertex_buf.unmap();

    let j = &model.jacobian;
    let mesh = vangers::model::Mesh {
        num_vertices,
        vertex_buf,
        offset: vec_i2f(model.off),
        bbox: vangers::model::BoundingBox {
            min: vec_i2f(model.min),
            max: vec_i2f(model.max),
            radius: model.rmax as f32,
        },
        physics: m3d::Physics {
            volume: model.volume as f32,
            rcm: [
                model.rcm[0] as f32,
                model.rcm[1] as f32,
                model.rcm[2] as f32,
            ],
            jacobi: [
                [j[0] as f32, j[1] as f32, j[2] as f32],
                [j[3] as f32, j[4] as f32, j[5] as f32],
                [j[6] as f32, j[7] as f32, j[8] as f32],
            ],
        },
    };
    let key = ctx.meshes.insert(Arc::new(mesh));
    key.data().as_ffi()
}

#[no_mangle]
pub extern "C" fn rv_model_destroy(ctx: &mut Context, handle: u64) {
    let _ = ctx.meshes.remove(slotmap::KeyData::from_ffi(handle).into());
}

#[no_mangle]
pub extern "C" fn rv_model_instance_create(
    ctx: &mut Context,
    model_handle: u64,
    color_id: u8,
) -> u64 {
    let mesh = &ctx.meshes[slotmap::KeyData::from_ffi(model_handle).into()];
    let key = ctx.instances.insert(MeshInstance {
        mesh: Arc::clone(mesh),
        transform: cgmath::One::one(),
        color_id,
        visible: true,
    });
    key.data().as_ffi()
}

#[no_mangle]
pub extern "C" fn rv_model_instance_set_transform(
    ctx: &mut Context,
    inst_handle: u64,
    t: Transform,
) {
    let inst = &mut ctx.instances[slotmap::KeyData::from_ffi(inst_handle).into()];
    inst.transform = vangers::space::Transform {
        disp: cgmath::vec3(t.position.x, t.position.y, t.position.z),
        scale: t.scale,
        rot: cgmath::Quaternion::new(t.rotation.w, t.rotation.x, t.rotation.y, t.rotation.z),
    };
}

#[no_mangle]
pub extern "C" fn rv_model_instance_set_visible(
    ctx: &mut Context,
    inst_handle: u64,
    visible: bool,
) {
    let inst = &mut ctx.instances[slotmap::KeyData::from_ffi(inst_handle).into()];
    inst.visible = visible;
}

#[no_mangle]
pub extern "C" fn rv_model_instance_destroy(ctx: &mut Context, handle: u64) {
    let _ = ctx
        .instances
        .remove(slotmap::KeyData::from_ffi(handle).into());
}
