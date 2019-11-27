use crate::{
    render::Shaders,
};

const WORK_GROUP_WIDTH: u32 = 64;

#[allow(dead_code)]
pub struct Data {
    pos_scale: [f32; 4],
    rot: [f32; 4],
    linear: [f32; 4],
    angular: [f32; 4],
    car_id: [u32; 4],
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
    buffer_data: wgpu::Buffer,
}

impl Store {
    pub fn new(device: &wgpu::Device) -> Self {
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
                wgpu::BindGroupLayoutBinding { // cars
                    binding: 2,
                    visibility: wgpu::ShaderStage::COMPUTE,
                    ty: wgpu::BindingType::StorageBuffer {
                        dynamic: false,
                        readonly: true,
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
        let buffer_data = device.create_buffer(&wgpu::BufferDescriptor {
            size: 4,
            usage: wgpu::BufferUsage::STORAGE | wgpu::BufferUsage::COPY_DST,
        });
        Store {
            pipeline_layout,
            pipelines,
            buffer_data,
        }
    }
}
