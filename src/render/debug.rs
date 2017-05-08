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


gfx_defines! {
    vertex DebugVertex {
        pos: [i8; 4] = "a_Pos",
    }

    constant DebugLocals {
        m_mvp: [[f32; 4]; 4] = "u_ModelViewProj",
        v_color: [f32; 4] = "u_Color",
    }

    pipeline debug {
        vbuf: gfx::VertexBuffer<DebugVertex> = (),
        locals: gfx::ConstantBuffer<DebugLocals> = "c_Locals",
        out_color: gfx::BlendTarget<ColorFormat> = ("Target0", gfx::state::MASK_ALL, gfx::preset::blend::ALPHA),
        out_depth: gfx::DepthTarget<DepthFormat> = gfx::preset::depth::LESS_EQUAL_TEST,
    }
}


pub struct DebugContext<R: gfx::Resources> {
    data: debug::Data<R>,
    pso_line_front: gfx::PipelineState<R, debug::Meta>,
    pso_line_behind: gfx::PipelineState<R, debug::Meta>,
    pso_triangle: gfx::PipelineState<R, debug::Meta>,
}

impl<R: gfx::Resources> DebugContext<R> {
    fn with_data<F: gfx::Factory<R>>(factory: &mut F, data: debug::Data<R>)
                 -> DebugContext<R>
    {
        let program = factory.link_program(
            &read_file("data/shader/debug.vert"),
            &read_file("data/shader/debug.frag"),
            ).unwrap();
        let raster = gfx::state::Rasterizer::new_fill();
        DebugContext {
            data: data,
            pso_line_front: factory.create_pipeline_from_program(
                &program, gfx::Primitive::LineList, raster, debug::Init {
                    out_color: ("Target0", gfx::state::MASK_ALL, BLEND_FRONT),
                    out_depth: gfx::preset::depth::LESS_EQUAL_WRITE,
                    .. debug::new()
                }).unwrap(),
            pso_line_behind: factory.create_pipeline_from_program(
                &program, gfx::Primitive::LineList, raster, debug::Init {
                    out_color: ("Target0", gfx::state::MASK_ALL, BLEND_BEHIND),
                    out_depth: DEPTH_BEHIND,
                    .. debug::new()
                }).unwrap(),
            pso_triangle: factory.create_pipeline_from_program(
                &program, gfx::Primitive::TriangleList, raster, debug::new()
                ).unwrap(),
        }
    }

    pub fn new<F: gfx::Factory<R>>(factory: &mut F,
               out_color: gfx::handle::RenderTargetView<R, ColorFormat>,
               out_depth: gfx::handle::DepthStencilView<R, DepthFormat>)
               -> DebugContext<R>
    {
        let data = debug::Data {
            vbuf: factory.create_vertex_buffer(&[]),
            locals: factory.create_constant_buffer(1),
            out_color: out_color,
            out_depth: out_depth,
        };
        Self::with_data(factory, data)
    }

    pub fn reload<F: gfx::Factory<R>>(&mut self, factory: &mut F) {
        *self = Self::with_data(factory, self.data.clone());
    }

    pub fn draw<C>(&mut self, shape: &model::DebugShape<R>,
                   transform: cgmath::Matrix4<f32>,
                   encoder: &mut gfx::Encoder<R, C>) where
        C: gfx::CommandBuffer<R>,
    {
        let mut locals = DebugLocals {
            m_mvp: transform.into(),
            v_color: [0.0, 1.0, 0.0, 0.1],
        };
        self.data.vbuf = shape.bound_vb.clone();
        encoder.update_constant_buffer(&self.data.locals, &locals);
        encoder.draw(&shape.bound_slice, &self.pso_triangle, &self.data);
        self.data.vbuf = shape.sample_vb.clone();
        locals.v_color = [1.0, 0.0, 0.0, 0.5];
        encoder.update_constant_buffer(&self.data.locals, &locals);
        let slice = gfx::Slice::new_match_vertex_buffer(&shape.sample_vb);
        encoder.draw(&slice, &self.pso_line_front, &self.data);
        encoder.draw(&slice, &self.pso_line_behind, &self.data);
    }
}