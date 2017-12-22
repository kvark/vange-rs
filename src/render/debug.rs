use std::collections::HashMap;

use cgmath;
use gfx;
pub use gfx::pso::buffer::{InstanceRate, VertexBufferCommon};
use gfx::traits::FactoryExt; // reduce the line width at use site

use super::{read_file, ColorFormat, DepthFormat, MainTargets};
use config::settings;
use model;


const BLEND_FRONT: gfx::state::Blend = gfx::state::Blend {
    color: gfx::state::BlendChannel {
        equation: gfx::state::Equation::Add,
        source: gfx::state::Factor::One,
        destination: gfx::state::Factor::Zero,
    },
    alpha: gfx::state::BlendChannel {
        equation: gfx::state::Equation::Add,
        source: gfx::state::Factor::One,
        destination: gfx::state::Factor::Zero,
    },
};

const BLEND_BEHIND: gfx::state::Blend = gfx::state::Blend {
    color: gfx::state::BlendChannel {
        equation: gfx::state::Equation::Add,
        source: gfx::state::Factor::ZeroPlus(gfx::state::BlendValue::ConstAlpha),
        destination: gfx::state::Factor::OneMinus(gfx::state::BlendValue::ConstAlpha),
    },
    alpha: gfx::state::BlendChannel {
        equation: gfx::state::Equation::Add,
        source: gfx::state::Factor::ZeroPlus(gfx::state::BlendValue::ConstAlpha),
        destination: gfx::state::Factor::OneMinus(gfx::state::BlendValue::ConstAlpha),
    },
};

const DEPTH_BEHIND: gfx::state::Depth = gfx::state::Depth {
    fun: gfx::state::Comparison::Greater,
    write: false,
};

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum Visibility {
    Front,
    Behind,
}
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum ColorRate {
    Vertex,
    Instance,
}
type Selector = (Visibility, ColorRate);

gfx_defines! {
    vertex DebugPos {
        pos: [f32; 4] = "a_Pos",
    }

    vertex DebugColor {
        color: [f32; 4] = "a_Color",
    }

    constant DebugLocals {
        m_mvp: [[f32; 4]; 4] = "u_ModelViewProj",
    }

    pipeline debug {
        buf_pos: gfx::VertexBuffer<DebugPos> = (),
        buf_col: VertexBufferCommon<DebugColor, InstanceRate> = 1,
        locals: gfx::ConstantBuffer<DebugLocals> = "c_Locals",
        out_color: gfx::BlendTarget<ColorFormat> = ("Target0", gfx::state::MASK_ALL, gfx::preset::blend::ALPHA),
        out_depth: gfx::DepthTarget<DepthFormat> = gfx::preset::depth::LESS_EQUAL_TEST,
        blend_ref: gfx::BlendRef = (),
    }
}

pub struct LineBuffer {
    vertices: Vec<DebugPos>,
    colors: Vec<DebugColor>,
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
        self.vertices.push(DebugPos {
            pos: [from[0], from[1], from[2], 1.0],
        });
        self.vertices.push(DebugPos {
            pos: [to[0], to[1], to[2], 1.0],
        });
        let color = DebugColor {
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

pub struct DebugRender<R: gfx::Resources> {
    settings: settings::DebugRender,
    buf_pos: gfx::handle::Buffer<R, DebugPos>,
    buf_col: gfx::handle::Buffer<R, DebugColor>,
    data: debug::Data<R>,
    psos_line: HashMap<Selector, gfx::PipelineState<R, debug::Meta>>,
    pso_face: Option<gfx::PipelineState<R, debug::Meta>>,
    pso_edge: Option<gfx::PipelineState<R, debug::Meta>>,
}

impl<R: gfx::Resources> DebugRender<R> {
    pub fn new<F: gfx::Factory<R>>(
        factory: &mut F,
        targets: MainTargets<R>,
        settings: &settings::DebugRender,
    ) -> Self {
        let data = debug::Data {
            buf_pos: factory
                .create_buffer(
                    settings.max_vertices,
                    gfx::buffer::Role::Vertex,
                    gfx::memory::Usage::Dynamic,
                    gfx::Bind::empty(),
                )
                .unwrap(),
            buf_col: factory
                .create_buffer(
                    settings.max_vertices,
                    gfx::buffer::Role::Vertex,
                    gfx::memory::Usage::Dynamic,
                    gfx::Bind::empty(),
                )
                .unwrap(),
            locals: factory.create_constant_buffer(1),
            out_color: targets.color,
            out_depth: targets.depth,
            blend_ref: [0.0, 0.0, 0.0, 0.5],
        };
        let mut result = DebugRender {
            settings: settings.clone(),
            buf_pos: data.buf_pos.clone(),
            buf_col: data.buf_col.clone(),
            data,
            psos_line: HashMap::new(),
            pso_face: None,
            pso_edge: None,
        };
        result.reload(factory);
        result
    }

    pub fn reload<F: gfx::Factory<R>>(
        &mut self,
        factory: &mut F,
    ) {
        let program = factory
            .link_program(
                &read_file("data/shader/debug.vert"),
                &read_file("data/shader/debug.frag"),
            )
            .unwrap();
        let raster = gfx::state::Rasterizer::new_fill();

        if self.settings.collision_shapes {
            self.pso_face = Some(
                factory
                    .create_pipeline_from_program(
                        &program,
                        gfx::Primitive::TriangleList,
                        raster,
                        debug::new(),
                    )
                    .unwrap(),
            );
            self.pso_edge = Some(
                factory
                    .create_pipeline_from_program(
                        &program,
                        gfx::Primitive::TriangleList,
                        gfx::state::Rasterizer {
                            method: gfx::state::RasterMethod::Line(1),
                            .. raster
                        },
                        debug::new(),
                    )
                    .unwrap(),
            );
        }

        self.psos_line.clear();
        if self.settings.impulses {
            for &visibility in &[Visibility::Front, Visibility::Behind] {
                for &color_rate in &[ColorRate::Vertex, ColorRate::Instance] {
                    let (blend, depth) = match visibility {
                        Visibility::Front => (BLEND_FRONT, gfx::preset::depth::LESS_EQUAL_WRITE),
                        Visibility::Behind => (BLEND_BEHIND, DEPTH_BEHIND),
                    };
                    let rate = match color_rate {
                        ColorRate::Vertex => 0,
                        ColorRate::Instance => 1,
                    };
                    let pso = factory
                        .create_pipeline_from_program(
                            &program,
                            gfx::Primitive::LineList,
                            raster,
                            debug::Init {
                                out_color: ("Target0", gfx::state::MASK_ALL, blend),
                                out_depth: depth,
                                buf_col: rate,
                                ..debug::new()
                            },
                        )
                        .unwrap();
                    self.psos_line.insert((visibility, color_rate), pso);
                }
            }
        }
    }

    pub fn resize(&mut self, targets: MainTargets<R>) {
        self.data.out_color = targets.color;
        self.data.out_depth = targets.depth;
    }

    fn draw_liner<C>(
        &mut self,
        buf: gfx::handle::Buffer<R, DebugPos>,
        num_verts: Option<usize>,
        encoder: &mut gfx::Encoder<R, C>,
    ) where
        C: gfx::CommandBuffer<R>,
    {
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
        }
    }

    pub fn draw_shape<C>(
        &mut self,
        shape: &model::DebugShape<R>,
        transform: cgmath::Matrix4<f32>,
        encoder: &mut gfx::Encoder<R, C>,
    ) where
        C: gfx::CommandBuffer<R>,
    {
        encoder.update_constant_buffer(
            &self.data.locals,
            &DebugLocals {
                m_mvp: transform.into(),
            },
        );

        // draw collision polygon faces
        if let Some(ref pso) = self.pso_face {
            self.data.buf_pos = shape.bound_vb.clone();
            encoder
                .update_buffer(
                    &self.buf_col,
                    &[
                        DebugColor {
                            color: [0.0, 1.0, 0.0, 0.1],
                        },
                    ],
                    0,
                )
                .unwrap();
            encoder.draw(&shape.bound_slice, pso, &self.data);
        }

        // draw collision polygon edges
        if let Some(ref pso) = self.pso_edge {
            encoder
                .update_buffer(
                    &self.buf_col,
                    &[
                        DebugColor {
                            color: [1.0, 1.0, 0.0, 0.1],
                        },
                    ],
                    0,
                )
                .unwrap();
            encoder.draw(&shape.bound_slice, pso, &self.data);
        }

        // draw sample normals
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
        self.draw_liner(shape.sample_vb.clone(), None, encoder);
    }

    pub fn draw_lines<C>(
        &mut self,
        linebuf: &LineBuffer,
        transform: cgmath::Matrix4<f32>,
        encoder: &mut gfx::Encoder<R, C>,
    ) where
        C: gfx::CommandBuffer<R>,
    {
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
            },
        );

        encoder.update_buffer(&self.buf_pos, vertices, 0).unwrap();
        encoder.update_buffer(&self.buf_col, colors, 0).unwrap();
        let buf = self.buf_pos.clone();
        self.draw_liner(buf, Some(vertices.len()), encoder);
    }
}
