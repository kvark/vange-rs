use cgmath;
use glutin::Event;
use gfx;
use {level, model, render};
use config::{self, Settings};


pub struct CarView<R: gfx::Resources> {
    model: model::Model<R>,
    transform: super::Transform,
    pso: gfx::PipelineState<R, render::object::Meta>,
    data: render::object::Data<R>,
    cam: super::Camera,
}

impl<R: gfx::Resources> CarView<R> {
    pub fn new<F: gfx::Factory<R>>(settings: &Settings,
               out_color: gfx::handle::RenderTargetView<R, render::ColorFormat>,
               out_depth: gfx::handle::DepthStencilView<R, render::DepthFormat>,
               factory: &mut F) -> CarView<R>
    {
        use gfx::traits::FactoryExt;

        info!("Loading car registry");
        let careg = config::game::Registry::load(settings, factory);
        let model = careg.models[&settings.car.id].clone();

        let pal_data = level::load_palette(&settings.get_object_palette_path());
        let data = render::object::Data {
            vbuf: model.body.buffer.clone(),
            locals: factory.create_constant_buffer(1),
            ctable: render::Render::create_color_table(factory),
            palette: render::Render::create_palette(&pal_data, factory),
            out_color: out_color,
            out_depth: out_depth,
        };

        CarView {
            model: model,
            transform: cgmath::Decomposed {
                scale: 1.0,
                disp: cgmath::Vector3::unit_z(),
                rot: cgmath::One::one(),
            },
            pso: render::Render::create_object_pso(factory),
            data: data,
            cam: super::Camera {
                loc: cgmath::vec3(0.0, -60.0, 30.0),
                rot: cgmath::Rotation3::from_axis_angle(cgmath::Vector3::unit_x(), cgmath::Angle::turn_div_6()),
                proj: cgmath::PerspectiveFov {
                    fovy: cgmath::deg(45.0).into(),
                    aspect: settings.get_screen_aspect(),
                    near: 1.0,
                    far: 100.0,
                },
            },
        }
    }

    fn rotate(&mut self, angle: cgmath::Rad<f32>) {
        use cgmath::Transform;
        let other = cgmath::Decomposed {
            scale: 1.0,
            rot: cgmath::Rotation3::from_axis_angle(cgmath::Vector3::unit_z(), angle),
            disp: cgmath::Zero::zero(),
        };
        self.transform = other.concat(&self.transform);
    }
}

impl<R: gfx::Resources> super::App<R> for CarView<R> {
    fn update<I: Iterator<Item=Event>, F: gfx::Factory<R>>(&mut self,
              events: I, delta: f32, factory: &mut F) -> bool {
        use glutin::VirtualKeyCode as Key;
        let angle = cgmath::rad(delta * 2.0);
        for event in events {
            match event {
                Event::KeyboardInput(_, _, Some(Key::Escape)) |
                Event::Closed => return false,
                Event::KeyboardInput(_, _, Some(Key::A)) => self.rotate(-angle),
                Event::KeyboardInput(_, _, Some(Key::D)) => self.rotate(angle),
                Event::KeyboardInput(_, _, Some(Key::L)) =>
                    self.pso = render::Render::create_object_pso(factory),
                _ => {}, //TODO
            }
        }
        true
    }

    fn draw<C: gfx::CommandBuffer<R>>(&mut self, enc: &mut gfx::Encoder<R, C>) {
        enc.clear(&self.data.out_color, [0.1, 0.2, 0.3, 1.0]);
        enc.clear_depth(&self.data.out_depth, 1.0);

        let local: cgmath::Matrix4<f32> = self.transform.into();
        let mvp = self.cam.get_view_proj() * local;
        render::Render::draw_model(enc, &self.model, mvp, &self.pso, &mut self.data);
    }
}
