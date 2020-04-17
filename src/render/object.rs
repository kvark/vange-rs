use crate::{
    render::{
        GpuTransform, Palette, Shaders,
        COLOR_FORMAT, DEPTH_FORMAT,
        body::GpuBody,
        global::Context as GlobalContext,
    },
    space::Transform,
};
use bytemuck::{Pod, Zeroable};
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
#[derive(Clone, Copy)]
pub struct Vertex {
    pub pos: [i8; 4],
    pub color: u32,
    pub normal: [i8; 4],
}
unsafe impl Pod for Vertex {}
unsafe impl Zeroable for Vertex {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Instance {
    pos_scale: [f32; 4],
    orientation: [f32; 4],
    shape_scale: f32,
    body_and_color_id: [u32; 2],
}
unsafe impl Pod for Instance {}
unsafe impl Zeroable for Instance {}

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
    attributes: &wgpu::vertex_attr_array![3 => Float4, 4 => Float4, 5 => Float, 6 => Uint2],
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
            vertex_state: wgpu::VertexStateDescriptor {
                index_format: wgpu::IndexFormat::Uint16,
                vertex_buffers: &[
                    wgpu::VertexBufferDescriptor {
                        stride: mem::size_of::<Vertex>() as wgpu::BufferAddress,
                        step_mode: wgpu::InputStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Char4, 1 => Uint, 2 => Char4Norm],
                    },
                    INSTANCE_DESCRIPTOR,
                ],
            },
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
            width: NUM_COLOR_IDS,
            height: 1,
            depth: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Color table"),
            size: extent,
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
                bytes_per_row: NUM_COLOR_IDS * 2,
                rows_per_image: 0,
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
            compare: wgpu::CompareFunction::Always,
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
            label: Some("Object"),
            bindings: &[
                wgpu::BindGroupLayoutEntry { // color map
                    binding: 0,
                    visibility: wgpu::ShaderStage::VERTEX,
                    ty: wgpu::BindingType::SampledTexture {
                        dimension: wgpu::TextureViewDimension::D1,
                        component_type: wgpu::TextureComponentType::Float,
                        multisampled: false,
                    },
                },
                wgpu::BindGroupLayoutEntry { // palette map
                    binding: 1,
                    visibility: wgpu::ShaderStage::VERTEX | wgpu::ShaderStage::FRAGMENT,
                    ty: wgpu::BindingType::SampledTexture {
                        dimension: wgpu::TextureViewDimension::D1,
                        component_type: wgpu::TextureComponentType::Float,
                        multisampled: false,
                    },
                },
                wgpu::BindGroupLayoutEntry { // color table sampler
                    binding: 2,
                    visibility: wgpu::ShaderStage::VERTEX,
                    ty: wgpu::BindingType::Sampler { comparison: false },
                },
            ],
        });
        let shape_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shape"),
            bindings: &[
                wgpu::BindGroupLayoutEntry { // shape locals
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
            label: Some("Object"),
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
