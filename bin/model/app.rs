use cgmath;
use gfx;

use boilerplate::{Application, KeyboardInput};
use vangers::{config, level, model, render, space};

pub struct ResourceView<R: gfx::Resources> {
    model: model::Model<R>,
    transform: space::Transform,
    pso: gfx::PipelineState<R, render::object::Meta>,
    data: render::object::Data<R>,
    cam: space::Camera,
    rotation: cgmath::Rad<f32>,
}

impl<R: gfx::Resources> ResourceView<R> {
    pub fn new<F: gfx::Factory<R>>(
        path: &str,
        settings: &config::settings::Settings,
        targets: render::MainTargets<R>,
        factory: &mut F,
    ) -> Self {
        use gfx::traits::FactoryExt;
        use std::io::BufReader;

        let pal_data = level::read_palette(settings.open_palette());
        let (width, height, _, _) = targets.color.get_dimensions();

        info!("Loading model {}", path);
        let mut file = BufReader::new(settings.open_relative(path));
        let model = model::load_m3d(&mut file, factory);
        let data = render::object::Data {
            vbuf: model.body.buffer.clone(),
            locals: factory.create_constant_buffer(1),
            globals: factory.create_constant_buffer(1),
            ctable: render::Render::create_color_table(factory),
            palette: render::Render::create_palette(&pal_data, factory),
            out_color: targets.color,
            out_depth: targets.depth,
        };

        ResourceView {
            model,
            transform: cgmath::Decomposed {
                scale: 1.0,
                disp: cgmath::Vector3::unit_z(),
                rot: cgmath::One::one(),
            },
            pso: render::Render::create_object_pso(factory),
            data,
            cam: space::Camera {
                loc: cgmath::vec3(0.0, -200.0, 100.0),
                rot: cgmath::Rotation3::from_angle_x::<cgmath::Rad<_>>(
                    cgmath::Angle::turn_div_6(),
                ),
                proj: space::Projection::Perspective(cgmath::PerspectiveFov {
                    fovy: cgmath::Deg(45.0).into(),
                    aspect: width as f32 / height as f32,
                    near: 5.0,
                    far: 400.0,
                }),
            },
            rotation: cgmath::Rad(0.),
        }
    }
}

impl<R: gfx::Resources> Application<R> for ResourceView<R> {
    fn on_resize<F: gfx::Factory<R>>(
        &mut self, targets: render::MainTargets<R>, _factory: &mut F
    ) {
        let (w, h, _, _) = targets.color.get_dimensions();
        self.cam.proj.update(w, h);
        self.data.out_color = targets.color;
        self.data.out_depth = targets.depth;
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
                Key::A => self.rotation = -angle,
                Key::D => self.rotation = angle,
                _ => (),
            }
            KeyboardInput {
                state: ElementState::Released,
                virtual_keycode: Some(key),
                ..
            } => match key {
                Key::A | Key::D => self.rotation = cgmath::Rad(0.0),
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
        use cgmath::Transform;

        if self.rotation != cgmath::Rad(0.) {
            let angle = self.rotation * delta;
            let other = cgmath::Decomposed {
                scale: 1.0,
                rot: cgmath::Rotation3::from_angle_z(angle),
                disp: cgmath::Zero::zero(),
            };
            self.transform = other.concat(&self.transform);
        }
    }

    fn draw<C: gfx::CommandBuffer<R>>(
        &mut self,
        enc: &mut gfx::Encoder<R, C>,
    ) {
        enc.clear(&self.data.out_color, [0.1, 0.2, 0.3, 1.0]);
        enc.clear_depth(&self.data.out_depth, 1.0);

        render::Render::set_globals(enc, &self.cam, &self.data.globals);

        render::Render::draw_model(
            enc,
            &self.model,
            self.transform,
            &self.pso,
            &mut self.data,
            None,
        );
    }

    fn reload_shaders<F: gfx::Factory<R>>(&mut self, factory: &mut F) {
        self.pso = render::Render::create_object_pso(factory);
    }
}
