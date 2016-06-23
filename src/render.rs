use gfx;
use gfx::traits::FactoryExt;
use level::{Level, NUM_TERRAINS};
use Camera;

struct MaterialParams {
    dx: f32,
    sd: f32,
    jj: f32,
}

const NUM_MATERIALS: usize = 2;
const TERRAIN_MATERIAL: [usize; NUM_TERRAINS] = [1, 0, 0, 0, 0, 0, 0, 0];
const MATERIALS: [MaterialParams; NUM_MATERIALS] = [
    MaterialParams { dx: 1.0, sd: 1.0, jj: 1.0},
    MaterialParams { dx: 5.0, sd: 1.25, jj: 0.5 },
];
const SHADOW_DEPTH: usize = 0x180; // each 0x100 is 1 voxel/step
const TEX_HEIGHT: i32 = 4096;

pub type ColorFormat = gfx::format::Rgba8;
pub type DepthFormat = gfx::format::DepthStencil;

gfx_defines!{
    vertex TerrainVertex {
        pos: [i8; 4] = "a_Pos",
    }

    constant TerrainLocals {
        cam_pos: [f32; 4] = "u_CamPos",
        m_vp: [[f32; 4]; 4] = "u_ViewProj",
        m_inv_vp: [[f32; 4]; 4] = "u_InvViewProj",
    }

    pipeline terrain {
        vbuf: gfx::VertexBuffer<TerrainVertex> = (),
        locals: gfx::ConstantBuffer<TerrainLocals> = "c_Locals",
        height: gfx::TextureSampler<f32> = "t_Height",
        meta: gfx::TextureSampler<u32> = "t_Meta",
        palette: gfx::TextureSampler<[f32; 4]> = "t_Palette",
        table: gfx::TextureSampler<f32> = "t_Table",
        out: gfx::RenderTarget<ColorFormat> = "Target0",
    }

    vertex ObjectVertex {
        pos: [f32; 4] = "a_Pos",
        color: [u32; 2] = "a_Color",
        normal: [gfx::format::I8Norm; 4] = "a_Normal",
    }

    constant ObjectLocals {
        m_mvp: [[f32; 4]; 4] = "u_ModelViewProj",
    }

    pipeline object {
        vbuf: gfx::VertexBuffer<ObjectVertex> = (),
        locals: gfx::ConstantBuffer<ObjectLocals> = "c_Locals",
        palette: gfx::TextureSampler<[f32; 4]> = "t_Palette",
        out: gfx::RenderTarget<ColorFormat> = "Target0",
    }
}

pub struct Render<R: gfx::Resources> {
    terrain: gfx::Bundle<R, terrain::Data<R>>,
    object_pso: gfx::PipelineState<R, object::Meta>,
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
            level: &Level) -> Render<R>
{
    use gfx::{format, tex};

    let mut light_clr_material = [[0; 0x200]; NUM_MATERIALS];
    {
        let dx_scale = 8.0;
        let sd_scale = 0x100 as f32 / SHADOW_DEPTH as f32;
        for (lcm, mat) in light_clr_material.iter_mut().zip(MATERIALS.iter()) {
            let dx = mat.dx * dx_scale;
            let sd = mat.sd * sd_scale;
            for i in 0..0x200 {
                let jj = mat.jj * (i as f32 - 255.5);
                let v = (dx * sd - jj) / ((1.0 + sd * sd) * (dx * dx + jj * jj)).sqrt();
                lcm[i] = (v.max(0.0).min(1.0) * 255.0 + 0.5) as u8;
            }
        }
    }
    let mut color_table = [[0; 0x400]; NUM_TERRAINS];
    for (ct, (terr, &mid)) in color_table.iter_mut().zip(level.terrains.iter().zip(TERRAIN_MATERIAL.iter())) {
        for (c, lcm) in ct[0x000..0x200].iter_mut().zip(light_clr_material[mid].iter()) {
            *c = *lcm;
        }
        for c in ct[0x200..0x300].iter_mut() {
            *c = terr.color_range.0;
        }
        let color_num = (terr.color_range.1 - terr.color_range.0) as usize;
        for j in 0..0x100 {
            //TODO: separate case for the first terrain type
            ct[0x300+j] = terr.color_range.0 + ((j * color_num) / 0x100) as u8;
        }
    }

    let kind = tex::Kind::D2Array(level.size.0 as tex::Size, TEX_HEIGHT as tex::Size,
        (level.size.1/TEX_HEIGHT) as tex::Size, tex::AaMode::Single);
    let table_kind = tex::Kind::D2Array(0x200, 2, NUM_TERRAINS as tex::Size, tex::AaMode::Single);
    let height_chunks: Vec<_> = level.height.chunks((level.size.0 * TEX_HEIGHT) as usize).collect();
    let meta_chunks: Vec<_> = level.meta.chunks((level.size.0 * TEX_HEIGHT) as usize).collect();
    let table_chunks: Vec<_> = color_table.iter().map(|t| &t[..]).collect();
    let (_, height) = factory.create_texture_const::<(format::R8, format::Unorm)>(kind, &height_chunks).unwrap();
    let (_, meta)   = factory.create_texture_const::<(format::R8, format::Uint)>(kind, &meta_chunks).unwrap();
    let (_, pal)    = factory.create_texture_const::<format::Rgba8>(tex::Kind::D1(0x100), &[&level.palette]).unwrap();
    let (_, table)  = factory.create_texture_const::<(format::R8, format::Unorm)>(table_kind, &table_chunks).unwrap();
    let sm_height = factory.create_sampler(tex::SamplerInfo::new(
        tex::FilterMethod::Scale, tex::WrapMode::Tile));
        //tex::FilterMethod::Anisotropic(4), tex::WrapMode::Tile));
    let sm_meta = factory.create_sampler(tex::SamplerInfo::new(
        tex::FilterMethod::Scale, tex::WrapMode::Tile));
    let sm_pal = factory.create_sampler(tex::SamplerInfo::new(
        tex::FilterMethod::Bilinear, tex::WrapMode::Clamp));
    let sm_table = factory.create_sampler(tex::SamplerInfo::new(
        tex::FilterMethod::Bilinear, tex::WrapMode::Clamp));

    Render {
        terrain: {
            let pso = Render::create_terrain_pso(factory);
            let vertices = [
                TerrainVertex{ pos: [0,0,0,1] },
                TerrainVertex{ pos: [-1,0,0,0] },
                TerrainVertex{ pos: [0,-1,0,0] },
                TerrainVertex{ pos: [1,0,0,0] },
                TerrainVertex{ pos: [0,1,0,0] },
            ];
            let indices: &[u16] = &[0,1,2, 0,2,3, 0,3,4, 0,4,1];
            let (vbuf, slice) = factory.create_vertex_buffer_with_slice(&vertices, indices);
            let data = terrain::Data {
                vbuf: vbuf,
                locals: factory.create_constant_buffer(1),
                height: (height, sm_height),
                meta: (meta, sm_meta),
                palette: (pal, sm_pal),
                table: (table, sm_table),
                out: main_color,
            };
            gfx::Bundle::new(slice, pso, data)
        },
        object_pso: Render::create_object_pso(factory),
    }
}

impl<R: gfx::Resources> Render<R> {
    pub fn draw<C: gfx::CommandBuffer<R>>(&self, encoder: &mut gfx::Encoder<R, C>, cam: &Camera) {
        use cgmath::SquareMatrix;
        let mx_vp = cam.get_view_proj();
        let cpos: [f32; 3] = cam.loc.into();
        let locals = TerrainLocals {
            cam_pos: [cpos[0], cpos[1], cpos[2], 1.0],
            m_vp: mx_vp.into(),
            m_inv_vp: mx_vp.invert().unwrap().into(),
        };
        encoder.update_constant_buffer(&self.terrain.data.locals, &locals);
        encoder.clear(&self.terrain.data.out, [0.1,0.2,0.3,1.0]);
        self.terrain.encode(encoder);
    }

    fn create_terrain_pso<F: gfx::Factory<R>>(factory: &mut F) -> gfx::PipelineState<R, terrain::Meta> {
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
    fn create_object_pso<F: gfx::Factory<R>>(factory: &mut F) -> gfx::PipelineState<R, object::Meta> {
        let program = factory.link_program(
            &read("data/shader/object.vert"),
            &read("data/shader/object.frag"),
            ).unwrap();
        factory.create_pipeline_from_program(
            &program, gfx::Primitive::TriangleList,
            gfx::state::Rasterizer::new_fill().with_cull_back(),
            object::new()
        ).unwrap()
    }

    pub fn reload<F: gfx::Factory<R>>(&mut self, factory: &mut F) {
        info!("Reloading shaders");
        self.terrain.pso = Render::create_terrain_pso(factory);
        self.object_pso  = Render::create_object_pso(factory);
    }
}
