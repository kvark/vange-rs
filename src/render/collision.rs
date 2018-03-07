use model::Shape;
use render::{DebugPos, read_file};
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
        mvp: [[f32; 4]; 4] = "u_Mvp",
    }

    constant CollisionPolygon {
        origin: [f32; 4] = "u_Origin",
        normal: [f32; 4] = "u_Normal",
    }

    pipeline collision {
        vbuf: gfx::VertexBuffer<DebugPos> = (),
        locals: gfx::ConstantBuffer<CollisionLocals> = "c_Locals",
        polys: gfx::ConstantBuffer<CollisionPolygon> = "c_Polys",
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
        let program = factory
            .link_program(
                &read_file("data/shader/downsample.vert"),
                &read_file("data/shader/downsample.frag"),
            )
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


#[derive(Copy, Clone, Debug, PartialEq)]
struct Epoch(usize);

pub struct GpuCollider<R: gfx::Resources> {
    downsampler: Downsampler<R>,
    pso: gfx::PipelineState<R, collision::Meta>,
    locals: h::Buffer<R, CollisionLocals>,
    polys: h::Buffer<R, CollisionPolygon>,
    inputs: Vec<Rect>,
    epoch: Epoch,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ShapeId(usize, Epoch);

impl<R: gfx::Resources> ops::Index<ShapeId> for GpuCollider<R> {
    type Output = Rect;
    fn index(&self, id: ShapeId) -> &Self::Output {
        assert_eq!(id.1, self.epoch);
        &self.inputs[id.0]
    }
}

impl<R: gfx::Resources> GpuCollider<R> {
    fn create_pso<F: gfx::Factory<R>>(
        factory: &mut F,
    ) -> gfx::PipelineState<R, collision::Meta> {
        let program = factory
            .link_program(
                &read_file("data/shader/collision.vert"),
                &read_file("data/shader/collision.frag"),
            )
            .unwrap();
        factory
            .create_pipeline_from_program(
                &program,
                gfx::Primitive::TriangleList,
                gfx::state::Rasterizer::new_fill(),
                collision::new(),
            )
            .unwrap()
    }

    pub fn new<F>(
        factory: &mut F,
        size: (Size, Size),
        max_downsample_vertices: usize,
        max_shape_polygons: usize,
    ) -> Self
    where
        F: gfx::Factory<R>
    {
        GpuCollider {
            downsampler: Downsampler::new(factory, size, max_downsample_vertices),
            pso: Self::create_pso(factory),
            locals: factory.create_constant_buffer(1),
            polys: factory.create_constant_buffer(max_shape_polygons),
            inputs: Vec::new(),
            epoch: Epoch(0),
        }
    }

    pub fn reset(&mut self) {
        self.epoch.0 += 1;
        self.downsampler.reset();
        self.inputs.clear();
    }

    pub fn add<C>(
        &mut self,
        shape: &Shape<R>,
        transform: &Transform,
        encoder: &mut gfx::Encoder<R, C>,
    ) -> ShapeId
    where
        C: gfx::CommandBuffer<R>,
    {
        use cgmath;

        let size = (
            (shape.bounds.coord_max[0] - shape.bounds.coord_min[0]) as Size,
            (shape.bounds.coord_max[1] - shape.bounds.coord_min[1]) as Size,
        );
        let rect = self.downsampler.atlas.add(size);
        self.inputs.push(rect);

        //TODO: build orthographic projection based on the target rect

        encoder.update_constant_buffer(
            &self.locals,
            &CollisionLocals {
                mvp: cgmath::Matrix4::from(transform.clone()).into(),
            },
        );

        let poly_data: Vec<CollisionPolygon> = Vec::new(); //TODO
        encoder
            .update_buffer(&self.polys, &poly_data, 0)
            .unwrap();

        let debug = shape.debug.as_ref().unwrap(); //TODO

        encoder.draw(&debug.bound_slice, &self.pso, &collision::Data {
            vbuf: debug.bound_vb.clone(),
            locals: self.locals.clone(),
            polys: self.polys.clone(),
            destination: self.downsampler.primary.0.clone(),
        });

        ShapeId(self.inputs.len() - 1, self.epoch.clone())
    }

    pub fn process<C>(
        &mut self, encoder: &mut gfx::Encoder<R, C>
    ) -> h::ShaderResourceView<R, CollisionFormatView>
    where
        C: gfx::CommandBuffer<R>,
    {
        while self.inputs.iter().any(|rect| rect.w > 1 || rect.h > 1) {
            for rect in &mut self.inputs {
                *rect = self.downsampler.downsample(*rect);
            }
            self.downsampler.flush(encoder);
        }

        self.downsampler.primary.1.clone()
    }

    pub fn reload<F>(
        &mut self, factory: &mut F
    ) where
        F: gfx::Factory<R>
    {
        self.pso = Self::create_pso(factory);
        self.downsampler.reload(factory);
    }
}
