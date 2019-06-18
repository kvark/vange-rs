use crate::{
    config::settings::Terrain as TerrainSettings,
    level,
    render::{
        Palette, Shaders,
        COLOR_FORMAT, DEPTH_FORMAT,
        global::Context as GlobalContext,
        mipmap::MaxMipper,
    },
    space::Camera,
};

use wgpu;

use std::mem;

pub const HEIGHT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
const SCATTER_GROUP_SIZE: [u32; 3] = [16, 16, 1];

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    _pos: [i8; 4],
}

#[derive(Clone, Copy)]
struct SurfaceConstants {
    _tex_scale: [f32; 4],
}

#[derive(Clone, Copy)]
struct Constants {
    _scr_size: [u32; 4],
    _params: [u32; 4],
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
struct ScatterConstants {
    origin: [f32; 2],
    dir: [f32; 2],
    y_range: [f32; 2],
    x_range: [f32; 2],
}

fn compute_scatter_constants(cam: &Camera) -> ScatterConstants {
    use cgmath::{prelude::*, Point2, Point3, Vector2, Vector3};

    let cam_origin = Point2::new(cam.loc.x, cam.loc.y);
    let cam_dir = {
        let vec = cam.rot.rotate_vector(Vector3::unit_z());
        let v2 = Vector2::new(vec.x, vec.y);
        if v2.magnitude2() > 0.0 {
            v2.normalize()
        } else {
            Vector2::new(0.0, 1.0)
        }
    };

    fn intersect(base: &Vector3<f32>, target: Point3<f32>, height: u32) -> Point2<f32> {
        let dir = target.to_vec() - *base;
        let t = if dir.z == 0.0 {
            0.0
        } else {
            (height as f32 - base.z)/dir.z
        };
        let end = base + dir * t.max(0.0);
        Point2::new(end.x, end.y)
    }

    let mx_invp = cam.get_view_proj().invert().unwrap();
    let y_center = {
        let center = mx_invp
            .transform_point(Point3::new(0.0, 0.0, 0.0));
        let center_base = intersect(&cam.loc, center, 0);
        (center_base - cam_origin).dot(cam_dir)
    };
    let mut y_range = y_center .. y_center;
    let mut x0 = 0f32 .. 0.0;
    let mut x1 = 0f32 .. 0.0;

    let local_positions = [
        Point3::new(1.0, 1.0, 0.0),
        Point3::new(-1.0, 1.0, 0.0),
        Point3::new(1.0, -1.0, 0.0),
        Point3::new(-1.0, -1.0, 0.0),
    ];

    for &lp in &local_positions {
        let wp = mx_invp.transform_point(lp);
        let pa = intersect(&cam.loc, wp, 0);
        let pb = intersect(&cam.loc, wp, level::HEIGHT_SCALE);
        for p in &[pa, pb] {
            let dir = *p - cam_origin;
            let y = dir.dot(cam_dir);
            y_range.start = y_range.start.min(y);
            y_range.end = y_range.end.max(y);
            let x = dir.x * cam_dir.y - dir.y * cam_dir.x;
            let range = if y > y_center { &mut x1 } else { &mut x0 };
            range.start = range.start.min(x);
            range.end = range.end.max(x);
        }
    }

    ScatterConstants {
        origin: cam_origin.into(),
        dir: cam_dir.into(),
        y_range: [y_range.start, y_range.end],
        x_range: [x0.end.max(-x0.start), x1.end.max(-x1.start)],
    }
}

struct Geometry {
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    num_indices: usize,
}

impl Geometry {
    fn new(vertices: &[Vertex], indices: &[u16], device: &wgpu::Device) -> Self {
        Geometry {
            vertex_buf: device
                .create_buffer_mapped(vertices.len(), wgpu::BufferUsage::VERTEX)
                .fill_from_slice(&vertices),
            index_buf: device
                .create_buffer_mapped(indices.len(), wgpu::BufferUsage::INDEX)
                .fill_from_slice(&indices),
            num_indices: indices.len(),
        }
    }
}

enum Kind {
    Ray {
        pipeline: wgpu::RenderPipeline,
        geo: Geometry,
    },
    RayMip {
        pipeline: wgpu::RenderPipeline,
        geo: Geometry,
        mipper: MaxMipper,
        params: [u32; 4],
    },
    /*Tess {
        low: gfx::PipelineState<R, terrain::Meta>,
        high: gfx::PipelineState<R, terrain::Meta>,
        screen_space: bool,
    },*/
    Slice {
        pipeline: wgpu::RenderPipeline,
        geo: Geometry,
    },
    Scatter {
        pipeline_layout: wgpu::PipelineLayout,
        bg_layout: wgpu::BindGroupLayout,
        scatter_pipeline: wgpu::ComputePipeline,
        clear_pipeline: wgpu::ComputePipeline,
        copy_pipeline: wgpu::RenderPipeline,
        bind_group: wgpu::BindGroup,
        constant_buf: wgpu::Buffer,
        compute_groups: [u32; 3],
        density: [u32; 3],
    },
}

pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

pub struct Context {
    pub uniform_buf: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    pipeline_layout: wgpu::PipelineLayout,
    kind: Kind,
    dirty_rects: Vec<Rect>,
    pending_resize: Option<wgpu::Extent3d>,
}

impl Context {
    fn create_ray_pipeline(
        layout: &wgpu::PipelineLayout,
        device: &wgpu::Device,
        name: &str,
    ) -> wgpu::RenderPipeline {
        let shaders = Shaders::new(name, &[], device)
            .unwrap();
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            layout,
            vertex_stage: wgpu::PipelineStageDescriptor {
                module: &shaders.vs,
                entry_point: "main",
            },
            fragment_stage: Some(wgpu::PipelineStageDescriptor {
                module: &shaders.fs,
                entry_point: "main",
            }),
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
                    alpha_blend: wgpu::BlendDescriptor::REPLACE,
                    color_blend: wgpu::BlendDescriptor::REPLACE,
                    write_mask: wgpu::ColorWrite::all(),
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
                    stride: mem::size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::InputStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttributeDescriptor {
                            offset: 0,
                            format: wgpu::VertexFormat::Char4,
                            shader_location: 0,
                        },
                    ],
                },
            ],
            sample_count: 1,
        })
    }

    fn create_slice_pipeline(
        layout: &wgpu::PipelineLayout,
        device: &wgpu::Device,
    ) -> wgpu::RenderPipeline {
        let shaders = Shaders::new("terrain/slice", &[], device)
            .unwrap();
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            layout,
            vertex_stage: wgpu::PipelineStageDescriptor {
                module: &shaders.vs,
                entry_point: "main",
            },
            fragment_stage: Some(wgpu::PipelineStageDescriptor {
                module: &shaders.fs,
                entry_point: "main",
            }),
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
                    alpha_blend: wgpu::BlendDescriptor::REPLACE,
                    color_blend: wgpu::BlendDescriptor::REPLACE,
                    write_mask: wgpu::ColorWrite::all(),
                },
            ],
            depth_stencil_state: Some(wgpu::DepthStencilStateDescriptor {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil_front: wgpu::StencilStateFaceDescriptor::IGNORE,
                stencil_back: wgpu::StencilStateFaceDescriptor::IGNORE,
                stencil_read_mask: !0,
                stencil_write_mask: !0,
            }),
            index_format: wgpu::IndexFormat::Uint16,
            vertex_buffers: &[
                wgpu::VertexBufferDescriptor {
                    stride: mem::size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::InputStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttributeDescriptor {
                            offset: 0,
                            format: wgpu::VertexFormat::Char4,
                            shader_location: 0,
                        },
                    ],
                },
            ],
            sample_count: 1,
        })
    }

    fn create_scatter_pipelines(
        layout: &wgpu::PipelineLayout,
        device: &wgpu::Device,
    ) -> (wgpu::ComputePipeline, wgpu::ComputePipeline, wgpu::RenderPipeline) {
        let scatter_shader = Shaders::new_compute("terrain/scatter", SCATTER_GROUP_SIZE, &[], device)
            .unwrap();
        let scatter_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            layout,
            compute_stage: wgpu::PipelineStageDescriptor {
                module: &scatter_shader,
                entry_point: "main",
            },
        });
        let clear_shader = Shaders::new_compute("terrain/scatter_clear", SCATTER_GROUP_SIZE, &[], device)
            .unwrap();
        let clear_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            layout,
            compute_stage: wgpu::PipelineStageDescriptor {
                module: &clear_shader,
                entry_point: "main",
            },
        });

        let copy_shaders = Shaders::new("terrain/scatter_copy", &[], device)
            .unwrap();
        let copy_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            layout,
            vertex_stage: wgpu::PipelineStageDescriptor {
                module: &copy_shaders.vs,
                entry_point: "main",
            },
            fragment_stage: Some(wgpu::PipelineStageDescriptor {
                module: &copy_shaders.fs,
                entry_point: "main",
            }),
            rasterization_state: wgpu::RasterizationStateDescriptor {
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: wgpu::CullMode::None,
                depth_bias: 0,
                depth_bias_slope_scale: 0.0,
                depth_bias_clamp: 0.0,
            },
            primitive_topology: wgpu::PrimitiveTopology::TriangleStrip,
            color_states: &[
                wgpu::ColorStateDescriptor {
                    format: COLOR_FORMAT,
                    alpha_blend: wgpu::BlendDescriptor::REPLACE,
                    color_blend: wgpu::BlendDescriptor::REPLACE,
                    write_mask: wgpu::ColorWrite::all(),
                },
            ],
            depth_stencil_state: Some(wgpu::DepthStencilStateDescriptor {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil_front: wgpu::StencilStateFaceDescriptor::IGNORE,
                stencil_back: wgpu::StencilStateFaceDescriptor::IGNORE,
                stencil_read_mask: !0,
                stencil_write_mask: !0,
            }),
            index_format: wgpu::IndexFormat::Uint16,
            vertex_buffers: &[],
            sample_count: 1,
        });

        (scatter_pipeline, clear_pipeline, copy_pipeline)
    }

    fn create_scatter_resources(
        extent: wgpu::Extent3d,
        constant_buf: &wgpu::Buffer,
        layout: &wgpu::BindGroupLayout,
        device: &wgpu::Device,
    ) -> (wgpu::BindGroup, [u32; 3]) {
        let size = 4 * (extent.width * extent.height) as wgpu::BufferAddress;
        let storage_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            size,
            usage: wgpu::BufferUsage::STORAGE,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout,
            bindings: &[
                wgpu::Binding {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer {
                        buffer: &storage_buffer,
                        range: 0 .. size,
                    },
                },
                wgpu::Binding {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer {
                        buffer: constant_buf,
                        range: 0 .. mem::size_of::<ScatterConstants>() as wgpu::BufferAddress,
                    },
                },
            ],
        });

        let group_count = [
            (extent.width / SCATTER_GROUP_SIZE[0]) + (extent.width % SCATTER_GROUP_SIZE[0]).min(1),
            (extent.height / SCATTER_GROUP_SIZE[1]) + (extent.height % SCATTER_GROUP_SIZE[1]).min(1),
            1,
        ];
        (bind_group, group_count)
    }

    pub fn new(
        init_encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
        level: &level::Level,
        global: &GlobalContext,
        config: &TerrainSettings,
        screen_extent: wgpu::Extent3d,
    ) -> Self {
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
        let (terrain_mip_count, terrain_extra_usage) = match *config {
            TerrainSettings::RayMipTraced { mip_count, .. } =>
                (mip_count, wgpu::TextureUsage::OUTPUT_ATTACHMENT),
            _ => (1, wgpu::TextureUsage::empty()),
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
            array_layer_count: 1,
            mip_level_count: terrain_mip_count as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HEIGHT_FORMAT,
            usage: wgpu::TextureUsage::SAMPLED | wgpu::TextureUsage::TRANSFER_DST | terrain_extra_usage,
        });
        let meta_texture = device.create_texture(&wgpu::TextureDescriptor {
            size: extent,
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Uint,
            usage: wgpu::TextureUsage::SAMPLED | wgpu::TextureUsage::TRANSFER_DST,
        });
        let flood_texture = device.create_texture(&wgpu::TextureDescriptor {
            size: flood_extent,
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D1,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsage::SAMPLED | wgpu::TextureUsage::TRANSFER_DST,
        });
        let table_texture = device.create_texture(&wgpu::TextureDescriptor {
            size: table_extent,
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D1,
            format: wgpu::TextureFormat::Rgba8Uint,
            usage: wgpu::TextureUsage::SAMPLED | wgpu::TextureUsage::TRANSFER_DST,
        });

        let height_staging = device
            .create_buffer_mapped(level.height.len(), wgpu::BufferUsage::TRANSFER_SRC)
            .fill_from_slice(&level.height);
        let meta_staging = device
            .create_buffer_mapped(level.meta.len(), wgpu::BufferUsage::TRANSFER_SRC)
            .fill_from_slice(&level.meta);
        let flood_staging = device
            .create_buffer_mapped(level.flood_map.len(), wgpu::BufferUsage::TRANSFER_SRC)
            .fill_from_slice(&level.flood_map);
        let table_staging = device
            .create_buffer_mapped(terrrain_table.len(), wgpu::BufferUsage::TRANSFER_SRC)
            .fill_from_slice(&terrrain_table);

        init_encoder.copy_buffer_to_texture(
            wgpu::BufferCopyView {
                buffer: &height_staging,
                offset: 0,
                row_pitch: level.size.0 as u32,
                image_height: level.size.1 as u32,
            },
            wgpu::TextureCopyView {
                texture: &height_texture,
                mip_level: 0,
                array_layer: 0,
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
                mip_level: 0,
                array_layer: 0,
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
                mip_level: 0,
                array_layer: 0,
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
                mip_level: 0,
                array_layer: 0,
                origin,
            },
            table_extent,
        );
        let palette = Palette::new(init_encoder, device, &level.palette);

        let repeat_nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: 0.0,
            compare_function: wgpu::CompareFunction::Always,
        });
        let flood_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: 0.0,
            compare_function: wgpu::CompareFunction::Always,
        });
        let table_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: 0.0,
            compare_function: wgpu::CompareFunction::Always,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            bindings: &[
                wgpu::BindGroupLayoutBinding { // surface uniforms
                    binding: 0,
                    visibility: wgpu::ShaderStage::all(),
                    ty: wgpu::BindingType::UniformBuffer,
                },
                wgpu::BindGroupLayoutBinding { // terrain locals
                    binding: 1,
                    visibility: wgpu::ShaderStage::FRAGMENT | wgpu::ShaderStage::COMPUTE,
                    ty: wgpu::BindingType::UniformBuffer,
                },
                wgpu::BindGroupLayoutBinding { // height map
                    binding: 2,
                    visibility: wgpu::ShaderStage::FRAGMENT | wgpu::ShaderStage::COMPUTE,
                    ty: wgpu::BindingType::SampledTexture,
                },
                wgpu::BindGroupLayoutBinding { // meta map
                    binding: 3,
                    visibility: wgpu::ShaderStage::FRAGMENT | wgpu::ShaderStage::COMPUTE,
                    ty: wgpu::BindingType::SampledTexture,
                },
                wgpu::BindGroupLayoutBinding { // flood map
                    binding: 4,
                    visibility: wgpu::ShaderStage::FRAGMENT | wgpu::ShaderStage::COMPUTE,
                    ty: wgpu::BindingType::SampledTexture,
                },
                wgpu::BindGroupLayoutBinding { // table map
                    binding: 5,
                    visibility: wgpu::ShaderStage::FRAGMENT | wgpu::ShaderStage::COMPUTE,
                    ty: wgpu::BindingType::SampledTexture,
                },
                wgpu::BindGroupLayoutBinding { // palette map
                    binding: 6,
                    visibility: wgpu::ShaderStage::FRAGMENT,
                    ty: wgpu::BindingType::SampledTexture,
                },
                wgpu::BindGroupLayoutBinding { // main sampler
                    binding: 7,
                    visibility: wgpu::ShaderStage::FRAGMENT | wgpu::ShaderStage::COMPUTE,
                    ty: wgpu::BindingType::Sampler,
                },
                wgpu::BindGroupLayoutBinding { // flood sampler
                    binding: 8,
                    visibility: wgpu::ShaderStage::FRAGMENT | wgpu::ShaderStage::COMPUTE,
                    ty: wgpu::BindingType::Sampler,
                },
                wgpu::BindGroupLayoutBinding { // table sampler
                    binding: 9,
                    visibility: wgpu::ShaderStage::FRAGMENT,
                    ty: wgpu::BindingType::Sampler,
                },
            ],
        });

        let surface_uni_buf = device
            .create_buffer_mapped(1, wgpu::BufferUsage::UNIFORM)
            .fill_from_slice(&[SurfaceConstants {
                _tex_scale: [
                    level.size.0 as f32,
                    level.size.1 as f32,
                    level::HEIGHT_SCALE as f32,
                    0.0,
                ],
            }]);
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            size: mem::size_of::<Constants>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsage::UNIFORM | wgpu::BufferUsage::TRANSFER_DST,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout,
            bindings: &[
                wgpu::Binding {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer {
                        buffer: &surface_uni_buf,
                        range: 0 .. mem::size_of::<SurfaceConstants>() as wgpu::BufferAddress,
                    },
                },
                wgpu::Binding {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer {
                        buffer: &uniform_buf,
                        range: 0 .. mem::size_of::<Constants>() as wgpu::BufferAddress,
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
                    resource: wgpu::BindingResource::TextureView(&palette.view),
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            bind_group_layouts: &[
                &global.bind_group_layout,
                &bind_group_layout,
            ],
        });

        let kind = match *config {
            TerrainSettings::RayTraced => {
                let geo = Geometry::new(
                    &[
                        Vertex { _pos: [0, 0, 0, 1] },
                        Vertex { _pos: [-1, 0, 0, 0] },
                        Vertex { _pos: [0, -1, 0, 0] },
                        Vertex { _pos: [1, 0, 0, 0] },
                        Vertex { _pos: [0, 1, 0, 0] },
                    ],
                    &[0u16, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 1],
                    device,
                );

                let pipeline = Self::create_ray_pipeline(
                    &pipeline_layout,
                    device,
                    "terrain/ray",
                );
                Kind::Ray {
                    pipeline,
                    geo,
                }
            }
            TerrainSettings::RayMipTraced { mip_count, max_jumps, max_steps, debug } => {
                let geo = Geometry::new(
                    &[
                        Vertex { _pos: [0, 0, 0, 1] },
                        Vertex { _pos: [-1, 0, 0, 0] },
                        Vertex { _pos: [0, -1, 0, 0] },
                        Vertex { _pos: [1, 0, 0, 0] },
                        Vertex { _pos: [0, 1, 0, 0] },
                    ],
                    &[0u16, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 1],
                    device,
                );

                let pipeline = Self::create_ray_pipeline(
                    &pipeline_layout,
                    device,
                    "terrain/ray_mip",
                );
                let mipper = MaxMipper::new(&height_texture, extent, mip_count, device);

                Kind::RayMip {
                    pipeline,
                    geo,
                    mipper,
                    params: [
                        mip_count - 1,
                        max_jumps,
                        max_steps,
                        if debug { 1 } else { 0 },
                    ],
                }
            }
            TerrainSettings::Tessellated { .. } => unimplemented!(),
            TerrainSettings::Sliced => {
                let geo = Geometry::new(
                    &[
                        Vertex { _pos: [-1, -1, 0, 1] },
                        Vertex { _pos: [1, -1, 0, 1] },
                        Vertex { _pos: [1, 1, 0, 1] },
                        Vertex { _pos: [-1, 1, 0, 1] },
                    ],
                    &[0u16, 1, 2, 0, 2, 3],
                    device,
                );

                let pipeline = Self::create_slice_pipeline(&pipeline_layout, device);

                Kind::Slice {
                    pipeline,
                    geo,
                }
            }
            TerrainSettings::Scattered { density } => {
                let local_bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    bindings: &[
                        wgpu::BindGroupLayoutBinding { // output map
                            binding: 0,
                            visibility: wgpu::ShaderStage::FRAGMENT | wgpu::ShaderStage::COMPUTE,
                            ty: wgpu::BindingType::StorageBuffer,
                        },
                        wgpu::BindGroupLayoutBinding { // constants
                            binding: 1,
                            visibility: wgpu::ShaderStage::COMPUTE,
                            ty: wgpu::BindingType::UniformBuffer,
                        },
                    ],
                });
                let local_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    bind_group_layouts: &[
                        &global.bind_group_layout,
                        &bind_group_layout,
                        &local_bg_layout,
                    ],
                });

                let (scatter_pipeline, clear_pipeline, copy_pipeline) =
                    Self::create_scatter_pipelines(&local_pipeline_layout, device);
                let constant_buf = device.create_buffer(&wgpu::BufferDescriptor {
                    size: mem::size_of::<ScatterConstants>() as wgpu::BufferAddress,
                    usage: wgpu::BufferUsage::UNIFORM | wgpu::BufferUsage::TRANSFER_DST,
                });
                let (local_bg, compute_groups) = Self::create_scatter_resources(
                    screen_extent,
                    &constant_buf,
                    &local_bg_layout,
                    device,
                );
                Kind::Scatter {
                    pipeline_layout: local_pipeline_layout,
                    bg_layout: local_bg_layout,
                    scatter_pipeline,
                    clear_pipeline,
                    copy_pipeline,
                    bind_group: local_bg,
                    constant_buf,
                    compute_groups,
                    density,
                }
            }
        };

        Context {
            uniform_buf,
            bind_group,
            pipeline_layout,
            kind,
            dirty_rects: vec![
                Rect {
                    x: 0,
                    y: 0,
                    w: level.size.0 as u16,
                    h: level.size.1 as u16,
                },
            ],
            pending_resize: Some(screen_extent),
        }
    }

    pub fn reload(&mut self, device: &wgpu::Device) {
        match self.kind {
            Kind::Ray { ref mut pipeline, .. } => {
                *pipeline = Self::create_ray_pipeline(
                    &self.pipeline_layout,
                    device,
                    "terrain/ray",
                );
            }
            Kind::RayMip { ref mut pipeline, ref mut mipper, .. } => {
                *pipeline = Self::create_ray_pipeline(
                    &self.pipeline_layout,
                    device,
                    "terrain/ray_mip",
                );
                mipper.reload(device);
            }
            /*
            Terrain::Tess { ref mut low, ref mut high, screen_space } => {
                let (lo, hi) = Render::create_terrain_tess_psos(factory, screen_space);
                *low = lo;
                *high = hi;
            }*/
            Kind::Slice { ref mut pipeline, .. } => {
                *pipeline = Self::create_slice_pipeline(
                    &self.pipeline_layout,
                    device,
                );
            }
            Kind::Scatter {
                ref pipeline_layout,
                ref mut scatter_pipeline,
                ref mut clear_pipeline,
                ref mut copy_pipeline,
                ..
            } => {
                let (scatter, clear, copy) = Self::create_scatter_pipelines(pipeline_layout, device);
                *scatter_pipeline = scatter;
                *clear_pipeline = clear;
                *copy_pipeline = copy;
            }
        }
    }

    pub fn resize(
        &mut self,
        extent: wgpu::Extent3d,
        device: &wgpu::Device,
    ) {
        self.pending_resize = Some(extent);

        if let Kind::Scatter {
            ref bg_layout,
            ref mut bind_group,
            ref constant_buf,
            ref mut compute_groups,
            ..
        } = self.kind {
            let (bg, gs) = Self::create_scatter_resources(extent, constant_buf, bg_layout, device);
            *bind_group = bg;
            *compute_groups = gs;
        }
    }

    pub fn prepare(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
        global: &GlobalContext,
        cam: &Camera,
    ) {
        if !self.dirty_rects.is_empty() {
            if let Kind::RayMip { ref mipper, .. } = self.kind {
                mipper.update(&self.dirty_rects, encoder, device);
            }
            self.dirty_rects.clear();
        }

        let mut _params = match self.kind {
            Kind::RayMip { params, .. } => params,
            _ => [0; 4],
        };

        if let Some(size) = self.pending_resize {
            let staging = device
                .create_buffer_mapped(1, wgpu::BufferUsage::TRANSFER_SRC)
                .fill_from_slice(&[
                    Constants {
                        _scr_size: [size.width, size.height, 0, 0],
                        _params,
                    },
                ]);
            encoder.copy_buffer_to_buffer(
                &staging,
                0,
                &self.uniform_buf,
                0,
                mem::size_of::<Constants>() as wgpu::BufferAddress,
            );
        }

        if let Kind::Scatter {
            ref clear_pipeline,
            ref scatter_pipeline,
            ref bind_group,
            ref constant_buf,
            compute_groups,
            density,
            ..
        } = self.kind {
            let scatter_constants = compute_scatter_constants(cam);
            let staging = device
                .create_buffer_mapped(1, wgpu::BufferUsage::TRANSFER_SRC)
                .fill_from_slice(&[
                    scatter_constants,
                ]);
            encoder.copy_buffer_to_buffer(
                &staging,
                0,
                constant_buf,
                0,
                mem::size_of::<ScatterConstants>() as wgpu::BufferAddress,
            );

            let mut pass = encoder.begin_compute_pass();
            pass.set_bind_group(0, &global.bind_group, &[]);
            pass.set_bind_group(1, &self.bind_group, &[]);
            pass.set_bind_group(2, bind_group, &[]);
            pass.set_pipeline(clear_pipeline);
            pass.dispatch(compute_groups[0], compute_groups[1], compute_groups[2]);
            pass.set_pipeline(scatter_pipeline);
            pass.dispatch(compute_groups[0] * density[0], compute_groups[1] * density[1], density[2]);
        }

        self.pending_resize = None;
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass) {
        pass.set_bind_group(1, &self.bind_group, &[]);
        // draw terrain
        match self.kind {
            Kind::Ray { ref pipeline, ref geo } |
            Kind::RayMip { ref pipeline, ref geo, .. } => {
                pass.set_pipeline(pipeline);
                pass.set_index_buffer(&geo.index_buf, 0);
                pass.set_vertex_buffers(&[(&geo.vertex_buf, 0)]);
                pass.draw_indexed(0 .. geo.num_indices as u32, 0, 0 .. 1);
            }
            /*
            Kind::Tess { ref low, ref high, .. } => {
                encoder.draw(&self.terrain_slice, low, &self.terrain_data);
                encoder.draw(&self.terrain_slice, high, &self.terrain_data);
            }*/
            Kind::Slice { ref pipeline, ref geo } => {
                pass.set_pipeline(pipeline);
                pass.set_index_buffer(&geo.index_buf, 0);
                pass.set_vertex_buffers(&[(&geo.vertex_buf, 0)]);
                pass.draw_indexed(0 .. geo.num_indices as u32, 0, 0 .. 1);
            }
            Kind::Scatter { ref copy_pipeline, ref bind_group, .. } => {
                pass.set_pipeline(copy_pipeline);
                pass.set_bind_group(2, bind_group, &[]);
                pass.draw(0 .. 4, 0 .. 1);
            }
        }
    }
}
