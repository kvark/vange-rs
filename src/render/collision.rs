use crate::{
    config::{common::Common, settings},
    model::{Shape},
    space::Transform,
    render::{
        SHAPE_POLYGON_BUFFER,
        Shaders,
        object::Context as ObjectContext,
        terrain::{Context as TerrainContext},
    },
};

use std::{mem, ops::Range, sync::{Arc, Mutex}};


#[repr(C)]
#[derive(Clone, Copy, zerocopy::FromBytes)]
pub struct PolygonData {
    pub force: [i32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, zerocopy::AsBytes, zerocopy::FromBytes)]
struct Locals {
    model: [[f32; 4]; 4],
    scale: [f32; 4],
    index_offset: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, zerocopy::AsBytes, zerocopy::FromBytes)]
struct Globals {
    target: [f32; 4],
    penetration: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
struct Epoch(usize);

#[must_use]
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeId(Range<usize>, Epoch);

struct PendingResult {
    buffer: wgpu::Buffer,
    count: usize,
    epoch: Epoch,
}

pub struct GpuCollider {
    pipeline_layout: wgpu::PipelineLayout,
    pipeline: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    uniform_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    dynamic_bind_group: wgpu::BindGroup,
    //TODO: remove it when WebGPU permits this
    dummy_target: wgpu::TextureView,
    locals_size: usize,
    epoch: Epoch,
    pending_result: Option<PendingResult>,
    latest: Arc<Mutex<(Vec<PolygonData>, Epoch)>>,
}

pub struct GpuSession<'this, 'pass> {
    pass: wgpu::RenderPass<'pass>,
    buffer: &'this wgpu::Buffer,
    uniform_buf: &'this wgpu::Buffer,
    dynamic_bind_group: &'this wgpu::BindGroup,
    locals_size: usize,
    object_locals: Vec<Locals>,
    polygon_id: usize,
    epoch: Epoch,
    pending_result: &'this mut Option<PendingResult>,
}

const DUMMY_TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

impl GpuCollider {
    fn create_pipeline(
        layout: &wgpu::PipelineLayout,
        device: &wgpu::Device,
    ) -> wgpu::RenderPipeline {
        let shaders = Shaders::new("collision", &[], device)
            .unwrap();
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            layout,
            vertex_stage: wgpu::ProgrammableStageDescriptor {
                module: &shaders.vs,
                entry_point: "main",
            },
            fragment_stage: Some(wgpu::ProgrammableStageDescriptor {
                module: &shaders.fs,
                entry_point: "main",
            }),
            rasterization_state: Some(wgpu::RasterizationStateDescriptor {
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: wgpu::CullMode::Back,
                depth_bias: 0,
                depth_bias_slope_scale: 0.0,
                depth_bias_clamp: 0.0,
            }),
            primitive_topology: wgpu::PrimitiveTopology::TriangleStrip,
            color_states: &[
                wgpu::ColorStateDescriptor {
                    format: DUMMY_TARGET_FORMAT,
                    color_blend: wgpu::BlendDescriptor::REPLACE,
                    alpha_blend: wgpu::BlendDescriptor::REPLACE,
                    write_mask: wgpu::ColorWrite::empty(),
                },
            ],
            depth_stencil_state: None,
            index_format: wgpu::IndexFormat::Uint16,
            vertex_buffers: &[
                SHAPE_POLYGON_BUFFER.clone(),
            ],
            sample_count: 1,
            alpha_to_coverage_enabled: false,
            sample_mask: !0,
        })
    }

    pub fn new(
        device: &wgpu::Device,
        settings: &settings::GpuCollision,
        common: &Common,
        object: &ObjectContext,
        terrain: &TerrainContext,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            bindings: &[
                wgpu::BindGroupLayoutBinding {
                    binding: 0,
                    visibility: wgpu::ShaderStage::VERTEX | wgpu::ShaderStage::FRAGMENT,
                    ty: wgpu::BindingType::UniformBuffer {
                        dynamic: false,
                    },
                },
                wgpu::BindGroupLayoutBinding {
                    binding: 1,
                    visibility: wgpu::ShaderStage::FRAGMENT,
                    ty: wgpu::BindingType::StorageBuffer {
                        dynamic: false,
                        readonly: false,
                    },
                },
            ],
        });
        let dynamic_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            bindings: &[
                wgpu::BindGroupLayoutBinding {
                    binding: 0,
                    visibility: wgpu::ShaderStage::VERTEX,
                    ty: wgpu::BindingType::UniformBuffer {
                        dynamic: true,
                    },
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            bind_group_layouts: &[
                &bind_group_layout,
                &terrain.bind_group_layout,
                &object.shape_bind_group_layout,
                &dynamic_bind_group_layout,
            ],
        });
        let pipeline = Self::create_pipeline(&pipeline_layout, device);
        let buf_size = (settings.max_polygons_total * mem::size_of::<PolygonData>()) as wgpu::BufferAddress;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            size: buf_size,
            usage: wgpu::BufferUsage::STORAGE | wgpu::BufferUsage::COPY_SRC,
        });

        let global_uniforms = device
            .create_buffer_mapped(1, wgpu::BufferUsage::UNIFORM)
            .fill_from_slice(&[Globals {
                target: [
                    2.0 / settings.max_raster_size.0 as f32,
                    2.0 / settings.max_raster_size.1 as f32,
                    0.0,
                    0.0,
                ],
                penetration: [
                    common.contact.k_elastic_spring,
                    common.impulse.elastic_restriction,
                    0.0,
                    0.0,
                ],
            }]);
        let locals_size = mem::size_of::<Locals>().max(256); //TODO: use constant from wgpu-rs
        let locals_total_size = (settings.max_objects * locals_size) as wgpu::BufferAddress;
        let local_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            size: locals_total_size,
            usage: wgpu::BufferUsage::COPY_DST | wgpu::BufferUsage::UNIFORM,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout,
            bindings: &[
                wgpu::Binding {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer {
                        buffer: &global_uniforms,
                        range: 0 .. mem::size_of::<Globals>() as wgpu::BufferAddress,
                    },
                },
                wgpu::Binding {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer {
                        buffer: &buffer,
                        range: 0 .. buf_size,
                    },
                },
            ],
        });
        let dynamic_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &dynamic_bind_group_layout,
            bindings: &[
                wgpu::Binding {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer {
                        buffer: &local_uniforms,
                        range: 0 .. locals_total_size,
                    },
                },
            ],
        });

        let dummy_target = device
            .create_texture(&wgpu::TextureDescriptor {
                size: wgpu::Extent3d {
                    width: settings.max_raster_size.0,
                    height: settings.max_raster_size.1,
                    depth: 1,
                },
                array_layer_count: 1,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DUMMY_TARGET_FORMAT,
                usage: wgpu::TextureUsage::OUTPUT_ATTACHMENT,
            })
            .create_default_view();

        GpuCollider {
            pipeline_layout,
            pipeline,
            buffer,
            uniform_buf: local_uniforms,
            bind_group,
            dynamic_bind_group,
            dummy_target,
            locals_size,
            epoch: Epoch(0),
            pending_result: None,
            latest: Arc::new(Mutex::new((Vec::new(), Epoch(0)))),
        }
    }

    pub fn reload(
        &mut self, device: &wgpu::Device,
    ) {
        self.pipeline = Self::create_pipeline(&self.pipeline_layout, device);
    }

    pub fn begin<'this, 'pass, 'dev>(
        &'this mut self,
        encoder: &'pass mut wgpu::CommandEncoder,
        terrain: &TerrainContext,
    ) -> GpuSession<'this, 'pass> {
        if let Some(pr) = self.pending_result.take() {
            let latest = Arc::clone(&self.latest);
            let epoch = pr.epoch;
            pr.buffer.map_read_async(0, pr.count, move |result| {
                let mut storage = latest.lock().unwrap();
                storage.1 = epoch;
                storage.0.clear();
                storage.0.extend_from_slice(&result.unwrap().data);
            });
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[
                wgpu::RenderPassColorAttachmentDescriptor {
                    attachment: &self.dummy_target,
                    resolve_target: None,
                    load_op: wgpu::LoadOp::Clear,
                    store_op: wgpu::StoreOp::Clear,
                    clear_color: wgpu::Color::BLACK,
                },
            ],
            depth_stencil_attachment: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_bind_group(1, &terrain.bind_group, &[]);

        GpuSession {
            pass,
            buffer: &self.buffer,
            uniform_buf: &self.uniform_buf,
            dynamic_bind_group: &self.dynamic_bind_group,
            locals_size: self.locals_size,
            object_locals: Vec::new(),
            polygon_id: 0,
            epoch: self.epoch.clone(),
            pending_result: &mut self.pending_result,
        }
    }
}

impl GpuSession<'_, '_> {
    pub fn add(&mut self, shape: &Shape, transform: Transform) -> ShapeId {
        let locals = Locals {
            model: cgmath::Matrix4::from(transform).into(),
            scale: [transform.scale; 4],
            index_offset: [self.polygon_id as u32, 0, 0, 0],
        };
        let offset = (self.object_locals.len() * self.locals_size) as wgpu::BufferAddress;

        self.pass.set_bind_group(2, &shape.bind_group, &[]);
        self.pass.set_bind_group(3, self.dynamic_bind_group, &[offset]);
        self.pass.set_vertex_buffers(0, &[(&shape.polygon_buf, 0)]);
        self.pass.draw(0 .. 4, 0 .. shape.polygons.len() as u32);

        let range = self.polygon_id .. self.polygon_id + shape.polygons.len();
        self.polygon_id = range.end;
        self.object_locals.push(locals);
        ShapeId(range, self.epoch.clone())
    }

    pub fn finish(self, device: &wgpu::Device) -> (wgpu::CommandBuffer, wgpu::CommandBuffer) {
        let GpuSession { buffer, epoch, polygon_id, pending_result, locals_size, object_locals, .. } = self;

        let prepare_comb = {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                todo: 0,
            });
            let temp = device
                .create_buffer_mapped(object_locals.len(), wgpu::BufferUsage::COPY_SRC)
                .fill_from_slice(&object_locals);
            for i in 0 .. object_locals.len() {
                encoder.copy_buffer_to_buffer(
                    &temp, (mem::size_of::<Locals>() * i) as wgpu::BufferAddress,
                    self.uniform_buf, (locals_size * i) as wgpu::BufferAddress,
                    mem::size_of::<Locals>() as wgpu::BufferAddress,
                );
            }
            encoder.finish()
        };
        let post_comb = {
            let size = (polygon_id * mem::size_of::<PolygonData>()) as wgpu::BufferAddress;
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                todo: 0,
            });
            let temp = device.create_buffer(&wgpu::BufferDescriptor {
                size,
                usage: wgpu::BufferUsage::COPY_DST | wgpu::BufferUsage::MAP_READ,
            });
            encoder.copy_buffer_to_buffer(buffer, 0, &temp, 0, size);
            *pending_result = Some(PendingResult {
                buffer: temp,
                count: polygon_id,
                epoch,
            });
            encoder.finish()
        };

        (prepare_comb, post_comb)
    }
}

/*
use config::common::Common;
use model::Shape;
use render::{read_shaders,
    ShapePolygon, SurfaceConstants, SurfaceData,
};
use space::Transform;

use gfx::{self, handle as h};
use gfx::format::Formatted;
use gfx::memory::Typed;
use gfx::texture::Size;
use gfx::traits::FactoryExt;
use gfx::Rect;

use std::{mem, ops};


pub use render::ColorFormat;
pub type CollisionFormat = gfx::format::Rgba32F;
pub type CollisionFormatView = <CollisionFormat as Formatted>::View;
pub type CollisionFormatSurface = <CollisionFormat as Formatted>::Surface;

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
    pub target: h::RenderTargetView<R, ColorFormat>,
    pub scale: Size,
}

//TODO: use mipmaps
struct RichTexture<R: gfx::Resources> {
    texture: h::Texture<R, CollisionFormatSurface>,
    rtv: h::RenderTargetView<R, CollisionFormat>,
    srv: h::ShaderResourceView<R, CollisionFormatView>,
}

impl<R: gfx::Resources> RichTexture<R> {
    fn new<F: gfx::Factory<R>>(
        size: (Size, Size), factory: &mut F
    ) -> Self {
        use gfx::texture as t;
        use gfx::format::{ChannelTyped, Swizzle};
        use gfx::memory::Bind;

        let kind = t::Kind::D2(size.0, size.1, t::AaMode::Single);
        let bind = Bind::SHADER_RESOURCE | Bind::RENDER_TARGET | Bind::TRANSFER_SRC;
        let cty = <<CollisionFormat as Formatted>::Channel as ChannelTyped>::get_channel_type();
        let texture = factory
            .create_texture(kind, 1, bind, gfx::memory::Usage::Data, Some(cty))
            .unwrap();
        let srv = factory
            .view_texture_as_shader_resource::<CollisionFormat>(&texture, (0, 0), Swizzle::new())
            .unwrap();
        let rtv = factory
            .view_texture_as_render_target(&texture, 0, None)
            .unwrap();
        RichTexture { texture, srv, rtv }
    }
}

struct Downsampler<R: gfx::Resources> {
    primary: RichTexture<R>,
    secondary: RichTexture<R>,
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
        let shaders = read_shaders("downsample", false, &[])
            .unwrap();
        let program = factory
            .link_program(&shaders.vs, &shaders.fs)
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

    pub fn new<F: gfx::Factory<R>>(
        factory: &mut F,
        size: (Size, Size),
        max_vertices: usize,
    ) -> Self {
        let primary = RichTexture::new(size, factory);
        let secondary = RichTexture::new(size, factory);
        let (pso, pso_debug) = Self::create_psos(factory);
        Downsampler {
            primary,
            secondary,
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

        let (dw, dh, _, _) = self.secondary.rtv.get_dimensions();
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
            source: (self.primary.srv.clone(), self.sampler.clone()),
            destination: self.secondary.rtv.raw().clone(),
        });

        mem::swap(&mut self.primary, &mut self.secondary);
        self.reset();
    }

    fn debug_blit<C>(
        &mut self,
        encoder: &mut gfx::Encoder<R ,C>,
        destination: h::RawRenderTargetView<R>,
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
            source: (self.primary.srv.clone(), self.sampler.clone()),
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
    read_buffer: Option<&'a h::Buffer<R, [f32; 4]>>,
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

        if let Some(read_buffer) = self.read_buffer {
            let cty = <<CollisionFormat as Formatted>::Channel as gfx::format::ChannelTyped>::get_channel_type();
            let source_image_info = self.downsampler.primary.texture
                .get_info()
                .to_raw_image_info(cty, 0);
            self.encoder
                .copy_texture_to_buffer_raw(
                    self.downsampler.primary.texture.raw(),
                    None,
                    gfx::texture::RawImageInfo {
                        height: 1,
                        .. source_image_info
                    },
                    read_buffer.raw(),
                    0,
                )
                .unwrap();
        }

        CollisionResults {
            results: self.inputs,
            epoch: self.epoch,
            view: self.downsampler.primary.srv.clone(),
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
    read_buffer: h::Buffer<R, [f32; 4]>,
    epoch: Epoch,
}

impl<R: gfx::Resources> GpuCollider<R> {
    fn create_pso<F: gfx::Factory<R>>(
        factory: &mut F,
    ) -> gfx::PipelineState<R, collision::Meta> {
        let shaders = read_shaders("collision", false, &[])
            .unwrap();
        let program = factory
            .link_program(&shaders.vs, &shaders.fs)
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
            read_buffer: factory
                .create_download_buffer(size.0 as _)
                .unwrap(),
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

        let destination = self.downsampler.primary.rtv.clone();
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
            read_buffer: Some(&self.read_buffer),
            epoch: self.epoch.clone(),
        }
    }

    pub fn readback(&self) -> &h::Buffer<R, [f32; 4]> {
        &self.read_buffer
    }
}
*/
