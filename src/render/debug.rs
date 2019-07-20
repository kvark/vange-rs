use crate::{
    config::settings,
    model,
    render::{
        Shaders, ShapePolygon,
        COLOR_FORMAT, DEPTH_FORMAT,
        global::Context as GlobalContext,
    },
};

use std::{
    mem,
    collections::HashMap,
};


const BLEND_FRONT: wgpu::BlendDescriptor = wgpu::BlendDescriptor::REPLACE;
const BLEND_BEHIND: wgpu::BlendDescriptor = wgpu::BlendDescriptor {
    src_factor: wgpu::BlendFactor::BlendColor,
    dst_factor: wgpu::BlendFactor::OneMinusBlendColor,
    operation: wgpu::BlendOperation::Add,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Visibility {
    Front,
    Behind,
}
type Selector = (Visibility, wgpu::InputStepMode);

#[repr(C)]
#[derive(Clone, Copy, Debug, zerocopy::AsBytes, zerocopy::FromBytes)]
pub struct Position {
    pub pos: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, zerocopy::AsBytes, zerocopy::FromBytes)]
pub struct Color {
    pub color: u32,
}

#[repr(C)]
#[derive(Clone, Copy, zerocopy::AsBytes, zerocopy::FromBytes)]
struct Locals {
    color: [f32; 4],
    _pad: [f32; 60],
}

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

    pub fn add(
        &mut self,
        from: [f32; 3],
        to: [f32; 3],
        color: u32,
    ) {
        self.vertices.push(Position {
            pos: [from[0], from[1], from[2], 1.0],
        });
        self.vertices.push(Position {
            pos: [to[0], to[1], to[2], 1.0],
        });
        let color = Color {
            color,
        };
        self.colors.push(color);
        self.colors.push(color);
    }
}

pub struct Context {
    settings: settings::DebugRender,
    pipeline_layout: wgpu::PipelineLayout,
    pipelines_line: HashMap<Selector, wgpu::RenderPipeline>,
    pipeline_face: Option<wgpu::RenderPipeline>,
    pipeline_edge: Option<wgpu::RenderPipeline>,
    line_color_buf: wgpu::Buffer,
    bind_group_line: wgpu::BindGroup,
    bind_group_face: wgpu::BindGroup,
    bind_group_edge: wgpu::BindGroup,
}

impl Context {
    pub fn new(
        device: &wgpu::Device,
        settings: &settings::DebugRender,
        global: &GlobalContext,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            bindings: &[
                wgpu::BindGroupLayoutBinding { // locals
                    binding: 0,
                    visibility: wgpu::ShaderStage::FRAGMENT,
                    ty: wgpu::BindingType::UniformBuffer { dynamic: false },
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            bind_group_layouts: &[
                &global.bind_group_layout,
                &bind_group_layout,
            ],
        });

        let line_color_buf = device
            .create_buffer_mapped(1, wgpu::BufferUsage::VERTEX)
            .fill_from_slice(&[
                Color { color: 0xFF000080 }, // line
            ]);

        let locals_buf = device
            .create_buffer_mapped(3, wgpu::BufferUsage::UNIFORM)
            .fill_from_slice(&[
                Locals::new([1.0; 4]), // line
                Locals::new([0.0, 1.0, 0.0, 0.1]), // face
                Locals::new([1.0, 1.0, 0.0, 0.1]), // edge
            ]);
        let locals_size = mem::size_of::<Locals>() as wgpu::BufferAddress;
        let bind_group_line = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout,
            bindings: &[
                wgpu::Binding {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer {
                        buffer: &locals_buf,
                        range: 0*locals_size .. 1*locals_size,
                    },
                },
            ],
        });
        let bind_group_face = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout,
            bindings: &[
                wgpu::Binding {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer {
                        buffer: &locals_buf,
                        range: 1*locals_size .. 2*locals_size,
                    },
                },
            ],
        });
        let bind_group_edge = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout,
            bindings: &[
                wgpu::Binding {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer {
                        buffer: &locals_buf,
                        range: 2*locals_size .. 3*locals_size,
                    },
                },
            ],
        });

        let mut result = Context {
            settings: settings.clone(),
            pipeline_layout,
            pipelines_line: HashMap::new(),
            pipeline_face: None,
            pipeline_edge: None,
            line_color_buf,
            bind_group_line,
            bind_group_face,
            bind_group_edge,
        };
        result.reload(device);
        result
    }

    pub fn reload(&mut self, device: &wgpu::Device) {
        let rasterization_state = wgpu::RasterizationStateDescriptor {
            front_face: wgpu::FrontFace::Ccw,
            // original was not drawn with rasterizer, used no culling
            cull_mode: wgpu::CullMode::None,
            depth_bias: 0,
            depth_bias_slope_scale: 0.0,
            depth_bias_clamp: 0.0,
        };

        if self.settings.collision_shapes {
            let shaders = Shaders::new("debug_shape", &[], device)
                .unwrap();
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                layout: &self.pipeline_layout,
                vertex_stage: wgpu::ProgrammableStageDescriptor {
                    module: &shaders.vs,
                    entry_point: "main",
                },
                fragment_stage: Some(wgpu::ProgrammableStageDescriptor {
                    module: &shaders.fs,
                    entry_point: "main",
                }),
                rasterization_state: Some(rasterization_state.clone()),
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
                    depth_write_enabled: false,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil_front: wgpu::StencilStateFaceDescriptor::IGNORE,
                    stencil_back: wgpu::StencilStateFaceDescriptor::IGNORE,
                    stencil_read_mask: !0,
                    stencil_write_mask: !0,
                }),
                index_format: wgpu::IndexFormat::Uint16,
                vertex_buffers: &[
                    wgpu::VertexBufferDescriptor {
                        stride: mem::size_of::<ShapePolygon>() as wgpu::BufferAddress,
                        step_mode: wgpu::InputStepMode::Instance,
                        attributes: &[
                            wgpu::VertexAttributeDescriptor { // indices
                                offset: 0,
                                format: wgpu::VertexFormat::Ushort4,
                                shader_location: 0,
                            },
                            wgpu::VertexAttributeDescriptor { // normal
                                offset: 8,
                                format: wgpu::VertexFormat::Uchar4Norm,
                                shader_location: 1,
                            },
                            wgpu::VertexAttributeDescriptor { // origin square
                                offset: 12,
                                format: wgpu::VertexFormat::Float4,
                                shader_location: 2,
                            },
                        ],
                    },
                ],
                sample_count: 1,
                alpha_to_coverage_enabled: false,
                sample_mask: !0,
            });
            self.pipeline_face = Some(pipeline);
            self.pipeline_edge = None; //TODO: line raster
        }

        self.pipelines_line.clear();
        if self.settings.impulses {
            let shaders = Shaders::new("debug", &[], device)
                .unwrap();
            for &visibility in &[Visibility::Front, Visibility::Behind] {
                let (blend, depth_write_enabled, depth_compare) = match visibility {
                    Visibility::Front => (&BLEND_FRONT, true, wgpu::CompareFunction::LessEqual),
                    Visibility::Behind => (&BLEND_BEHIND, false, wgpu::CompareFunction::Greater),
                };
                for &color_rate in &[wgpu::InputStepMode::Vertex, wgpu::InputStepMode::Instance] {
                    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        layout: &self.pipeline_layout,
                        vertex_stage: wgpu::ProgrammableStageDescriptor {
                            module: &shaders.vs,
                            entry_point: "main",
                        },
                        fragment_stage: Some(wgpu::ProgrammableStageDescriptor {
                            module: &shaders.fs,
                            entry_point: "main",
                        }),
                        rasterization_state: Some(rasterization_state.clone()),
                        primitive_topology: wgpu::PrimitiveTopology::TriangleStrip,
                        color_states: &[
                            wgpu::ColorStateDescriptor {
                                format: COLOR_FORMAT,
                                alpha_blend: blend.clone(),
                                color_blend: blend.clone(),
                                write_mask: wgpu::ColorWrite::all(),
                            },
                        ],
                        depth_stencil_state: Some(wgpu::DepthStencilStateDescriptor {
                            format: DEPTH_FORMAT,
                            depth_write_enabled,
                            depth_compare,
                            stencil_front: wgpu::StencilStateFaceDescriptor::IGNORE,
                            stencil_back: wgpu::StencilStateFaceDescriptor::IGNORE,
                            stencil_read_mask: !0,
                            stencil_write_mask: !0,
                        }),
                        index_format: wgpu::IndexFormat::Uint16,
                        vertex_buffers: &[
                            wgpu::VertexBufferDescriptor {
                                stride: mem::size_of::<Position>() as wgpu::BufferAddress,
                                step_mode: wgpu::InputStepMode::Vertex,
                                attributes: &[
                                    wgpu::VertexAttributeDescriptor {
                                        offset: 0,
                                        format: wgpu::VertexFormat::Float4,
                                        shader_location: 0,
                                    },
                                ],
                            },
                            wgpu::VertexBufferDescriptor {
                                stride: mem::size_of::<Color>() as wgpu::BufferAddress,
                                step_mode: color_rate,
                                attributes: &[
                                    wgpu::VertexAttributeDescriptor {
                                        offset: 0,
                                        format: wgpu::VertexFormat::Float4,
                                        shader_location: 1,
                                    },
                                ],
                            },
                        ],
                        sample_count: 1,
                        alpha_to_coverage_enabled: false,
                        sample_mask: !0,
                    });
                    self.pipelines_line.insert((visibility, color_rate), pipeline);
                }
            }
        }
    }

    fn draw_liner(
        &self,
        pass: &mut wgpu::RenderPass,
        vertex_buf: &wgpu::Buffer,
        color_buf: &wgpu::Buffer,
        color_rate: wgpu::InputStepMode,
        num_vert: usize,
    ) {
        pass.set_blend_color(wgpu::Color::WHITE);
        pass.set_vertex_buffers(0, &[
            (vertex_buf, 0),
            (color_buf, 0),
        ]);
        for &vis in &[Visibility::Front, Visibility::Behind] {
            if let Some(ref pipeline) = self.pipelines_line.get(&(vis, color_rate)) {
                pass.set_pipeline(pipeline);
                pass.draw(0 .. num_vert as u32, 0 .. 1);
            }
        }
    }

    pub fn draw_shape(
        &self,
        pass: &mut wgpu::RenderPass,
        shape: &model::Shape,
    ) {
        // draw collision polygon faces
        if let Some(ref pipeline) = self.pipeline_face {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(1, &self.bind_group_face, &[]);
            pass.set_vertex_buffers(0, &[(&shape.polygon_buf, 0)]);
            //pass.draw(); TODO
        }
        // draw collision polygon edges
        if let Some(ref pipeline) = self.pipeline_edge {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(1, &self.bind_group_edge, &[]);
            pass.set_vertex_buffers(0, &[(&shape.polygon_buf, 0)]);
            //pass.draw(); TODO
        }

        // draw sample normals
        if let Some((ref sample_buf, num_vert)) = shape.sample_buf {
            pass.set_bind_group(1, &self.bind_group_line, &[]);
            self.draw_liner(
                pass,
                sample_buf,
                &self.line_color_buf,
                wgpu::InputStepMode::Instance,
                num_vert,
            );
        }
    }

    pub fn draw_lines(
        &self,
        pass: &mut wgpu::RenderPass,
        device: &wgpu::Device,
        linebuf: &LineBuffer,
    ){
        let vertex_buf = device
            .create_buffer_mapped(linebuf.vertices.len(), wgpu::BufferUsage::VERTEX)
            .fill_from_slice(&linebuf.vertices);
        let color_buf = device
            .create_buffer_mapped(linebuf.colors.len(), wgpu::BufferUsage::VERTEX)
            .fill_from_slice(&linebuf.colors);
        assert_eq!(linebuf.vertices.len(), linebuf.colors.len());

        self.draw_liner(
            pass,
            &vertex_buf,
            &color_buf,
            wgpu::InputStepMode::Vertex,
            linebuf.vertices.len(),
        );
    }
}
