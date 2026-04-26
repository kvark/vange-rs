use crate::{
    render::{
        DEPTH_FORMAT, GpuTransform, Palette, PipelineSet, SHADOW_FORMAT, VertexStorageNotSupported,
        global::Context as GlobalContext,
    },
    space::Transform,
};
use bytemuck::{Pod, Zeroable};
use m3d::NUM_COLOR_IDS;

use std::{mem, slice};

pub const COLOR_TABLE: [[u8; 2]; NUM_COLOR_IDS as usize] = [
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
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub enum BodyColor {
    Dummy = 1,
    Green = 21,
    Red = 7,
    Blue = 8,
    Yellow = 9,
    Gray = 10,
}

impl BodyColor {
    pub fn name(self) -> &'static str {
        match self {
            Self::Dummy => "dummy",
            Self::Green => "green",
            Self::Red => "red",
            Self::Blue => "blue",
            Self::Yellow => "yellow",
            Self::Gray => "gray",
        }
    }

    pub fn from_value(v: u8) -> Self {
        match v as u32 {
            21 => Self::Green,
            7 => Self::Red,
            8 => Self::Blue,
            9 => Self::Yellow,
            10 => Self::Gray,
            _ => Self::Green,
        }
    }
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
    pub fn new(transform: &Transform, shape_scale: f32, color_id: u8) -> Self {
        let gt = GpuTransform::new(transform);
        Instance {
            pos_scale: gt.pos_scale,
            orientation: gt.orientation,
            shape_scale,
            body_and_color_id: [0, color_id as u32],
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
            attributes: wgpu::vertex_attr_array![
                3 => Float32x4,
                4 => Float32x4,
                5 => Float32,
                6 => Uint32x2,
            ],
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

/// Resources the object pipeline needs to do a per-pixel underwater
/// check: the surface uniforms (for `texture_scale`), the terrain meta
/// texture (cell type), and the flood texture (per-Y water level). The
/// game-side render pipeline supplies these from the terrain context;
/// the standalone model viewer supplies stubs via [`create_stub_surface`].
pub struct SurfaceInputs<'a> {
    pub uniform_buf: &'a wgpu::Buffer,
    pub terrain_view: &'a wgpu::TextureView,
    pub flood_view: &'a wgpu::TextureView,
}

/// Build dummy surface resources for tools (the model viewer) that draw
/// vehicles without a real terrain. The uniform's `texture_scale.x` is
/// 0, which the object shader uses as the "skip the underwater branch"
/// signal.
pub struct StubSurface {
    pub uniform_buf: wgpu::Buffer,
    pub terrain_view: wgpu::TextureView,
    pub flood_view: wgpu::TextureView,
}

pub fn create_stub_surface(device: &wgpu::Device) -> StubSurface {
    use wgpu::util::DeviceExt as _;

    // SurfaceConstants layout: vec4 texture_scale, u32 terrain_bits,
    // u32 delta_mode, u32 pad0, u32 pad1. We need texture_scale.x and
    // .y to be non-zero so the shader's wrap math doesn't divide by 0,
    // and texture_scale.z = 0 so the flood-fed water Z stays at 0
    // (keeps the underwater check inert for vehicles above ground).
    let mut bytes = [0u8; 32];
    bytes[0..4].copy_from_slice(&1.0f32.to_ne_bytes()); // texture_scale.x
    bytes[4..8].copy_from_slice(&1.0f32.to_ne_bytes()); // texture_scale.y
    let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Stub surface uniforms"),
        contents: &bytes,
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let make_view = |label, format| {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            view_formats: &[],
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
        });
        tex.create_view(&wgpu::TextureViewDescriptor::default())
    };
    StubSurface {
        uniform_buf,
        terrain_view: make_view("Stub terrain", wgpu::TextureFormat::Rgba8Uint),
        flood_view: make_view("Stub flood", wgpu::TextureFormat::R8Unorm),
    }
}

impl StubSurface {
    pub fn inputs(&self) -> SurfaceInputs<'_> {
        SurfaceInputs {
            uniform_buf: &self.uniform_buf,
            terrain_view: &self.terrain_view,
            flood_view: &self.flood_view,
        }
    }
}

pub struct Context {
    pub surface_bind_group: wgpu::BindGroup,
    pub bind_group: wgpu::BindGroup,
    pub shape_bind_group_layout: Result<wgpu::BindGroupLayout, VertexStorageNotSupported>,
    pub pipeline_layout: wgpu::PipelineLayout,
    pub pipelines: PipelineSet,
    pub color_format: wgpu::TextureFormat,
    front_face: wgpu::FrontFace,
}

impl Context {
    fn create_pipelines(
        layout: &wgpu::PipelineLayout,
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        front_face: wgpu::FrontFace,
    ) -> PipelineSet {
        let vertex_descriptor = wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Sint8x4, 1 => Uint32, 2 => Snorm8x4],
        };
        let instance_desc = InstanceDesc::new();
        let shader = super::load_shader("object", &[], device).unwrap();

        let main = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("object"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("color_vs"),
                compilation_options: Default::default(),
                buffers: &[vertex_descriptor.clone(), instance_desc.buffer_desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("color_fs"),
                compilation_options: Default::default(),
                targets: &[Some(color_format.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face,
                // original was not drawn with rasterizer, used no culling
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("object-shadow"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("geometry_vs"),
                compilation_options: Default::default(),
                buffers: &[vertex_descriptor, instance_desc.buffer_desc()],
            },
            fragment: None,
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                unclipped_depth: false,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        PipelineSet { main, shadow }
    }

    fn create_color_table(gfx: &super::GraphicsContext) -> (wgpu::TextureView, wgpu::Sampler) {
        let extent = wgpu::Extent3d {
            width: NUM_COLOR_IDS,
            height: 1,
            depth_or_array_layers: 1,
        };
        let texture = gfx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Color table"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg8Uint,
            view_formats: &[],
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        });

        gfx.queue.write_texture(
            texture.as_image_copy(),
            unsafe { slice::from_raw_parts(COLOR_TABLE[0].as_ptr(), NUM_COLOR_IDS as usize * 2) },
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(NUM_COLOR_IDS * 2),
                rows_per_image: None,
            },
            extent,
        );

        let sampler = gfx.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        (
            texture.create_view(&wgpu::TextureViewDescriptor::default()),
            sampler,
        )
    }

    pub fn new(
        gfx: &super::GraphicsContext,
        front_face: wgpu::FrontFace,
        palette_data: &[[u8; 4]],
        global: &GlobalContext,
        surface: SurfaceInputs<'_>,
    ) -> Self {
        let bind_group_layout =
            gfx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Object"),
                    entries: &[
                        // color map
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::VERTEX,
                            ty: wgpu::BindingType::Texture {
                                view_dimension: wgpu::TextureViewDimension::D2,
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
                                view_dimension: wgpu::TextureViewDimension::D2,
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                multisampled: false,
                            },
                            count: None,
                        },
                        // color table sampler
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::VERTEX,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                            count: None,
                        },
                    ],
                });
        let shape_bind_group_layout = if gfx
            .downlevel_caps
            .flags
            .contains(wgpu::DownlevelFlags::VERTEX_STORAGE)
            && gfx.device.limits().max_storage_buffers_per_shader_stage != 0
        {
            let bgl = gfx
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
            Ok(bgl)
        } else {
            Err(VertexStorageNotSupported)
        };

        let palette = Palette::new(&gfx.device);
        palette.init(&gfx.queue, palette_data);

        let (color_table_view, color_table_sampler) = Self::create_color_table(gfx);
        let bind_group = gfx.device.create_bind_group(&wgpu::BindGroupDescriptor {
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
        let surface_bind_group_layout =
            gfx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Object surface"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                view_dimension: wgpu::TextureViewDimension::D2,
                                sample_type: wgpu::TextureSampleType::Uint,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 4,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                view_dimension: wgpu::TextureViewDimension::D2,
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                multisampled: false,
                            },
                            count: None,
                        },
                    ],
                });
        let surface_bind_group = gfx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Object surface"),
            layout: &surface_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: surface.uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(surface.terrain_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(surface.flood_view),
                },
            ],
        });

        // Surface goes at @group(1) so the object shader can `//!include
        // surface.inc` directly (it hard-codes group 1 to match the
        // terrain and water pipelines). Object-local resources move to
        // @group(2).
        let pipeline_layout = gfx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("object"),
                bind_group_layouts: &[
                    Some(&global.bind_group_layout),
                    Some(&surface_bind_group_layout),
                    Some(&bind_group_layout),
                ],
                immediate_size: 0,
            });
        let pipelines =
            Self::create_pipelines(&pipeline_layout, &gfx.device, gfx.color_format, front_face);

        Context {
            surface_bind_group,
            bind_group,
            shape_bind_group_layout,
            pipeline_layout,
            pipelines,
            color_format: gfx.color_format,
            front_face,
        }
    }

    pub fn reload(&mut self, device: &wgpu::Device) {
        self.pipelines = Self::create_pipelines(
            &self.pipeline_layout,
            device,
            self.color_format,
            self.front_face,
        );
    }
}
