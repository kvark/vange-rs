use config::common::Common;
use model::Shape;
use render::{DebugPos, SurfaceConstants, SurfaceData, read_shaders};
use space::Transform;

use gfx::{self, handle as h};
use gfx::Rect;
use gfx::texture::Size;
use gfx::traits::FactoryExt;

use std::{mem, ops};


pub type CollisionFormat = gfx::format::Rgba32F;
pub type CollisionFormatView = <CollisionFormat as gfx::format::Formatted>::View;

gfx_defines!{
    constant CollisionLocals {
        model: [[f32; 4]; 4] = "u_Model",
        target: [f32; 4] = "u_TargetCenterScale",
    }

    constant CollisionGlobals {
        penetration: [f32; 4] = "u_Penetration",
    }

    constant CollisionPolygon {
        origin: [f32; 4] = "u_Origin",
        normal: [f32; 4] = "u_Normal",
    }

    pipeline collision {
        vbuf: gfx::VertexBuffer<DebugPos> = (),
        suf_constants: gfx::ConstantBuffer<SurfaceConstants> = "c_Surface",
        locals: gfx::ConstantBuffer<CollisionLocals> = "c_Locals",
        globals: gfx::ConstantBuffer<CollisionGlobals> = "c_Globals",
        polys: gfx::ConstantBuffer<CollisionPolygon> = "c_Polys",
        height: gfx::TextureSampler<f32> = "t_Height",
        meta: gfx::TextureSampler<u32> = "t_Meta",
        destination: gfx::RenderTarget<CollisionFormat> = "Target0",
    }

    //TODO: use fixed point
    vertex DownsampleVertex {
        src: [f32; 2] = "a_SourcePos",
        dst: [f32; 2] = "a_DestPos",
    }

    pipeline downsample {
        vbuf: gfx::VertexBuffer<DownsampleVertex> = (),
        source: gfx::TextureSampler<CollisionFormatView> = "t_Source",
        destination: gfx::RenderTarget<CollisionFormat> = "Target0",
    }
}


struct AtlasMap {
    size: (Size, Size),
    cur_position: (Size, Size),
    lane_height: Size,
}

impl AtlasMap {
    fn new(size: (Size, Size)) -> Self {
        AtlasMap {
            size,
            cur_position: (0, 0),
            lane_height: 0,
        }
    }

    fn reset(&mut self) {
        self.cur_position = (0, 0);
        self.lane_height = 0;
    }

    fn add(&mut self, size: (Size, Size)) -> Rect {
        if self.cur_position.0 + size.0 > self.size.0 {
            assert!(size.0 <= self.size.0);
            self.cur_position = (0, self.cur_position.1 + self.lane_height);
            self.lane_height = 0;
        }

        if size.1 > self.lane_height {
            assert!(self.cur_position.1 + size.1 <= self.size.1);
            self.lane_height = size.1;
        }
        self.cur_position.0 += size.0;

        Rect {
            x: self.cur_position.0 - size.0,
            y: self.cur_position.1,
            w: size.0,
            h: size.1,
        }
    }
}




struct Downsampler<R: gfx::Resources> {
    primary: (h::RenderTargetView<R, CollisionFormat>, h::ShaderResourceView<R, CollisionFormatView>),
    secondary: (h::RenderTargetView<R, CollisionFormat>, h::ShaderResourceView<R, CollisionFormatView>),
    sampler: h::Sampler<R>,
    atlas: AtlasMap,
    vertices: Vec<DownsampleVertex>,
    v_buf: h::Buffer<R, DownsampleVertex>,
    pso: gfx::PipelineState<R, downsample::Meta>,
}

impl<R: gfx::Resources> Downsampler<R> {
    fn create_pso<F: gfx::Factory<R>>(
        factory: &mut F,
    ) -> gfx::PipelineState<R, downsample::Meta> {
        let (vs, fs) = read_shaders("downsample")
            .unwrap();
        let program = factory
            .link_program(&vs, &fs)
            .unwrap();
        factory
            .create_pipeline_from_program(
                &program,
                gfx::Primitive::TriangleList,
                gfx::state::Rasterizer::new_fill(),
                downsample::new(),
            )
            .unwrap()
    }

    pub fn new<F>(
        factory: &mut F,
        size: (Size, Size),
        max_vertices: usize,
    ) -> Self
    where
        F: gfx::Factory<R>
    {
        let (_, pri_srv, pri_rtv) = factory
            .create_render_target(size.0, size.1)
            .unwrap();
        let (_, sec_srv, sec_rtv) = factory
            .create_render_target(size.0, size.1)
            .unwrap();
        Downsampler {
            primary: (pri_rtv, pri_srv),
            secondary: (sec_rtv, sec_srv),
            sampler: factory.create_sampler_linear(),
            atlas: AtlasMap::new(size),
            vertices: Vec::new(),
            v_buf: factory.create_buffer(
                max_vertices,
                gfx::buffer::Role::Vertex,
                gfx::memory::Usage::Dynamic,
                gfx::memory::Bind::empty(),
            ).unwrap(),
            pso: Self::create_pso(factory),
        }
    }

    fn reset(&mut self) {
        self.atlas.reset();
        self.vertices.clear();
    }

    fn downsample(&mut self, source: Rect) -> Rect {
        let (atlas_w, atlas_h, _, _) = self.primary.0.get_dimensions();
        let size = ((source.w + 1) >> 1, (source.h + 1) >> 1);
        let dest = self.atlas.add(size);

        const RELATIVE: [(Size, Size); 6] = [(0, 0), (1, 0), (0, 1), (0, 1), (1, 0), (1, 1)];
        self.vertices.extend(RELATIVE.iter().map(|&(rx, ry)| DownsampleVertex {
            src: [
                (source.x + rx * source.w) as f32 / atlas_w as f32,
                (source.y + ry * source.h) as f32 / atlas_h as f32,
            ],
            dst: [
                2.0 * (dest.x + rx * dest.w) as f32 / atlas_w as f32 - 1.0,
                2.0 * (dest.y + ry * dest.h) as f32 / atlas_h as f32 - 1.0,
            ],
        }));

        dest
    }

    fn flush<C>(
        &mut self, encoder: &mut gfx::Encoder<R ,C>
    ) where
        C: gfx::CommandBuffer<R>,
    {
        assert!(self.vertices.len() < self.v_buf.len());
        encoder
            .update_buffer(&self.v_buf, &self.vertices, 0)
            .unwrap();

        let slice = gfx::Slice {
            end: self.vertices.len() as _,
            .. gfx::Slice::new_match_vertex_buffer(&self.v_buf)
        };
        encoder.draw(&slice, &self.pso, &downsample::Data {
            vbuf: self.v_buf.clone(),
            source: (self.primary.1.clone(), self.sampler.clone()),
            destination: self.secondary.0.clone(),
        });

        mem::swap(&mut self.primary, &mut self.secondary);
        self.reset();
    }

    pub fn reload<F>(
        &mut self, factory: &mut F
    ) where
        F: gfx::Factory<R>
    {
        self.pso = Self::create_pso(factory);
    }
}


#[derive(Clone, Debug, PartialEq)]
struct Epoch(usize);

#[must_use]
#[derive(Debug, PartialEq)]
pub struct ShapeId(usize, Epoch);

#[must_use]
pub struct CollisionResults<R: gfx::Resources> {
    results: Vec<Rect>,
    epoch: Epoch,
    pub view: h::ShaderResourceView<R, CollisionFormatView>,
}

impl<R: gfx::Resources> ops::Index<ShapeId> for CollisionResults<R> {
    type Output = Rect;
    fn index(&self, id: ShapeId) -> &Self::Output {
        assert_eq!(id.1, self.epoch);
        &self.results[id.0]
    }
}

pub struct CollisionBuilder<'a, R: gfx::Resources, C: 'a> {
    downsampler: &'a mut Downsampler<R>,
    encoder: &'a mut gfx::Encoder<R, C>,
    pso: &'a gfx::PipelineState<R, collision::Meta>,
    shader_data: collision::Data<R>,
    inputs: Vec<Rect>,
    epoch: Epoch,
}

impl<'a,
    R: gfx::Resources,
    C: gfx::CommandBuffer<R>,
> CollisionBuilder<'a, R, C> {
    pub fn add(
        &mut self, shape: &Shape<R>, transform: Transform,
    ) -> ShapeId {
        use cgmath;

        let size = (
            ((shape.bounds.coord_max[0] - shape.bounds.coord_min[0]) as f32 * transform.scale) as Size,
            ((shape.bounds.coord_max[1] - shape.bounds.coord_min[1]) as f32 * transform.scale) as Size,
        );
        let rect = self.downsampler.atlas.add(size);
        self.inputs.push(rect);

        self.encoder.update_constant_buffer(
            &self.shader_data.locals,
            &CollisionLocals {
                model: cgmath::Matrix4::from(transform).into(),
                target: [
                    rect.x as f32 - shape.bounds.coord_min[0] as f32 * transform.scale,
                    rect.y as f32 - shape.bounds.coord_min[1] as f32 * transform.scale,
                    2.0 / self.downsampler.atlas.size.0 as f32,
                    2.0 / self.downsampler.atlas.size.1 as f32,
                ],
            },
        );

        let poly_data = shape.polygons
            .iter()
            .map(|p| CollisionPolygon {
                origin: [p.middle[0], p.middle[1], p.middle[2], 1.0],
                normal: [p.normal[0], p.normal[1], p.normal[2], 0.0],
            })
            .collect::<Vec<_>>();
        self.encoder
            .update_buffer(&self.shader_data.polys, &poly_data, 0)
            .unwrap();

        //TODO: make `bound_vb` and `bound_slice` permanent
        let debug = shape.debug.as_ref().unwrap();
        self.shader_data.vbuf = debug.bound_vb.clone();

        self.encoder.draw(&debug.bound_slice, self.pso, &self.shader_data);

        ShapeId(self.inputs.len() - 1, self.epoch.clone())
    }

    pub fn finish(mut self) -> CollisionResults<R> {
        self.downsampler.atlas.reset();
        while self.inputs.iter().any(|rect| rect.w > 1 || rect.h > 1) {
            for rect in &mut self.inputs {
                *rect = self.downsampler.downsample(*rect);
            }
            self.downsampler.flush(&mut self.encoder);
        }

        CollisionResults {
            results: self.inputs,
            epoch: self.epoch,
            view: self.downsampler.primary.1.clone(),
        }
    }
}

pub struct GpuCollider<R: gfx::Resources> {
    downsampler: Downsampler<R>,
    pso: gfx::PipelineState<R, collision::Meta>,
    dummy_vb: h::Buffer<R, DebugPos>,
    surface_data: SurfaceData<R>,
    locals: h::Buffer<R, CollisionLocals>,
    globals: h::Buffer<R, CollisionGlobals>,
    polys: h::Buffer<R, CollisionPolygon>,
    epoch: Epoch,
}

impl<R: gfx::Resources> GpuCollider<R> {
    fn create_pso<F: gfx::Factory<R>>(
        factory: &mut F,
    ) -> gfx::PipelineState<R, collision::Meta> {
        let (vs, fs) = read_shaders("collision")
            .unwrap();
        let program = factory
            .link_program(&vs, &fs)
            .unwrap();
        factory
            .create_pipeline_from_program(
                &program,
                gfx::Primitive::TriangleList,
                gfx::state::Rasterizer::new_fill()
                    .with_cull_back(),
                collision::new(),
            )
            .unwrap()
    }

    pub fn new<F>(
        factory: &mut F,
        size: (Size, Size),
        max_downsample_vertices: usize,
        max_shape_polygons: usize,
        surface_data: SurfaceData<R>,
    ) -> Self
    where
        F: gfx::Factory<R>
    {
        GpuCollider {
            downsampler: Downsampler::new(factory, size, max_downsample_vertices),
            pso: Self::create_pso(factory),
            dummy_vb: factory.create_vertex_buffer(&[]),
            surface_data,
            locals: factory.create_constant_buffer(1),
            globals: factory.create_constant_buffer(1),
            polys: factory.create_constant_buffer(max_shape_polygons),
            epoch: Epoch(0),
        }
    }

    pub fn reload<F>(
        &mut self, factory: &mut F
    ) where
        F: gfx::Factory<R>
    {
        self.pso = Self::create_pso(factory);
        self.downsampler.reload(factory);
    }

    pub fn start<'a, C>(
        &'a mut self, encoder: &'a mut gfx::Encoder<R, C>, common: &Common,
    ) -> CollisionBuilder<'a, R, C>
    where
        C: gfx::CommandBuffer<R>,
    {
        self.epoch.0 += 1;
        self.downsampler.reset();

        encoder.update_constant_buffer(
            &self.globals,
            &CollisionGlobals {
                penetration: [
                    common.contact.k_elastic_spring,
                    common.impulse.elastic_restriction,
                    0.0,
                    0.0,
                ],
            },
        );

        let destination = self.downsampler.primary.0.clone();
        encoder.clear(&destination, [0.0; 4]);

        CollisionBuilder {
            downsampler: &mut self.downsampler,
            encoder,
            pso: &self.pso,
            shader_data: collision::Data {
                vbuf: self.dummy_vb.clone(),
                suf_constants: self.surface_data.constants.clone(),
                locals: self.locals.clone(),
                globals: self.globals.clone(),
                polys: self.polys.clone(),
                height: self.surface_data.height.clone(),
                meta: self.surface_data.meta.clone(),
                destination,
            },
            inputs: Vec::new(),
            epoch: self.epoch.clone(),
        }
    }
}
