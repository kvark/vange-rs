use gfx;


pub type ColorFormat = gfx::format::Srgba8;
pub type DepthFormat = gfx::format::DepthStencil;

gfx_defines!{
    vertex Vertex {
        pos: [i8; 4] = "a_Pos",
    }

    pipeline terrain {
        vbuf: gfx::VertexBuffer<Vertex> = (),
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
            main_color: gfx::handle::RenderTargetView<R, ColorFormat>)
            -> Render<R>
{
    use gfx::traits::FactoryExt;

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
    let data = terrain::Data {
        vbuf: vbuf,
        out: main_color,
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
