//! store GPU texture layout
//! an object occupies a row
//! Data as vec4s:
//!  0: (position, scale)
//!  1: quaternion
//!  2: linear velocity
//!  3: angular velocity

use cgmath::{Vector3};
use gfx;
use gfx::traits::FactoryExt;

use render::{read_shaders};
use render::collision::{CollisionFormatView};
use space::Transform;


pub type StoreFormat = gfx::format::Rgba32F;
pub type StoreFormatSurface = <StoreFormat as gfx::format::Formatted>::Surface;
pub type StoreFormatView = <StoreFormat as gfx::format::Formatted>::View;

gfx_defines!{
    vertex PulseVertex {
        linear: [f32; 4] = "a_Linear",
        angular: [f32; 4] = "a_Angular",
        entry: [f32; 4] = "a_Entry",
    }

    pipeline pulse {
        instances: gfx::InstanceBuffer<PulseVertex> = (),
        output: gfx::BlendTarget<StoreFormat> = (
            "Target0",
            gfx::state::ColorMask::all(),
            gfx::preset::blend::ADD,
        ),
    }

    constant ForceGlobals {
        force: [f32; 4] = "u_GlobalForce",
    }

    constant ForceLocals {
        time: [f32; 4] = "u_Time",
    }

    pipeline force {
        globals: gfx::ConstantBuffer<ForceGlobals> = "c_Globals",
        locals: gfx::ConstantBuffer<ForceLocals> = "c_Locals",
        collisions: gfx::TextureSampler<CollisionFormatView> = "t_Collisions",
        velocities: gfx::TextureSampler<StoreFormatView> = "t_Velocities",
        output: gfx::RenderTarget<StoreFormat> = "Target0",
    }

    vertex StepVertex {
        entry_delta: [f32; 4] = "a_EntryDelta",
    }

    pipeline step {
        instances: gfx::InstanceBuffer<StepVertex> = (),
        output: gfx::BlendTarget<StoreFormat> = (
            "Target0",
            gfx::state::ColorMask::all(),
            gfx::preset::blend::ADD,
        ),
    }
}


pub struct Entry(usize);

pub struct Store<R: gfx::Resources> {
    capacity: usize,
    entries: Vec<bool>,
    actions: Vec<(usize, Action)>,
    texture: gfx::handle::Texture<R, StoreFormatSurface>,
    texture_vel: gfx::handle::Texture<R, StoreFormatSurface>,
    rtv: gfx::handle::RenderTargetView<R, StoreFormat>,
    srv: gfx::handle::ShaderResourceView<R, StoreFormatView>,
    srv_vel: gfx::handle::ShaderResourceView<R, StoreFormatView>,
    sampler: gfx::handle::Sampler<R>,
    pso_pulse: gfx::PipelineState<R, pulse::Meta>,
    pso_force: gfx::PipelineState<R, force::Meta>,
    pso_step: gfx::PipelineState<R, step::Meta>,
    inst_pulse: gfx::handle::Buffer<R, PulseVertex>,
    inst_step: gfx::handle::Buffer<R, StepVertex>,
    cb_force_globals: gfx::handle::Buffer<R, ForceGlobals>,
    cb_force_locals: gfx::handle::Buffer<R, ForceLocals>,
}

enum Action {
    Init(Transform),
    Pulse { v: Vector3<f32>, w: Vector3<f32> },
    Force { time: f32 },
    Step { time: f32 },
}

impl<R: gfx::Resources> Store<R> {
    fn create_psos<F: gfx::Factory<R>>(
        factory: &mut F,
    ) -> (
        gfx::PipelineState<R, pulse::Meta>,
        gfx::PipelineState<R, force::Meta>,
        gfx::PipelineState<R, step::Meta>,
    ) {
        let pso_pulse = {
            let (vs, fs) = read_shaders("pulse")
                .unwrap();
            let program = factory
                .link_program(&vs, &fs)
                .unwrap();
            factory
                .create_pipeline_from_program(
                    &program,
                    gfx::Primitive::LineList,
                    gfx::state::Rasterizer::new_fill(),
                    pulse::new(),
                )
                .unwrap()
        };
        let pso_force = {
            let (vs, fs) = read_shaders("force")
                .unwrap();
            let program = factory
                .link_program(&vs, &fs)
                .unwrap();
            factory
                .create_pipeline_from_program(
                    &program,
                    gfx::Primitive::LineList,
                    gfx::state::Rasterizer::new_fill(),
                    force::new(),
                )
                .unwrap()
        };
        let pso_step = {
            let (vs, fs) = read_shaders("step")
                .unwrap();
            let program = factory
                .link_program(&vs, &fs)
                .unwrap();
            factory
                .create_pipeline_from_program(
                    &program,
                    gfx::Primitive::LineList,
                    gfx::state::Rasterizer::new_fill(),
                    step::new(),
                )
                .unwrap()
        };

        (pso_pulse, pso_force, pso_step)
    }

    pub fn new<F: gfx::Factory<R>>(
        capacity: usize, factory: &mut F
    ) -> Self {
        use gfx::texture as t;
        use gfx::format::{ChannelTyped, Formatted, Swizzle};
        use gfx::memory::{Bind, Usage};

        let cty = <<StoreFormat as Formatted>::Channel as ChannelTyped>::get_channel_type();

        let texture = {
            let kind = t::Kind::D2(4, capacity as _, t::AaMode::Single);
            let bind = Bind::SHADER_RESOURCE | Bind::RENDER_TARGET | Bind::TRANSFER_SRC;
            factory
                .create_texture(kind, 1, bind, Usage::Data, Some(cty))
                .unwrap()
        };
        let srv = factory
            .view_texture_as_shader_resource::<StoreFormat>(&texture, (0, 0), Swizzle::new())
            .unwrap();
        let rtv = factory
            .view_texture_as_render_target(&texture, 0, None)
            .unwrap();

        let texture_vel = {
            let kind = t::Kind::D2(2, capacity as _, t::AaMode::Single);
            let bind = Bind::SHADER_RESOURCE | Bind::TRANSFER_DST;
            factory
                .create_texture(kind, 1, bind, Usage::Data, Some(cty))
                .unwrap()
        };
        let srv_vel = factory
            .view_texture_as_shader_resource::<StoreFormat>(&texture_vel, (0, 0), Swizzle::new())
            .unwrap();

        let (pso_pulse, pso_force, pso_step) = Self::create_psos(factory);

        Store {
            capacity,
            entries: vec![false; capacity],
            actions: Vec::new(),
            texture,
            texture_vel,
            rtv,
            srv,
            srv_vel,
            sampler: factory.create_sampler_linear(),
            pso_pulse,
            pso_force,
            pso_step,
            inst_pulse: factory
                .create_buffer(
                    10, //TODO
                    gfx::buffer::Role::Vertex,
                    gfx::memory::Usage::Dynamic,
                    gfx::memory::Bind::empty(),
                ).unwrap(),
            inst_step: factory
                .create_buffer(
                    10, //TODO
                    gfx::buffer::Role::Vertex,
                    gfx::memory::Usage::Dynamic,
                    gfx::memory::Bind::empty(),
                ).unwrap(),
            cb_force_globals: factory.create_constant_buffer(1),
            cb_force_locals: factory.create_constant_buffer(1),
        }
    }

    pub fn add(&mut self, transform: Transform) -> Entry {
        let index = self.entries.iter().position(|e| !*e).unwrap();
        self.actions.push((index, Action::Init(transform)));
        Entry(index)
    }

    pub fn entry_pulse(&mut self, entry: &Entry, v: Vector3<f32>, w: Vector3<f32>) {
        self.actions.push((entry.0, Action::Pulse { v, w }));
    }

    pub fn entry_step(&mut self, entry: &Entry, time: f32) {
        self.actions.push((entry.0, Action::Force { time }));
        self.actions.push((entry.0, Action::Step { time }));
    }

    pub fn update<C: gfx::CommandBuffer<R>>(
        &mut self,
        collision_view: gfx::handle::ShaderResourceView<R, CollisionFormatView>,
        encoder: &mut gfx::Encoder<R, C>,
    ) {
        encoder.update_constant_buffer(&self.cb_force_globals, &ForceGlobals {
            force: [0.0, 0.0, -10.0, 0.0], //TEMP
        });
        let slice = gfx::Slice {
            start: 0,
            end: 2,
            base_vertex: 0,
            instances: Some((1, 0)),
            buffer: gfx::IndexBuffer::Auto,
        };

        // TODO:
        // 1. flat out actions per type
        // 2. execute each group at once, instanced
        // 3. copy velocities in one go

        for (entry_id, action) in self.actions.drain(..) {
            let coord = (2 * entry_id + 1) as f32 / self.capacity as f32 - 1.0;
            match action {
                Action::Init(transform) => {
                    encoder.update_texture::<StoreFormatSurface, StoreFormat>(
                        &self.texture,
                        None,
                        gfx::texture::ImageInfoCommon {
                            xoffset: 0,
                            yoffset: entry_id as _,
                            zoffset: 0,
                            width: 4,
                            height: 1,
                            depth: 1,
                            format: (),
                            mipmap: 0,
                        },
                        gfx::memory::cast_slice(&[
                            [transform.disp.x, transform.disp.y, transform.disp.z, transform.scale],
                            transform.rot.into(),
                            [0.0; 4],
                            [0.0; 4],
                        ]),
                    ).unwrap();
                }
                Action::Pulse { v, w } => {
                    let data = pulse::Data {
                        instances: self.inst_pulse.clone(),
                        output: self.rtv.clone(),
                    };
                    let instance = PulseVertex {
                        linear: v.extend(0.0).into(),
                        angular: w.extend(0.0).into(),
                        entry: [coord, 0.0, 0.0, 0.0],
                    };
                    encoder
                        .update_buffer(&self.inst_pulse, &[instance], 0)
                        .unwrap();
                    encoder.draw(&slice, &self.pso_pulse, &data);
                }
                Action::Force { time } => {
                    use gfx::memory::Typed;
                    let format = <StoreFormat as gfx::format::Formatted>::get_format();
                    // backup the velocities
                    encoder.copy_texture_to_texture_raw(
                        self.texture.raw(),
                        None,
                        gfx::texture::ImageInfoCommon {
                            xoffset: 2,
                            yoffset: entry_id as _,
                            zoffset: 0,
                            width: 2,
                            height: 1,
                            depth: 1,
                            format,
                            mipmap: 0,
                        },
                        self.texture_vel.raw(),
                        None,
                        gfx::texture::ImageInfoCommon {
                            xoffset: 0,
                            yoffset: entry_id as _,
                            zoffset: 0,
                            width: 2,
                            height: 1,
                            depth: 1,
                            format,
                            mipmap: 0,
                        },
                    ).unwrap();
                    // integrate forces
                    encoder.update_constant_buffer(&self.cb_force_locals, &ForceLocals {
                        time: [time, 0.0, 0.0, 0.0],
                    });
                    let data = force::Data {
                        globals: self.cb_force_globals.clone(),
                        locals: self.cb_force_locals.clone(),
                        collisions: (collision_view.clone(), self.sampler.clone()),
                        velocities: (self.srv_vel.clone(), self.sampler.clone()),
                        output: self.rtv.clone(),
                    };
                    encoder.draw(&slice, &self.pso_force, &data);
                }
                Action::Step { time } => {
                    let data = step::Data {
                        instances: self.inst_step.clone(),
                        output: self.rtv.clone(),
                    };
                    let instance = StepVertex {
                        entry_delta: [coord, time, 0.0, 0.0],
                    };
                    encoder
                        .update_buffer(&self.inst_step, &[instance], 0)
                        .unwrap();
                    encoder.draw(&slice, &self.pso_step, &data);
                }
            }
        }
    }

    pub fn view(&self) -> gfx::handle::ShaderResourceView<R, StoreFormatView> {
        self.srv.clone()
    }

    pub fn reload<F>(
        &mut self, factory: &mut F
    ) where
        F: gfx::Factory<R>
    {
        let (pso_pulse, pso_force, pso_step) = Self::create_psos(factory);
        self.pso_pulse = pso_pulse;
        self.pso_force = pso_force;
        self.pso_step = pso_step;
    }
}
