use gfx;


pub type ColorFormat = gfx::format::Srgba8;
pub type DepthFormat = gfx::format::DepthStencil;

gfx_defines!{
    vertex Vertex {
        pos: [i8; 4] = "a_Pos",
    }

    pipeline terrain {
        vbuf: gfx::VertexBuffer<Vertex> = (),
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

pub fn init<R: gfx::Resources, F: gfx::Factory<R>>(factory: &mut F,
            main_color: gfx::handle::RenderTargetView<R, ColorFormat>,
            size: (i32, i32), height_data: &[u8], meta_data: &[u8])
            -> Render<R>
{
    use gfx::traits::FactoryExt;
    use gfx::{format, tex};

    let program = factory.link_program(
        &read("data/shader/terrain.vert"),
        &read("data/shader/terrain.frag"),
        ).unwrap();
    let pso = factory.create_pipeline_from_program(
        &program, gfx::Primitive::TriangleList,
        gfx::state::Rasterizer::new_fill(),
        terrain::new()
    ).unwrap();
    let vertices = [
        Vertex{ pos: [0,0,0,1] },
        Vertex{ pos: [-1,0,0,0] },
        Vertex{ pos: [0,-1,0,0] },
        Vertex{ pos: [1,0,0,0] },
        Vertex{ pos: [0,1,0,0] },
    ];
    let indices: &[u16] = &[0,1,2, 0,2,3, 0,3,4, 0,4,1];
    let (vbuf, slice) = factory.create_vertex_buffer_with_slice(&vertices, indices);

    let kind = tex::Kind::D2(size.0 as tex::Size, size.1 as tex::Size, tex::AaMode::Single);
    let (_, height) = factory.create_texture_const::<(format::R8, format::Unorm)>(kind, &[height_data]).unwrap();
    let (_, meta) = factory.create_texture_const::<(format::R8, format::Uint)>(kind, &[meta_data]).unwrap();
    let sm_height = factory.create_sampler(tex::SamplerInfo::new(
        tex::FilterMethod::Anisotropic(4), tex::WrapMode::Tile));
    let sm_meta = factory.create_sampler(tex::SamplerInfo::new(
        tex::FilterMethod::Scale, tex::WrapMode::Tile));

    let data = terrain::Data {
        vbuf: vbuf,
        out: main_color,
        height: (height, sm_height),
        meta: (meta, sm_meta),
    };
    Render {
        terrain: gfx::Bundle::new(slice, pso, data),
    }
}

impl<R: gfx::Resources> Render<R> {
    pub fn draw<C: gfx::CommandBuffer<R>>(&self, encoder: &mut gfx::Encoder<R, C>) {
        encoder.clear(&self.terrain.data.out, [0.1,0.2,0.3,1.0]);
        self.terrain.encode(encoder);
    }
}
