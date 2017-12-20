use cgmath;
use gfx;
use glutin::WindowEvent as Event;
use vangers::{config, level, model, render, space};

pub struct CarView<R: gfx::Resources> {
    model: model::Model<R>,
    transform: space::Transform,
    pso: gfx::PipelineState<R, render::object::Meta>,
    debug_render: render::DebugRender<R>,
    physics: config::car::CarPhysics,
    data: render::object::Data<R>,
    cam: space::Camera,
    rotation: (cgmath::Rad<f32>, cgmath::Rad<f32>),
}

impl<R: gfx::Resources> CarView<R> {
    pub fn new<F: gfx::Factory<R>>(
        settings: &config::Settings,
        out_color: gfx::handle::RenderTargetView<R, render::ColorFormat>,
        out_depth: gfx::handle::DepthStencilView<R, render::DepthFormat>,
        factory: &mut F,
    ) -> CarView<R> {
        use gfx::traits::FactoryExt;

        info!("Loading car registry");
        let game_reg = config::game::Registry::load(settings);
        let car_reg = config::car::load_registry(settings, &game_reg, factory);
        let cinfo = &car_reg[&settings.car.id];
        let mut model = cinfo.model.clone();
        for (ms, sid) in model.slots.iter_mut().zip(settings.car.slots.iter()) {
            let info = &game_reg.model_infos[sid];
            let mut file = settings.open_relative(&info.path);
            ms.mesh = Some(model::load_c3d(&mut file, factory));
            ms.scale = info.scale;
        }

        let pal_data = level::read_palette(settings.open_palette());
        let data = render::object::Data {
            vbuf: model.body.buffer.clone(),
            locals: factory.create_constant_buffer(1),
            ctable: render::Render::create_color_table(factory),
            palette: render::Render::create_palette(&pal_data, factory),
            out_color: out_color.clone(),
            out_depth: out_depth.clone(),
        };

        CarView {
            model: model,
            transform: cgmath::Decomposed {
                scale: cinfo.scale,
                disp: cgmath::Vector3::unit_z(),
                rot: cgmath::One::one(),
            },
            pso: render::Render::create_object_pso(factory),
            debug_render: render::DebugRender::new(factory, 512, out_color, out_depth),
            physics: cinfo.physics.clone(),
            data: data,
            cam: space::Camera {
                loc: cgmath::vec3(0.0, -64.0, 32.0),
                rot: cgmath::Rotation3::from_axis_angle::<cgmath::Rad<_>>(
                    cgmath::Vector3::unit_x(),
                    cgmath::Angle::turn_div_6(),
                ),
                proj: cgmath::PerspectiveFov {
                    fovy: cgmath::Deg(45.0).into(),
                    aspect: settings.get_screen_aspect(),
                    near: 1.0,
                    far: 100.0,
                },
            },
            rotation: (cgmath::Rad(0.), cgmath::Rad(0.)),
        }
    }

    fn rotate_z(
        &mut self,
        angle: cgmath::Rad<f32>,
    ) {
        use cgmath::Transform;
        let other = cgmath::Decomposed {
            scale: 1.0,
            rot: cgmath::Rotation3::from_axis_angle(cgmath::Vector3::unit_z(), angle),
            disp: cgmath::Zero::zero(),
        };
        self.transform = other.concat(&self.transform);
    }

    fn rotate_x(
        &mut self,
        angle: cgmath::Rad<f32>,
    ) {
        use cgmath::Transform;
        let other = cgmath::Decomposed {
            scale: 1.0,
            rot: cgmath::Rotation3::from_axis_angle(cgmath::Vector3::unit_x(), angle),
            disp: cgmath::Zero::zero(),
        };
        self.transform = self.transform.concat(&other);
    }

    pub fn react<F>(
        &mut self,
        event: Event,
        factory: &mut F,
    ) -> bool
    where
        F: gfx::Factory<R>,
    {
        use glutin::{KeyboardInput, VirtualKeyCode as Key};
        use glutin::ElementState::*;
        let angle = cgmath::Rad(2.0);
        match event {
            Event::Closed => return false,
            Event::KeyboardInput {
                input:
                    KeyboardInput {
                        state: Pressed,
                        virtual_keycode: Some(key),
                        ..
                    },
                ..
            } => match key {
                Key::Escape => return false,
                Key::A => self.rotation.0 = -angle,
                Key::D => self.rotation.0 = angle,
                Key::W => self.rotation.1 = -angle,
                Key::S => self.rotation.1 = angle,
                Key::L => self.pso = render::Render::create_object_pso(factory),
                _ => (),
            },
            Event::KeyboardInput {
                input:
                    KeyboardInput {
                        state: Released,
                        virtual_keycode: Some(key),
                        ..
                    },
                ..
            } => match key {
                Key::A | Key::D => self.rotation.0 = cgmath::Rad(0.),
                Key::W | Key::S => self.rotation.1 = cgmath::Rad(0.),
                _ => (),
            },
            _ => (),
        }
        true
    }

    pub fn update(
        &mut self,
        delta: f32,
    ) {
        if self.rotation.0 != cgmath::Rad(0.) {
            let rot = self.rotation.0 * delta;
            self.rotate_z(rot);
        }
        if self.rotation.1 != cgmath::Rad(0.) {
            let rot = self.rotation.1 * delta;
            self.rotate_x(rot);
        }
    }

    pub fn draw<C: gfx::CommandBuffer<R>>(
        &mut self,
        enc: &mut gfx::Encoder<R, C>,
    ) {
        enc.clear(&self.data.out_color, [0.1, 0.2, 0.3, 1.0]);
        enc.clear_depth(&self.data.out_depth, 1.0);

        render::Render::draw_model(
            enc,
            &self.model,
            self.transform,
            &self.cam,
            &self.pso,
            &mut self.data,
            Some((&mut self.debug_render, self.physics.scale_bound)),
        );
    }
}
