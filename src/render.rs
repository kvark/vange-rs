use gfx;
use Camera;

const TEX_HEIGHT: i32 = 4096;

pub type ColorFormat = gfx::format::Srgba8;
pub type DepthFormat = gfx::format::DepthStencil;

gfx_defines!{
    vertex Vertex {
        pos: [i8; 4] = "a_Pos",
    }

    constant Locals {
        cam_pos: [f32; 4] = "u_CamPose",
        m_vp: [[f32; 4]; 4] = "u_ViewProj",
        m_inv_vp: [[f32; 4]; 4] = "u_InvViewProj",
    }

    pipeline terrain {
        vbuf: gfx::VertexBuffer<Vertex> = (),
        locals: gfx::ConstantBuffer<Locals> = "c_Locals",
        height: gfx::TextureSampler<f32> = "t_Height",
        meta: gfx::TextureSampler<u32> = "t_Meta",
        out: gfx::RenderTarget<ColorFormat> = "Target0",
    }
}

pub struct Render<R: gfx::Resources> {
    terrain: gfx::Bundle<R, terrain::Data<R>>,
}

fn read(name: &str) -> Vec<u8> {
    use std::io::{BufReader, Read};
    use std::fs::File;
    let mut buf = Vec::new();
    let mut file = BufReader::new(File::open(name).unwrap());
    file.read_to_end(&mut buf).unwrap();
    buf
}

fn load_pso<R: gfx::Resources, F: gfx::Factory<R>>(factory: &mut F)
    -> gfx::PipelineState<R, terrain::Meta> {
    use gfx::traits::FactoryExt;
    let program = factory.link_program(
        &read("data/shader/terrain.vert"),
        &read("data/shader/terrain.frag"),
        ).unwrap();
    factory.create_pipeline_from_program(
        &program, gfx::Primitive::TriangleList,
        gfx::state::Rasterizer::new_fill(),
        terrain::new()
    ).unwrap()
}

pub fn init<R: gfx::Resources, F: gfx::Factory<R>>(factory: &mut F,
            main_color: gfx::handle::RenderTargetView<R, ColorFormat>,
            size: (i32, i32), height_data: &[u8], meta_data: &[u8])
            -> Render<R>
{
    use gfx::traits::FactoryExt;
    use gfx::{format, tex};

    let pso = load_pso(factory);
    let vertices = [
        Vertex{ pos: [0,0,0,1] },
        Vertex{ pos: [-1,0,0,0] },
        Vertex{ pos: [0,-1,0,0] },
        Vertex{ pos: [1,0,0,0] },
        Vertex{ pos: [0,1,0,0] },
    ];
    let indices: &[u16] = &[0,1,2, 0,2,3, 0,3,4, 0,4,1];
    let (vbuf, slice) = factory.create_vertex_buffer_with_slice(&vertices, indices);

    let kind = tex::Kind::D2Array(size.0 as tex::Size, TEX_HEIGHT as tex::Size, (size.1/TEX_HEIGHT) as tex::Size, tex::AaMode::Single);
    let height_chunks: Vec<_> = height_data.chunks((size.0 * TEX_HEIGHT) as usize).collect();
    let meta_chunks: Vec<_> = meta_data.chunks((size.0 * TEX_HEIGHT) as usize).collect();
    let (_, height) = factory.create_texture_const::<(format::R8, format::Unorm)>(kind, &height_chunks).unwrap();
    let (_, meta) = factory.create_texture_const::<(format::R8, format::Uint)>(kind, &meta_chunks).unwrap();
    let sm_height = factory.create_sampler(tex::SamplerInfo::new(
        tex::FilterMethod::Anisotropic(4), tex::WrapMode::Tile));
    let sm_meta = factory.create_sampler(tex::SamplerInfo::new(
        tex::FilterMethod::Scale, tex::WrapMode::Tile));

    let data = terrain::Data {
        vbuf: vbuf,
        locals: factory.create_constant_buffer(1),
        height: (height, sm_height),
        meta: (meta, sm_meta),
        out: main_color,
    };
    Render {
        terrain: gfx::Bundle::new(slice, pso, data),
    }
}

impl<R: gfx::Resources> Render<R> {
    pub fn draw<C: gfx::CommandBuffer<R>>(&self, encoder: &mut gfx::Encoder<R, C>, cam: &Camera) {
        use cgmath::SquareMatrix;
        let mx_vp = cam.get_view_proj();
        let cpos: [f32; 3] = cam.loc.into();
        let locals = Locals {
            cam_pos: [cpos[0], cpos[1], cpos[2], 1.0],
            m_vp: mx_vp.into(),
            m_inv_vp: mx_vp.invert().unwrap().into(),
        };
        encoder.update_constant_buffer(&self.terrain.data.locals, &locals);
        encoder.clear(&self.terrain.data.out, [0.1,0.2,0.3,1.0]);
        self.terrain.encode(encoder);
    }
    pub fn reload<F: gfx::Factory<R>>(&mut self, factory: &mut F) {
        info!("Reloading shaders");
        self.terrain.pso = load_pso(factory);
    }
}
