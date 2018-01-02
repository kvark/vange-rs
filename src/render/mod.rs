use cgmath::{Decomposed, Matrix4};
use gfx;
use gfx::traits::FactoryExt;

use {level, model};
use config::settings;
use space::{Camera, Transform};

mod debug;

pub use self::debug::{DebugPos, DebugRender, LineBuffer};

pub struct MainTargets<R: gfx::Resources> {
    pub color: gfx::handle::RenderTargetView<R, ColorFormat>,
    pub depth: gfx::handle::DepthStencilView<R, DepthFormat>,
}

struct MaterialParams {
    dx: f32,
    sd: f32,
    jj: f32,
}

const NUM_MATERIALS: usize = 2;
const TERRAIN_MATERIAL: [usize; level::NUM_TERRAINS] = [1, 0, 0, 0, 0, 0, 0, 0];
const MATERIALS: [MaterialParams; NUM_MATERIALS] = [
    MaterialParams {
        dx: 1.0,
        sd: 1.0,
        jj: 1.0,
    },
    MaterialParams {
        dx: 5.0,
        sd: 1.25,
        jj: 0.5,
    },
];
const SHADOW_DEPTH: usize = 0x180; // each 0x100 is 1 voxel/step
const MAX_TEX_HEIGHT: i32 = 4096;
pub const NUM_COLOR_IDS: u32 = 25;
pub const COLOR_ID_BODY: u32 = 1;

const COLOR_TABLE: [[u8; 2]; NUM_COLOR_IDS as usize] = [
    [0, 0],   // reserved
    [128, 3], // body
    [176, 4], // window
    [224, 7], // wheel
    [184, 4], // defence
    [224, 3], // weapon
    [224, 7], // tube
    [128, 3], // body red
    [144, 3], // body blue
    [160, 3], // body yellow
    [228, 4], // body gray
    [112, 4], // yellow (charged)
    [0, 2],   // material 0
    [32, 2],  // material 1
    [64, 4],  // material 2
    [72, 3],  // material 3
    [88, 3],  // material 4
    [104, 4], // material 5
    [112, 4], // material 6
    [120, 4], // material 7
    [184, 4], // black
    [240, 3], // body green
    [136, 4], // skyfarmer kenoboo
    [128, 4], // skyfarmer pipetka
    [224, 4], // rotten item
];

pub type ColorFormat = gfx::format::Rgba8; //should be Srgba8
pub type DepthFormat = gfx::format::DepthStencil;

gfx_defines!{
    vertex TerrainVertex {
        pos: [i8; 4] = "a_Pos",
    }

    constant TerrainLocals {
        scr_size: [f32; 4] = "u_ScreenSize",
        tex_scale: [f32; 4] = "u_TextureScale",
    }

    constant Globals {
        camera_pos: [f32; 4] = "u_CameraPos",
        m_vp: [[f32; 4]; 4] = "u_ViewProj",
        m_inv_vp: [[f32; 4]; 4] = "u_InvViewProj",
        light_pos: [f32; 4] = "u_LightPos",
        light_color: [f32; 4] = "u_LightColor",
    }

    pipeline terrain {
        vbuf: gfx::VertexBuffer<TerrainVertex> = (),
        globals: gfx::ConstantBuffer<Globals> = "c_Globals",
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
        m_model: [[f32; 4]; 4] = "u_Model",
    }

    pipeline object {
        vbuf: gfx::VertexBuffer<ObjectVertex> = (),
        globals: gfx::ConstantBuffer<Globals> = "c_Globals",
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
    pub light_config: settings::Light,
    pub debug: debug::DebugRender<R>,
}

pub struct RenderModel<'a, R: gfx::Resources> {
    pub model: &'a model::Model<R>,
    pub transform: Transform,
    pub debug_shape_scale: Option<f32>,
}

#[doc(hidden)]
pub fn read_file(name: &str) -> Vec<u8> {
    use std::fs::File;
    use std::io::{BufReader, Read};
    let mut buf = Vec::new();
    let mut file = BufReader::new(File::open(name).unwrap());
    file.read_to_end(&mut buf).unwrap();
    buf
}

pub fn init<R: gfx::Resources, F: gfx::Factory<R>>(
    factory: &mut F,
    targets: MainTargets<R>,
    level: &level::Level,
    object_palette: &[[u8; 4]],
    settings: &settings::Render,
) -> Render<R> {
    use gfx::{format, texture as tex};

    let mut light_clr_material = [[0; 0x200]; NUM_MATERIALS];
    {
        let dx_scale = 8.0;
        let sd_scale = 256f32 / SHADOW_DEPTH as f32;
        for (lcm, mat) in light_clr_material.iter_mut().zip(MATERIALS.iter()) {
            let dx = mat.dx * dx_scale;
            let sd = mat.sd * sd_scale;
            for (i, out) in lcm.iter_mut().enumerate() {
                let jj = mat.jj * (i as f32 - 255.5);
                let v = (dx * sd - jj) / ((1.0 + sd * sd) * (dx * dx + jj * jj)).sqrt();
                *out = (v.max(0.0).min(1.0) * 255.0) as u8;
            }
        }
    }
    // This table has 2 lines 0x200 width each, on each layer
    // of a layered texture, where layer = terrain ID.
    // First line corresponds to `lightCLR` table of the original,
    // which is computed in `light_clr_material` here.
    // Second line corresponds to `palCLR` of the original.
    let mut color_table = [[0; 0x400]; level::NUM_TERRAINS];
    for (ct, (terr, &mid)) in color_table
        .iter_mut()
        .zip(level.terrains.iter().zip(TERRAIN_MATERIAL.iter()))
    {
        for (c, lcm) in ct[0x000 .. 0x200]
            .iter_mut()
            .zip(light_clr_material[mid].iter())
        {
            *c = *lcm;
        }
        for c in ct[0x200 .. 0x300].iter_mut() {
            *c = terr.colors.start;
        }
        let color_num = (terr.colors.end - terr.colors.start) as usize;
        for (j, c) in ct[0x300 .. 0x400].iter_mut().enumerate() {
            //TODO: separate case for the first terrain type
            *c = terr.colors.start + ((j * color_num) / 0x100) as u8;
        }
    }

    let real_height = if level.size.1 >= MAX_TEX_HEIGHT {
        assert_eq!(level.size.1 % MAX_TEX_HEIGHT, 0);
        MAX_TEX_HEIGHT
    } else {
        level.size.1
    };
    let num_layers = level.size.1 / real_height;
    let kind = tex::Kind::D2Array(
        level.size.0 as tex::Size,
        real_height as tex::Size,
        num_layers as tex::Size,
        tex::AaMode::Single,
    );
    let table_kind = tex::Kind::D2Array(
        0x200,
        2,
        level::NUM_TERRAINS as tex::Size,
        tex::AaMode::Single,
    );
    let height_chunks: Vec<_> = level
        .height
        .chunks((level.size.0 * real_height) as usize)
        .collect();
    let meta_chunks: Vec<_> = level
        .meta
        .chunks((level.size.0 * real_height) as usize)
        .collect();
    let table_chunks: Vec<_> = color_table.iter().map(|t| &t[..]).collect();
    let (_, height) = factory
        .create_texture_immutable::<(format::R8, format::Unorm)>(kind, tex::Mipmap::Provided, &height_chunks)
        .unwrap();
    let (_, meta) = factory
        .create_texture_immutable::<(format::R8, format::Uint)>(kind, tex::Mipmap::Provided, &meta_chunks)
        .unwrap();
    let (_, table) = factory
        .create_texture_immutable::<(format::R8, format::Unorm)>(table_kind, tex::Mipmap::Provided, &table_chunks)
        .unwrap();
    let sm_height = factory.create_sampler(tex::SamplerInfo::new(
        tex::FilterMethod::Scale,
        tex::WrapMode::Tile,
    ));
    let sm_meta = factory.create_sampler(tex::SamplerInfo::new(
        tex::FilterMethod::Scale,
        tex::WrapMode::Tile,
    ));
    let sm_table = factory.create_sampler(tex::SamplerInfo::new(
        tex::FilterMethod::Bilinear,
        tex::WrapMode::Clamp,
    ));

    let palette = Render::create_palette(&level.palette, factory);
    let globals = factory.create_constant_buffer(1);

    Render {
        terrain: {
            let pso = Render::create_terrain_pso(factory);
            let vertices = [
                TerrainVertex { pos: [0, 0, 0, 1] },
                TerrainVertex { pos: [-1, 0, 0, 0] },
                TerrainVertex { pos: [0, -1, 0, 0] },
                TerrainVertex { pos: [1, 0, 0, 0] },
                TerrainVertex { pos: [0, 1, 0, 0] },
            ];
            let indices: &[u16] = &[0, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 1];
            let (vbuf, slice) = factory.create_vertex_buffer_with_slice(&vertices, indices);
            let data = terrain::Data {
                vbuf,
                locals: factory.create_constant_buffer(1),
                globals: globals.clone(),
                height: (height, sm_height),
                meta: (meta, sm_meta),
                palette,
                table: (table, sm_table),
                out_color: targets.color.clone(),
                out_depth: targets.depth.clone(),
            };
            gfx::Bundle::new(slice, pso, data)
        },
        terrain_scale: [
            level.size.0 as f32,
            real_height as f32,
            level::HEIGHT_SCALE as f32,
            num_layers as f32,
        ],
        object_pso: Render::create_object_pso(factory),
        object_data: object::Data {
            vbuf: factory.create_vertex_buffer(&[]), //dummy
            locals: factory.create_constant_buffer(1),
            globals,
            palette: Render::create_palette(&object_palette, factory),
            ctable: Render::create_color_table(factory),
            out_color: targets.color.clone(),
            out_depth: targets.depth.clone(),
        },
        light_config: settings.light.clone(),
        debug: DebugRender::new(factory, targets, &settings.debug),
    }
}

impl<R: gfx::Resources> Render<R> {
    pub fn set_globals<C>(
        encoder: &mut gfx::Encoder<R, C>,
        cam: &Camera,
        light: &settings::Light,
        buffer: &gfx::handle::Buffer<R, Globals>,
    ) -> Matrix4<f32>
    where
        C: gfx::CommandBuffer<R>,
    {
        use cgmath::SquareMatrix;

        let mx_vp = cam.get_view_proj();
        let globals = Globals {
            camera_pos: cam.loc.extend(1.0).into(),
            m_vp: mx_vp.into(),
            m_inv_vp: mx_vp.invert().unwrap().into(),
            light_pos: light.pos,
            light_color: light.color,
        };

        encoder.update_constant_buffer(buffer, &globals);
        mx_vp
    }

    pub fn draw_mesh<C>(
        encoder: &mut gfx::Encoder<R, C>,
        mesh: &model::Mesh<R>,
        model2world: Transform,
        pso: &gfx::PipelineState<R, object::Meta>,
        data: &mut object::Data<R>,
    ) where
        C: gfx::CommandBuffer<R>,
    {
        let mx_world = Matrix4::from(model2world);
        let locals = ObjectLocals {
            m_model: mx_world.into(),
        };
        data.vbuf = mesh.buffer.clone();
        encoder.update_constant_buffer(&data.locals, &locals);
        encoder.draw(&mesh.slice, pso, data);
    }

    pub fn draw_model<C>(
        encoder: &mut gfx::Encoder<R, C>,
        model: &model::Model<R>,
        model2world: Transform,
        pso: &gfx::PipelineState<R, object::Meta>,
        data: &mut object::Data<R>,
        debug_context: Option<(&mut DebugRender<R>, f32, &Matrix4<f32>)>,
    ) where
        C: gfx::CommandBuffer<R>,
    {
        use cgmath::{Deg, One, Quaternion, Rad, Rotation3, Transform, Vector3};

        // body
        Render::draw_mesh(encoder, &model.body, model2world, pso, data);
        // debug render
        if let (Some((debug, scale, world2screen)), Some(shape)) = (debug_context, model.shape.debug.as_ref()) {
            let mut mx_shape = model2world;
            mx_shape.scale *= scale;
            let transform = world2screen * Matrix4::from(mx_shape);
            debug.draw_shape(shape, transform, encoder);
        }
        // wheels
        for w in model.wheels.iter() {
            if let Some(ref mesh) = w.mesh {
                let transform = model2world.concat(&Decomposed {
                    disp: mesh.offset.into(),
                    rot: Quaternion::one(),
                    scale: 1.0,
                });
                Render::draw_mesh(encoder, mesh, transform, pso, data);
            }
        }
        // slots
        for s in model.slots.iter() {
            if let Some(ref mesh) = s.mesh {
                let mut local = Decomposed {
                    disp: Vector3::from(s.pos),
                    rot: Quaternion::from_angle_y(Rad::from(Deg(s.angle as f32))),
                    scale: s.scale / model2world.scale,
                };
                local.disp -= local.transform_vector(Vector3::from(mesh.offset));
                let transform = model2world.concat(&local);
                Render::draw_mesh(encoder, mesh, transform, pso, data);
            }
        }
    }

    pub fn draw_world<'a, C>(
        &mut self,
        encoder: &mut gfx::Encoder<R, C>,
        render_models: &[RenderModel<'a, R>],
        cam: &Camera,
    ) where
        C: gfx::CommandBuffer<R>,
    {
        let mx_vp = Self::set_globals(
            encoder,
            cam,
            &self.light_config,
            &self.terrain.data.globals,
        );

        // clear buffers
        encoder.clear(&self.terrain.data.out_color, [0.1, 0.2, 0.3, 1.0]);
        encoder.clear_depth(&self.terrain.data.out_depth, 1.0);

        // draw terrain
        let (wid, het, _, _) = self.terrain.data.out_color.get_dimensions();
        let locals = TerrainLocals {
            scr_size: [wid as f32, het as f32, 0.0, 0.0],
            tex_scale: self.terrain_scale,
        };
        encoder.update_constant_buffer(&self.terrain.data.locals, &locals);
        self.terrain.encode(encoder);

        // draw vehicle models
        for rm in render_models {
            Render::draw_model(
                encoder,
                rm.model,
                rm.transform,
                &self.object_pso,
                &mut self.object_data,
                match rm.debug_shape_scale {
                    Some(scale) => Some((&mut self.debug, scale, &mx_vp)),
                    None => None,
                },
            );
        }
    }

    pub fn create_palette<F: gfx::Factory<R>>(
        data: &[[u8; 4]],
        factory: &mut F,
    ) -> (
        gfx::handle::ShaderResourceView<R, [f32; 4]>,
        gfx::handle::Sampler<R>,
    ) {
        use gfx::texture as tex;
        let (_, view) = factory
            .create_texture_immutable::<gfx::format::Srgba8>(tex::Kind::D1(0x100), tex::Mipmap::Provided, &[data])
            .unwrap();
        let sampler = factory.create_sampler(tex::SamplerInfo::new(
            tex::FilterMethod::Bilinear,
            tex::WrapMode::Clamp,
        ));
        (view, sampler)
    }

    pub fn create_color_table<F: gfx::Factory<R>>(
        factory: &mut F,
    ) -> (
        gfx::handle::ShaderResourceView<R, [u32; 2]>,
        gfx::handle::Sampler<R>,
    ) {
        use gfx::texture as tex;
        type Format = (gfx::format::R8_G8, gfx::format::Uint);
        let kind = tex::Kind::D1(NUM_COLOR_IDS as tex::Size);
        let (_, view) = factory
            .create_texture_immutable::<Format>(kind, tex::Mipmap::Provided, &[&COLOR_TABLE])
            .unwrap();
        let sampler = factory.create_sampler(tex::SamplerInfo::new(
            tex::FilterMethod::Scale,
            tex::WrapMode::Clamp,
        ));
        (view, sampler)
    }

    fn create_terrain_pso<F: gfx::Factory<R>>(
        factory: &mut F,
    ) -> gfx::PipelineState<R, terrain::Meta> {
        let program = factory
            .link_program(
                &read_file("data/shader/terrain.vert"),
                &read_file("data/shader/terrain.frag"),
            )
            .unwrap();
        factory
            .create_pipeline_from_program(
                &program,
                gfx::Primitive::TriangleList,
                gfx::state::Rasterizer::new_fill(),
                terrain::new(),
            )
            .unwrap()
    }

    pub fn create_object_pso<F: gfx::Factory<R>>(
        factory: &mut F,
    ) -> gfx::PipelineState<R, object::Meta> {
        let program = factory
            .link_program(
                &read_file("data/shader/object.vert"),
                &read_file("data/shader/object.frag"),
            )
            .unwrap();
        // no culling because the old rasterizer was not polygonal
        factory
            .create_pipeline_from_program(
                &program,
                gfx::Primitive::TriangleList,
                gfx::state::Rasterizer::new_fill(),
                object::new(),
            )
            .unwrap()
    }

    pub fn reload<F: gfx::Factory<R>>(
        &mut self,
        factory: &mut F,
    ) {
        info!("Reloading shaders");
        self.terrain.pso = Render::create_terrain_pso(factory);
        self.object_pso = Render::create_object_pso(factory);
    }

    pub fn resize(&mut self, targets: MainTargets<R>) {
        self.terrain.data.out_color = targets.color.clone();
        self.terrain.data.out_depth = targets.depth.clone();
        self.object_data.out_color = targets.color.clone();
        self.object_data.out_depth = targets.depth.clone();
        self.debug.resize(targets);
    }
}
