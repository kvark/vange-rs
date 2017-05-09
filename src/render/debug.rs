use std::collections::HashMap;

use cgmath;
use gfx;
use gfx::traits::FactoryExt;

use model;
use super::{ColorFormat, DepthFormat, read_file};


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
        source: gfx::state::Factor::ZeroPlus(gfx::state::BlendValue::DestColor),
        destination: gfx::state::Factor::OneMinus(gfx::state::BlendValue::DestColor),
    },
    alpha: gfx::state::BlendChannel {
        equation: gfx::state::Equation::Add,
        source: gfx::state::Factor::ZeroPlus(gfx::state::BlendValue::DestAlpha),
        destination: gfx::state::Factor::OneMinus(gfx::state::BlendValue::DestAlpha),
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
        pos: [i8; 4] = "a_Pos",
    }

    vertex DebugColor {
        color: [f32; 4] = "a_Color",
    }

    constant DebugLocals {
        m_mvp: [[f32; 4]; 4] = "u_ModelViewProj",
    }

    pipeline debug {
        buf_pos: gfx::VertexBuffer<DebugPos> = (),
        buf_col: gfx::pso::buffer::VertexBufferCommon<DebugColor, gfx::pso::buffer::InstanceRate> = 1,
        locals: gfx::ConstantBuffer<DebugLocals> = "c_Locals",
        out_color: gfx::BlendTarget<ColorFormat> = ("Target0", gfx::state::MASK_ALL, gfx::preset::blend::ALPHA),
        out_depth: gfx::DepthTarget<DepthFormat> = gfx::preset::depth::LESS_EQUAL_TEST,
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
}


pub struct DebugContext<R: gfx::Resources> {
    buf_color: gfx::handle::Buffer<R, DebugColor>,
    data: debug::Data<R>,
    psos_line: HashMap<Selector, gfx::PipelineState<R, debug::Meta>>,
    pso_triangle: Option<gfx::PipelineState<R, debug::Meta>>,
}

impl<R: gfx::Resources> DebugContext<R> {
    pub fn new<F: gfx::Factory<R>>(factory: &mut F,
               out_color: gfx::handle::RenderTargetView<R, ColorFormat>,
               out_depth: gfx::handle::DepthStencilView<R, DepthFormat>)
               -> DebugContext<R>
    {
        let data = debug::Data {
            buf_pos: factory.create_vertex_buffer(&[]),
            buf_col: factory.create_buffer(1, gfx::buffer::Role::Vertex,
                gfx::memory::Usage::Dynamic, gfx::Bind::empty()).unwrap(),
            locals: factory.create_constant_buffer(1),
            out_color: out_color,
            out_depth: out_depth,
        };
        let mut result = DebugContext {
            buf_color: data.buf_col.clone(),
            data: data,
            psos_line: HashMap::new(),
            pso_triangle: None,
        };
        result.reload(factory);
        result
    }

    pub fn reload<F: gfx::Factory<R>>(&mut self, factory: &mut F) {
        let program = factory.link_program(
            &read_file("data/shader/debug.vert"),
            &read_file("data/shader/debug.frag"),
            ).unwrap();
        let raster = gfx::state::Rasterizer::new_fill();

        self.pso_triangle = Some(factory.create_pipeline_from_program(
            &program, gfx::Primitive::TriangleList, raster, debug::new()
            ).unwrap());

        self.psos_line.clear();
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
                let pso = factory.create_pipeline_from_program(
                    &program, gfx::Primitive::LineList, raster, debug::Init {
                        out_color: ("Target0", gfx::state::MASK_ALL, blend),
                        out_depth: depth,
                        buf_col: rate,
                        .. debug::new()
                    }).unwrap();
                self.psos_line.insert((visibility, color_rate), pso);
            }
        }
    }

    pub fn draw_shape<C>(&mut self, shape: &model::DebugShape<R>,
                      transform: cgmath::Matrix4<f32>,
                      encoder: &mut gfx::Encoder<R, C>) where
        C: gfx::CommandBuffer<R>,
    {
        encoder.update_constant_buffer(&self.data.locals, &DebugLocals {
            m_mvp: transform.into(),
        });

        if let Some(ref pso) = self.pso_triangle {
            self.data.buf_pos = shape.bound_vb.clone();
            encoder.update_buffer(&self.buf_color, &[DebugColor {
                color: [0.0, 1.0, 0.0, 0.1],
            }], 0).unwrap();
            encoder.draw(&shape.bound_slice, pso, &self.data);
        }

        self.data.buf_pos = shape.sample_vb.clone();
        encoder.update_buffer(&self.buf_color, &[DebugColor {
            color: [1.0, 0.0, 0.0, 0.5],
        }], 0).unwrap();
        let slice = gfx::Slice::new_match_vertex_buffer(&shape.sample_vb);
        for &vis in &[Visibility::Front, Visibility::Behind] {
            if let Some(ref pso) = self.psos_line.get(&(vis, ColorRate::Instance)) {
                encoder.draw(&slice, pso, &self.data);
            }
        }
    }
}
