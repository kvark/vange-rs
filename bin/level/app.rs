use cgmath;
use glutin::WindowEvent as Event;
use gfx;
use vangers::{config, level, render, space};


#[derive(Debug)]
enum Input {
    Hor { dir: f32, alt: bool },
    Ver { dir: f32, alt: bool },
    Dep { dir: f32, alt: bool },
    Empty,
}

pub struct LevelView<R: gfx::Resources> {
    render: render::Render<R>,
    //level: level::Level,
    cam: space::Camera,
    input: Input,
}

impl<R: gfx::Resources> LevelView<R> {
    pub fn new<F: gfx::Factory<R>>(
        settings: &config::settings::Settings,
        out_color: gfx::handle::RenderTargetView<R, render::ColorFormat>,
        out_depth: gfx::handle::DepthStencilView<R, render::DepthFormat>,
        factory: &mut F,
    ) -> Self {
        let level = match settings.get_level() {
            Some(lev_config) => level::load(&lev_config),
            None => level::Level::new_test(),
        };
        let pal_data = level::load_palette(&settings.get_object_palette_path());

        LevelView {
            render: render::init(factory, out_color, out_depth, &level, &pal_data),
            //level: level,
            cam: space::Camera {
                loc: cgmath::vec3(0.0, 0.0, 200.0),
                rot: cgmath::Quaternion::new(1.0, 0.0, 0.0, 0.0),
                proj: cgmath::PerspectiveFov {
                    fovy: cgmath::Deg(45.0).into(),
                    aspect: settings.get_screen_aspect(),
                    near: 10.0,
                    far: 10000.0,
                },
            },
            input: Input::Empty,
        }
    }

    pub fn update(&mut self, delta: f32) {
        use cgmath::{InnerSpace, Rotation3, Zero};

        match self.input {
            Input::Hor { dir, alt: false } if dir != 0.0 => {
                let mut vec = self.cam.rot * cgmath::Vector3::unit_x();
                vec.z = 0.0;
                self.cam.loc += 100.0 * delta * dir * vec.normalize();
            }
            Input::Ver { dir, alt: false } if dir != 0.0 => {
                let mut vec = self.cam.rot * cgmath::Vector3::unit_z();
                vec.z = 0.0;
                if vec == cgmath::Vector3::zero() {
                    vec = self.cam.rot * -cgmath::Vector3::unit_y();
                    vec.z = 0.0;
                }
                self.cam.loc -= 100.0 * delta * dir * vec.normalize();
            }
            Input::Dep { dir, alt: false } if dir != 0.0 => {
                let vec = cgmath::Vector3::unit_z();
                self.cam.loc += 100.0 * delta * dir * vec.normalize();
            }
            Input::Hor { dir, alt: true } if dir != 0.0 => {
                let rot = cgmath::Quaternion::from_angle_z(cgmath::Rad(-1.0 * delta * dir));
                self.cam.rot = rot * self.cam.rot;
            }
            Input::Ver { dir, alt: true } if dir != 0.0 => {
                let rot = cgmath::Quaternion::from_angle_x(cgmath::Rad(1.0 * delta * dir));
                self.cam.rot = self.cam.rot * rot;
            }
            _ => {}
        }
    }

    pub fn react<F>(
        &mut self, event: Event, factory: &mut F
    ) -> bool
    where
        F: gfx::Factory<R>,
    {
        use glutin::{VirtualKeyCode as Key, KeyboardInput, ModifiersState};
        use glutin::ElementState::*;

        let i = &mut self.input;
        match event {
            Event::Closed => return false,
            //Event::Resized(width, height) => self.render.resize(width, height),
            Event::KeyboardInput { input: KeyboardInput {
                state: Pressed,
                virtual_keycode: Some(key),
                modifiers: ModifiersState { alt, .. },
                ..
            }, ..} => match key {
                Key::Escape => return false,
                Key::L => self.render.reload(factory),
                Key::W => *i = Input::Ver { dir: 1.0, alt },
                Key::S => *i = Input::Ver { dir: -1.0, alt },
                Key::A => *i = Input::Hor { dir: -1.0, alt },
                Key::D => *i = Input::Hor { dir: 1.0, alt },
                Key::Z => *i = Input::Dep { dir: -1.0, alt },
                Key::X => *i = Input::Dep { dir: 1.0, alt },
                _ => (),
            },
            Event::KeyboardInput { input: KeyboardInput {
                state: Released,
                virtual_keycode: Some(key),
                ..
            }, ..} => match key {
                Key::W | Key::S |
                Key::A | Key::D |
                Key::Z | Key::X => *i = Input::Empty,
                _ => (),
            },
            /*
            Event::KeyboardInput(_, _, Some(Key::R)) =>
                self.cam.rot = self.cam.rot * cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_x(), angle),
            Event::KeyboardInput(_, _, Some(Key::F)) =>
                self.cam.rot = self.cam.rot * cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_x(), -angle),
            */
            _ => (),
        }
        true
    }

    pub fn draw<C: gfx::CommandBuffer<R>>(&mut self, enc: &mut gfx::Encoder<R, C>) {
        self.render.draw_world(enc, None.into_iter(), &self.cam, false);
    }
}
