use crate::{
    config::{common::Common, settings},
    model::Shape,
    render::{
        object::Context as ObjectContext, terrain::Context as TerrainContext, Shaders,
        ShapeVertexDesc,
    },
};

use bytemuck::{Pod, Zeroable};
use futures::executor::LocalSpawner;
use wgpu::util::DeviceExt as _;

use std::mem;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PolygonData {
    middle: u32,
    depth_soft: u32,
    depth_hard: u32,
    //normal: [f32; 2],
}
unsafe impl Pod for PolygonData {}
unsafe impl Zeroable for PolygonData {}

pub type GpuRange = u32;
fn encode_gpu_range(base: usize, count: usize) -> GpuRange {
    base as u32 | (((base + count) as u32) << 16)
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Locals {
    indexes: [u32; 2],
}
unsafe impl Pod for Locals {}
unsafe impl Zeroable for Locals {}

#[repr(C)]
#[derive(Clone, Copy)]
struct Globals {
    target: [f32; 4],
    penetration: [f32; 4],
}
unsafe impl Pod for Globals {}
unsafe impl Zeroable for Globals {}

#[repr(C)]
#[derive(Clone, Copy)]
struct ClearLocals {
    count: [u32; 4],
}
unsafe impl Pod for ClearLocals {}
unsafe impl Zeroable for ClearLocals {}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, PartialOrd)]
pub struct GpuEpoch(usize);

#[derive(Debug, Default)]
pub struct GpuResult {
    pub depths: Vec<f32>,
    pub epoch: GpuEpoch,
}

pub struct GpuCollider {
    pipeline_layout: wgpu::PipelineLayout,
    clear_pipeline_layout: wgpu::PipelineLayout,
    pipeline: wgpu::RenderPipeline,
    clear_pipeline: wgpu::ComputePipeline,
    buffer: wgpu::Buffer,
    uniform_buf: wgpu::Buffer,
    capacity: usize,
    bind_group: wgpu::BindGroup,
    dynamic_bind_group: wgpu::BindGroup,
    //TODO: remove it when WebGPU permits this
    dummy_target: wgpu::TextureView,
    locals_size: usize,
    dirty_group_count: u32,
    epoch: GpuEpoch,
    ranges: Vec<GpuRange>,
}

pub struct GpuSession<'pass, 'this> {
    pass: wgpu::RenderPass<'pass>,
    uniform_buf: &'this wgpu::Buffer,
    dynamic_bind_group: &'this wgpu::BindGroup,
    locals_size: usize,
    object_locals: Vec<Locals>,
    ranges: &'this mut [GpuRange],
    polygon_id: usize,
    dirty_group_count: &'this mut u32,
    pub epoch: GpuEpoch,
}

const DUMMY_TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
const CLEAR_WORK_GROUP_WIDTH: u32 = 64;

impl GpuCollider {
    fn create_pipelines(
        layout: &wgpu::PipelineLayout,
        clear_layout: &wgpu::PipelineLayout,
        device: &wgpu::Device,
    ) -> (wgpu::RenderPipeline, wgpu::ComputePipeline) {
        let shaders = Shaders::new("physics/collision_add", &[], device).unwrap();
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("collision"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &shaders.vs,
                entry_point: "main",
                buffers: &[ShapeVertexDesc::new().buffer_desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shaders.fs,
                entry_point: "main",
                targets: &[wgpu::ColorTargetState {
                    format: DUMMY_TARGET_FORMAT,
                    blend: None,
                    write_mask: if cfg!(debug_assertions) {
                        wgpu::ColorWrites::all()
                    } else {
                        wgpu::ColorWrites::empty()
                    },
                }],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                front_face: wgpu::FrontFace::Cw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let clear_shader = Shaders::new_compute(
            "physics/collision_clear",
            [CLEAR_WORK_GROUP_WIDTH, 1, 1],
            &[],
            device,
        )
        .unwrap();
        let clear_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("collision-clear"),
            layout: Some(clear_layout),
            module: &clear_shader,
            entry_point: "main",
        });

        (pipeline, clear_pipeline)
    }

    pub fn new(
        device: &wgpu::Device,
        settings: &settings::GpuCollision,
        common: &Common,
        object: &ObjectContext,
        terrain: &TerrainContext,
        store_buffer: wgpu::BindingResource<'_>,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Collision"),
            entries: &[
                // global uniforms
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // collisions
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // data
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let dynamic_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Collision has_dynamic_offset"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("collision"),
            bind_group_layouts: &[
                &bind_group_layout,
                &terrain.bind_group_layout,
                object.shape_bind_group_layout.as_ref().unwrap(),
                &dynamic_bind_group_layout,
            ],
            push_constant_ranges: &[],
        });

        let clear_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("collision-clear"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });
        let (pipeline, clear_pipeline) =
            Self::create_pipelines(&pipeline_layout, &clear_pipeline_layout, device);

        // ensure the total size fits complete number of workgroups

        let globals = Globals {
            target: [
                2.0 / settings.max_raster_size.0 as f32,
                2.0 / settings.max_raster_size.1 as f32,
                1.0 / 256.0,
                0.0,
            ],
            penetration: [common.terrain.min_wall_delta, 0.0, 0.0, 0.0],
        };
        let global_uniforms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("collision-globals"),
            contents: bytemuck::bytes_of(&globals),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        //TODO: device.limits().min_uniform_buffer_offset_alignment
        let locals_size = mem::size_of::<Locals>().max(256);
        let locals_total_size = (settings.max_objects * locals_size) as wgpu::BufferAddress;
        let local_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Collision Locals"),
            size: locals_total_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let max_polygons_total =
            (settings.max_polygons_total - 1) | ((CLEAR_WORK_GROUP_WIDTH - 1) as usize + 1);
        let buf_size = (max_polygons_total * mem::size_of::<PolygonData>()) as wgpu::BufferAddress;
        let collision_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Collision"),
            size: buf_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Collision"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: global_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: collision_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: store_buffer,
                },
            ],
        });
        let dynamic_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Collision has_dynamic_offset"),
            layout: &dynamic_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: local_uniforms.as_entire_binding(),
            }],
        });

        let dummy_target = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Dummy"),
                size: wgpu::Extent3d {
                    width: settings.max_raster_size.0,
                    height: settings.max_raster_size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DUMMY_TARGET_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            })
            .create_view(&wgpu::TextureViewDescriptor::default());

        GpuCollider {
            pipeline_layout,
            clear_pipeline_layout,
            pipeline,
            clear_pipeline,
            buffer: collision_buffer,
            uniform_buf: local_uniforms,
            bind_group,
            dynamic_bind_group,
            dummy_target,
            capacity: max_polygons_total,
            dirty_group_count: max_polygons_total as u32 / CLEAR_WORK_GROUP_WIDTH,
            locals_size,
            epoch: GpuEpoch::default(),
            ranges: {
                let alignment = 64; // ensure compatibility with `WORK_GROU_WIDTH`
                vec![0; ((settings.max_objects - 1) | (alignment - 1)) + 1]
            },
        }
    }

    pub fn reload(&mut self, device: &wgpu::Device) {
        let (pipeline, clear_pipeline) =
            Self::create_pipelines(&self.pipeline_layout, &self.clear_pipeline_layout, device);
        self.pipeline = pipeline;
        self.clear_pipeline = clear_pipeline;
    }

    pub fn begin<'pass, 'this: 'pass>(
        &'this mut self,
        encoder: &'pass mut wgpu::CommandEncoder,
        terrain: &'pass TerrainContext,
        _spawner: &LocalSpawner,
    ) -> GpuSession<'pass, 'this> {
        if self.dirty_group_count != 0 {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("collider/clear"),
            });
            pass.set_pipeline(&self.clear_pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.dispatch(self.dirty_group_count, 1, 1);
            self.dirty_group_count = 0;
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("collider"),
            color_attachments: &[wgpu::RenderPassColorAttachment {
                view: &self.dummy_target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: false,
                },
            }],
            depth_stencil_attachment: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_bind_group(1, &terrain.bind_group, &[]);

        self.epoch.0 += 1;
        for r in self.ranges.iter_mut() {
            *r = 0;
        }

        GpuSession {
            pass,
            uniform_buf: &self.uniform_buf,
            dynamic_bind_group: &self.dynamic_bind_group,
            locals_size: self.locals_size,
            object_locals: Vec::with_capacity(self.capacity),
            ranges: &mut self.ranges,
            polygon_id: 0,
            dirty_group_count: &mut self.dirty_group_count,
            epoch: self.epoch,
        }
    }

    pub fn collision_buffer(&self) -> wgpu::BindingResource<'_> {
        self.buffer.as_entire_binding()
    }
}

impl<'pass, 'this: 'pass> GpuSession<'pass, 'this> {
    pub fn add(&mut self, shape: &'pass Shape, range_id: usize) -> usize {
        let locals = Locals {
            indexes: [range_id as u32, self.polygon_id as u32],
        };
        let offset = (self.object_locals.len() * self.locals_size) as wgpu::DynamicOffset;

        self.pass
            .set_bind_group(2, shape.bind_group.as_ref().unwrap(), &[]);
        self.pass
            .set_bind_group(3, self.dynamic_bind_group, &[offset]);
        self.pass.set_vertex_buffer(0, shape.polygon_buf.slice(..));
        self.pass.draw(0..4, 0..shape.polygons.len() as u32);

        let offset = self.polygon_id;
        self.ranges[range_id] = encode_gpu_range(offset, shape.polygons.len());
        self.polygon_id += shape.polygons.len();
        self.object_locals.push(locals);
        offset
    }

    pub fn finish(
        self,
        prep_encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
    ) -> &'this [GpuRange] {
        let mut num_groups = self.polygon_id as u32 / CLEAR_WORK_GROUP_WIDTH;
        if num_groups * CLEAR_WORK_GROUP_WIDTH < self.polygon_id as u32 {
            num_groups += 1;
        }
        *self.dirty_group_count = (*self.dirty_group_count).max(num_groups);

        let temp = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("temp-collision"),
            contents: bytemuck::cast_slice(&self.object_locals),
            usage: wgpu::BufferUsages::COPY_SRC,
        });
        for i in 0..self.object_locals.len() {
            prep_encoder.copy_buffer_to_buffer(
                &temp,
                (mem::size_of::<Locals>() * i) as wgpu::BufferAddress,
                self.uniform_buf,
                (self.locals_size * i) as wgpu::BufferAddress,
                mem::size_of::<Locals>() as wgpu::BufferAddress,
            );
        }

        self.ranges
    }
}
