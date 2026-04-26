use crate::{
    config::settings,
    level, model,
    space::{Camera, Transform},
};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt as _;

use std::{collections::HashMap, io::Error as IoError, mem, ops::Range, sync::Arc};

#[cfg(not(target_arch = "wasm32"))]
use std::{
    fs::File,
    io::{BufReader, Read},
    path::PathBuf,
};

pub mod debug;
pub mod global;
pub mod object;
mod shadow;
pub mod terrain;
mod water;

pub use shadow::FORMAT as SHADOW_FORMAT;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[derive(Clone, Copy, Debug)]
pub struct VertexStorageNotSupported;

#[derive(Copy, Clone)]
pub struct GpuTransform {
    pub pos_scale: [f32; 4],
    pub orientation: [f32; 4],
}

impl GpuTransform {
    pub fn new(t: &Transform) -> Self {
        GpuTransform {
            pos_scale: [t.disp.x, t.disp.y, t.disp.z, t.scale],
            orientation: [t.rot.x, t.rot.y, t.rot.z, t.rot.w],
        }
    }
}

pub struct ScreenTargets<'a> {
    pub extent: wgpu::Extent3d,
    pub color: &'a wgpu::TextureView,
    pub depth: &'a wgpu::TextureView,
}

pub struct SurfaceData {
    pub constants: wgpu::Buffer,
    pub height: (wgpu::TextureView, wgpu::Sampler),
    pub meta: (wgpu::TextureView, wgpu::Sampler),
}

pub struct DirtyRect {
    pub rect: Rect,
    pub z_range: Range<u16>,
    pub need_upload: bool,
}

pub type ShapeVertex = [f32; 4];

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShapePolygon {
    pub indices: [u16; 4],
    pub normal: [i8; 4],
    pub origin_square: [f32; 4],
}
unsafe impl Pod for ShapePolygon {}
unsafe impl Zeroable for ShapePolygon {}

#[derive(Copy, Clone)]
pub struct ShapeVertexDesc {
    attributes: [wgpu::VertexAttribute; 3],
}

impl ShapeVertexDesc {
    pub fn new() -> Self {
        ShapeVertexDesc {
            attributes: wgpu::vertex_attr_array![0 => Uint16x4, 1 => Snorm8x4, 2 => Float32x4],
        }
    }

    pub fn buffer_desc(&self) -> wgpu::VertexBufferLayout<'_> {
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<ShapePolygon>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &self.attributes,
        }
    }
}

/// On WASM, shaders are embedded at compile time since there's no filesystem.
#[cfg(target_arch = "wasm32")]
fn embedded_shader(name: &str) -> Option<&'static str> {
    Some(match name {
        "body.inc.wgsl" => include_str!("../../res/shader/body.inc.wgsl"),
        "debug.wgsl" => include_str!("../../res/shader/debug.wgsl"),
        "globals.inc.wgsl" => include_str!("../../res/shader/globals.inc.wgsl"),
        "morton.inc.wgsl" => include_str!("../../res/shader/morton.inc.wgsl"),
        "object.wgsl" => include_str!("../../res/shader/object.wgsl"),
        "quat.inc.wgsl" => include_str!("../../res/shader/quat.inc.wgsl"),
        "shadow.inc.wgsl" => include_str!("../../res/shader/shadow.inc.wgsl"),
        "surface.inc.wgsl" => include_str!("../../res/shader/surface.inc.wgsl"),
        "water.wgsl" => include_str!("../../res/shader/water.wgsl"),
        "terrain/color.inc.wgsl" => include_str!("../../res/shader/terrain/color.inc.wgsl"),
        "terrain/locals.inc.wgsl" => include_str!("../../res/shader/terrain/locals.inc.wgsl"),
        "terrain/paint.wgsl" => include_str!("../../res/shader/terrain/paint.wgsl"),
        "terrain/ray.wgsl" => include_str!("../../res/shader/terrain/ray.wgsl"),
        "terrain/scatter.wgsl" => include_str!("../../res/shader/terrain/scatter.wgsl"),
        "terrain/slice.wgsl" => include_str!("../../res/shader/terrain/slice.wgsl"),
        "terrain/voxel-bake.wgsl" => include_str!("../../res/shader/terrain/voxel-bake.wgsl"),
        "terrain/voxel-draw.wgsl" => include_str!("../../res/shader/terrain/voxel-draw.wgsl"),
        "terrain/voxel.inc.wgsl" => include_str!("../../res/shader/terrain/voxel.inc.wgsl"),
        _ => return None,
    })
}

#[cfg(target_arch = "wasm32")]
fn read_shader_source(base: &str, name_with_ext: &str) -> Result<String, IoError> {
    let key = if base.is_empty() {
        name_with_ext.to_string()
    } else {
        format!("{}/{}", base, name_with_ext)
    };
    embedded_shader(&key).map(|s| s.to_string()).ok_or_else(|| {
        IoError::new(
            std::io::ErrorKind::NotFound,
            format!("Shader not found: {}", key),
        )
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn read_shader_source(base_path: &str, name_with_ext: &str) -> Result<String, IoError> {
    let path = PathBuf::from("res")
        .join("shader")
        .join(base_path)
        .join(name_with_ext);
    let mut source = String::new();
    BufReader::new(File::open(&path)?).read_to_string(&mut source)?;
    Ok(source)
}

pub fn make_shader_code(name: &str, substitutions: &[(&str, String)]) -> Result<String, IoError> {
    // Split "terrain/slice" into base="terrain", stem="slice"
    let (base, stem) = match name.rsplit_once('/') {
        Some((b, s)) => (b, s),
        None => ("", name),
    };

    let source = read_shader_source(base, &format!("{}.wgsl", stem))?;
    let mut buf = String::new();

    // parse meta-data: //!include directives
    {
        let mut lines = source.lines();
        let first = lines.next().unwrap();
        if first.starts_with("//!include") {
            for include in first.split_whitespace().skip(1) {
                let inc_name = format!("{}.wgsl", include);
                // Includes can be "globals.inc" (root) or "terrain/locals.inc" (subdir)
                let (inc_base, inc_file) = match inc_name.rsplit_once('/') {
                    Some((b, f)) => (b, f),
                    None => ("", inc_name.as_str()),
                };
                match read_shader_source(inc_base, inc_file) {
                    Ok(content) => buf.push_str(&content),
                    Err(e) => panic!("Unable to include {:?}: {:?}", inc_name, e),
                };
            }
        }
    }

    buf.push_str(&source);
    for &(key_inner, ref value) in substitutions {
        let key = format!("`{}`", key_inner);
        buf = buf.replace(&key, value);
    }
    Ok(buf)
}

pub fn load_shader(
    name: &str,
    substitutions: &[(&str, String)],
    device: &wgpu::Device,
) -> Result<wgpu::ShaderModule, IoError> {
    profiling::scope!("Load Shaders", name);

    let code = make_shader_code(name, substitutions)?;
    debug!("shader '{}':\n{}", name, code);
    if cfg!(debug_assertions) {
        std::fs::write("last-shader.wgsl", &code).unwrap();
    }

    Ok(device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(name),
        source: wgpu::ShaderSource::Wgsl(code.into()),
    }))
}

pub struct Palette {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

impl Palette {
    pub fn new(device: &wgpu::Device) -> Self {
        profiling::scope!("Create Palette");
        let extent = wgpu::Extent3d {
            width: 0x100,
            height: 1,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Palette"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            view_formats: &[],
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        });

        Palette {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            texture,
        }
    }

    pub fn init(&self, queue: &wgpu::Queue, data: &[[u8; 4]]) {
        queue.write_texture(
            self.texture.as_image_copy(),
            bytemuck::cast_slice(data),
            wgpu::TexelCopyBufferLayout::default(),
            wgpu::Extent3d {
                width: 0x100,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
    }
}

struct InstanceArray {
    data: Vec<object::Instance>,
    // holding the mesh alive, while the key is just a raw pointer
    mesh: Arc<model::Mesh>,
    // actual hardware buffer for this data
    buffer: Option<wgpu::Buffer>,
}

pub struct Batcher {
    instances: HashMap<*const model::Mesh, InstanceArray>,
    debug_shapes: Vec<Arc<model::Shape>>,
    debug_instances: Vec<object::Instance>,
}

impl Batcher {
    pub fn new() -> Self {
        Batcher {
            instances: HashMap::new(),
            debug_shapes: Vec::new(),
            debug_instances: Vec::new(),
        }
    }

    pub fn add_mesh(&mut self, mesh: &Arc<model::Mesh>, instance: object::Instance) {
        self.instances
            .entry(&**mesh)
            .or_insert_with(|| InstanceArray {
                data: Vec::new(),
                mesh: Arc::clone(mesh),
                buffer: None,
            })
            .data
            .push(instance);
    }

    pub fn add_model(
        &mut self,
        model: &model::VisualModel,
        base_transform: &Transform,
        debug_shape_scale: Option<f32>,
        color: object::BodyColor,
    ) {
        // body
        self.add_mesh(
            &model.body,
            object::Instance::new(base_transform, 0.0, color as u8),
        );
        if let Some(shape_scale) = debug_shape_scale {
            self.debug_shapes.push(Arc::clone(&model.shape));
            self.debug_instances.push(object::Instance::new(
                base_transform,
                shape_scale,
                color as u8,
            ));
        }

        // wheels
        for w in model.wheels.iter() {
            if let Some(ref mesh) = w.mesh {
                let transform = base_transform.concat(&Transform {
                    disp: glam::Vec3::from(mesh.offset),
                    rot: glam::Quat::IDENTITY,
                    scale: 1.0,
                });
                self.add_mesh(mesh, object::Instance::new(&transform, 0.0, color as u8));
            }
        }

        // slots
        for s in model.slots.iter() {
            if let Some(ref mesh) = s.mesh {
                let mut local = Transform {
                    disp: glam::Vec3::new(s.pos[0] as f32, s.pos[1] as f32, s.pos[2] as f32),
                    rot: glam::Quat::from_rotation_y((s.angle as f32).to_radians()),
                    scale: s.scale / base_transform.scale,
                };
                local.disp -= local.transform_vector(glam::Vec3::from(mesh.offset));
                let transform = base_transform.concat(&local);
                self.add_mesh(mesh, object::Instance::new(&transform, 0.0, color as u8));
            }
        }
    }

    pub fn prepare(&mut self, device: &wgpu::Device) {
        for array in self.instances.values_mut() {
            if !array.data.is_empty() {
                array.buffer = Some(
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("instance"),
                        contents: bytemuck::cast_slice(&array.data),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                );
            }
        }
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        for array in self.instances.values() {
            if array.data.is_empty() {
                continue;
            }
            pass.set_vertex_buffer(0, array.mesh.vertex_buf.slice(..));
            pass.set_vertex_buffer(1, array.buffer.as_ref().unwrap().slice(..));
            pass.draw(
                0..array.mesh.num_vertices as u32,
                0..array.data.len() as u32,
            );
        }
    }

    pub fn clear(&mut self) {
        for array in self.instances.values_mut() {
            array.data.clear();
            array.buffer = None;
        }
        self.debug_shapes.clear();
        self.debug_instances.clear();
    }
}

pub struct PipelineSet {
    main: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
}

#[derive(Copy, Clone)]
pub enum PipelineKind {
    Main,
    Shadow,
}

impl PipelineSet {
    pub fn select(&self, kind: PipelineKind) -> &wgpu::RenderPipeline {
        match kind {
            PipelineKind::Main => &self.main,
            PipelineKind::Shadow => &self.shadow,
        }
    }
}

#[derive(Copy, Clone)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

pub struct GraphicsContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub downlevel_caps: wgpu::DownlevelCapabilities,
    pub color_format: wgpu::TextureFormat,
    pub screen_size: wgpu::Extent3d,
}

pub struct Render {
    global: global::Context,
    pub object: object::Context,
    pub terrain: terrain::Context,
    pub water: water::Context,
    pub debug: debug::Context,
    pub shadow: Option<shadow::Shadow>,
    pub light_config: settings::Light,
    pub fog_config: settings::Fog,
    screen_size: wgpu::Extent3d,
}

impl Render {
    pub fn new(
        gfx: &GraphicsContext,
        level: &level::LevelConfig,
        object_palette: &[[u8; 4]],
        settings: &settings::Render,
        geometry: &settings::Geometry,
        front_face: wgpu::FrontFace,
    ) -> Self {
        profiling::scope!("Init Renderer");

        info!("Creating shadow...");
        let shadow = if settings.light.shadow.size != 0 {
            Some(shadow::Shadow::new(&settings.light.shadow, &gfx.device))
        } else {
            None
        };

        info!("Creating global context...");
        let global = global::Context::new(gfx, shadow.as_ref().map(|shadow| &shadow.view));
        info!("Creating terrain context...");
        let terrain = terrain::Context::new(
            gfx,
            level,
            geometry.height,
            &global,
            &settings.terrain,
            &settings.light.shadow.terrain,
        );
        info!("Creating object context...");
        let terrain_view = terrain
            .terrain_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let flood_view = terrain
            .flood
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let object = object::Context::new(
            gfx,
            front_face,
            object_palette,
            &global,
            object::SurfaceInputs {
                uniform_buf: &terrain.surface_uni_buf,
                terrain_view: &terrain_view,
                flood_view: &flood_view,
            },
        );
        info!("Creating water context...");
        let water = water::Context::new(&gfx.device, &settings.water, &global, &terrain);
        info!("Creating debug context...");
        let debug = debug::Context::new(&gfx.device, &settings.debug, &global, &object);
        info!("Renderer initialized");

        Render {
            global,
            object,
            terrain,
            water,
            debug,
            shadow,
            light_config: settings.light,
            fog_config: settings.fog,
            screen_size: gfx.screen_size,
        }
    }

    pub fn draw_ui(&mut self, ui: &mut egui::Ui) {
        let lpos = &mut self.light_config.pos;
        ui.horizontal(|ui| {
            ui.label("Sun pos");
            ui.add(egui::DragValue::new(&mut lpos[0]).speed(1.0).prefix("x:"));
            ui.add(egui::DragValue::new(&mut lpos[1]).speed(1.0).prefix("y:"));
            ui.add(egui::DragValue::new(&mut lpos[2]).speed(1.0).prefix("z:"));
        });
        ui.horizontal(|ui| {
            ui.label("Sun color");
            ui.color_edit_button_rgb(&mut self.light_config.color);
        });
        ui.horizontal(|ui| {
            ui.label("Fog color");
            ui.color_edit_button_rgb(&mut self.fog_config.color);
        });
        ui.add(egui::Slider::new(&mut self.fog_config.depth, 0.0..=100.0).text("Fog depth"));
        ui.group(|ui| {
            ui.label("Terrain:");
            self.terrain.draw_ui(ui);
        });
    }

    pub fn draw_world(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        batcher: &mut Batcher,
        level: &level::Level,
        cam: &Camera,
        targets: ScreenTargets<'_>,
        viewport: Option<Rect>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        profiling::scope!("draw_world");
        batcher.prepare(device);
        self.terrain.update_dirty(encoder, level, device);

        //TODO: common routine for draw passes
        //TODO: use `write_buffer`

        if let Some(ref mut shadow) = self.shadow {
            profiling::scope!("Shadow Pass");
            shadow.update_view(&self.light_config.pos, cam, level.geometry.height as f32);

            let constants = global::Constants::new(&shadow.cam, &self.light_config, None);
            queue.write_buffer(
                &self.global.shadow_uniform_buf,
                0,
                bytemuck::bytes_of(&constants),
            );

            self.terrain.prepare_shadow(
                encoder,
                device,
                cam,
                wgpu::Extent3d {
                    width: shadow.size,
                    height: shadow.size,
                    depth_or_array_layers: 1,
                },
            );

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &shadow.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });

            pass.set_bind_group(0, &self.global.shadow_bind_group, &[]);
            pass.push_debug_group("terrain");
            self.terrain.draw_shadow(&mut pass);
            pass.pop_debug_group();

            pass.push_debug_group("vehicles");
            pass.set_pipeline(&self.object.pipelines.shadow);
            pass.set_bind_group(1, &self.object.bind_group, &[]);
            pass.set_bind_group(2, &self.object.surface_bind_group, &[]);
            batcher.draw(&mut pass);
            pass.pop_debug_group();
        }
        // main pass
        {
            profiling::scope!("Main Pass");
            let constants = global::Constants::new(
                cam,
                &self.light_config,
                self.shadow.as_ref().map(|shadow| &shadow.cam),
            );
            queue.write_buffer(&self.global.uniform_buf, 0, bytemuck::bytes_of(&constants));

            self.terrain.prepare(
                encoder,
                device,
                &self.global,
                &self.fog_config,
                level.geometry.height,
                cam,
                viewport.unwrap_or(Rect {
                    x: 0,
                    y: 0,
                    w: self.screen_size.width as u16,
                    h: self.screen_size.height as u16,
                }),
            );
            self.water.prepare(encoder, device, cam);

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: targets.color,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear({
                            let c = self.fog_config.color;
                            wgpu::Color {
                                r: c[0] as f64,
                                g: c[1] as f64,
                                b: c[2] as f64,
                                a: 1.0,
                            }
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });

            if let Some(ref r) = viewport {
                pass.set_viewport(r.x as f32, r.y as f32, r.w as f32, r.h as f32, 0.0, 1.0);
            }

            pass.set_bind_group(0, &self.global.bind_group, &[]);
            pass.push_debug_group("terrain");
            self.terrain.draw(&mut pass);
            pass.pop_debug_group();

            pass.push_debug_group("vehicles");
            pass.set_pipeline(&self.object.pipelines.main);
            pass.set_bind_group(1, &self.object.bind_group, &[]);
            pass.set_bind_group(2, &self.object.surface_bind_group, &[]);
            batcher.draw(&mut pass);
            pass.pop_debug_group();

            pass.push_debug_group("water");
            pass.set_bind_group(1, &self.terrain.bind_group, &[]);
            self.water.draw(&mut pass);
            pass.pop_debug_group();
        }
    }

    pub fn reload(&mut self, device: &wgpu::Device) {
        info!("Reloading shaders");
        self.object.reload(device);
        self.terrain.reload(device);
        self.water.reload(device);
    }

    pub fn resize(&mut self, extent: wgpu::Extent3d, device: &wgpu::Device) {
        self.terrain.resize(extent, device);
        self.screen_size = extent;
    }

    /*
    pub fn surface_data(&self) -> SurfaceData {
        SurfaceData {
            constants: self.terrain_data.suf_constants.clone(),
            height: self.terrain_data.height.clone(),
            meta: self.terrain_data.meta.clone(),
        }
    }*/

    /*
    pub fn target_color(&self) -> gfx::handle::RenderTargetView<R, ColorFormat> {
        self.terrain_data.out_color.clone()
    }*/
}
