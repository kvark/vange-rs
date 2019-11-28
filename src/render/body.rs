use crate::{
    config::{common::Common, settings},
    render::Shaders,
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
}

impl Pipelines {
    fn new(layout: &wgpu::PipelineLayout, device: &wgpu::Device) -> Self {
        Pipelines {
            step: device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                layout,
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
        }
    }
}

#[allow(dead_code)]
pub struct Store {
    pipeline_layout: wgpu::PipelineLayout,
    pipelines: Pipelines,
    buf_data: wgpu::Buffer,
    buf_uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    gravity: f32,
}

impl Store {
    pub fn new(
        device: &wgpu::Device,
        settings: &settings::GpuCollision,
        common: &Common,
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            bind_group_layouts: &[
                &bind_group_layout,
            ],
        });
        let pipelines = Pipelines::new(&pipeline_layout, device);
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

        Store {
            pipeline_layout,
            pipelines,
            buf_data,
            buf_uniforms,
            bind_group,
            gravity: common.nature.gravity,
        }
    }

    pub fn reload(&mut self, device: &wgpu::Device) {
        self.pipelines = Pipelines::new(&self.pipeline_layout, device);
    }

    pub fn step(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, delta: f32) {
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

        let mut pass = encoder.begin_compute_pass();
        pass.set_pipeline(&self.pipelines.step);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.dispatch(1, 1, 1);
    }
}
