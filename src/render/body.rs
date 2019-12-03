use crate::{
    config::{
        car::CarPhysics,
        common::Common,
        settings,
    },
    freelist::{self, FreeList},
    model::VisualModel,
    render::{
        collision::GpuRange,
        GpuTransform,
        Shaders,
    },
    space::Transform,
};

use cgmath::SquareMatrix as _;
use zerocopy::AsBytes as _;

use std::mem;


const WORK_GROUP_WIDTH: u32 = 64;
const MAX_WHEELS: usize = 4;

pub type GpuControl = [f32; 4];

#[repr(C)]
#[derive(zerocopy::AsBytes)]
pub struct Data {
    control: GpuControl,
    pos_scale: [f32; 4],
    orientation: [f32; 4],
    linear: [f32; 4],
    angular: [f32; 4],
    collision: [f32; 4],
    scale_volume_zomc: [f32; 4],
    jacobian_inv: [[f32; 4]; 4],
    wheels: [[f32; 4]; MAX_WHEELS],
}

impl Data {
    const DUMMY: Self = Data {
        control: [0.0; 4],
        pos_scale: [0.0, 0.0, 0.0, 1.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
        linear: [0.0; 4],
        angular: [0.0; 4],
        collision: [0.0; 4],
        scale_volume_zomc: [1.0, 1.0, 0.0, 0.0],
        jacobian_inv: [[0.0; 4]; 4],
        wheels: [[0.0; 4]; MAX_WHEELS],
    };
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

pub struct GpuStoreInit {
    buffer: wgpu::Buffer,
    rounded_max_objects: usize,
}

impl GpuStoreInit {
    pub fn new(
        device: &wgpu::Device,
        settings: &settings::GpuCollision,
    ) -> Self {
        let rounded_max_objects = {
            let tail = settings.max_objects as u32 % WORK_GROUP_WIDTH;
            settings.max_objects + if tail != 0 {
                (WORK_GROUP_WIDTH - tail) as usize
            } else {
                0
            }
        };

        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            size: (rounded_max_objects * mem::size_of::<Data>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsage::STORAGE | wgpu::BufferUsage::STORAGE_READ | wgpu::BufferUsage::COPY_DST,
        });

        GpuStoreInit {
            buffer,
            rounded_max_objects,
        }
    }

    pub fn new_dummy(device: &wgpu::Device) -> Self {
        let buffer = device.create_buffer_with_data(
            [Data::DUMMY].as_bytes(),
            wgpu::BufferUsage::STORAGE_READ,
        );

        GpuStoreInit {
            buffer,
            rounded_max_objects: 1,
        }
    }

    pub fn resource(&self) -> wgpu::BindingResource {
        wgpu::BindingResource::Buffer {
            buffer: &self.buffer,
            range: 0 .. (self.rounded_max_objects * mem::size_of::<Data>()) as wgpu::BufferAddress,
        }
    }
}

enum Pending {
    InitData { index: usize },
    SetControl { index: usize },
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
    pending: Vec<(usize, Pending)>,
    pending_data: Vec<Data>,
    pending_control: Vec<GpuControl>,
}

impl GpuStore {
    pub fn new(
        device: &wgpu::Device,
        common: &Common,
        init: GpuStoreInit,
        collider_buffer: wgpu::BindingResource,
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
        let desc_uniforms = wgpu::BufferDescriptor {
            size: mem::size_of::<Uniforms>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsage::UNIFORM | wgpu::BufferUsage::COPY_DST,
        };
        let buf_uniforms = device.create_buffer(&desc_uniforms);
        let desc_ranges = wgpu::BufferDescriptor {
            size: (init.rounded_max_objects * mem::size_of::<GpuRange>()) as wgpu::BufferAddress,
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
                    resource: init.resource(),
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
                    resource: collider_buffer,
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
            buf_data: init.buffer,
            buf_uniforms,
            buf_ranges,
            capacity: init.rounded_max_objects,
            bind_group,
            bind_group_gather,
            free_list: FreeList::new(),
            pending: Vec::new(),
            pending_data: Vec::new(),
            pending_control: Vec::new(),
        }
    }

    pub fn update_control(&mut self, body: &GpuBody, control: GpuControl) {
        self.pending.push((
            body.index(),
            Pending::SetControl { index: self.pending_control.len() },
        ));
        self.pending_control.push(control);
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
        model: &VisualModel,
        car_physics: &CarPhysics,
    ) -> GpuBody {
        let id = self.free_list.alloc();
        assert!(id.index() < self.capacity);

        let matrix = cgmath::Matrix3::from(model.body.physics.jacobi).invert().unwrap();
        let gt = GpuTransform::new(transform);
        let mut wheels = [[0.0; 4]; MAX_WHEELS];
        for (wo, wi) in wheels.iter_mut().zip(model.wheels.iter()) {
            //TODO: take X bounds like the original did?
            wo[0] = wi.pos[0];
            wo[1] = wi.pos[1];
            wo[2] = wi.pos[2];
            if wi.steer != 0 {
                wo[3] = 1.0;
            }
        }
        let data = Data {
            control: [0.0; 4],
            pos_scale: gt.pos_scale,
            orientation: gt.orientation,
            linear: [0.0; 4],
            angular: [0.0; 4],
            collision: [0.0; 4],
            scale_volume_zomc: [
                car_physics.scale_bound,
                model.body.physics.volume,
                car_physics.z_offset_of_mass_center,
                0.0,
            ],
            jacobian_inv: cgmath::Matrix4::from(matrix).into(),
            wheels,
        };

        self.pending.push((
            id.index(),
            Pending::InitData { index: self.pending_data.len() }
        ));
        self.pending_data.push(data);
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
        let buf_init_data = if self.pending_data.is_empty() {
            None
        } else {
            let buf = device.create_buffer_with_data(
                self.pending_data.as_bytes(),
                wgpu::BufferUsage::COPY_SRC,
            );
            self.pending_data.clear();
            Some(buf)
        };
        let buf_set_control = if self.pending_control.is_empty() {
            None
        } else {
            let buf = device.create_buffer_with_data(
                self.pending_control.as_bytes(),
                wgpu::BufferUsage::COPY_SRC,
            );
            self.pending_control.clear();
            Some(buf)
        };

        for (body_id, pending) in self.pending.drain(..) {
            let data_size = mem::size_of::<Data>();
            match pending {
                Pending::InitData { index } => {
                    encoder.copy_buffer_to_buffer(
                        buf_init_data.as_ref().unwrap(),
                        (index * data_size) as wgpu::BufferAddress,
                        &self.buf_data,
                        (body_id * data_size) as wgpu::BufferAddress,
                        data_size as wgpu::BufferAddress,
                    );
                }
                Pending::SetControl { index } => {
                    let size = mem::size_of::<GpuControl>();
                    encoder.copy_buffer_to_buffer(
                        buf_set_control.as_ref().unwrap(),
                        (index * size) as wgpu::BufferAddress,
                        &self.buf_data,
                        (body_id * data_size + 0) as wgpu::BufferAddress,
                        size as wgpu::BufferAddress,
                    );
                }
            }
        }
    }

    pub fn step(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        delta: f32,
        raw_ranges: &[GpuRange],
    ) {
        assert!(self.pending.is_empty());
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
