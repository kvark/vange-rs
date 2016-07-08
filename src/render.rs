use cgmath::Matrix4;
use gfx;
use gfx::traits::FactoryExt;
use app::{Agent, Camera};
use {level, model};


struct MaterialParams {
    dx: f32,
    sd: f32,
    jj: f32,
}

const NUM_MATERIALS: usize = 2;
const TERRAIN_MATERIAL: [usize; level::NUM_TERRAINS] = [1, 0, 0, 0, 0, 0, 0, 0];
const MATERIALS: [MaterialParams; NUM_MATERIALS] = [
    MaterialParams { dx: 1.0, sd: 1.0, jj: 1.0},
    MaterialParams { dx: 5.0, sd: 1.25, jj: 0.5 },
];
const SHADOW_DEPTH: usize = 0x180; // each 0x100 is 1 voxel/step
const TEX_HEIGHT: i32 = 4096;
pub const NUM_COLOR_IDS: u32 = 25;
pub const COLOR_ID_BODY: u32 = 1;

const COLOR_TABLE: [[u8; 2]; NUM_COLOR_IDS as usize] = [
    [0, 0],     // reserved
    [128, 3],   // body
    [176, 4],   // window
    [224, 7],   // wheel
    [184, 4],   // defence
    [224, 3],   // weapon
    [224, 7],   // tube
    [128, 3],   // body red
    [144, 3],   // body blue
    [160, 3],   // body yellow
    [228, 4],   // body gray
    [112, 4],   // yellow (charged)
    [0, 2],     // material 0
    [32, 2],    // material 1
    [64, 4],    // material 2
    [72, 3],    // material 3
    [88, 3],    // material 4
    [104, 4],   // material 5
    [112, 4],   // material 6
    [120, 4],   // material 7
    [184, 4],   // black
    [240, 3],   // body green
    [136, 4],   // skyfarmer kenoboo
    [128, 4],   // skyfarmer pipetka
    [224, 4],   // rotten item
];


pub type ColorFormat = gfx::format::Rgba8;
pub type DepthFormat = gfx::format::DepthStencil;

gfx_defines!{
    vertex TerrainVertex {
        pos: [i8; 4] = "a_Pos",
    }

    constant TerrainLocals {
        cam_pos: [f32; 4] = "u_CamPos",
        scr_size: [f32; 4] = "u_ScreenSize",
        tex_scale: [f32; 4] = "u_TextureScale",
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
        out_color: gfx::RenderTarget<ColorFormat> = "Target0",
        out_depth: gfx::DepthTarget<DepthFormat> = gfx::preset::depth::LESS_EQUAL_WRITE,
    }

    vertex ObjectVertex {
        pos: [i8; 4] = "a_Pos",
        color: u32 = "a_ColorIndex",
        normal: [gfx::format::I8Norm; 4] = "a_Normal",
    }

    constant ObjectLocals {
        m_mvp: [[f32; 4]; 4] = "u_ModelViewProj",
    }

    pipeline object {
        vbuf: gfx::VertexBuffer<ObjectVertex> = (),
        locals: gfx::ConstantBuffer<ObjectLocals> = "c_Locals",
        ctable: gfx::TextureSampler<[u32; 2]> = "t_ColorTable",
        palette: gfx::TextureSampler<[f32; 4]> = "t_Palette",
        out_color: gfx::RenderTarget<ColorFormat> = "Target0",
        out_depth: gfx::DepthTarget<DepthFormat> = gfx::preset::depth::LESS_EQUAL_WRITE,
    }
}

pub struct Render<R: gfx::Resources> {
    terrain: gfx::Bundle<R, terrain::Data<R>>,
    terrain_scale: [f32; 4],
    object_pso: gfx::PipelineState<R, object::Meta>,
    object_data: object::Data<R>,
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
            main_depth: gfx::handle::DepthStencilView<R, DepthFormat>,
            level: &level::Level, object_palette: &[[u8; 4]])
            -> Render<R>
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
    let mut color_table = [[0; 0x400]; level::NUM_TERRAINS];
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

    let num_layers = level.size.1/TEX_HEIGHT;
    let kind = tex::Kind::D2Array(level.size.0 as tex::Size, TEX_HEIGHT as tex::Size,
        num_layers as tex::Size, tex::AaMode::Single);
    let table_kind = tex::Kind::D2Array(0x200, 2, level::NUM_TERRAINS as tex::Size, tex::AaMode::Single);
    let height_chunks: Vec<_> = level.height.chunks((level.size.0 * TEX_HEIGHT) as usize).collect();
    let meta_chunks: Vec<_> = level.meta.chunks((level.size.0 * TEX_HEIGHT) as usize).collect();
    let table_chunks: Vec<_> = color_table.iter().map(|t| &t[..]).collect();
    let (_, height) = factory.create_texture_const::<(format::R8, format::Unorm)>(kind, &height_chunks).unwrap();
    let (_, meta)   = factory.create_texture_const::<(format::R8, format::Uint)>(kind, &meta_chunks).unwrap();
    let (_, table)  = factory.create_texture_const::<(format::R8, format::Unorm)>(table_kind, &table_chunks).unwrap();
    let sm_height = factory.create_sampler(tex::SamplerInfo::new(
        tex::FilterMethod::Scale, tex::WrapMode::Tile));
    let sm_meta = factory.create_sampler(tex::SamplerInfo::new(
        tex::FilterMethod::Scale, tex::WrapMode::Tile));
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
                palette: Render::create_palette(&level.palette, factory),
                table: (table, sm_table),
                out_color: main_color.clone(),
                out_depth: main_depth.clone(),
            };
            gfx::Bundle::new(slice, pso, data)
        },
        terrain_scale: [level.size.0 as f32, TEX_HEIGHT as f32, level::HEIGHT_SCALE as f32, num_layers as f32],
        object_pso: Render::create_object_pso(factory),
        object_data: object::Data {
            vbuf: factory.create_vertex_buffer(&[]), //dummy
            locals: factory.create_constant_buffer(1),
            palette: Render::create_palette(&object_palette, factory),
            ctable: Render::create_color_table(factory),
            out_color: main_color,
            out_depth: main_depth,
        },
    }
}

impl<R: gfx::Resources> Render<R> {
    pub fn draw_mesh<C>(encoder: &mut gfx::Encoder<R, C>, mesh: &model::Mesh<R>, transform: Matrix4<f32>,
                     pso: &gfx::PipelineState<R, object::Meta>, data: &mut object::Data<R>) where
        C: gfx::CommandBuffer<R>,
    {
        let scale = Matrix4::from_scale(1.0 / 4.0);
        let offset = Matrix4::from_translation(mesh.offset.into());
        let locals = ObjectLocals {
            m_mvp: (transform * scale * offset).into(),
        };
        data.vbuf = mesh.buffer.clone();
        encoder.update_constant_buffer(&data.locals, &locals);
        encoder.draw(&mesh.slice, pso, data);
    }

    pub fn draw_model<C>(encoder: &mut gfx::Encoder<R, C>, model: &model::Model<R>, transform: Matrix4<f32>,
                      pso: &gfx::PipelineState<R, object::Meta>, data: &mut object::Data<R>) where
        C: gfx::CommandBuffer<R>,
    {
        Render::draw_mesh(encoder, &model.body, transform, pso, data);
        for w in model.wheels.iter() {
            if let Some(ref mesh) = w.mesh {
                Render::draw_mesh(encoder, mesh, transform, pso, data);
            }
        }
    }

    pub fn draw<C>(&mut self, encoder: &mut gfx::Encoder<R, C>, agents: &[Agent<R>], cam: &Camera) where
        C: gfx::CommandBuffer<R>,
    {
        let mx_vp = cam.get_view_proj();
        // clear buffers
        encoder.clear(&self.terrain.data.out_color, [0.1,0.2,0.3,1.0]);
        encoder.clear_depth(&self.terrain.data.out_depth, 1.0);
        // draw terrain
        {
            use cgmath::SquareMatrix;
            let cpos: [f32; 3] = cam.loc.into();
            let (wid, het, _, _) = self.terrain.data.out_color.get_dimensions();
            let locals = TerrainLocals {
                cam_pos: [cpos[0], cpos[1], cpos[2], 1.0],
                scr_size: [wid as f32, het as f32, 0.0, 0.0],
                tex_scale: self.terrain_scale,
                m_vp: mx_vp.into(),
                m_inv_vp: mx_vp.invert().unwrap().into(),
            };
            encoder.update_constant_buffer(&self.terrain.data.locals, &locals);
        }
        self.terrain.encode(encoder);
        // draw vehicle models
        for ag in agents.iter() {
            let local: Matrix4<f32> = ag.transform.into();
            Render::draw_model(encoder, &ag.model, mx_vp * local, &self.object_pso, &mut self.object_data);
        }
    }

    pub fn create_palette<F: gfx::Factory<R>>(data: &[[u8; 4]], factory: &mut F)
                          -> (gfx::handle::ShaderResourceView<R, [f32; 4]>, gfx::handle::Sampler<R>)
    {
        use gfx::tex;
        let (_, view) = factory.create_texture_const::<gfx::format::Rgba8>(tex::Kind::D1(0x100), &[data]).unwrap();
        let sampler = factory.create_sampler(tex::SamplerInfo::new(
            tex::FilterMethod::Bilinear, tex::WrapMode::Clamp));
        (view, sampler)
    }

    pub fn create_color_table<F: gfx::Factory<R>>(factory: &mut F)
                              -> (gfx::handle::ShaderResourceView<R, [u32; 2]>, gfx::handle::Sampler<R>)
    {
        use gfx::tex;
        type Format = (gfx::format::R8_G8, gfx::format::Uint);
        let kind = tex::Kind::D1(NUM_COLOR_IDS as tex::Size);
        let (_, view) = factory.create_texture_const::<Format>(kind, &[&COLOR_TABLE]).unwrap();
        let sampler = factory.create_sampler(tex::SamplerInfo::new(
            tex::FilterMethod::Scale, tex::WrapMode::Clamp));
        (view, sampler)
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

    pub fn create_object_pso<F: gfx::Factory<R>>(factory: &mut F) -> gfx::PipelineState<R, object::Meta> {
        let program = factory.link_program(
            &read("data/shader/object.vert"),
            &read("data/shader/object.frag"),
            ).unwrap();
        factory.create_pipeline_from_program(
            &program, gfx::Primitive::TriangleList,
            gfx::state::Rasterizer::new_fill(),
            object::new()
        ).unwrap()
    }

    pub fn reload<F: gfx::Factory<R>>(&mut self, factory: &mut F) {
        info!("Reloading shaders");
        self.terrain.pso = Render::create_terrain_pso(factory);
        self.object_pso  = Render::create_object_pso(factory);
    }
}
