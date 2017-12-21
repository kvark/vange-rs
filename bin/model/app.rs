use cgmath;
use gfx;
use glutin::WindowEvent as Event;
use vangers::{config, level, model, render, space};

pub struct ResourceView<R: gfx::Resources> {
    model: model::Model<R>,
    transform: space::Transform,
    pso: gfx::PipelineState<R, render::object::Meta>,
    data: render::object::Data<R>,
    cam: space::Camera,
}

impl<R: gfx::Resources> ResourceView<R> {
    pub fn new<F: gfx::Factory<R>>(
        path: &str,
        settings: &config::settings::Settings,
        targets: render::MainTargets<R>,
        factory: &mut F,
    ) -> ResourceView<R> {
        use gfx::traits::FactoryExt;
        use std::io::BufReader;

        let pal_data = level::read_palette(settings.open_palette());

        info!("Loading model {}", path);
        let mut file = BufReader::new(settings.open_relative(path));
        let model = model::load_m3d(&mut file, factory);
        let data = render::object::Data {
            vbuf: model.body.buffer.clone(),
            locals: factory.create_constant_buffer(1),
            ctable: render::Render::create_color_table(factory),
            palette: render::Render::create_palette(&pal_data, factory),
            out_color: targets.color,
            out_depth: targets.depth,
        };

        ResourceView {
            model: model,
            transform: cgmath::Decomposed {
                scale: 1.0,
                disp: cgmath::Vector3::unit_z(),
                rot: cgmath::One::one(),
            },
            pso: render::Render::create_object_pso(factory),
            data: data,
            cam: space::Camera {
                loc: cgmath::vec3(0.0, -200.0, 100.0),
                rot: cgmath::Rotation3::from_axis_angle::<cgmath::Rad<_>>(
                    cgmath::Vector3::unit_x(),
                    cgmath::Angle::turn_div_6(),
                ),
                proj: cgmath::PerspectiveFov {
                    fovy: cgmath::Deg(45.0).into(),
                    aspect: settings.get_screen_aspect(),
                    near: 5.0,
                    far: 400.0,
                },
            },
        }
    }

    fn rotate(
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

    pub fn react<F>(
        &mut self,
        event: Event,
        delta: f32,
        factory: &mut F,
    ) -> bool
    where
        F: gfx::Factory<R>,
    {
        use glutin::{KeyboardInput, VirtualKeyCode as Key};
        use glutin::ElementState::Pressed;

        let angle = cgmath::Rad(delta * 2.0);
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
                Key::A => self.rotate(-angle),
                Key::D => self.rotate(angle),
                Key::L => self.pso = render::Render::create_object_pso(factory),
                _ => (),
            },
            _ => (),
        }
        true
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
            None,
        );
    }
}
