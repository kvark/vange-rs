use cgmath::{Decomposed, Matrix4};
use gfx;
use gfx::traits::FactoryExt;

use {level, model};
use m3d::NUM_COLOR_IDS;
use config::settings;
use space::{Camera, Transform};

use std::io::Error as IoError;

mod collision;
mod debug;

pub use self::collision::{DebugBlit, GpuCollider, ShapeId};
pub use self::debug::{DebugPos, DebugRender, LineBuffer};


pub struct MainTargets<R: gfx::Resources> {
    pub color: gfx::handle::RenderTargetView<R, ColorFormat>,
    pub depth: gfx::handle::DepthStencilView<R, DepthFormat>,
}

pub struct SurfaceData<R: gfx::Resources> {
    pub constants: gfx::handle::Buffer<R, SurfaceConstants>,
    pub height: (gfx::handle::ShaderResourceView<R, f32>, gfx::handle::Sampler<R>),
    pub meta: (gfx::handle::ShaderResourceView<R, u32>, gfx::handle::Sampler<R>),
}

const MAX_TEX_HEIGHT: i32 = 4096;

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
pub type ShapeVertex = [f32; 4];

gfx_defines!{
    vertex ShapePolygon {
        indices: [u16; 4] = "a_Indices",
        normal: [gfx::format::I8Norm; 4] = "a_Normal",
        origin_square: [f32; 4] = "a_OriginSquare",
    }

    vertex TerrainVertex {
        pos: [i8; 4] = "a_Pos",
    }

    constant SurfaceConstants {
        tex_scale: [f32; 4] = "u_TextureScale",
    }

    constant TerrainConstants {
        scr_size: [f32; 4] = "u_ScreenSize",
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
        suf_constants: gfx::ConstantBuffer<SurfaceConstants> = "c_Surface",
        terr_constants: gfx::ConstantBuffer<TerrainConstants> = "c_Locals",
        height: gfx::TextureSampler<f32> = "t_Height",
        meta: gfx::TextureSampler<u32> = "t_Meta",
        flood: gfx::TextureSampler<f32> = "t_Flood",
        palette: gfx::TextureSampler<[f32; 4]> = "t_Palette",
        table: gfx::TextureSampler<[u32; 4]> = "t_Table",
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

enum Terrain<R: gfx::Resources> {
    Ray(gfx::PipelineState<R, terrain::Meta>),
    Tess {
        low: gfx::PipelineState<R, terrain::Meta>,
        high: gfx::PipelineState<R, terrain::Meta>,
        screen_space: bool,
    },
}

pub struct Render<R: gfx::Resources> {
    terrain: Terrain<R>,
    terrain_data: terrain::Data<R>,
    terrain_slice: gfx::Slice<R>,
    terrain_scale: [f32; 4],
    object_pso: gfx::PipelineState<R, object::Meta>,
    object_data: object::Data<R>,
    pub light_config: settings::Light,
    pub debug: debug::DebugRender<R>,
}

pub struct RenderModel<'a, R: gfx::Resources> {
    pub model: &'a model::RenderModel<R>,
    pub transform: Transform,
    pub debug_shape_scale: Option<f32>,
}

pub struct Shaders {
    vs: Vec<u8>,
    tec: Vec<u8>,
    tev: Vec<u8>,
    fs: Vec<u8>,
}

#[doc(hidden)]
pub fn read_shaders(name: &str, tessellate: bool, specialization: &[&str]) -> Result<Shaders, IoError> {
    use std::fs::File;
    use std::io::{BufReader, Read, Write};
    use std::path::PathBuf;

    let path = PathBuf::from("data")
        .join("shader")
        .join(name)
        .with_extension("glsl");
    if !path.is_file() {
        panic!("Shader not found: {:?}", path);
    }

    let prelude = format!("#version 150 core\n// shader: {}\n", name);
    let mut buf_vs = Vec::new();
    write!(buf_vs, "{}", prelude)?;
    let mut buf_fs = Vec::new();
    write!(buf_fs, "{}", prelude)?;

    let mut buf_tec = Vec::new();
    let mut buf_tev = Vec::new();
    if tessellate {
        let ext = format!("#extension GL_ARB_tessellation_shader: require\n");
        write!(buf_tec, "{}{}", prelude, ext)?;
        write!(buf_tev, "{}{}", prelude, ext)?;
    }

    let mut code = String::new();
    BufReader::new(File::open(&path)?)
        .read_to_string(&mut code)?;
    // parse meta-data
    {
        let mut lines = code.lines();
        let first = lines.next().unwrap();
        if first.starts_with("//!include") {
            for include_pair in first.split_whitespace().skip(1) {
                let mut temp = include_pair.split(':');
                let target = match temp.next().unwrap() {
                    "vs" => &mut buf_vs,
                    "tec" => &mut buf_tec,
                    "tev" => &mut buf_tev,
                    "fs" => &mut buf_fs,
                    other => panic!("Unknown target: {}", other),
                };
                let include = temp.next().unwrap();
                let inc_path = path
                    .with_file_name(include)
                    .with_extension("inc.glsl");
                BufReader::new(File::open(inc_path)?)
                    .read_to_end(target)?;
            }
        }
        let second = lines.next().unwrap();
        if second.starts_with("//!specialization") {
            for define in second.split_whitespace().skip(1) {
                let value = if specialization.contains(&define) {
                    1
                } else {
                    0
                };
                write!(buf_vs, "#define {} {}\n", define, value)?;
                write!(buf_fs, "#define {} {}\n", define, value)?;
                if tessellate {
                    write!(buf_tec, "#define {} {}\n", define, value)?;
                    write!(buf_tev, "#define {} {}\n", define, value)?;
                }
            }
        }
    }

    write!(buf_vs, "\n#define SHADER_VS\n{}", code
        .replace("attribute", "in")
        .replace("varying", "out")
    )?;
    write!(buf_fs, "\n#define SHADER_FS\n{}", code
        .replace("varying", "in")
    )?;

    debug!("vs:\n{}", String::from_utf8_lossy(&buf_vs));
    debug!("fs:\n{}", String::from_utf8_lossy(&buf_fs));

    if tessellate {
        write!(buf_tec, "\n#define SHADER_TEC\n{}", code)?;
        write!(buf_tev, "\n#define SHADER_TEV\n{}", code)?;
        debug!("tec:\n{}", String::from_utf8_lossy(&buf_tec));
        debug!("tev:\n{}", String::from_utf8_lossy(&buf_tev));
    }

    Ok(Shaders {
        vs: buf_vs,
        tec: buf_tec,
        tev: buf_tev,
        fs: buf_fs,
    })
}

pub fn init<R: gfx::Resources, F: gfx::Factory<R>>(
    factory: &mut F,
    targets: MainTargets<R>,
    level: &level::Level,
    object_palette: &[[u8; 4]],
    settings: &settings::Render,
) -> Render<R> {
    use gfx::{format, texture as tex};

    let terrrain_table = level.terrains
        .iter()
        .map(|terr| [
            terr.shadow_offset,
            terr.height_shift,
            terr.colors.start,
            terr.colors.end,
        ])
        .collect::<Vec<_>>();

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
    let height_chunks: Vec<_> = level
        .height
        .chunks((level.size.0 * real_height) as usize)
        .collect();
    let meta_chunks: Vec<_> = level
        .meta
        .chunks((level.size.0 * real_height) as usize)
        .collect();

    let (_, height) = factory
        .create_texture_immutable::<(format::R8, format::Unorm)>(kind, tex::Mipmap::Provided, &height_chunks)
        .unwrap();
    let (_, meta) = factory
        .create_texture_immutable::<(format::R8, format::Uint)>(kind, tex::Mipmap::Provided, &meta_chunks)
        .unwrap();
    let (_, flood) = factory
        .create_texture_immutable::<(format::R8, format::Unorm)>(
            tex::Kind::D1(level.size.1 as _),
            tex::Mipmap::Provided,
            &[&level.flood_map],
        ).unwrap();
    let (_, table) = factory
        .create_texture_immutable::<(format::R8_G8_B8_A8, format::Uint)>(
            tex::Kind::D1(level::NUM_TERRAINS as _),
            tex::Mipmap::Provided,
            &[&terrrain_table],
        ).unwrap();

    let sm_height = factory.create_sampler(tex::SamplerInfo::new(
        tex::FilterMethod::Scale,
        tex::WrapMode::Tile,
    ));
    let sm_meta = factory.create_sampler(tex::SamplerInfo::new(
        tex::FilterMethod::Scale,
        tex::WrapMode::Tile,
    ));
    let sm_flood = factory.create_sampler(tex::SamplerInfo::new(
        tex::FilterMethod::Bilinear,
        tex::WrapMode::Tile,
    ));
    let sm_table = factory.create_sampler(tex::SamplerInfo::new(
        tex::FilterMethod::Scale,
        tex::WrapMode::Clamp,
    ));

    let palette = Render::create_palette(&level.palette, factory);
    let globals = factory.create_constant_buffer(1);

    let (terrain, terrain_slice, terrain_data) = {
        let (terrain, vbuf, slice) = if let Some(ref tessellation) = settings.terrain.tessellate {
            let screen_space = tessellation.screen_space;
            let (low, high) = Render::create_terrain_tess_psos(factory, screen_space);
            let vertices = [
                TerrainVertex { pos: [0, 0, 0, 1] },
                TerrainVertex { pos: [1, 0, 0, 1] },
                TerrainVertex { pos: [1, 1, 0, 1] },
                TerrainVertex { pos: [0, 1, 0, 1] },
            ];
            let (vbuf, mut slice) = factory.create_vertex_buffer_with_slice(&vertices, ());
            let num_instances = if screen_space { 16 * 12 } else { 256 };
            slice.instances = Some((num_instances, 0));
            (Terrain::Tess { low, high, screen_space }, vbuf, slice)
        } else {
            let pso = Render::create_terrain_ray_pso(factory);
            let vertices = [
                TerrainVertex { pos: [0, 0, 0, 1] },
                TerrainVertex { pos: [-1, 0, 0, 0] },
                TerrainVertex { pos: [0, -1, 0, 0] },
                TerrainVertex { pos: [1, 0, 0, 0] },
                TerrainVertex { pos: [0, 1, 0, 0] },
            ];
            let indices: &[u16] = &[0, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 1];
            let (vbuf, slice) = factory.create_vertex_buffer_with_slice(&vertices, indices);
            (Terrain::Ray(pso), vbuf, slice)
        };
        let data = terrain::Data {
            vbuf,
            suf_constants: factory.create_constant_buffer(1),
            terr_constants: factory.create_constant_buffer(1),
            globals: globals.clone(),
            height: (height, sm_height),
            meta: (meta, sm_meta),
            flood: (flood, sm_flood),
            palette,
            table: (table, sm_table),
            out_color: targets.color.clone(),
            out_depth: targets.depth.clone(),
        };
        (terrain, slice, data)
    };

    Render {
        terrain,
        terrain_slice,
        terrain_data,
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
        model: &model::RenderModel<R>,
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
        if let Some((debug, scale, world2screen)) = debug_context {
            let mut mx_shape = model2world;
            mx_shape.scale *= scale;
            let transform = world2screen * Matrix4::from(mx_shape);
            debug.draw_shape(&model.shape, transform, encoder);
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
                    disp: Vector3::new(s.pos[0] as f32, s.pos[1] as f32, s.pos[2] as f32),
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
            &self.terrain_data.globals,
        );

        // clear buffers
        encoder.clear(&self.terrain_data.out_color, [0.1, 0.2, 0.3, 1.0]);
        encoder.clear_depth(&self.terrain_data.out_depth, 1.0);

        // draw terrain
        let (wid, het, _, _) = self.terrain_data.out_color.get_dimensions();
        let suf_constants = SurfaceConstants {
            tex_scale: self.terrain_scale,
        };
        encoder.update_constant_buffer(&self.terrain_data.suf_constants, &suf_constants);
        let terr_constants = TerrainConstants {
            scr_size: [wid as f32, het as f32, 0.0, 0.0],
        };
        encoder.update_constant_buffer(&self.terrain_data.terr_constants, &terr_constants);
        match self.terrain {
            Terrain::Ray(ref pso) => {
                encoder.draw(&self.terrain_slice, pso, &self.terrain_data);
            }
            Terrain::Tess { ref low, ref high, .. } => {
                encoder.draw(&self.terrain_slice, low, &self.terrain_data);
                encoder.draw(&self.terrain_slice, high, &self.terrain_data);
            }
        }

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

    fn create_terrain_ray_pso<F: gfx::Factory<R>>(
        factory: &mut F,
    ) -> gfx::PipelineState<R, terrain::Meta> {
        let shaders = read_shaders("terrain_ray", false, &[])
            .unwrap();
        let program = factory
            .link_program(&shaders.vs, &shaders.fs)
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

    fn create_terrain_tess_pso_impl<F: gfx::Factory<R>>(
        factory: &mut F, specialization: &[&str]
    ) -> gfx::PipelineState<R, terrain::Meta> {
        let shaders = read_shaders("terrain_tess", true, specialization)
            .unwrap();
        let set = factory
            .create_shader_set_tessellation(
                &shaders.vs,
                &shaders.tec,
                &shaders.tev,
                &shaders.fs
            )
            .unwrap();
        factory
            .create_pipeline_state(
                &set,
                gfx::Primitive::PatchList(4),
                gfx::state::Rasterizer::new_fill(),
                terrain::new(),
            )
            .unwrap()
    }

    fn create_terrain_tess_psos<F: gfx::Factory<R>>(
        factory: &mut F, screen_space: bool,
    ) -> (gfx::PipelineState<R, terrain::Meta>, gfx::PipelineState<R, terrain::Meta>) {
        let ss_spec = if screen_space { "SCREEN_SPACE" } else { "" };
        let lo = Self::create_terrain_tess_pso_impl(factory, &[ss_spec]);
        let hi = Self::create_terrain_tess_pso_impl(factory, &["HIGH_LEVEL", "USE_DISCARD", ss_spec]);
        (lo, hi)
    }

    pub fn create_object_pso<F: gfx::Factory<R>>(
        factory: &mut F,
    ) -> gfx::PipelineState<R, object::Meta> {
        let shaders = read_shaders("object", false, &[])
            .unwrap();
        let program = factory
            .link_program(&shaders.vs, &shaders.fs)
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
        match self.terrain {
            Terrain::Ray(ref mut pso) => {
                *pso = Render::create_terrain_ray_pso(factory);
            }
            Terrain::Tess { ref mut low, ref mut high, screen_space } => {
                let (lo, hi) = Render::create_terrain_tess_psos(factory, screen_space);
                *low = lo;
                *high = hi;
            }
        }
        self.object_pso = Render::create_object_pso(factory);
    }

    pub fn resize(&mut self, targets: MainTargets<R>) {
        self.terrain_data.out_color = targets.color.clone();
        self.terrain_data.out_depth = targets.depth.clone();
        self.object_data.out_color = targets.color.clone();
        self.object_data.out_depth = targets.depth.clone();
        self.debug.resize(targets);
    }

    pub fn surface_data(&self) -> SurfaceData<R> {
        SurfaceData {
            constants: self.terrain_data.suf_constants.clone(),
            height: self.terrain_data.height.clone(),
            meta: self.terrain_data.meta.clone(),
        }
    }

    pub fn target_color(&self) -> gfx::handle::RenderTargetView<R, ColorFormat> {
        self.terrain_data.out_color.clone()
    }
}
