use crate::{
    level,
    render::{
        GlobalContext, Palette, Shaders,
        COLOR_FORMAT, DEPTH_FORMAT,
    },
};

use wgpu;

use std::mem;


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
pub struct Constants {
    _scr_size: [f32; 4],
}

impl Constants {
    pub fn new(extent: &wgpu::Extent3d) -> Self {
        Constants {
            _scr_size: [extent.width as f32, extent.height as f32, 0.0, 0.0],
        }
    }
}

pub enum Kind {
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

pub struct Context {
    pub uniform_buf: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    pipeline_layout: wgpu::PipelineLayout,
    pub kind: Kind,
}

impl Context {
    fn create_ray_pipeline(
        layout: &wgpu::PipelineLayout,
        device: &wgpu::Device,
    ) -> wgpu::RenderPipeline {
        let shaders = Shaders::new("terrain_ray", &[], device)
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
                    stride: mem::size_of::<Vertex>() as u32,
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

    pub fn new(
        init_encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
        level: &level::Level,
        global: &GlobalContext,
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
        let palette = Palette::new(init_encoder, device, &level.palette);

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

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            size: mem::size_of::<Constants>() as u32,
            usage: wgpu::BufferUsageFlags::UNIFORM | wgpu::BufferUsageFlags::TRANSFER_DST,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout,
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
                        buffer: &uniform_buf,
                        range: 0 .. mem::size_of::<Constants>() as u32,
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

        let vertices = [
            Vertex { _pos: [0, 0, 0, 1] },
            Vertex { _pos: [-1, 0, 0, 0] },
            Vertex { _pos: [0, -1, 0, 0] },
            Vertex { _pos: [1, 0, 0, 0] },
            Vertex { _pos: [0, 1, 0, 0] },
        ];
        let indices = [0u16, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 1];

        let vertex_buf = device
            .create_buffer_mapped(vertices.len(), wgpu::BufferUsageFlags::VERTEX)
            .fill_from_slice(&vertices);
        let index_buf = device
            .create_buffer_mapped(indices.len(), wgpu::BufferUsageFlags::INDEX)
            .fill_from_slice(&indices);

        let pipeline = Self::create_ray_pipeline(&pipeline_layout, device);

        Context {
            uniform_buf,
            bind_group,
            pipeline_layout,
            kind: Kind::Ray {
                pipeline,
                vertex_buf,
                index_buf,
                num_indices: indices.len(),
            },
        }
    }

    pub fn reload(&mut self, device: &wgpu::Device) {
        match self.kind {
            Kind::Ray { ref mut pipeline, .. } => {
                *pipeline = Self::create_ray_pipeline(
                    &self.pipeline_layout,
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
    }
}
