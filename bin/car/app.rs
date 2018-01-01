use cgmath;
use gfx;

use boilerplate::{Application, KeyboardInput};
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
        targets: render::MainTargets<R>,
        factory: &mut F,
    ) -> Self {
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
        let (width, height, _, _) = targets.color.get_dimensions();
        let data = render::object::Data {
            vbuf: model.body.buffer.clone(),
            globals: factory.create_constant_buffer(1),
            locals: factory.create_constant_buffer(1),
            ctable: render::Render::create_color_table(factory),
            palette: render::Render::create_palette(&pal_data, factory),
            out_color: targets.color.clone(),
            out_depth: targets.depth.clone(),
        };

        CarView {
            model,
            transform: cgmath::Decomposed {
                scale: cinfo.scale,
                disp: cgmath::Vector3::unit_z(),
                rot: cgmath::One::one(),
            },
            pso: render::Render::create_object_pso(factory),
            debug_render: render::DebugRender::new(factory, targets, &settings.render.debug),
            physics: cinfo.physics.clone(),
            data,
            cam: space::Camera {
                loc: cgmath::vec3(0.0, -64.0, 32.0),
                rot: cgmath::Rotation3::from_angle_x::<cgmath::Rad<_>>(
                    cgmath::Angle::turn_div_6(),
                ),
                proj: space::Projection::Perspective(cgmath::PerspectiveFov {
                    fovy: cgmath::Deg(45.0).into(),
                    aspect: width as f32 / height as f32,
                    near: 1.0,
                    far: 100.0,
                }),
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
            rot: cgmath::Rotation3::from_angle_z(angle),
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
            rot: cgmath::Rotation3::from_angle_x(angle),
            disp: cgmath::Zero::zero(),
        };
        self.transform = self.transform.concat(&other);
    }
}

impl<R: gfx::Resources> Application<R> for CarView<R> {
    fn on_resize<F: gfx::Factory<R>>(
        &mut self, targets: render::MainTargets<R>, _factory: &mut F
    ) {
        let (w, h, _, _) = targets.color.get_dimensions();
        self.cam.proj.update(w, h);
        self.data.out_color = targets.color.clone();
        self.data.out_depth = targets.depth.clone();
        self.debug_render.resize(targets);
    }

    fn on_key(&mut self, input: KeyboardInput) -> bool {
        use boilerplate::{ElementState, Key};

        let angle = cgmath::Rad(2.0);
        match input {
            KeyboardInput {
                state: ElementState::Pressed,
                virtual_keycode: Some(key),
                ..
            } => match key {
                Key::Escape => return false,
                Key::A => self.rotation.0 = -angle,
                Key::D => self.rotation.0 = angle,
                Key::W => self.rotation.1 = -angle,
                Key::S => self.rotation.1 = angle,
                _ => (),
            }
            KeyboardInput {
                state: ElementState::Released,
                virtual_keycode: Some(key),
                ..
            } => match key {
                Key::A | Key::D => self.rotation.0 = cgmath::Rad(0.),
                Key::W | Key::S => self.rotation.1 = cgmath::Rad(0.),
                _ => (),
            }
            _ => {}
        }

        true
    }

    fn update(
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

    fn draw<C: gfx::CommandBuffer<R>>(
        &mut self,
        enc: &mut gfx::Encoder<R, C>,
    ) {
        enc.clear(&self.data.out_color, [0.1, 0.2, 0.3, 1.0]);
        enc.clear_depth(&self.data.out_depth, 1.0);

        let mx_vp = render::Render::set_globals(enc, &self.cam, &self.data.globals);

        render::Render::draw_model(
            enc,
            &self.model,
            self.transform,
            &self.pso,
            &mut self.data,
            Some((&mut self.debug_render, self.physics.scale_bound, &mx_vp)),
        );
    }

    fn reload_shaders<F: gfx::Factory<R>>(&mut self, factory: &mut F) {
        self.pso = render::Render::create_object_pso(factory);
    }
}
