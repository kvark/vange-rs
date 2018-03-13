use config::common::Common;
use model::Shape;
use render::{read_shaders,
    ShapePolygon, SurfaceConstants, SurfaceData,
};
use space::Transform;

use gfx::{self, handle as h};
use gfx::format::Formatted;
use gfx::memory::Typed;
use gfx::Rect;
use gfx::texture::Size;
use gfx::traits::FactoryExt;

use std::{mem, ops};


pub use render::ColorFormat;
pub type CollisionFormat = gfx::format::Rgba32F;
pub type CollisionFormatView = <CollisionFormat as gfx::format::Formatted>::View;

gfx_defines!{
    constant CollisionLocals {
        model: [[f32; 4]; 4] = "u_Model",
        scale: [f32; 4] = "u_ModelScale",
        target: [f32; 4] = "u_TargetCenterScale",
    }

    constant CollisionGlobals {
        penetration: [f32; 4] = "u_Penetration",
    }

    pipeline collision {
        vertices: gfx::ShaderResource<[f32; 4]> = "t_Position",
        polygons: gfx::InstanceBuffer<ShapePolygon> = (),
        suf_constants: gfx::ConstantBuffer<SurfaceConstants> = "c_Surface",
        locals: gfx::ConstantBuffer<CollisionLocals> = "c_Locals",
        globals: gfx::ConstantBuffer<CollisionGlobals> = "c_Globals",
        height: gfx::TextureSampler<f32> = "t_Height",
        meta: gfx::TextureSampler<u32> = "t_Meta",
        destination: gfx::RenderTarget<CollisionFormat> = "Target0",
    }

    //TODO: use fixed point
    vertex DownsampleVertex {
        src: [u16; 4] = "a_SourceRect",
        dst: [u16; 4] = "a_DestRect",
    }

    pipeline downsample {
        vbuf: gfx::InstanceBuffer<DownsampleVertex> = (),
        dest_size: gfx::Global<[f32; 2]> = "u_DestSize",
        source: gfx::TextureSampler<CollisionFormatView> = "t_Source",
        destination: gfx::RawRenderTarget =
            ("Target0", CollisionFormat::get_format(), gfx::state::ColorMask::all(), None),
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

pub struct DebugBlit<R: gfx::Resources> {
    pub shape: ShapeId,
    pub target: gfx::handle::RenderTargetView<R, ColorFormat>,
    pub scale: Size,
}


struct Downsampler<R: gfx::Resources> {
    primary: (h::RenderTargetView<R, CollisionFormat>, h::ShaderResourceView<R, CollisionFormatView>),
    secondary: (h::RenderTargetView<R, CollisionFormat>, h::ShaderResourceView<R, CollisionFormatView>),
    sampler: h::Sampler<R>,
    atlas: AtlasMap,
    vertices: Vec<DownsampleVertex>,
    v_buf: h::Buffer<R, DownsampleVertex>,
    pso: gfx::PipelineState<R, downsample::Meta>,
    pso_debug: gfx::PipelineState<R, downsample::Meta>,
}

impl<R: gfx::Resources> Downsampler<R> {
    fn create_psos<F: gfx::Factory<R>>(
        factory: &mut F,
    ) -> (
        gfx::PipelineState<R, downsample::Meta>,
        gfx::PipelineState<R, downsample::Meta>,
    ) {
        let (vs, fs) = read_shaders("downsample")
            .unwrap();
        let program = factory
            .link_program(&vs, &fs)
            .unwrap();

        let pso = factory
            .create_pipeline_from_program(
                &program,
                gfx::Primitive::TriangleStrip,
                gfx::state::Rasterizer::new_fill(),
                downsample::new(),
            )
            .unwrap();
        let pso_debug = factory
            .create_pipeline_from_program(
                &program,
                gfx::Primitive::TriangleStrip,
                gfx::state::Rasterizer::new_fill(),
                downsample::Init {
                    destination: ("Target0", ColorFormat::get_format(), gfx::state::ColorMask::all(), None),
                    .. downsample::new()
                },
            )
            .unwrap();

        (pso, pso_debug)
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
        let (pso, pso_debug) = Self::create_psos(factory);
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
            pso,
            pso_debug,
        }
    }

    fn reset(&mut self) {
        self.atlas.reset();
        self.vertices.clear();
    }

    fn downsample(&mut self, source: Rect) -> Rect {
        let size = ((source.w + 1) >> 1, (source.h + 1) >> 1);
        let dest = self.atlas.add(size);

        self.vertices.push(DownsampleVertex {
            src: [source.x, source.y, source.w, source.h],
            dst: [dest.x, dest.y, dest.w, dest.h],
        });

        dest
    }

    fn flush<C>(
        &mut self,
        encoder: &mut gfx::Encoder<R ,C>,
    ) where
        C: gfx::CommandBuffer<R>,
    {
        assert!(self.vertices.len() < self.v_buf.len());
        encoder
            .update_buffer(&self.v_buf, &self.vertices, 0)
            .unwrap();

        let (dw, dh, _, _) = self.secondary.0.get_dimensions();
        let slice = gfx::Slice {
            start: 0,
            end: 4,
            base_vertex: 0,
            instances: Some((self.vertices.len() as _, 0)),
            buffer: gfx::IndexBuffer::Auto,
        };
        encoder.draw(&slice, &self.pso, &downsample::Data {
            vbuf: self.v_buf.clone(),
            dest_size: [dw as f32, dh as f32],
            source: (self.primary.1.clone(), self.sampler.clone()),
            destination: self.secondary.0.raw().clone(),
        });

        mem::swap(&mut self.primary, &mut self.secondary);
        self.reset();
    }

    fn debug_blit<C>(
        &mut self,
        encoder: &mut gfx::Encoder<R ,C>,
        destination: gfx::handle::RawRenderTargetView<R>,
        src: Rect,
        dst: Rect,
    ) where
        C: gfx::CommandBuffer<R>,
    {
        let slice = gfx::Slice {
            start: 0,
            end: 4,
            base_vertex: 0,
            instances: Some((1, 0)),
            buffer: gfx::IndexBuffer::Auto,
        };
        encoder.update_constant_buffer(&self.v_buf, &DownsampleVertex {
            src: [src.x, src.y, src.w, src.h],
            dst: [dst.x, dst.y, dst.w, dst.h],
        });
        let (w, h, _, _) = destination.get_dimensions();
        encoder.draw(&slice, &self.pso_debug, &downsample::Data {
            vbuf: self.v_buf.clone(),
            dest_size: [w as f32, h as f32],
            source: (self.primary.1.clone(), self.sampler.clone()),
            destination,
        });
    }

    pub fn reload<F>(
        &mut self, factory: &mut F
    ) where
        F: gfx::Factory<R>
    {
        let (pso, pso_debug) = Self::create_psos(factory);
        self.pso = pso;
        self.pso_debug = pso_debug;
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
            ((1 + shape.bounds.coord_max[0] - shape.bounds.coord_min[0]) as f32 * transform.scale).ceil() as Size,
            ((1 + shape.bounds.coord_max[1] - shape.bounds.coord_min[1]) as f32 * transform.scale).ceil() as Size,
        );
        let rect = self.downsampler.atlas.add(size);
        self.inputs.push(rect);

        self.encoder.update_constant_buffer(
            &self.shader_data.locals,
            &CollisionLocals {
                model: cgmath::Matrix4::from(transform).into(),
                scale: [transform.scale; 4],
                target: [
                    rect.x as f32 + (1.0 - shape.bounds.coord_min[0] as f32) * transform.scale,
                    rect.y as f32 + (1.0 - shape.bounds.coord_min[1] as f32) * transform.scale,
                    2.0 / self.downsampler.atlas.size.0 as f32,
                    2.0 / self.downsampler.atlas.size.1 as f32,
                ],
            },
        );

        self.shader_data.vertices = shape.vertex_view.clone();
        self.shader_data.polygons = shape.polygon_buf.clone();
        let slice = shape.make_draw_slice();

        self.encoder.draw(&slice, self.pso, &self.shader_data);

        ShapeId(self.inputs.len() - 1, self.epoch.clone())
    }

    pub fn finish(
        mut self, debug_blit: Option<DebugBlit<R>>
    ) -> CollisionResults<R> {
        let mut debug = debug_blit.map(|d| {
            let (w, h, _, _) = d.target.get_dimensions();
            (d, AtlasMap::new((w, h)))
        });
        self.downsampler.atlas.reset();

        while self.inputs.iter().any(|rect| rect.w > 1 || rect.h > 1) {
            if let Some((ref blit, ref mut atlas)) = debug {
                let src = self.inputs[blit.shape.0];
                let dst = atlas.add((src.w * blit.scale, src.h * blit.scale));
                self.downsampler.debug_blit(
                    &mut self.encoder,
                    blit.target.raw().clone(),
                    src,
                    dst
                );
            }
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
    dummy_view: h::ShaderResourceView<R, [f32; 4]>,
    dummy_poly: h::Buffer<R, ShapePolygon>,
    surface_data: SurfaceData<R>,
    locals: h::Buffer<R, CollisionLocals>,
    globals: h::Buffer<R, CollisionGlobals>,
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
                gfx::Primitive::TriangleStrip,
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
        surface_data: SurfaceData<R>,
    ) -> Self
    where
        F: gfx::Factory<R>
    {
        let dummy_vert = factory
            .create_buffer_immutable(
                &[],
                gfx::buffer::Role::Vertex,
                gfx::memory::Bind::SHADER_RESOURCE,
            ).unwrap();
        GpuCollider {
            downsampler: Downsampler::new(factory, size, max_downsample_vertices),
            pso: Self::create_pso(factory),
            dummy_view: factory
                .view_buffer_as_shader_resource(&dummy_vert)
                .unwrap(),
            dummy_poly: factory.create_vertex_buffer(&[]),
            surface_data,
            locals: factory.create_constant_buffer(1),
            globals: factory.create_constant_buffer(1),
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
        &'a mut self,
        encoder: &'a mut gfx::Encoder<R, C>,
        common: &Common,
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
                vertices: self.dummy_view.clone(),
                polygons: self.dummy_poly.clone(),
                suf_constants: self.surface_data.constants.clone(),
                locals: self.locals.clone(),
                globals: self.globals.clone(),
                height: self.surface_data.height.clone(),
                meta: self.surface_data.meta.clone(),
                destination,
            },
            inputs: Vec::new(),
            epoch: self.epoch.clone(),
        }
    }
}
