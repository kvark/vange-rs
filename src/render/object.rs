use crate::{
    render::{
        body::GpuBody, global::Context as GlobalContext, GpuTransform, Palette, PipelineSet,
        Shaders, COLOR_FORMAT, DEPTH_FORMAT, SHADOW_FORMAT,
    },
    space::Transform,
};
use bytemuck::{Pod, Zeroable};
use m3d::NUM_COLOR_IDS;

use std::{mem, num::NonZeroU32, slice};

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
            shape_scale,
            body_and_color_id: [body.index() as u32, color as u32],
        }
    }
}

#[derive(Copy, Clone)]
pub struct InstanceDesc {
    attributes: [wgpu::VertexAttribute; 4],
}

impl InstanceDesc {
    pub fn new() -> Self {
        InstanceDesc {
            attributes: wgpu::vertex_attr_array![3 => Float32x4, 4 => Float32x4, 5 => Float32, 6 => Uint32x2],
        }
    }

    pub fn buffer_desc(&self) -> wgpu::VertexBufferLayout<'_> {
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<Instance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &self.attributes,
        }
    }
}

pub struct Context {
    pub bind_group: wgpu::BindGroup,
    pub shape_bind_group_layout: wgpu::BindGroupLayout,
    pub pipeline_layout: wgpu::PipelineLayout,
    pub pipelines: PipelineSet,
}

impl Context {
    fn create_pipelines(layout: &wgpu::PipelineLayout, device: &wgpu::Device) -> PipelineSet {
        let vertex_descriptor = wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Sint8x4, 1 => Uint32, 2 => Snorm8x4],
        };
        let instance_desc = InstanceDesc::new();

        let main_shaders = Shaders::new("object", &["COLOR"], device).unwrap();
        let main = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("object"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &main_shaders.vs,
                entry_point: "main",
                buffers: &[vertex_descriptor.clone(), instance_desc.buffer_desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &main_shaders.fs,
                entry_point: "main",
                targets: &[COLOR_FORMAT.into()],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                // original was not drawn with rasterizer, used no culling
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
        });

        let shadow_shaders = Shaders::new("object", &[], device).unwrap();
        let shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("object-shadow"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &shadow_shaders.vs,
                entry_point: "main",
                buffers: &[vertex_descriptor, instance_desc.buffer_desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shadow_shaders.fs,
                entry_point: "main",
                targets: &[],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                clamp_depth: false,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
        });

        PipelineSet { main, shadow }
    }

    fn create_color_table(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> (wgpu::TextureView, wgpu::Sampler) {
        let extent = wgpu::Extent3d {
            width: NUM_COLOR_IDS,
            height: 1,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Color table"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D1,
            format: wgpu::TextureFormat::Rg8Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        });

        queue.write_texture(
            texture.as_image_copy(),
            unsafe { slice::from_raw_parts(COLOR_TABLE[0].as_ptr(), NUM_COLOR_IDS as usize * 2) },
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: NonZeroU32::new(NUM_COLOR_IDS * 2),
                rows_per_image: None,
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
            ..Default::default()
        });
        (
            texture.create_view(&wgpu::TextureViewDescriptor::default()),
            sampler,
        )
    }

    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        palette_data: &[[u8; 4]],
        global: &GlobalContext,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Object"),
            entries: &[
                // color map
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Texture {
                        view_dimension: wgpu::TextureViewDimension::D1,
                        sample_type: wgpu::TextureSampleType::Uint,
                        multisampled: false,
                    },
                    count: None,
                },
                // palette map
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        view_dimension: wgpu::TextureViewDimension::D1,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        multisampled: false,
                    },
                    count: None,
                },
                // color table sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Sampler {
                        filtering: false,
                        comparison: false,
                    },
                    count: None,
                },
            ],
        });
        let shape_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Shape"),
                entries: &[
                    // shape locals
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let palette = Palette::new(device, queue, palette_data);
        let (color_table_view, color_table_sampler) = Self::create_color_table(device, queue);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Object"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color_table_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&palette.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&color_table_sampler),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("object"),
            bind_group_layouts: &[&global.bind_group_layout, &bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipelines = Self::create_pipelines(&pipeline_layout, device);

        Context {
            bind_group,
            shape_bind_group_layout,
            pipeline_layout,
            pipelines,
        }
    }

    pub fn reload(&mut self, device: &wgpu::Device) {
        self.pipelines = Self::create_pipelines(&self.pipeline_layout, device);
    }
}
