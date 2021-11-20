use crate::{
    config::settings,
    model,
    render::{
        global::Context as GlobalContext,
        object::{Context as ObjectContext, Instance as ObjectInstance},
        VertexStorageNotSupported, COLOR_FORMAT, DEPTH_FORMAT,
    },
};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt as _;

use std::{collections::HashMap, mem, num::NonZeroU64};

const BLEND_FRONT: wgpu::BlendComponent = wgpu::BlendComponent::REPLACE;
const BLEND_BEHIND: wgpu::BlendComponent = wgpu::BlendComponent {
    src_factor: wgpu::BlendFactor::Constant,
    dst_factor: wgpu::BlendFactor::OneMinusConstant,
    operation: wgpu::BlendOperation::Add,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Visibility {
    Front,
    Behind,
}
type Selector = (Visibility, wgpu::VertexStepMode);

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Position {
    pub pos: [f32; 4],
}
unsafe impl Pod for Position {}
unsafe impl Zeroable for Position {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Color {
    pub color: u32,
}
unsafe impl Pod for Color {}
unsafe impl Zeroable for Color {}

#[repr(C)]
#[derive(Clone, Copy)]
struct Locals {
    color: [f32; 4],
    _pad: [f32; 60],
}
unsafe impl Pod for Locals {}
unsafe impl Zeroable for Locals {}

impl Locals {
    fn new(color: [f32; 4]) -> Self {
        Locals {
            color,
            _pad: [0.0; 60],
        }
    }
}

pub struct LineBuffer {
    vertices: Vec<Position>,
    colors: Vec<Color>,
}

impl LineBuffer {
    pub fn new() -> Self {
        LineBuffer {
            vertices: Vec::new(),
            colors: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.vertices.clear();
        self.colors.clear();
    }

    pub fn add(&mut self, from: [f32; 3], to: [f32; 3], color: u32) {
        self.vertices.push(Position {
            pos: [from[0], from[1], from[2], 1.0],
        });
        self.vertices.push(Position {
            pos: [to[0], to[1], to[2], 1.0],
        });
        let color = Color { color };
        self.colors.push(color);
        self.colors.push(color);
    }
}

pub struct Context {
    settings: settings::DebugRender,
    pipeline_layout: Result<wgpu::PipelineLayout, VertexStorageNotSupported>,
    pipelines_line: HashMap<Selector, wgpu::RenderPipeline>,
    pipeline_face: Option<wgpu::RenderPipeline>,
    pipeline_edge: Option<wgpu::RenderPipeline>,
    line_color_buf: wgpu::Buffer,
    bind_group_line: wgpu::BindGroup,
    bind_group_face: wgpu::BindGroup,
    bind_group_edge: wgpu::BindGroup,
    // hold the buffers alive
    vertex_buf: Option<wgpu::Buffer>,
    color_buf: Option<wgpu::Buffer>,
}

impl Context {
    pub fn new(
        device: &wgpu::Device,
        settings: &settings::DebugRender,
        global: &GlobalContext,
        object: &ObjectContext,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Debug"),
            entries: &[
                // locals
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
            ],
        });
        let pipeline_layout = match object.shape_bind_group_layout {
            Ok(ref shape_bgl) => {
                let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("debug"),
                    bind_group_layouts: &[&global.bind_group_layout, &bind_group_layout, shape_bgl],
                    push_constant_ranges: &[],
                });
                Ok(pl)
            }
            Err(e) => Err(e),
        };

        let line_color_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("debug-line-color"),
            contents: bytemuck::bytes_of(&Color { color: 0xFF000080 }), // line
            usage: wgpu::BufferUsages::VERTEX,
        });
        let locals_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("debug-locals"),
            contents: bytemuck::cast_slice(&[
                Locals::new([1.0; 4]),             // line
                Locals::new([0.0, 1.0, 0.0, 0.2]), // face
                Locals::new([1.0, 1.0, 0.0, 0.2]), // edge
            ]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let locals_size = mem::size_of::<Locals>() as wgpu::BufferAddress;
        let bind_group_line = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Debug line"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &locals_buf,
                    offset: 0 * locals_size,
                    size: NonZeroU64::new(locals_size),
                }),
            }],
        });
        let bind_group_face = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Debug face"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &locals_buf,
                    offset: 1 * locals_size,
                    size: NonZeroU64::new(locals_size),
                }),
            }],
        });
        let bind_group_edge = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Debug edge"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &locals_buf,
                    offset: 2 * locals_size,
                    size: NonZeroU64::new(locals_size),
                }),
            }],
        });

        let mut result = Context {
            settings: *settings,
            pipeline_layout,
            pipelines_line: HashMap::new(),
            pipeline_face: None,
            pipeline_edge: None,
            line_color_buf,
            bind_group_line,
            bind_group_face,
            bind_group_edge,
            vertex_buf: None,
            color_buf: None,
        };
        result.reload(device);
        result
    }

    pub fn reload(&mut self, device: &wgpu::Device) {
        let primitive = wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            front_face: wgpu::FrontFace::Ccw,
            // original was not drawn with rasterizer, used no culling
            ..Default::default()
        };

        #[cfg(feature = "glsl")]
        if self.settings.collision_shapes && self.pipeline_layout.is_ok() {
            let shaders = super::Shaders::new("debug_shape", &[], device).unwrap();
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("debug-shape"),
                layout: self.pipeline_layout.as_ref().ok(),
                vertex: wgpu::VertexState {
                    module: &shaders.vs,
                    entry_point: "main",
                    buffers: &[
                        super::ShapeVertexDesc::new().buffer_desc(),
                        super::object::InstanceDesc::new().buffer_desc(),
                    ],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shaders.fs,
                    entry_point: "main",
                    targets: &[wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend: Some(wgpu::BlendState {
                            alpha: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::SrcAlpha,
                                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                                operation: wgpu::BlendOperation::Add,
                            },
                        }),
                        write_mask: wgpu::ColorWrites::all(),
                    }],
                }),
                primitive,
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: false,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
            });
            self.pipeline_face = Some(pipeline);
            self.pipeline_edge = None; //TODO: line raster
        }

        self.pipelines_line.clear();
        if self.settings.impulses {
            let shader = super::load_shader("debug", device).unwrap();
            for &visibility in &[Visibility::Front, Visibility::Behind] {
                let (blend, depth_write_enabled, depth_compare) = match visibility {
                    Visibility::Front => (BLEND_FRONT, true, wgpu::CompareFunction::LessEqual),
                    Visibility::Behind => (BLEND_BEHIND, false, wgpu::CompareFunction::Greater),
                };
                for &color_rate in &[wgpu::VertexStepMode::Vertex, wgpu::VertexStepMode::Instance] {
                    let name = format!("debug-line-{:?}-{:?}", visibility, color_rate);
                    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some(&name),
                        layout: match self.pipeline_layout {
                            Ok(ref layout) => Some(layout),
                            Err(_) => continue,
                        },
                        vertex: wgpu::VertexState {
                            module: &shader,
                            entry_point: "main_vs",
                            buffers: &[
                                wgpu::VertexBufferLayout {
                                    array_stride: mem::size_of::<Position>() as wgpu::BufferAddress,
                                    step_mode: wgpu::VertexStepMode::Vertex,
                                    attributes: &[wgpu::VertexAttribute {
                                        offset: 0,
                                        format: wgpu::VertexFormat::Float32x4,
                                        shader_location: 0,
                                    }],
                                },
                                wgpu::VertexBufferLayout {
                                    array_stride: mem::size_of::<Color>() as wgpu::BufferAddress,
                                    step_mode: color_rate,
                                    attributes: &[wgpu::VertexAttribute {
                                        offset: 0,
                                        format: wgpu::VertexFormat::Unorm8x4,
                                        shader_location: 1,
                                    }],
                                },
                            ],
                        },
                        fragment: Some(wgpu::FragmentState {
                            module: &shader,
                            entry_point: "main_fs",
                            targets: &[wgpu::ColorTargetState {
                                format: COLOR_FORMAT,
                                blend: Some(wgpu::BlendState {
                                    color: blend,
                                    alpha: blend,
                                }),
                                write_mask: wgpu::ColorWrites::all(),
                            }],
                        }),
                        primitive,
                        depth_stencil: Some(wgpu::DepthStencilState {
                            format: DEPTH_FORMAT,
                            depth_write_enabled,
                            depth_compare,
                            stencil: Default::default(),
                            bias: Default::default(),
                        }),
                        multisample: wgpu::MultisampleState::default(),
                    });
                    self.pipelines_line
                        .insert((visibility, color_rate), pipeline);
                }
            }
        }
    }

    fn draw_liner<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        vertex_buf: &'a wgpu::Buffer,
        color_buf: &'a wgpu::Buffer,
        color_rate: wgpu::VertexStepMode,
        num_vert: usize,
    ) {
        pass.set_blend_constant(wgpu::Color::WHITE);
        pass.set_vertex_buffer(0, vertex_buf.slice(..));
        pass.set_vertex_buffer(1, color_buf.slice(..));
        for &vis in &[Visibility::Front, Visibility::Behind] {
            if let Some(pipeline) = self.pipelines_line.get(&(vis, color_rate)) {
                pass.set_pipeline(pipeline);
                pass.draw(0..num_vert as u32, 0..1);
            }
        }
    }

    pub fn draw_shape<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        shape: &'a model::Shape,
        instance_buf: &'a wgpu::Buffer,
        instance_id: usize,
    ) {
        if !self.settings.collision_shapes {
            return;
        }
        let shape_bg = match shape.bind_group {
            Ok(ref bg) => bg,
            Err(_) => return,
        };

        //TODO: this is broken - both regular rendering and debug one
        // require instancing now, one has to yield and be refactored.
        let instance_offset = instance_id * mem::size_of::<ObjectInstance>();
        pass.set_bind_group(2, shape_bg, &[]);
        pass.set_vertex_buffer(0, shape.polygon_buf.slice(..));
        pass.set_vertex_buffer(
            1,
            instance_buf.slice(
                instance_offset as wgpu::BufferAddress
                    ..mem::size_of::<ObjectInstance>() as wgpu::BufferAddress,
            ),
        );

        // draw collision polygon faces
        if let Some(ref pipeline) = self.pipeline_face {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(1, &self.bind_group_face, &[]);
            pass.draw(0..4, 0..shape.polygons.len() as u32);
        }
        // draw collision polygon edges
        if let Some(ref pipeline) = self.pipeline_edge {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(1, &self.bind_group_edge, &[]);
            pass.draw(0..4, 0..shape.polygons.len() as u32);
        }

        // draw sample normals
        if let Some((ref sample_buf, num_vert)) = shape.sample_buf {
            pass.set_bind_group(1, &self.bind_group_line, &[]);
            self.draw_liner(
                pass,
                sample_buf,
                &self.line_color_buf,
                wgpu::VertexStepMode::Instance,
                num_vert,
            );
        }
    }

    pub fn draw_lines<'a>(
        &'a mut self,
        pass: &mut wgpu::RenderPass<'a>,
        device: &wgpu::Device,
        linebuf: &LineBuffer,
    ) {
        self.vertex_buf = Some(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("debug-vertices"),
                contents: bytemuck::cast_slice(&linebuf.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
        );
        self.color_buf = Some(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("debug-colors"),
                contents: bytemuck::cast_slice(&linebuf.colors),
                usage: wgpu::BufferUsages::VERTEX,
            }),
        );
        assert_eq!(linebuf.vertices.len(), linebuf.colors.len());

        self.draw_liner(
            pass,
            self.vertex_buf.as_ref().unwrap(),
            self.color_buf.as_ref().unwrap(),
            wgpu::VertexStepMode::Vertex,
            linebuf.vertices.len(),
        );
    }
}
