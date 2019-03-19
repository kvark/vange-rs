use crate::{
    config::settings,
    level,
    model,
    space::{Camera, Transform},
};

use cgmath::{Decomposed, Matrix4};
use wgpu;

use std::io::Error as IoError;
use std::mem;


//mod collision; ../TODO
mod debug;
pub mod object;

//pub use self::collision::{DebugBlit, GpuCollider, ShapeId};
pub use self::debug::{DebugPos, DebugRender, LineBuffer};


pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::D32Float;

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

pub type ShapeVertex = [f32; 4];

#[repr(C)]
#[derive(Clone, Copy)]
struct TerrainVertex {
    _pos: [i8; 4],
}

#[derive(Clone, Copy)]
pub struct ShapePolygon {
    pub indices: [u16; 4],
    pub normal: [i8; 4],
    pub origin_square: [f32; 4],
}

#[derive(Clone, Copy)]
struct SurfaceConstants {
    _tex_scale: [f32; 4],
}

#[derive(Clone, Copy)]
struct TerrainConstants {
    _scr_size: [f32; 4],
}

#[derive(Clone, Copy)]
pub struct GlobalConstants {
    _camera_pos: [f32; 4],
    _m_vp: [[f32; 4]; 4],
    _m_inv_vp: [[f32; 4]; 4],
    _light_pos: [f32; 4],
    _light_color: [f32; 4],
}

impl GlobalConstants {
    pub fn new(cam: &Camera, light: &settings::Light) -> Self {
        use cgmath::SquareMatrix;

        let mx_vp = cam.get_view_proj();
        GlobalConstants {
            _camera_pos: cam.loc.extend(1.0).into(),
            _m_vp: mx_vp.into(),
            _m_inv_vp: mx_vp.invert().unwrap().into(),
            _light_pos: light.pos,
            _light_color: light.color,
        }
    }
}

enum Terrain {
    Ray {
        pipeline: wgpu::RenderPipeline,
        vertex_buf: wgpu::Buffer,
        index_buf: wgpu::Buffer,
        num_indices: usize,
    },
    /*Tess {
        low: gfx::PipelineState<R, terrain::Meta>,
        high: gfx::PipelineState<R, terrain::Meta>,
        screen_space: bool,
    },*/
}

pub struct Updater<'a> {
    command_encoder: wgpu::CommandEncoder,
    device: &'a wgpu::Device,
}

impl<'a> Updater<'a> {
    pub fn new(device: &'a wgpu::Device) -> Self {
        Updater {
            command_encoder: device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                todo: 0,
            }),
            device,
        }
    }

    pub fn update<T: 'static + Copy>(&mut self, buffer: &wgpu::Buffer, data: &[T]) {
        let staging = self.device
            .create_buffer_mapped(data.len(), wgpu::BufferUsageFlags::TRANSFER_SRC)
            .fill_from_slice(data);
        self.command_encoder.copy_buffer_to_buffer(
            &staging,
            0,
            buffer,
            0,
            mem::size_of::<T>() as u32,
        );
    }

    pub fn finish(self) -> wgpu::CommandBuffer {
        self.command_encoder.finish()
    }
}

pub struct GlobalContext {
    pub uniform_buf: wgpu::Buffer,
    pub palette_sampler: wgpu::Sampler,
    pub bind_group: wgpu::BindGroup,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

impl GlobalContext {
    pub fn new(device: &wgpu::Device) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            bindings: &[
                wgpu::BindGroupLayoutBinding {
                    binding: 0,
                    visibility: wgpu::ShaderStageFlags::VERTEX | wgpu::ShaderStageFlags::FRAGMENT,
                    ty: wgpu::BindingType::UniformBuffer,
                },
                wgpu::BindGroupLayoutBinding { // palette sampler
                    binding: 1,
                    visibility: wgpu::ShaderStageFlags::FRAGMENT,
                    ty: wgpu::BindingType::Sampler,
                },
            ],
        });
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            size: mem::size_of::<GlobalConstants>() as u32,
            usage: wgpu::BufferUsageFlags::UNIFORM | wgpu::BufferUsageFlags::TRANSFER_DST,
        });
        let palette_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            r_address_mode: wgpu::AddressMode::ClampToEdge,
            s_address_mode: wgpu::AddressMode::ClampToEdge,
            t_address_mode: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: 0.0,
            max_anisotropy: 0,
            compare_function: wgpu::CompareFunction::Always,
            border_color: wgpu::BorderColor::TransparentBlack,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout,
            bindings: &[
                wgpu::Binding {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer {
                        buffer: &uniform_buf,
                        range: 0 .. mem::size_of::<GlobalConstants>() as u32,
                    },
                },
                wgpu::Binding {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&palette_sampler),
                },
            ],
        });

        GlobalContext {
            uniform_buf,
            palette_sampler,
            bind_group_layout,
            bind_group,
        }
    }
}


pub struct Render {
    global: GlobalContext,
    object: object::Context,
    terrain_bg: wgpu::BindGroup,
    terrain_uni_buf: wgpu::Buffer,
    terrain_pipeline_layout: wgpu::PipelineLayout,
    terrain: Terrain,
    pub light_config: settings::Light,
    pub debug: debug::DebugRender,
}

pub struct RenderModel<'a> {
    pub model: &'a model::RenderModel,
    pub transform: Transform,
    pub debug_shape_scale: Option<f32>,
}

pub struct Shaders {
    vs: wgpu::ShaderModule,
    fs: wgpu::ShaderModule,
}

pub fn read_shaders(
    name: &str,
    specialization: &[&str],
    device: &wgpu::Device,
) -> Result<Shaders, IoError> {
    use glsl_to_spirv;
    use std::fs::File;
    use std::io::{BufReader, Read, Write};
    use std::path::PathBuf;

    let path = PathBuf::from("data")
        .join("shader")
        .join(name)
        .with_extension("glsl");
    if !path.is_file() {
        panic!("Shader not found: {:?}", path);
    }

    let mut buf_vs = b"#version 450\n#define SHADER_VS\n".to_vec();
    let mut buf_fs = b"#version 450\n#define SHADER_FS\n".to_vec();

    let mut code = String::new();
    BufReader::new(File::open(&path)?)
        .read_to_string(&mut code)?;
    // parse meta-data
    {
        let mut lines = code.lines();
        let first = lines.next().unwrap();
        if first.starts_with("//!include") {
            for include_pair in first.split_whitespace().skip(1) {
                let mut temp = include_pair.split(':');
                let target = match temp.next().unwrap() {
                    "vs" => &mut buf_vs,
                    "fs" => &mut buf_fs,
                    other => panic!("Unknown target: {}", other),
                };
                let include = temp.next().unwrap();
                let inc_path = path
                    .with_file_name(include)
                    .with_extension("inc.glsl");
                BufReader::new(File::open(inc_path)?)
                    .read_to_end(target)?;
            }
        }
        let second = lines.next().unwrap();
        if second.starts_with("//!specialization") {
            for define in second.split_whitespace().skip(1) {
                let value = if specialization.contains(&define) {
                    1
                } else {
                    0
                };
                write!(buf_vs, "#define {} {}\n", define, value)?;
                write!(buf_fs, "#define {} {}\n", define, value)?;
            }
        }
    }

    write!(buf_vs, "\n{}", code
        .replace("attribute", "in")
        .replace("varying", "out")
    )?;
    write!(buf_fs, "\n{}", code
        .replace("varying", "in")
    )?;

    let str_vs = String::from_utf8_lossy(&buf_vs);
    let str_fs = String::from_utf8_lossy(&buf_fs);
    debug!("vs:\n{}", str_vs);
    debug!("fs:\n{}", str_fs);

    let mut output_vs = match glsl_to_spirv::compile(&str_vs, glsl_to_spirv::ShaderType::Vertex) {
        Ok(file) => file,
        Err(e) => {
            println!("Generated VS shader:\n{}", str_vs);
            panic!("\nUnable to compile '{}': {:?}", name, e);
        }
    };
    let mut spv_vs = Vec::new();
    output_vs.read_to_end(&mut spv_vs).unwrap();

    let mut output_fs = match glsl_to_spirv::compile(&str_fs, glsl_to_spirv::ShaderType::Fragment) {
        Ok(file) => file,
        Err(e) => {
            println!("Generated FS shader:\n{}", str_fs);
            panic!("\nUnable to compile '{}': {:?}", name, e);
        }
    };
    let mut spv_fs = Vec::new();
    output_fs.read_to_end(&mut spv_fs).unwrap();

    Ok(Shaders {
        vs: device.create_shader_module(&spv_vs),
        fs: device.create_shader_module(&spv_fs),
    })
}

pub fn init(
    device: &mut wgpu::Device,
    level: &level::Level,
    object_palette: &[[u8; 4]],
    settings: &settings::Render,
) -> Render {
    let origin = wgpu::Origin3d { x: 0.0, y: 0.0, z: 0.0 };
    let extent = wgpu::Extent3d {
        width: level.size.0 as u32,
        height: level.size.1 as u32,
        depth: 1,
    };
    let flood_extent = wgpu::Extent3d {
        width: level.size.1 as u32 >> level.flood_section_power,
        height: 1,
        depth: 1,
    };
    let table_extent = wgpu::Extent3d {
        width: level::NUM_TERRAINS as u32,
        height: 1,
        depth: 1,
    };

    let terrrain_table = level.terrains
        .iter()
        .map(|terr| [
            terr.shadow_offset,
            terr.height_shift,
            terr.colors.start,
            terr.colors.end,
        ])
        .collect::<Vec<_>>();

    let height_texture = device.create_texture(&wgpu::TextureDescriptor {
        size: extent,
        array_size: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsageFlags::SAMPLED | wgpu::TextureUsageFlags::TRANSFER_DST,
    });
    let meta_texture = device.create_texture(&wgpu::TextureDescriptor {
        size: extent,
        array_size: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Uint,
        usage: wgpu::TextureUsageFlags::SAMPLED | wgpu::TextureUsageFlags::TRANSFER_DST,
    });
    let flood_texture = device.create_texture(&wgpu::TextureDescriptor {
        size: flood_extent,
        array_size: 1,
        dimension: wgpu::TextureDimension::D1,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsageFlags::SAMPLED | wgpu::TextureUsageFlags::TRANSFER_DST,
    });
    let table_texture = device.create_texture(&wgpu::TextureDescriptor {
        size: table_extent,
        array_size: 1,
        dimension: wgpu::TextureDimension::D1,
        format: wgpu::TextureFormat::Rgba8Uint,
        usage: wgpu::TextureUsageFlags::SAMPLED | wgpu::TextureUsageFlags::TRANSFER_DST,
    });

    let height_staging = device
        .create_buffer_mapped(level.height.len(), wgpu::BufferUsageFlags::TRANSFER_SRC)
        .fill_from_slice(&level.height);
    let meta_staging = device
        .create_buffer_mapped(level.meta.len(), wgpu::BufferUsageFlags::TRANSFER_SRC)
        .fill_from_slice(&level.meta);
    let flood_staging = device
        .create_buffer_mapped(level.flood_map.len(), wgpu::BufferUsageFlags::TRANSFER_SRC)
        .fill_from_slice(&level.flood_map);
    let table_staging = device
        .create_buffer_mapped(terrrain_table.len(), wgpu::BufferUsageFlags::TRANSFER_SRC)
        .fill_from_slice(&terrrain_table);

    let mut init_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        todo: 0,
    });
    init_encoder.copy_buffer_to_texture(
        wgpu::BufferCopyView {
            buffer: &height_staging,
            offset: 0,
            row_pitch: level.size.0 as u32,
            image_height: level.size.1 as u32,
        },
        wgpu::TextureCopyView {
            texture: &height_texture,
            level: 0,
            slice: 0,
            origin,
        },
        extent,
    );
    init_encoder.copy_buffer_to_texture(
        wgpu::BufferCopyView {
            buffer: &meta_staging,
            offset: 0,
            row_pitch: level.size.0 as u32,
            image_height: level.size.1 as u32,
        },
        wgpu::TextureCopyView {
            texture: &meta_texture,
            level: 0,
            slice: 0,
            origin,
        },
        extent,
    );
    init_encoder.copy_buffer_to_texture(
        wgpu::BufferCopyView {
            buffer: &flood_staging,
            offset: 0,
            row_pitch: flood_extent.width,
            image_height: 1,
        },
        wgpu::TextureCopyView {
            texture: &flood_texture,
            level: 0,
            slice: 0,
            origin,
        },
        flood_extent,
    );
    init_encoder.copy_buffer_to_texture(
        wgpu::BufferCopyView {
            buffer: &table_staging,
            offset: 0,
            row_pitch: table_extent.width * 4,
            image_height: 1,
        },
        wgpu::TextureCopyView {
            texture: &table_texture,
            level: 0,
            slice: 0,
            origin,
        },
        table_extent,
    );
    let level_palette_view = Render::create_palette(
        &mut init_encoder, &level.palette, device
    );

    let global = GlobalContext::new(device);
    let object = object::Context::new(&mut init_encoder, device, object_palette, &global);

    device.get_queue().submit(&[
        init_encoder.finish(),
    ]);

    let repeat_nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        r_address_mode: wgpu::AddressMode::Repeat,
        s_address_mode: wgpu::AddressMode::Repeat,
        t_address_mode: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::FilterMode::Nearest,
        lod_min_clamp: 0.0,
        lod_max_clamp: 0.0,
        max_anisotropy: 0,
        compare_function: wgpu::CompareFunction::Always,
        border_color: wgpu::BorderColor::TransparentBlack,
    });
    let flood_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        r_address_mode: wgpu::AddressMode::Repeat,
        s_address_mode: wgpu::AddressMode::ClampToEdge,
        t_address_mode: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Nearest,
        lod_min_clamp: 0.0,
        lod_max_clamp: 0.0,
        max_anisotropy: 0,
        compare_function: wgpu::CompareFunction::Always,
        border_color: wgpu::BorderColor::TransparentBlack,
    });
    let table_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        r_address_mode: wgpu::AddressMode::ClampToEdge,
        s_address_mode: wgpu::AddressMode::ClampToEdge,
        t_address_mode: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::FilterMode::Nearest,
        lod_min_clamp: 0.0,
        lod_max_clamp: 0.0,
        max_anisotropy: 0,
        compare_function: wgpu::CompareFunction::Always,
        border_color: wgpu::BorderColor::TransparentBlack,
    });

    let terrain_bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        bindings: &[
            wgpu::BindGroupLayoutBinding { // surface uniforms
                binding: 0,
                visibility: wgpu::ShaderStageFlags::FRAGMENT,
                ty: wgpu::BindingType::UniformBuffer,
            },
            wgpu::BindGroupLayoutBinding { // terrain locals
                binding: 1,
                visibility: wgpu::ShaderStageFlags::FRAGMENT,
                ty: wgpu::BindingType::UniformBuffer,
            },
            wgpu::BindGroupLayoutBinding { // height map
                binding: 2,
                visibility: wgpu::ShaderStageFlags::FRAGMENT,
                ty: wgpu::BindingType::SampledTexture,
            },
            wgpu::BindGroupLayoutBinding { // meta map
                binding: 3,
                visibility: wgpu::ShaderStageFlags::FRAGMENT,
                ty: wgpu::BindingType::SampledTexture,
            },
            wgpu::BindGroupLayoutBinding { // flood map
                binding: 4,
                visibility: wgpu::ShaderStageFlags::FRAGMENT,
                ty: wgpu::BindingType::SampledTexture,
            },
            wgpu::BindGroupLayoutBinding { // table map
                binding: 5,
                visibility: wgpu::ShaderStageFlags::FRAGMENT,
                ty: wgpu::BindingType::SampledTexture,
            },
            wgpu::BindGroupLayoutBinding { // palette map
                binding: 6,
                visibility: wgpu::ShaderStageFlags::FRAGMENT,
                ty: wgpu::BindingType::SampledTexture,
            },
            wgpu::BindGroupLayoutBinding { // main sampler
                binding: 7,
                visibility: wgpu::ShaderStageFlags::FRAGMENT,
                ty: wgpu::BindingType::Sampler,
            },
            wgpu::BindGroupLayoutBinding { // flood sampler
                binding: 8,
                visibility: wgpu::ShaderStageFlags::FRAGMENT,
                ty: wgpu::BindingType::Sampler,
            },
            wgpu::BindGroupLayoutBinding { // table sampler
                binding: 9,
                visibility: wgpu::ShaderStageFlags::FRAGMENT,
                ty: wgpu::BindingType::Sampler,
            },
        ],
    });

    let surface_uni_buf = device
        .create_buffer_mapped(1, wgpu::BufferUsageFlags::UNIFORM)
        .fill_from_slice(&[SurfaceConstants {
            _tex_scale: [
                level.size.0 as f32,
                level.size.1 as f32,
                level::HEIGHT_SCALE as f32,
                0.0,
            ],
        }]);
    let terrain_uni_buf = device.create_buffer(&wgpu::BufferDescriptor {
        size: mem::size_of::<TerrainConstants>() as u32,
        usage: wgpu::BufferUsageFlags::UNIFORM | wgpu::BufferUsageFlags::TRANSFER_DST,
    });

    let terrain_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &terrain_bg_layout,
        bindings: &[
            wgpu::Binding {
                binding: 0,
                resource: wgpu::BindingResource::Buffer {
                    buffer: &surface_uni_buf,
                    range: 0 .. mem::size_of::<SurfaceConstants>() as u32,
                },
            },
            wgpu::Binding {
                binding: 1,
                resource: wgpu::BindingResource::Buffer {
                    buffer: &terrain_uni_buf,
                    range: 0 .. mem::size_of::<TerrainConstants>() as u32,
                },
            },
            wgpu::Binding {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(
                    &height_texture.create_default_view(),
                ),
            },
            wgpu::Binding {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(
                    &meta_texture.create_default_view(),
                ),
            },
            wgpu::Binding {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(
                    &flood_texture.create_default_view(),
                ),
            },
            wgpu::Binding {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(
                    &table_texture.create_default_view(),
                ),
            },
            wgpu::Binding {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(&level_palette_view),
            },
            wgpu::Binding {
                binding: 7,
                resource: wgpu::BindingResource::Sampler(&repeat_nearest_sampler),
            },
            wgpu::Binding {
                binding: 8,
                resource: wgpu::BindingResource::Sampler(&flood_sampler),
            },
            wgpu::Binding {
                binding: 9,
                resource: wgpu::BindingResource::Sampler(&table_sampler),
            },
        ],
    });

    let terrain_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        bind_group_layouts: &[
            &global.bind_group_layout,
            &terrain_bg_layout,
        ],
    });

    let vertices = [
        TerrainVertex { _pos: [0, 0, 0, 1] },
        TerrainVertex { _pos: [-1, 0, 0, 0] },
        TerrainVertex { _pos: [0, -1, 0, 0] },
        TerrainVertex { _pos: [1, 0, 0, 0] },
        TerrainVertex { _pos: [0, 1, 0, 0] },
    ];
    let indices = [0u16, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 1];

    let terrain_vertex_buf = device
        .create_buffer_mapped(vertices.len(), wgpu::BufferUsageFlags::VERTEX)
        .fill_from_slice(&vertices);
    let terrain_index_buf = device
        .create_buffer_mapped(indices.len(), wgpu::BufferUsageFlags::INDEX)
        .fill_from_slice(&indices);

    let terrain_pipeline = Render::create_terrain_ray_pipeline(&terrain_pipeline_layout, device);

    Render {
        global,
        object,
        terrain_bg,
        terrain_uni_buf,
        terrain_pipeline_layout,
        terrain: Terrain::Ray {
            pipeline: terrain_pipeline,
            vertex_buf: terrain_vertex_buf,
            index_buf: terrain_index_buf,
            num_indices: indices.len(),
        },
        light_config: settings.light.clone(),
        debug: DebugRender::new(device, &settings.debug),
    }
}

impl Render {
    pub fn draw_mesh(
        pass: &mut wgpu::RenderPass,
        updater: &mut Updater,
        mesh: &model::Mesh,
        model2world: Transform,
        locals_buf: &wgpu::Buffer,
    ) {
        let mx_world = Matrix4::from(model2world);
        updater.update(locals_buf, &[object::Locals {
            matrix: mx_world.into(),
        }]);
        pass.set_vertex_buffers(&[(&mesh.vertex_buf, 0)]);
        pass.draw(0 .. mesh.num_vertices as u32, 0 .. 1);
    }

    pub fn draw_model(
        pass: &mut wgpu::RenderPass,
        updater: &mut Updater,
        model: &model::RenderModel,
        model2world: Transform,
        locals_buf: &wgpu::Buffer,
        debug_context: Option<(&mut DebugRender, f32, &Matrix4<f32>)>,
    ) {
        use cgmath::{Deg, One, Quaternion, Rad, Rotation3, Transform, Vector3};

        // body
        Render::draw_mesh(pass, updater, &model.body, model2world.clone(), locals_buf);
        // debug render
        if let Some((debug, scale, world2screen)) = debug_context {
            let mut mx_shape =  model2world.clone();
            mx_shape.scale *= scale;
            let transform = world2screen * Matrix4::from(mx_shape);
            debug.draw_shape(pass, &model.shape, transform);
        }
        // wheels
        for w in model.wheels.iter() {
            if let Some(ref mesh) = w.mesh {
                let transform = model2world.concat(&Decomposed {
                    disp: mesh.offset.into(),
                    rot: Quaternion::one(),
                    scale: 1.0,
                });
                Render::draw_mesh(pass, updater, mesh, transform, locals_buf);
            }
        }
        // slots
        for s in model.slots.iter() {
            if let Some(ref mesh) = s.mesh {
                let mut local = Decomposed {
                    disp: Vector3::new(s.pos[0] as f32, s.pos[1] as f32, s.pos[2] as f32),
                    rot: Quaternion::from_angle_y(Rad::from(Deg(s.angle as f32))),
                    scale: s.scale / model2world.scale,
                };
                local.disp -= local.transform_vector(Vector3::from(mesh.offset));
                let transform = model2world.concat(&local);
                Render::draw_mesh(pass, updater, mesh, transform, locals_buf);
            }
        }
    }

    pub fn draw_world<'a>(
        &mut self,
        render_models: &[RenderModel<'a>],
        cam: &Camera,
        targets: ScreenTargets,
        device: &wgpu::Device,
    ) -> Vec<wgpu::CommandBuffer> {
        let mx_vp = cam.get_view_proj();

        let mut updater = Updater::new(device);
        updater.update(&self.global.uniform_buf, &[
            GlobalConstants::new(cam, &self.light_config),
        ]);
        updater.update(&self.terrain_uni_buf, &[TerrainConstants {
            _scr_size: [targets.extent.width as f32, targets.extent.height as f32, 0.0, 0.0],
        }]);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            todo: 0,
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[
                    wgpu::RenderPassColorAttachmentDescriptor {
                        attachment: targets.color,
                        load_op: wgpu::LoadOp::Clear,
                        store_op: wgpu::StoreOp::Store,
                        clear_color: wgpu::Color {
                            r: 0.1, g: 0.2, b: 0.3, a: 1.0,
                        },
                    },
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachmentDescriptor {
                    attachment: targets.depth,
                    depth_load_op: wgpu::LoadOp::Clear,
                    depth_store_op: wgpu::StoreOp::Store,
                    clear_depth: 1.0,
                    stencil_load_op: wgpu::LoadOp::Clear,
                    stencil_store_op: wgpu::StoreOp::Store,
                    clear_stencil: 0,
                }),
            });

            pass.set_bind_group(0, &self.global.bind_group);
            pass.set_bind_group(1, &self.terrain_bg);
            // draw terrain
            match self.terrain {
                Terrain::Ray { ref pipeline, ref index_buf, ref vertex_buf, num_indices } => {
                    pass.set_pipeline(pipeline);
                    pass.set_index_buffer(index_buf, 0);
                    pass.set_vertex_buffers(&[(vertex_buf, 0)]);
                    pass.draw_indexed(0 .. num_indices as u32, 0, 0 .. 1);
                }
                /*
                Terrain::Tess { ref low, ref high, .. } => {
                    encoder.draw(&self.terrain_slice, low, &self.terrain_data);
                    encoder.draw(&self.terrain_slice, high, &self.terrain_data);
                }*/
            }

            pass.set_pipeline(&self.object.pipeline);
            pass.set_bind_group(1, &self.object.bind_group);
            // draw vehicle models
            for rm in render_models {
                Render::draw_model(
                    &mut pass,
                    &mut updater,
                    &rm.model,
                    rm.transform,
                    &self.object.uniform_buf,
                    //&mut self.object_data,
                    match rm.debug_shape_scale {
                        Some(scale) => Some((&mut self.debug, scale, &mx_vp)),
                        None => None,
                    },
                );
            }
        }

        vec![
            updater.finish(),
            encoder.finish(),
        ]
    }

    pub fn create_palette(
        encoder: &mut wgpu::CommandEncoder,
        data: &[[u8; 4]],
        device: &wgpu::Device,
    ) -> wgpu::TextureView {
        let extent = wgpu::Extent3d {
            width: 0x100,
            height: 1,
            depth: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            size: extent,
            array_size: 1,
            dimension: wgpu::TextureDimension::D1,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsageFlags::SAMPLED | wgpu::TextureUsageFlags::TRANSFER_DST,
        });

        let staging = device
            .create_buffer_mapped(data.len(), wgpu::BufferUsageFlags::TRANSFER_SRC)
            .fill_from_slice(data);
        encoder.copy_buffer_to_texture(
            wgpu::BufferCopyView {
                buffer: &staging,
                offset: 0,
                row_pitch: 0x100 * 4,
                image_height: 1,
            },
            wgpu::TextureCopyView {
                texture: &texture,
                level: 0,
                slice: 0,
                origin: wgpu::Origin3d {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
            },
            extent,
        );

        texture.create_default_view()
    }

    fn create_terrain_ray_pipeline(
        layout: &wgpu::PipelineLayout,
        device: &wgpu::Device,
    ) -> wgpu::RenderPipeline {
        let shaders = read_shaders("terrain_ray", &[], device)
            .unwrap();
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            layout,
            vertex_stage: wgpu::PipelineStageDescriptor {
                module: &shaders.vs,
                entry_point: "main",
            },
            fragment_stage: wgpu::PipelineStageDescriptor {
                module: &shaders.fs,
                entry_point: "main",
            },
            rasterization_state: wgpu::RasterizationStateDescriptor {
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: wgpu::CullMode::None,
                depth_bias: 0,
                depth_bias_slope_scale: 0.0,
                depth_bias_clamp: 0.0,
            },
            primitive_topology: wgpu::PrimitiveTopology::TriangleList,
            color_states: &[
                wgpu::ColorStateDescriptor {
                    format: COLOR_FORMAT,
                    alpha: wgpu::BlendDescriptor::REPLACE,
                    color: wgpu::BlendDescriptor::REPLACE,
                    write_mask: wgpu::ColorWriteFlags::all(),
                },
            ],
            depth_stencil_state: Some(wgpu::DepthStencilStateDescriptor {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Always,
                stencil_front: wgpu::StencilStateFaceDescriptor::IGNORE,
                stencil_back: wgpu::StencilStateFaceDescriptor::IGNORE,
                stencil_read_mask: !0,
                stencil_write_mask: !0,
            }),
            index_format: wgpu::IndexFormat::Uint16,
            vertex_buffers: &[
                wgpu::VertexBufferDescriptor {
                    stride: mem::size_of::<TerrainVertex>() as u32,
                    step_mode: wgpu::InputStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttributeDescriptor {
                            offset: 0,
                            format: wgpu::VertexFormat::Char4,
                            attribute_index: 0,
                        },
                    ],
                },
            ],
            sample_count: 1,
        })
    }

    pub fn reload(&mut self, device: &wgpu::Device) {
        info!("Reloading shaders");
        match self.terrain {
            Terrain::Ray { ref mut pipeline, .. } => {
                *pipeline = Render::create_terrain_ray_pipeline(
                    &self.terrain_pipeline_layout,
                    device,
                );
            }
            /*
            Terrain::Tess { ref mut low, ref mut high, screen_space } => {
                let (lo, hi) = Render::create_terrain_tess_psos(factory, screen_space);
                *low = lo;
                *high = hi;
            }*/
        }
        self.object.reload(device);
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
