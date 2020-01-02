use crate::{
    render::{
        GpuTransform, Palette, Shaders,
        COLOR_FORMAT, DEPTH_FORMAT,
        body::GpuBody,
        global::Context as GlobalContext,
    },
    space::Transform,
};
use m3d::NUM_COLOR_IDS;

use std::{mem, slice};


const COLOR_TABLE: [[u8; 2]; NUM_COLOR_IDS as usize] = [
    [0, 0],   // reserved
    [128, 3], // body
    [176, 4], // window
    [224, 7], // wheel
    [184, 4], // defence
    [224, 3], // weapon
    [224, 7], // tube
    [128, 3], // body red
    [144, 3], // body blue
    [160, 3], // body yellow
    [228, 4], // body gray
    [112, 4], // yellow (charged)
    [0, 2],   // material 0
    [32, 2],  // material 1
    [64, 4],  // material 2
    [72, 3],  // material 3
    [88, 3],  // material 4
    [104, 4], // material 5
    [112, 4], // material 6
    [120, 4], // material 7
    [184, 4], // black
    [240, 3], // body green
    [136, 4], // skyfarmer kenoboo
    [128, 4], // skyfarmer pipetka
    [224, 4], // rotten item
];

#[repr(u32)]
#[derive(Clone, Copy, Debug, Deserialize)]
pub enum BodyColor {
    Dummy = 1,
    Green = 21,
    Red = 7,
    Blue = 8,
    Yellow = 9,
    Gray = 10,
}

#[repr(C)]
#[derive(Clone, Copy, zerocopy::AsBytes, zerocopy::FromBytes)]
pub struct Vertex {
    pub pos: [i8; 4],
    pub color: u32,
    pub normal: [i8; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, zerocopy::AsBytes, zerocopy::FromBytes)]
pub struct Instance {
    pos_scale: [f32; 4],
    orientation: [f32; 4],
    shape_scale: f32,
    body_and_color_id: [u32; 2],
}

impl Instance {
    pub fn new(transform: &Transform, shape_scale: f32, body: &GpuBody, color: BodyColor) -> Self {
        let gt = GpuTransform::new(transform);
        Instance {
            pos_scale: gt.pos_scale,
            orientation: gt.orientation,
            shape_scale: shape_scale,
            body_and_color_id: [body.index() as u32, color as u32],
        }
    }
}

pub const INSTANCE_DESCRIPTOR: wgpu::VertexBufferDescriptor = wgpu::VertexBufferDescriptor {
    stride: mem::size_of::<Instance>() as wgpu::BufferAddress,
    step_mode: wgpu::InputStepMode::Instance,
    attributes: &[
        wgpu::VertexAttributeDescriptor {
            offset: 0,
            format: wgpu::VertexFormat::Float4,
            shader_location: 3,
        },
        wgpu::VertexAttributeDescriptor {
            offset: 16,
            format: wgpu::VertexFormat::Float4,
            shader_location: 4,
        },
        wgpu::VertexAttributeDescriptor {
            offset: 32,
            format: wgpu::VertexFormat::Float,
            shader_location: 5,
        },
        wgpu::VertexAttributeDescriptor {
            offset: 36,
            format: wgpu::VertexFormat::Uint2,
            shader_location: 6,
        },
    ],
};

pub struct Context {
    pub bind_group: wgpu::BindGroup,
    pub shape_bind_group_layout: wgpu::BindGroupLayout,
    pub pipeline_layout: wgpu::PipelineLayout,
    pub pipeline: wgpu::RenderPipeline,
}

impl Context {
    fn create_pipeline(
        layout: &wgpu::PipelineLayout,
        device: &wgpu::Device,
    ) -> wgpu::RenderPipeline {
        let shaders = Shaders::new("object", &[], device)
            .unwrap();
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            layout,
            vertex_stage: wgpu::ProgrammableStageDescriptor {
                module: &shaders.vs,
                entry_point: "main",
            },
            fragment_stage: Some(wgpu::ProgrammableStageDescriptor {
                module: &shaders.fs,
                entry_point: "main",
            }),
            rasterization_state: Some(wgpu::RasterizationStateDescriptor {
                front_face: wgpu::FrontFace::Ccw,
                // original was not drawn with rasterizer, used no culling
                cull_mode: wgpu::CullMode::None,
                depth_bias: 0,
                depth_bias_slope_scale: 0.0,
                depth_bias_clamp: 0.0,
            }),
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
                depth_compare: wgpu::CompareFunction::LessEqual,
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
                        wgpu::VertexAttributeDescriptor {
                            offset: 4,
                            format: wgpu::VertexFormat::Uint,
                            shader_location: 1,
                        },
                        wgpu::VertexAttributeDescriptor {
                            offset: 8,
                            format: wgpu::VertexFormat::Char4Norm,
                            shader_location: 2,
                        },
                    ],
                },
                INSTANCE_DESCRIPTOR,
            ],
            sample_count: 1,
            alpha_to_coverage_enabled: false,
            sample_mask: !0,
        })
    }

    fn create_color_table(
        encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device
    ) -> (wgpu::TextureView, wgpu::Sampler) {
        let extent = wgpu::Extent3d {
            width: NUM_COLOR_IDS as u32,
            height: 1,
            depth: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            size: extent,
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D1,
            format: wgpu::TextureFormat::Rg8Uint,
            usage: wgpu::TextureUsage::SAMPLED | wgpu::TextureUsage::COPY_DST,
        });

        let staging = device.create_buffer_with_data(
            unsafe {
                slice::from_raw_parts(COLOR_TABLE[0].as_ptr(), NUM_COLOR_IDS as usize * 2)
            },
            wgpu::BufferUsage::COPY_SRC,
        );
        encoder.copy_buffer_to_texture(
            wgpu::BufferCopyView {
                buffer: &staging,
                offset: 0,
                row_pitch: NUM_COLOR_IDS as u32 * 2,
                image_height: 1,
            },
            wgpu::TextureCopyView {
                texture: &texture,
                mip_level: 0,
                array_layer: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: 0,
                },
            },
            extent,
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
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
        (texture.create_default_view(), sampler)
    }

    pub fn new(
        init_encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
        palette_data: &[[u8; 4]],
        global: &GlobalContext,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            bindings: &[
                wgpu::BindGroupLayoutBinding { // color map
                    binding: 0,
                    visibility: wgpu::ShaderStage::VERTEX,
                    ty: wgpu::BindingType::SampledTexture {
                        dimension: wgpu::TextureViewDimension::D1,
                        multisampled: false,
                    },
                },
                wgpu::BindGroupLayoutBinding { // palette map
                    binding: 1,
                    visibility: wgpu::ShaderStage::VERTEX,
                    ty: wgpu::BindingType::SampledTexture {
                        dimension: wgpu::TextureViewDimension::D1,
                        multisampled: false,
                    },
                },
                wgpu::BindGroupLayoutBinding { // color table sampler
                    binding: 2,
                    visibility: wgpu::ShaderStage::VERTEX,
                    ty: wgpu::BindingType::Sampler,
                },
            ],
        });
        let shape_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            bindings: &[
                wgpu::BindGroupLayoutBinding { // shape locals
                    binding: 0,
                    visibility: wgpu::ShaderStage::VERTEX,
                    ty: wgpu::BindingType::StorageBuffer { dynamic: false, readonly: true },
                },
            ],
        });

        let palette = Palette::new(init_encoder, device, palette_data);
        let (color_table_view, color_table_sampler) = Self::create_color_table(
            init_encoder, device
        );
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout,
            bindings: &[
                wgpu::Binding {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color_table_view),
                },
                wgpu::Binding {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&palette.view),
                },
                wgpu::Binding {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&color_table_sampler),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            bind_group_layouts: &[
                &global.bind_group_layout,
                &bind_group_layout,
            ],
        });
        let pipeline = Self::create_pipeline(&pipeline_layout, device);

        Context {
            bind_group,
            shape_bind_group_layout,
            pipeline_layout,
            pipeline,
        }
    }

    pub fn reload(&mut self, device: &wgpu::Device) {
        self.pipeline = Self::create_pipeline(&self.pipeline_layout, device);
    }
}
