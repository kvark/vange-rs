use crate::{
    config::{car::CarPhysics, common::Common, settings},
    freelist::{self, FreeList},
    model::VisualModel,
    render::{collision::GpuRange, GpuTransform, Shaders},
    space::Transform,
};

use bytemuck::{Pod, Zeroable};
use cgmath::SquareMatrix as _;
use futures::{executor::LocalSpawner, task::LocalSpawn as _, FutureExt};
use wgpu::util::DeviceExt as _;

use std::{
    mem, slice,
    sync::{Arc, Mutex, MutexGuard},
};

const WORK_GROUP_WIDTH: u32 = 32;
const MAX_WHEELS: usize = 4;

pub type GpuControl = [f32; 4];

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GpuPush {
    dir_id: [f32; 4],
}
unsafe impl Pod for GpuPush {}
unsafe impl Zeroable for GpuPush {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Physics {
    scale: [f32; 4],
    mobility_ship: [f32; 4],
    speed: [f32; 4],
}
unsafe impl Pod for Physics {}
unsafe impl Zeroable for Physics {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Model {
    jacobi0: [f32; 4],
    jacobi1: [f32; 4],
    jacobi2: [f32; 4],
}
unsafe impl Pod for Model {}
unsafe impl Zeroable for Model {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Data {
    control: GpuControl,
    engine: [f32; 4],
    pos_scale: [f32; 4],
    orientation: [f32; 4],
    linear: [f32; 4],
    angular: [f32; 4],
    collision: [f32; 4],
    model: Model,
    physics: Physics,
    wheels: [[f32; 4]; MAX_WHEELS],
}
unsafe impl Pod for Data {}
unsafe impl Zeroable for Data {}

impl Data {
    const DUMMY: Self = Data {
        control: [0.0; 4],
        engine: [0.0; 4],
        pos_scale: [0.0, 0.0, 0.0, 1.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
        linear: [0.0; 4],
        angular: [0.0; 4],
        collision: [0.0; 4],
        model: Model {
            jacobi0: [0.0; 4],
            jacobi1: [0.0; 4],
            jacobi2: [0.0; 4],
        },
        physics: Physics {
            scale: [0.0; 4],
            mobility_ship: [0.0; 4],
            speed: [0.0; 4],
        },
        wheels: [[0.0; 4]; MAX_WHEELS],
    };
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Uniforms {
    delta: [f32; 4],
}
unsafe impl Pod for Uniforms {}
unsafe impl Zeroable for Uniforms {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct DragConstants {
    free: [f32; 2],
    speed: [f32; 2],
    spring: [f32; 2],
    abs_min: [f32; 2],
    abs_stop: [f32; 2],
    coll: [f32; 2],
    other: [f32; 2],
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Constants {
    nature: [f32; 4],
    global_speed: [f32; 4],
    global_mobility: [f32; 4],
    car_rudder: [f32; 4],
    car_traction: [f32; 4],
    impulse_elastic: [f32; 4],
    impulse_factors: [f32; 4],
    impulse: [f32; 4],
    drag: DragConstants,
    contact_elastic: [f32; 4],
    force: [f32; 4],
}
unsafe impl Pod for Constants {}
unsafe impl Zeroable for Constants {}

pub type GpuBody = freelist::Id<Data>;

struct Pipelines {
    step: wgpu::ComputePipeline,
    gather: wgpu::ComputePipeline,
    push: wgpu::ComputePipeline,
}

impl Pipelines {
    fn new(
        layout_step: &wgpu::PipelineLayout,
        layout_gather: &wgpu::PipelineLayout,
        layout_push: &wgpu::PipelineLayout,
        device: &wgpu::Device,
    ) -> Self {
        Pipelines {
            step: device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("body-step"),
                layout: Some(layout_step),
                module: &Shaders::new_compute(
                    "physics/body_step",
                    [WORK_GROUP_WIDTH, 1, 1],
                    &[],
                    device,
                )
                .unwrap(),
                entry_point: "main",
            }),
            gather: device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("body-gather"),
                layout: Some(layout_gather),
                module: &Shaders::new_compute(
                    "physics/body_gather",
                    [WORK_GROUP_WIDTH, 1, 1],
                    &[],
                    device,
                )
                .unwrap(),
                entry_point: "main",
            }),
            push: device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("body-push"),
                layout: Some(layout_push),
                module: &Shaders::new_compute(
                    "physics/body_push",
                    [WORK_GROUP_WIDTH, 1, 1],
                    &[],
                    device,
                )
                .unwrap(),
                entry_point: "main",
            }),
        }
    }
}

pub struct GpuStoreInit {
    buffer: wgpu::Buffer,
    capacity: usize,
    rounded_max_objects: usize,
}

impl GpuStoreInit {
    pub fn new(device: &wgpu::Device, settings: &settings::GpuCollision) -> Self {
        let rounded_max_objects = {
            let tail = settings.max_objects as u32 % WORK_GROUP_WIDTH;
            settings.max_objects
                + if tail != 0 {
                    (WORK_GROUP_WIDTH - tail) as usize
                } else {
                    0
                }
        };

        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GpuStore"),
            size: (rounded_max_objects * mem::size_of::<Data>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        GpuStoreInit {
            buffer,
            capacity: settings.max_objects,
            rounded_max_objects,
        }
    }

    pub fn new_dummy(device: &wgpu::Device) -> Self {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("dummy"),
            contents: bytemuck::bytes_of(&Data::DUMMY),
            usage: wgpu::BufferUsages::STORAGE,
        });

        GpuStoreInit {
            buffer,
            capacity: 0,
            rounded_max_objects: 1,
        }
    }

    pub fn resource(&self) -> wgpu::BindingResource {
        self.buffer.as_entire_binding()
    }
}

enum Update {
    InitData { index: usize },
    SetControl { index: usize },
}

struct GpuResult {
    buffer: wgpu::Buffer,
    count: usize,
}

pub struct GpuStoreMirror {
    transforms: Vec<Transform>,
}

impl GpuStoreMirror {
    pub fn get(&self, body: &GpuBody) -> Option<&Transform> {
        self.transforms.get(body.index())
    }
}

pub struct GpuStore {
    pipeline_layout_step: wgpu::PipelineLayout,
    pipeline_layout_gather: wgpu::PipelineLayout,
    pipeline_layout_push: wgpu::PipelineLayout,
    pipelines: Pipelines,
    buf_data: wgpu::Buffer,
    buf_uniforms: wgpu::Buffer,
    buf_ranges: wgpu::Buffer,
    buf_pushes: wgpu::Buffer,
    capacity: usize,
    bind_group: wgpu::BindGroup,
    bind_group_gather: wgpu::BindGroup,
    bind_group_push: wgpu::BindGroup,
    free_list: FreeList<Data>,
    updates: Vec<(usize, Update)>,
    update_data: Vec<Data>,
    update_control: Vec<GpuControl>,
    pending_pushes: Vec<GpuPush>,
    gpu_result: Option<GpuResult>,
    cpu_mirror: Arc<Mutex<GpuStoreMirror>>,
}

impl GpuStore {
    pub fn new(
        device: &wgpu::Device,
        common: &Common,
        init: GpuStoreInit,
        collider_buffer: wgpu::BindingResource,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Body"),
            entries: &[
                // data
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // uniforms
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // constants
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout_step = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("body-step"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let bind_group_layout_gather =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Gather"),
                entries: &[
                    // collisions
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // ranges
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let pipeline_layout_gather =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("body-gather"),
                bind_group_layouts: &[&bind_group_layout, &bind_group_layout_gather],
                push_constant_ranges: &[],
            });

        let bind_group_layout_push =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Push"),
                entries: &[
                    // pushes
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let pipeline_layout_push = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("body-push"),
            bind_group_layouts: &[&bind_group_layout, &bind_group_layout_push],
            push_constant_ranges: &[],
        });

        let pipelines = Pipelines::new(
            &pipeline_layout_step,
            &pipeline_layout_gather,
            &pipeline_layout_push,
            device,
        );
        let desc_uniforms = wgpu::BufferDescriptor {
            label: Some("Uniforms"),
            size: mem::size_of::<Uniforms>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        };
        let buf_uniforms = device.create_buffer(&desc_uniforms);
        let desc_ranges = wgpu::BufferDescriptor {
            label: Some("Ranges"),
            size: (init.rounded_max_objects * mem::size_of::<GpuRange>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        };
        let buf_ranges = device.create_buffer(&desc_ranges);
        let desc_pushes = wgpu::BufferDescriptor {
            label: Some("Pushes"),
            size: (WORK_GROUP_WIDTH as usize * mem::size_of::<GpuPush>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        };
        let buf_pushes = device.create_buffer(&desc_pushes);

        let constants = Constants {
            nature: [
                common.nature.time_delta0,
                common.nature.density,
                common.nature.gravity,
                0.0,
            ],
            global_speed: [
                common.global.speed_factor,
                common.global.water_speed_factor,
                common.global.air_speed_factor,
                common.global.underground_speed_factor,
            ],
            global_mobility: [common.global.mobility_factor, 0.0, 0.0, 0.0],
            car_rudder: [
                common.car.rudder_step,
                common.car.rudder_max,
                common.car.rudder_k_decr,
                0.0,
            ],
            car_traction: [common.car.traction_incr, common.car.traction_decr, 0.0, 0.0],
            impulse_elastic: [
                common.impulse.elastic_restriction,
                common.impulse.elastic_time_scale_factor,
                0.0,
                0.0,
            ],
            impulse_factors: [
                common.impulse.factors[0],
                common.impulse.factors[1],
                0.0,
                0.0,
            ],
            impulse: [
                common.impulse.rolling_scale,
                common.impulse.normal_threshold,
                common.impulse.k_wheel,
                common.impulse.k_friction,
            ],
            drag: DragConstants {
                free: common.drag.free.to_array(),
                speed: common.drag.speed.to_array(),
                spring: common.drag.spring.to_array(),
                abs_min: common.drag.abs_min.to_array(),
                abs_stop: common.drag.abs_stop.to_array(),
                coll: common.drag.coll.to_array(),
                other: [common.drag.wheel_speed, common.drag.z],
                _pad: [0.0; 2],
            },
            contact_elastic: [
                common.contact.k_elastic_wheel,
                common.contact.k_elastic_spring,
                common.contact.k_elastic_xy,
                common.contact.k_elastic_db_coll,
            ],
            force: [common.force.k_distance_to_force, 0.0, 0.0, 0.0],
        };
        let buf_constants = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("body-constants"),
            contents: bytemuck::bytes_of(&constants),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Body"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: init.resource(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buf_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buf_constants.as_entire_binding(),
                },
            ],
        });
        let bind_group_gather = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Gather"),
            layout: &bind_group_layout_gather,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: collider_buffer,
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buf_ranges.as_entire_binding(),
                },
            ],
        });
        let bind_group_push = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Push"),
            layout: &bind_group_layout_push,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buf_pushes.as_entire_binding(),
            }],
        });

        GpuStore {
            pipeline_layout_step,
            pipeline_layout_gather,
            pipeline_layout_push,
            pipelines,
            buf_data: init.buffer,
            buf_uniforms,
            buf_ranges,
            buf_pushes,
            capacity: init.capacity,
            bind_group,
            bind_group_gather,
            bind_group_push,
            free_list: FreeList::new(),
            updates: Vec::new(),
            update_data: Vec::new(),
            update_control: Vec::new(),
            pending_pushes: Vec::with_capacity(WORK_GROUP_WIDTH as usize),
            gpu_result: None,
            cpu_mirror: Arc::new(Mutex::new(GpuStoreMirror {
                transforms: Vec::new(),
            })),
        }
    }

    pub fn update_control(&mut self, body: &GpuBody, control: GpuControl) {
        self.updates.push((
            body.index(),
            Update::SetControl {
                index: self.update_control.len(),
            },
        ));
        self.update_control.push(control);
    }

    pub fn add_push(&mut self, body: &GpuBody, vec: cgmath::Vector3<f32>) {
        self.pending_pushes.push(GpuPush {
            dir_id: [vec.x, vec.y, vec.z, body.index() as f32],
        });
    }

    pub fn reload(&mut self, device: &wgpu::Device) {
        self.pipelines = Pipelines::new(
            &self.pipeline_layout_step,
            &self.pipeline_layout_gather,
            &self.pipeline_layout_push,
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

        let matrix = cgmath::Matrix3::from(model.body.physics.jacobi)
            .invert()
            .unwrap();
        let ji: &[f32; 9] = matrix.as_ref();

        let gt = GpuTransform::new(transform);
        let mut wheels = [[0.0; 4]; MAX_WHEELS];
        for (wo, wi) in wheels.iter_mut().zip(model.wheels.iter()) {
            //TODO: take X bounds like the original did?
            wo[0] = wi.pos[0];
            wo[1] = wi.pos[1];
            wo[2] = wi.pos[2];
            wo[3] = if wi.steer != 0 { 1.0 } else { -1.0 };
        }
        let data = Data {
            control: [0.0, 0.0, 1.0, 0.0],
            engine: [0.0; 4],
            pos_scale: gt.pos_scale,
            orientation: gt.orientation,
            linear: [0.0; 4],
            angular: [0.0; 4],
            collision: [0.0; 4],
            model: Model {
                jacobi0: [ji[0], ji[1], ji[2], model.body.physics.volume],
                jacobi1: [ji[3], ji[4], ji[5], model.body.bbox.radius],
                jacobi2: [ji[6], ji[7], ji[8], 0.0],
            },
            physics: Physics {
                scale: [
                    car_physics.scale_size,
                    car_physics.scale_bound,
                    car_physics.scale_box,
                    car_physics.z_offset_of_mass_center,
                ],
                mobility_ship: [
                    car_physics.mobility_factor,
                    car_physics.k_archimedean,
                    car_physics.k_water_traction,
                    car_physics.k_water_rudder,
                ],
                speed: [
                    car_physics.speed_factor,
                    car_physics.water_speed_factor,
                    car_physics.air_speed_factor,
                    car_physics.underground_speed_factor,
                ],
            },
            wheels,
        };

        self.updates.push((
            id.index(),
            Update::InitData {
                index: self.update_data.len(),
            },
        ));
        self.update_data.push(data);
        id
    }

    pub fn free(&mut self, id: GpuBody) {
        self.free_list.free(id);
    }

    pub fn update_entries(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
        let buf_init_data = if self.update_data.is_empty() {
            None
        } else {
            let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("temp-data"),
                contents: bytemuck::cast_slice(&self.update_data),
                usage: wgpu::BufferUsages::COPY_SRC,
            });
            self.update_data.clear();
            Some(buf)
        };
        let buf_set_control = if self.update_control.is_empty() {
            None
        } else {
            let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("temp-control"),
                contents: bytemuck::cast_slice(&self.update_control),
                usage: wgpu::BufferUsages::COPY_SRC,
            });
            self.update_control.clear();
            Some(buf)
        };

        for (body_id, update) in self.updates.drain(..) {
            let data_size = mem::size_of::<Data>();
            match update {
                Update::InitData { index } => {
                    encoder.copy_buffer_to_buffer(
                        buf_init_data.as_ref().unwrap(),
                        (index * data_size) as wgpu::BufferAddress,
                        &self.buf_data,
                        (body_id * data_size) as wgpu::BufferAddress,
                        data_size as wgpu::BufferAddress,
                    );
                }
                Update::SetControl { index } => {
                    let size = mem::size_of::<GpuControl>();
                    encoder.copy_buffer_to_buffer(
                        buf_set_control.as_ref().unwrap(),
                        (index * size) as wgpu::BufferAddress,
                        &self.buf_data,
                        (body_id * data_size) as wgpu::BufferAddress,
                        size as wgpu::BufferAddress,
                    );
                }
            }
        }
    }

    pub fn step(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        delta: f32,
        raw_ranges: &[GpuRange],
    ) {
        assert!(self.updates.is_empty());
        if !self.pending_pushes.is_empty() {
            if self.pending_pushes.len() < WORK_GROUP_WIDTH as usize {
                self.pending_pushes
                    .resize_with(WORK_GROUP_WIDTH as usize, || GpuPush { dir_id: [-1.0; 4] });
            }
            let temp = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("temp-step"),
                contents: bytemuck::cast_slice(&self.pending_pushes[..WORK_GROUP_WIDTH as usize]),
                usage: wgpu::BufferUsages::COPY_SRC,
            });
            encoder.copy_buffer_to_buffer(
                &temp,
                0,
                &self.buf_pushes,
                0,
                (WORK_GROUP_WIDTH as usize * mem::size_of::<GpuPush>()) as wgpu::BufferAddress,
            );
        }

        let num_groups = {
            let num_objects = self.free_list.length();
            let reminder = num_objects % WORK_GROUP_WIDTH as usize;
            let extra = if reminder != 0 { 1 } else { 0 };
            num_objects as u32 / WORK_GROUP_WIDTH + extra
        };

        // update range buffer
        {
            let sub_range = &raw_ranges[..(num_groups * WORK_GROUP_WIDTH) as usize];
            let temp = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("temp-range"),
                contents: bytemuck::cast_slice(sub_range),
                usage: wgpu::BufferUsages::COPY_SRC,
            });
            encoder.copy_buffer_to_buffer(
                &temp,
                0,
                &self.buf_ranges,
                0,
                (sub_range.len() * mem::size_of::<GpuRange>()) as wgpu::BufferAddress,
            );
        }

        // update global uniforms
        {
            let uniforms = Uniforms {
                delta: [delta, 0.0, 0.0, 0.0],
            };
            let temp = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("temp-uniforms"),
                contents: bytemuck::bytes_of(&uniforms),
                usage: wgpu::BufferUsages::COPY_SRC,
            });
            encoder.copy_buffer_to_buffer(
                &temp,
                0,
                &self.buf_uniforms,
                0,
                mem::size_of::<Uniforms>() as wgpu::BufferAddress,
            );
        }

        // compute all the things
        let do_gather = true;
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("body"),
        });
        pass.set_bind_group(0, &self.bind_group, &[]);
        if do_gather {
            pass.set_pipeline(&self.pipelines.gather);
            pass.set_bind_group(1, &self.bind_group_gather, &[]);
            pass.dispatch(num_groups, 1, 1);
        }
        if !self.pending_pushes.is_empty() {
            pass.set_pipeline(&self.pipelines.push);
            pass.set_bind_group(1, &self.bind_group_push, &[]);
            pass.dispatch(1, 1, 1);
        }
        pass.set_pipeline(&self.pipelines.step);
        pass.dispatch(num_groups, 1, 1);

        // remove the first N pushes
        if !self.pending_pushes.is_empty() {
            self.pending_pushes.drain(..WORK_GROUP_WIDTH as usize);
        }
    }

    pub fn produce_gpu_results(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let count = self.free_list.length();
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Gpu Results"),
            size: (count * mem::size_of::<GpuTransform>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let offset = mem::size_of::<GpuControl>() + mem::size_of::<[f32; 4]>(); // skip control & engine
        for i in 0..count {
            encoder.copy_buffer_to_buffer(
                &self.buf_data,
                (i * mem::size_of::<Data>() + offset) as wgpu::BufferAddress,
                &buffer,
                (i * mem::size_of::<GpuTransform>()) as wgpu::BufferAddress,
                mem::size_of::<GpuTransform>() as wgpu::BufferAddress,
            );
        }

        self.gpu_result = Some(GpuResult { buffer, count })
    }

    pub fn consume_gpu_results(&mut self, spawner: &LocalSpawner) {
        let GpuResult { buffer, count } = match self.gpu_result.take() {
            Some(gr) => gr,
            None => return,
        };

        let latest = Arc::clone(&self.cpu_mirror);
        let end = (count * mem::size_of::<GpuTransform>()) as wgpu::BufferAddress;
        let future = buffer
            .slice(..end)
            .map_async(wgpu::MapMode::Read)
            .map(move |_| {
                let mapping = buffer.slice(..end).get_mapped_range();
                let data = unsafe {
                    slice::from_raw_parts(*mapping.as_ptr() as *const GpuTransform, count)
                };

                let transforms = data.iter().map(|gt| Transform {
                    disp: cgmath::vec3(gt.pos_scale[0], gt.pos_scale[1], gt.pos_scale[2]),
                    rot: cgmath::Quaternion::new(
                        gt.orientation[3],
                        gt.orientation[0],
                        gt.orientation[1],
                        gt.orientation[2],
                    ),
                    scale: gt.pos_scale[3],
                });

                let mut storage = latest.lock().unwrap();
                storage.transforms.clear();
                storage.transforms.extend(transforms);
            });
        spawner.spawn_local_obj(Box::new(future).into()).unwrap();
    }

    pub fn cpu_mirror(&self) -> MutexGuard<GpuStoreMirror> {
        self.cpu_mirror.lock().unwrap()
    }
}
