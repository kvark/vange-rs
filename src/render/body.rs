use crate::{
    config::{
        car::CarPhysics,
        common::Common,
        settings,
    },
    freelist::{self, FreeList},
    render::{
        collision::{GpuColliderInit, GpuRange, PolygonData},
        Shaders,
    },
    space::Transform,
};

use cgmath::SquareMatrix as _;
use zerocopy::AsBytes as _;

use std::mem;


const WORK_GROUP_WIDTH: u32 = 64;

#[repr(C)]
#[derive(zerocopy::AsBytes)]
pub struct Data {
    pos_scale: [f32; 4],
    rot: [f32; 4],
    linear: [f32; 4],
    angular: [f32; 4],
    collision: [f32; 4],
    scale_volume_zomc: [f32; 4],
    jacobian_inv: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, zerocopy::AsBytes, zerocopy::FromBytes)]
struct Uniforms {
    delta: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, zerocopy::AsBytes, zerocopy::FromBytes)]
struct Constants {
    nature: [f32; 4],
    drag_free: [f32; 2],
    drag_speed: [f32; 2],
    drag_spring: [f32; 2],
    drag_abs_min: [f32; 2],
    drag_abs_stop: [f32; 2],
    drag_coll: [f32; 2],
}

pub type GpuBody = freelist::Id<Data>;

struct Pipelines {
    step: wgpu::ComputePipeline,
    gather: wgpu::ComputePipeline,
}

impl Pipelines {
    fn new(
        layout_step: &wgpu::PipelineLayout,
        layout_gather: &wgpu::PipelineLayout,
        device: &wgpu::Device,
    ) -> Self {
        Pipelines {
            step: device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                layout: layout_step,
                compute_stage: wgpu::ProgrammableStageDescriptor {
                    module: &Shaders::new_compute(
                        "physics/body_step",
                        [WORK_GROUP_WIDTH, 1, 1],
                        &[],
                        device,
                    ).unwrap(),
                    entry_point: "main",
                },
            }),
            gather: device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                layout: layout_gather,
                compute_stage: wgpu::ProgrammableStageDescriptor {
                    module: &Shaders::new_compute(
                        "physics/body_gather",
                        [WORK_GROUP_WIDTH, 1, 1],
                        &[],
                        device,
                    ).unwrap(),
                    entry_point: "main",
                },
            }),
        }
    }
}

pub struct GpuStore {
    pipeline_layout_step: wgpu::PipelineLayout,
    pipeline_layout_gather: wgpu::PipelineLayout,
    pipelines: Pipelines,
    buf_data: wgpu::Buffer,
    buf_uniforms: wgpu::Buffer,
    buf_ranges: wgpu::Buffer,
    capacity: usize,
    bind_group: wgpu::BindGroup,
    bind_group_gather: wgpu::BindGroup,
    free_list: FreeList<Data>,
    pending_additions: Vec<(usize, Data)>,
}

impl GpuStore {
    pub fn new(
        device: &wgpu::Device,
        settings: &settings::GpuCollision,
        common: &Common,
        collider_init: &GpuColliderInit,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            bindings: &[
                wgpu::BindGroupLayoutBinding { // data
                    binding: 0,
                    visibility: wgpu::ShaderStage::COMPUTE,
                    ty: wgpu::BindingType::StorageBuffer {
                        dynamic: false,
                        readonly: false,
                    },
                },
                wgpu::BindGroupLayoutBinding { // uniforms
                    binding: 1,
                    visibility: wgpu::ShaderStage::COMPUTE,
                    ty: wgpu::BindingType::UniformBuffer {
                        dynamic: false,
                    },
                },
                wgpu::BindGroupLayoutBinding { // constants
                    binding: 2,
                    visibility: wgpu::ShaderStage::COMPUTE,
                    ty: wgpu::BindingType::UniformBuffer {
                        dynamic: false,
                    },
                },
            ],
        });
        let pipeline_layout_step = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            bind_group_layouts: &[
                &bind_group_layout,
            ],
        });

        let bind_group_layout_gather = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            bindings: &[
                wgpu::BindGroupLayoutBinding { // collisions
                    binding: 0,
                    visibility: wgpu::ShaderStage::COMPUTE,
                    ty: wgpu::BindingType::StorageBuffer {
                        dynamic: false,
                        readonly: true,
                    },
                },
                wgpu::BindGroupLayoutBinding { // ranges
                    binding: 1,
                    visibility: wgpu::ShaderStage::COMPUTE,
                    ty: wgpu::BindingType::StorageBuffer {
                        dynamic: false,
                        readonly: true,
                    },
                },
            ],
        });
        let pipeline_layout_gather = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            bind_group_layouts: &[
                &bind_group_layout,
                &bind_group_layout_gather,
            ],
        });

        let pipelines = Pipelines::new(&pipeline_layout_step, &pipeline_layout_gather, device);
        let rounded_max_objects = {
            let tail = settings.max_objects as u32 % WORK_GROUP_WIDTH;
            settings.max_objects + if tail != 0 {
                (WORK_GROUP_WIDTH - tail) as usize
            } else {
                0
            }
        };
        let desc_data = wgpu::BufferDescriptor {
            size: (rounded_max_objects * mem::size_of::<Data>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsage::STORAGE | wgpu::BufferUsage::COPY_DST,
        };
        let buf_data = device.create_buffer(&desc_data);
        let desc_uniforms = wgpu::BufferDescriptor {
            size: mem::size_of::<Uniforms>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsage::UNIFORM | wgpu::BufferUsage::COPY_DST,
        };
        let buf_uniforms = device.create_buffer(&desc_uniforms);
        let desc_ranges = wgpu::BufferDescriptor {
            size: (rounded_max_objects * mem::size_of::<GpuRange>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsage::STORAGE_READ | wgpu::BufferUsage::COPY_DST,
        };
        let buf_ranges = device.create_buffer(&desc_ranges);

        let constants = Constants {
            nature: [common.nature.time_delta0, 0.0, common.nature.gravity, 0.0],
            drag_free: common.drag.free.to_array(),
            drag_speed: common.drag.speed.to_array(),
            drag_spring: common.drag.spring.to_array(),
            drag_abs_min: common.drag.abs_min.to_array(),
            drag_abs_stop: common.drag.abs_stop.to_array(),
            drag_coll: common.drag.coll.to_array(),
        };
        let buf_constants = device.create_buffer_with_data(
            [constants].as_bytes(),
            wgpu::BufferUsage::UNIFORM,
        );

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout,
            bindings: &[
                wgpu::Binding {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer {
                        buffer: &buf_data,
                        range: 0 .. desc_data.size,
                    },
                },
                wgpu::Binding {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer {
                        buffer: &buf_uniforms,
                        range: 0 .. desc_uniforms.size,
                    },
                },
                wgpu::Binding {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer {
                        buffer: &buf_constants,
                        range: 0 .. mem::size_of::<Constants>() as wgpu::BufferAddress,
                    },
                },
            ],
        });
        let bind_group_gather = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout_gather,
            bindings: &[
                wgpu::Binding {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer {
                        buffer: &collider_init.buffer,
                        range: 0 .. (collider_init.max_polygons_total * mem::size_of::<PolygonData>()) as wgpu::BufferAddress,
                    },
                },
                wgpu::Binding {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer {
                        buffer: &buf_ranges,
                        range: 0 .. desc_ranges.size,
                    },
                },
            ],
        });

        GpuStore {
            pipeline_layout_step,
            pipeline_layout_gather,
            pipelines,
            buf_data,
            buf_uniforms,
            buf_ranges,
            capacity: rounded_max_objects,
            bind_group,
            bind_group_gather,
            free_list: FreeList::new(),
            pending_additions: Vec::new(),
        }
    }

    pub fn data_buffer(&self) -> (&wgpu::Buffer, wgpu::BufferAddress) {
        (&self.buf_data, (self.capacity * mem::size_of::<Data>()) as wgpu::BufferAddress)
    }

    pub fn reload(&mut self, device: &wgpu::Device) {
        self.pipelines = Pipelines::new(
            &self.pipeline_layout_step,
            &self.pipeline_layout_gather,
            device,
        );
    }

    pub fn alloc(
        &mut self,
        transform: &Transform,
        model_physics: &m3d::Physics,
        car_physics: &CarPhysics,
    ) -> GpuBody {
        let id = self.free_list.alloc();
        let matrix = cgmath::Matrix3::from(model_physics.jacobi).invert().unwrap();
        let data = Data {
            pos_scale: [transform.disp.x, transform.disp.y, transform.disp.z, transform.scale],
            rot: transform.rot.into(),
            linear: [0.0; 4],
            angular: [0.0; 4],
            collision: [0.0; 4],
            scale_volume_zomc: [car_physics.scale_bound, model_physics.volume, car_physics.z_offset_of_mass_center, 0.0],
            jacobian_inv: cgmath::Matrix4::from(matrix).into(),
        };
        self.pending_additions.push((id.index(), data));
        id
    }

    pub fn free(&mut self, id: GpuBody) {
        self.free_list.free(id);
    }

    pub fn update_entries(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        // fill out new data entries
        for (index, data) in self.pending_additions.drain(..) {
            let temp = device.create_buffer_with_data(
                [data].as_bytes(),
                wgpu::BufferUsage::COPY_SRC,
            );
            let size = mem::size_of::<Data>() as wgpu::BufferAddress;
            encoder.copy_buffer_to_buffer(
                &temp, 0,
                &self.buf_data,
                index as wgpu::BufferAddress * size,
                size,
            );
        }
    }

    pub fn step(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        delta: f32,
        raw_ranges: &[GpuRange],
    ) {
        assert!(self.pending_additions.is_empty());
        let num_groups = {
            let num_objects = self.free_list.length();
            let reminder = num_objects % WORK_GROUP_WIDTH as usize;
            let extra = if reminder != 0 { 1 } else { 0 };
            num_objects as u32 / WORK_GROUP_WIDTH + extra
        };

        // update range buffer
        {
            let sub_range = &raw_ranges[.. (num_groups * WORK_GROUP_WIDTH) as usize];
            let temp = device.create_buffer_with_data(
                sub_range.as_bytes(),
                wgpu::BufferUsage::COPY_SRC,
            );
            encoder.copy_buffer_to_buffer(
                &temp, 0,
                &self.buf_ranges, 0,
                (sub_range.len() * mem::size_of::<GpuRange>()) as wgpu::BufferAddress,
            );
        }

        // update global uniforms
        {
            let uniforms = Uniforms {
                delta: [delta, 0.0, 0.0, 0.0],
            };
            let temp = device.create_buffer_with_data(
                uniforms.as_bytes(),
                wgpu::BufferUsage::COPY_SRC,
            );
            encoder.copy_buffer_to_buffer(
                &temp, 0,
                &self.buf_uniforms, 0,
                mem::size_of::<Uniforms>() as wgpu::BufferAddress,
            );
        }

        // compute all the things
        let mut pass = encoder.begin_compute_pass();
        pass.set_pipeline(&self.pipelines.gather);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_bind_group(1, &self.bind_group_gather, &[]);
        pass.dispatch(num_groups, 1, 1);
        pass.set_pipeline(&self.pipelines.step);
        pass.dispatch(num_groups, 1, 1);
    }
}
