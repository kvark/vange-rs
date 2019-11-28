use crate::{
    config::{common::Common, settings},
    render::{
        collision::PolygonData,
        Shaders,
    },
};

use zerocopy::AsBytes as _;

use std::mem;


const WORK_GROUP_WIDTH: u32 = 64;

pub struct Data {
    _pos_scale: [f32; 4],
    _rot: [f32; 4],
    _linear: [f32; 4],
    _angular: [f32; 4],
    _collision: [f32; 4],
    _volume_zero_zomc: [f32; 4],
    _jacobian_inv: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, zerocopy::AsBytes, zerocopy::FromBytes)]
struct Uniforms {
    global_force: [f32; 4],
    delta: [f32; 4],
}

#[allow(dead_code)]
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

#[allow(dead_code)]
pub struct GpuStore {
    pipeline_layout_step: wgpu::PipelineLayout,
    pipeline_layout_gather: wgpu::PipelineLayout,
    pipelines: Pipelines,
    buf_data: wgpu::Buffer,
    buf_uniforms: wgpu::Buffer,
    buf_ranges: wgpu::Buffer,
    buf_empty: wgpu::Buffer, // empty buffer of WORK_GROUP_WIDTH words
    bind_group: wgpu::BindGroup,
    bind_group_gather: wgpu::BindGroup,
    gravity: f32,
}

impl GpuStore {
    pub fn new(
        device: &wgpu::Device,
        settings: &settings::GpuCollision,
        common: &Common,
        buf_collisions: &wgpu::Buffer,
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
        let desc_data = wgpu::BufferDescriptor {
            size: (settings.max_objects * mem::size_of::<Data>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsage::STORAGE | wgpu::BufferUsage::COPY_DST,
        };
        let buf_data = device.create_buffer(&desc_data);
        let desc_uniforms = wgpu::BufferDescriptor {
            size: mem::size_of::<Uniforms>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsage::UNIFORM | wgpu::BufferUsage::COPY_DST,
        };
        let buf_uniforms = device.create_buffer(&desc_uniforms);
        let desc_ranges = wgpu::BufferDescriptor {
            size: (settings.max_objects * 4) as wgpu::BufferAddress,
            usage: wgpu::BufferUsage::STORAGE_READ | wgpu::BufferUsage::COPY_DST,
        };
        let buf_ranges = device.create_buffer(&desc_ranges);
        let buf_empty = device.create_buffer_with_data(
            &[0u8; WORK_GROUP_WIDTH as usize * 4][..],
            wgpu::BufferUsage::COPY_SRC,
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
            ],
        });
        let bind_group_gather = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout_gather,
            bindings: &[
                wgpu::Binding {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer {
                        buffer: &buf_collisions,
                        range: 0 .. (settings.max_polygons_total * mem::size_of::<PolygonData>()) as wgpu::BufferAddress,
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
            buf_empty,
            bind_group,
            bind_group_gather,
            gravity: common.nature.gravity,
        }
    }

    pub fn reload(&mut self, device: &wgpu::Device) {
        self.pipelines = Pipelines::new(
            &self.pipeline_layout_step,
            &self.pipeline_layout_gather,
            device,
        );
    }

    pub fn step(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        delta: f32,
        ranges: &[u32],
    ) {
        let num_groups = 1;
        if !ranges.is_empty() {
            let temp = device.create_buffer_with_data(
                ranges.as_bytes(),
                wgpu::BufferUsage::COPY_SRC,
            );
            encoder.copy_buffer_to_buffer(
                &temp, 0,
                &self.buf_ranges, 0,
                4 * ranges.len() as wgpu::BufferAddress,
            );
            let tail = ranges.len() as u32 % WORK_GROUP_WIDTH;
            if tail != 0 {
                encoder.copy_buffer_to_buffer(
                    &self.buf_empty, 0,
                    &self.buf_ranges, 0,
                    4 * (WORK_GROUP_WIDTH - tail) as wgpu::BufferAddress,
                );
            }
        };
        {
            let uniforms = Uniforms {
                global_force: [0.0, 0.0, self.gravity, 0.0],
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

        let mut pass = encoder.begin_compute_pass();
        pass.set_pipeline(&self.pipelines.gather);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_bind_group(0, &self.bind_group_gather, &[]);
        pass.dispatch(num_groups, 1, 1);
        pass.set_pipeline(&self.pipelines.step);
        pass.dispatch(num_groups, 1, 1);
    }
}
