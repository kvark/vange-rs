use crate::{
    config::settings,
    model,
    render::{
        Shaders,
        COLOR_FORMAT, DEPTH_FORMAT,
        global::Context as GlobalContext,
    },
};

use cgmath;
use wgpu;

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
#[derive(Clone, Copy)]
pub struct Position {
    pub pos: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Color {
    pub color: [f32; 4],
}

/*
gfx_defines! {
    constant DebugLocals {
        m_mvp: [[f32; 4]; 4] = "u_ModelViewProj",
        color: [f32; 4] = "u_Color",
    }
}*/

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
        c: u32,
    ) {
        self.vertices.push(Position {
            pos: [from[0], from[1], from[2], 1.0],
        });
        self.vertices.push(Position {
            pos: [to[0], to[1], to[2], 1.0],
        });
        let color = Color {
            color: [
                ((c >> 24) & 0xFF) as f32 / 255.0,
                ((c >> 16) & 0xFF) as f32 / 255.0,
                ((c >> 8) & 0xFF) as f32 / 255.0,
                (c & 0xFF) as f32 / 255.0,
            ],
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
                    visibility: wgpu::ShaderStageFlags::VERTEX,
                    ty: wgpu::BindingType::UniformBuffer,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            bind_group_layouts: &[
                &global.bind_group_layout,
                &bind_group_layout,
            ],
        });
        let mut result = Context {
            settings: settings.clone(),
            pipeline_layout,
            pipelines_line: HashMap::new(),
            pipeline_face: None,
            pipeline_edge: None,
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
                vertex_stage: wgpu::PipelineStageDescriptor {
                    module: &shaders.vs,
                    entry_point: "main",
                },
                fragment_stage: wgpu::PipelineStageDescriptor {
                    module: &shaders.fs,
                    entry_point: "main",
                },
                rasterization_state: rasterization_state.clone(),
                primitive_topology: wgpu::PrimitiveTopology::TriangleStrip,
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
                        stride: mem::size_of::<Position>() as u32,
                        step_mode: wgpu::InputStepMode::Vertex,
                        attributes: &[
                            wgpu::VertexAttributeDescriptor {
                                offset: 0,
                                format: wgpu::VertexFormat::Float4,
                                attribute_index: 0,
                            },
                        ],
                    },
                    wgpu::VertexBufferDescriptor {
                        stride: mem::size_of::<Color>() as u32,
                        step_mode: wgpu::InputStepMode::Vertex,
                        attributes: &[
                            wgpu::VertexAttributeDescriptor {
                                offset: 0,
                                format: wgpu::VertexFormat::Float4,
                                attribute_index: 0,
                            },
                        ],
                    },
                ],
                sample_count: 1,
            });
            self.pipeline_face = Some(pipeline);
            self.pipeline_edge = None; //TODO: line raster
        }

        self.pipelines_line.clear();
        if self.settings.impulses {
            let shaders = Shaders::new("debug_shape", &[], device)
                .unwrap();
            for &visibility in &[Visibility::Front, Visibility::Behind] {
                let (blend, depth_write_enabled, depth_compare) = match visibility {
                    Visibility::Front => (&BLEND_FRONT, true, wgpu::CompareFunction::LessEqual),
                    Visibility::Behind => (&BLEND_BEHIND, false, wgpu::CompareFunction::Greater),
                };
                for &color_rate in &[wgpu::InputStepMode::Vertex, wgpu::InputStepMode::Instance] {
                    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        layout: &self.pipeline_layout,
                        vertex_stage: wgpu::PipelineStageDescriptor {
                            module: &shaders.vs,
                            entry_point: "main",
                        },
                        fragment_stage: wgpu::PipelineStageDescriptor {
                            module: &shaders.fs,
                            entry_point: "main",
                        },
                        rasterization_state: rasterization_state.clone(),
                        primitive_topology: wgpu::PrimitiveTopology::TriangleStrip,
                        color_states: &[
                            wgpu::ColorStateDescriptor {
                                format: COLOR_FORMAT,
                                alpha: blend.clone(),
                                color: blend.clone(),
                                write_mask: wgpu::ColorWriteFlags::all(),
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
                                stride: mem::size_of::<Position>() as u32,
                                step_mode: wgpu::InputStepMode::Vertex,
                                attributes: &[
                                    wgpu::VertexAttributeDescriptor {
                                        offset: 0,
                                        format: wgpu::VertexFormat::Float4,
                                        attribute_index: 0,
                                    },
                                ],
                            },
                            wgpu::VertexBufferDescriptor {
                                stride: mem::size_of::<Color>() as u32,
                                step_mode: color_rate,
                                attributes: &[
                                    wgpu::VertexAttributeDescriptor {
                                        offset: 0,
                                        format: wgpu::VertexFormat::Float4,
                                        attribute_index: 0,
                                    },
                                ],
                            },
                        ],
                        sample_count: 1,
                    });
                    self.pipelines_line.insert((visibility, color_rate), pipeline);
                }
            }
        }
    }

    fn _draw_liner(
        &mut self,
        _buf: &wgpu::Buffer,
        _num_verts: Option<usize>,
        _pass: &mut wgpu::RenderPass,
    ) {
        /* TODO
        let (color_rate, slice) = match num_verts {
            Some(num) => (
                ColorRate::Vertex,
                gfx::Slice {
                    end: num as gfx::VertexCount,
                    ..gfx::Slice::new_match_vertex_buffer(&buf)
                },
            ),
            None => (
                ColorRate::Instance,
                gfx::Slice::new_match_vertex_buffer(&buf),
            ),
        };
        self.data.buf_pos = buf;
        for &vis in &[Visibility::Front, Visibility::Behind] {
            if let Some(ref pso) = self.psos_line.get(&(vis, color_rate)) {
                encoder.draw(&slice, pso, &self.data);
            }
        }*/
    }

    pub fn draw_shape(
        &mut self,
        _pass: &mut wgpu::RenderPass,
        _shape: &model::Shape,
        _transform: cgmath::Matrix4<f32>,
    ) {
        /* TODO
        let mut locals = DebugLocals {
            m_mvp: transform.into(),
            color: [0.0, 1.0, 0.0, 0.1],
        };
        let slice = shape.make_draw_slice();

        let data = shape::Data {
            vertices: shape.vertex_view.clone(),
            polygons: shape.polygon_buf.clone(),
            locals: self.data.locals.clone(),
            out_color: self.data.out_color.clone(),
            out_depth: self.data.out_depth.clone(),
        };

        // draw collision polygon faces
        if let Some(ref pso) = self.pso_face {
            locals.color = [0.0, 1.0, 0.0, 0.1];
            encoder.update_constant_buffer(&self.data.locals, &locals);
            encoder.draw(&slice, pso, &data);
        }

        // draw collision polygon edges
        if let Some(ref pso) = self.pso_edge {
            locals.color = [1.0, 1.0, 0.0, 0.1];
            encoder.update_constant_buffer(&self.data.locals, &locals);
            encoder.draw(&slice, pso, &data);
        }

        // draw sample normals
        if let Some(ref samples) = shape.sample_buf {
            encoder.update_constant_buffer(&self.data.locals, &locals);
            encoder
                .update_buffer(
                    &self.buf_col,
                    &[
                        DebugColor {
                            color: [1.0, 0.0, 0.0, 0.5],
                        },
                    ],
                    0,
                )
                .unwrap();
            self.draw_liner(samples.clone(), None, encoder);
        }*/
    }

    pub fn draw_lines(
        &mut self,
        _linebuf: &LineBuffer,
        _transform: cgmath::Matrix4<f32>,
        _encoder: &mut wgpu::CommandEncoder,
    ){
        /* TODO
        let mut vertices = linebuf.vertices.as_slice();
        let mut colors = linebuf.colors.as_slice();
        if vertices.len() > self.settings.max_vertices {
            error!(
                "Exceeded the maximum vertex capacity: {} > {}",
                vertices.len(),
                self.settings.max_vertices
            );
            vertices = &vertices[.. self.settings.max_vertices];
        }
        if vertices.len() != colors.len() {
            error!(
                "Lengths of debug vertices {} != colors {}",
                vertices.len(),
                colors.len()
            );
            if vertices.len() > colors.len() {
                vertices = &vertices[.. colors.len()];
            } else {
                colors = &colors[.. vertices.len()];
            }
        }

        encoder.update_constant_buffer(
            &self.data.locals,
            &DebugLocals {
                m_mvp: transform.into(),
                color: [0.0; 4],
            },
        );

        encoder.update_buffer(&self.buf_pos, vertices, 0).unwrap();
        encoder.update_buffer(&self.buf_col, colors, 0).unwrap();
        let buf = self.buf_pos.clone();
        self.draw_liner(buf, Some(vertices.len()), encoder);
        */
    }
}
