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
    vertex ResetVertex {
        linear: [f32; 4] = "a_Linear",
        angular: [f32; 4] = "a_Angular",
        entry: [f32; 4] = "a_Entry",
    }

    pipeline reset {
        instances: gfx::InstanceBuffer<ResetVertex> = (),
        output: gfx::RenderTarget<StoreFormat> = "Target0",
    }

    pipeline pulse {
        instances: gfx::InstanceBuffer<ResetVertex> = (),
        output: gfx::BlendTarget<StoreFormat> = (
            "Target0",
            gfx::state::ColorMask::all(),
            gfx::preset::blend::ADD,
        ),
    }

    constant ForceGlobals {
        force: [f32; 4] = "u_GlobalForce",
    }

    vertex ForceVertex {
        entry_delta_did: [f32; 4] = "a_EntryDeltaDid",
    }

    pipeline force {
        globals: gfx::ConstantBuffer<ForceGlobals> = "c_Globals",
        instances: gfx::InstanceBuffer<ForceVertex> = (),
        collisions: gfx::TextureSampler<CollisionFormatView> = "t_Collisions",
        output: gfx::RenderTarget<StoreFormat> = "Target0",
    }

    vertex StepVertex {
        entry_delta: [f32; 4] = "a_EntryDelta",
    }

    pipeline step {
        instances: gfx::InstanceBuffer<StepVertex> = (),
        velocities: gfx::TextureSampler<StoreFormatView> = "t_Velocities",
        output: gfx::BlendTarget<StoreFormat> = (
            "Target0",
            gfx::state::ColorMask::all(),
            gfx::preset::blend::ADD,
        ),
    }
}


pub struct Entry(usize);

struct Pipelines<R: gfx::Resources> {
    reset: gfx::PipelineState<R, reset::Meta>,
    pulse: gfx::PipelineState<R, pulse::Meta>,
    force: gfx::PipelineState<R, force::Meta>,
    step: gfx::PipelineState<R, step::Meta>,
}

impl<R: gfx::Resources> Pipelines<R> {
    fn load<F: gfx::Factory<R>, I: gfx::pso::PipelineInit>(
        factory: &mut F, name: &str, init: I,
    ) -> gfx::PipelineState<R, I::Meta> {
        let (vs, fs) = read_shaders(name)
            .unwrap();
        let program = factory
            .link_program(&vs, &fs)
            .unwrap();
        factory
            .create_pipeline_from_program(
                &program,
                gfx::Primitive::LineList,
                gfx::state::Rasterizer::new_fill(),
                init,
            )
            .unwrap()
    }

    fn new<F: gfx::Factory<R>>(
        factory: &mut F,
    ) -> Self {
        Pipelines {
            reset: Self::load(factory, "e_reset", reset::new()),
            pulse: Self::load(factory, "e_pulse", pulse::new()),
            force: Self::load(factory, "e_force", force::new()),
            step: Self::load(factory, "e_step", step::new()),
        }
    }
}

pub struct Store<R: gfx::Resources> {
    capacity: usize,
    entries: Vec<bool>,
    removals: Vec<Entry>,
    texture: gfx::handle::Texture<R, StoreFormatSurface>,
    texture_vel: gfx::handle::Texture<R, StoreFormatSurface>,
    rtv: gfx::handle::RenderTargetView<R, StoreFormat>,
    rtv_vel: gfx::handle::RenderTargetView<R, StoreFormat>,
    srv: gfx::handle::ShaderResourceView<R, StoreFormatView>,
    srv_vel: gfx::handle::ShaderResourceView<R, StoreFormatView>,
    sampler: gfx::handle::Sampler<R>,
    pso: Pipelines<R>,
    inst_reset: gfx::handle::Buffer<R, ResetVertex>,
    inst_force: gfx::handle::Buffer<R, ForceVertex>,
    inst_step: gfx::handle::Buffer<R, StepVertex>,
    cb_force_globals: gfx::handle::Buffer<R, ForceGlobals>,
    pending_reset: Vec<ResetVertex>,
    pending_pulse: Vec<ResetVertex>,
    pending_force: Vec<ForceVertex>,
    pending_step: Vec<StepVertex>,
}

impl<R: gfx::Resources> Store<R> {
    pub fn new<F: gfx::Factory<R>>(
        factory: &mut F, capacity: usize
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
        let rtv_vel = factory
            .view_texture_as_render_target(&texture, 0, None)
            .unwrap();

        Store {
            capacity,
            entries: vec![false; capacity],
            removals: Vec::new(),
            texture,
            texture_vel,
            rtv,
            rtv_vel,
            srv,
            srv_vel,
            sampler: factory.create_sampler_linear(),
            pso: Pipelines::new(factory),
            inst_reset: factory
                .create_buffer(
                    capacity as _,
                    gfx::buffer::Role::Vertex,
                    gfx::memory::Usage::Dynamic,
                    gfx::memory::Bind::empty(),
                ).unwrap(),
            inst_force: factory
                .create_buffer(
                    capacity as _,
                    gfx::buffer::Role::Vertex,
                    gfx::memory::Usage::Dynamic,
                    gfx::memory::Bind::empty(),
                ).unwrap(),
            inst_step: factory
                .create_buffer(
                    capacity as _,
                    gfx::buffer::Role::Vertex,
                    gfx::memory::Usage::Dynamic,
                    gfx::memory::Bind::empty(),
                ).unwrap(),
            cb_force_globals: factory.create_constant_buffer(1),
            pending_reset: Vec::new(),
            pending_pulse: Vec::new(),
            pending_force: Vec::new(),
            pending_step: Vec::new(),
        }
    }

    pub fn add(&mut self, transform: &Transform) -> Entry {
        let index = self.entries.iter().position(|e| !*e).unwrap();
        self.entries[index] = true;
        self.entry_reset(&Entry(index), transform);
        Entry(index)
    }

    pub fn remove(&mut self, entry: Entry) {
        self.removals.push(entry);
    }

    fn entry_coord(&self, entry: &Entry) -> f32 {
        (2 * entry.0 + 1) as f32 / self.capacity as f32 - 1.0
    }

    pub fn entry_reset(&mut self, entry: &Entry, t: &Transform) {
        let coord = self.entry_coord(entry);
        self.pending_reset.push(ResetVertex {
            linear: t.disp.extend(t.scale).into(),
            angular: t.rot.into(),
            entry: [coord, 0.0, 0.0, 0.0],
        })
    }

    pub fn entry_pulse(&mut self, entry: &Entry, v: Vector3<f32>, w: Vector3<f32>) {
        let coord = self.entry_coord(entry);
        self.pending_pulse.push(ResetVertex {
            linear: v.extend(0.0).into(),
            angular: w.extend(0.0).into(),
            entry: [coord, 0.0, 0.0, 0.0],
        });
    }

    pub fn entry_force(&mut self, entry: &Entry, time: f32, downsample_id: usize) {
        let coord = self.entry_coord(entry);
        self.pending_force.push(ForceVertex {
            entry_delta_did: [coord, time, downsample_id as f32, 0.0],
        });
    }

    pub fn entry_step(&mut self, entry: &Entry, time: f32) {
        let coord = self.entry_coord(entry);
        self.pending_step.push(StepVertex {
            entry_delta: [coord, time, 0.0, 0.0],
        });
    }

    pub fn update<C: gfx::CommandBuffer<R>>(
        &mut self,
        collision_view: gfx::handle::ShaderResourceView<R, CollisionFormatView>,
        encoder: &mut gfx::Encoder<R, C>,
    ) {
        encoder.update_constant_buffer(&self.cb_force_globals, &ForceGlobals {
            force: [0.0, 0.0, -10.0, 0.0], //TEMP
        });
        let mut slice = gfx::Slice {
            start: 0,
            end: 2,
            base_vertex: 0,
            instances: Some((1, 0)),
            buffer: gfx::IndexBuffer::Auto,
        };

        // apply resets
        slice.instances = Some((self.pending_reset.len() as _, 0));
        encoder
            .update_buffer(&self.inst_reset, &self.pending_reset, 0)
            .unwrap();
        encoder.draw(&slice, &self.pso.reset, &reset::Data {
            instances: self.inst_reset.clone(),
            output: self.rtv.clone(),
        });
        self.pending_reset.clear();

        // apply pulses
        slice.instances = Some((self.pending_pulse.len() as _, 0));
        encoder
            .update_buffer(&self.inst_reset, &self.pending_pulse, 0)
            .unwrap();
        encoder.draw(&slice, &self.pso.pulse, &pulse::Data {
            instances: self.inst_reset.clone(),
            output: self.rtv.clone(),
        });
        self.pending_pulse.clear();

        // integrate forces
        slice.instances = Some((self.pending_force.len() as _, 0));
        encoder
            .update_buffer(&self.inst_force, &self.pending_force, 0)
            .unwrap();
        encoder.draw(&slice, &self.pso.force, &force::Data {
            globals: self.cb_force_globals.clone(),
            instances: self.inst_force.clone(),
            collisions: (collision_view.clone(), self.sampler.clone()),
            output: self.rtv_vel.clone(),
        });
        self.pending_force.clear();

        // perform a physics step
        slice.instances = Some((self.pending_step.len() as _, 0));
        encoder
            .update_buffer(&self.inst_step, &self.pending_step, 0)
            .unwrap();
        encoder.draw(&slice, &self.pso.step, &step::Data {
            instances: self.inst_step.clone(),
            velocities: (self.srv_vel.clone(), self.sampler.clone()),
            output: self.rtv.clone(),
        });
        self.pending_step.clear();

        // cleanup
        for Entry(index) in self.removals.drain(..) {
            self.entries[index] = false;
        }
    }

    pub fn view(&self) -> gfx::handle::ShaderResourceView<R, StoreFormatView> {
        self.srv.clone()
    }

    pub fn reload<F: gfx::Factory<R>>(
        &mut self, factory: &mut F
    ) {
        self.pso = Pipelines::new(factory);
    }
}
