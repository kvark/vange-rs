use cgmath;
use gfx;

use boilerplate::{Application, KeyboardInput, MouseScrollDelta};
use vangers::{config, level, render, space};

#[derive(Debug)]
enum Input {
    Hor { dir: f32, alt: bool },
    Ver { dir: f32, alt: bool },
    Dep { dir: f32, alt: bool },
    DepQuant {dir: f32},
    PlaneQuant {x: f32, y: f32},
    RotQuant {x: f32, y: f32},
    Empty,
}

pub struct LevelView<R: gfx::Resources> {
    render: render::Render<R>,
    _level: level::Level,
    cam: space::Camera,
    input: Input,
}

impl<R: gfx::Resources> LevelView<R> {
    pub fn new<F: gfx::Factory<R>>(
        settings: &config::settings::Settings,
        targets: render::MainTargets<R>,
        factory: &mut F,
    ) -> Self {
        let (level, pal_file) = if settings.game.level.is_empty() {
            info!("Using test level");
            (level::Level::new_test(), settings.open_palette())
        } else {
            let escaves = config::escaves::load(settings.open_relative("escaves.prm"));

            let pal_file = if settings.game.cycle.is_empty() {
                info!("Using default palette");
                settings.open_palette()
            } else {
                let escave = escaves
                    .iter()
                    .find(|e| e.world == settings.game.level)
                    .expect("Unable to find the world");
                let bunch = {
                    let file = settings.open_relative("bunches.prm");
                    let mut bunches = config::bunches::load(file);
                    let index = bunches
                        .iter()
                        .position(|b| b.escave == escave.name)
                        .expect("Unable to find the bunch");
                    bunches.swap_remove(index)
                };
                let cycle = bunch.cycles
                    .iter()
                    .find(|c| c.name == settings.game.cycle)
                    .expect("Unknown cycle is provided");
                info!("Using palette {}", cycle.palette_path);
                settings.open_relative(&cycle.palette_path)
            };

            let worlds = config::worlds::load(settings.open_relative("wrlds.dat"));
            let ini_name = &worlds[&settings.game.level];
            let ini_path = settings.data_path.join(ini_name);
            info!("Using level {}", ini_name);
            let level_config = level::LevelConfig::load(&ini_path);

            (level::load(&level_config), pal_file)
        };

        let pal_data = level::read_palette(pal_file);
        let aspect = targets.get_aspect();
        let render = render::init(factory, targets, &level, &pal_data, &settings.render);

        LevelView {
            render,
            _level: level,
            cam: space::Camera {
                loc: cgmath::vec3(0.0, 0.0, 400.0),
                rot: cgmath::Quaternion::new(1.0, 0.0, 0.0, 0.0),
                proj: cgmath::PerspectiveFov {
                    fovy: cgmath::Deg(45.0).into(),
                    aspect,
                    near: 10.0,
                    far: 10000.0,
                },
            },
            input: Input::Empty,
        }
    }
}

impl<R: gfx::Resources> Application<R> for LevelView<R> {
    fn on_resize<F: gfx::Factory<R>>(
        &mut self, targets: render::MainTargets<R>, _factory: &mut F
    ) {
        self.cam.proj.aspect = targets.get_aspect();
        self.render.resize(targets);
    }
    fn on_mouse_move(&mut self, delta_x: f32, delta_y: f32, alt: bool){
        if !alt {
            self.input = Input::PlaneQuant {x: delta_x, y: delta_y};
        }else {
            self.input = Input::RotQuant {x: delta_x, y: delta_y};
        }

    }

    fn on_mouse_wheel(&mut self, delta: MouseScrollDelta) {
        match delta {
            MouseScrollDelta::LineDelta(x, y) => {
                self.input =  Input::DepQuant { dir: y};
            }
            _ => {}
        }

    }

    fn on_key(&mut self, input: KeyboardInput) -> bool {
        use boilerplate::{ElementState, Key, ModifiersState};

        let i = &mut self.input;
        match input {
            KeyboardInput {
                state: ElementState::Pressed,
                virtual_keycode: Some(key),
                modifiers: ModifiersState { alt, .. },
                ..
            } => match key {
                Key::Escape => return false,
                Key::W => *i = Input::Ver { dir: 1.0, alt },
                Key::S => *i = Input::Ver { dir: -1.0, alt },
                Key::A => *i = Input::Hor { dir: -1.0, alt },
                Key::D => *i = Input::Hor { dir: 1.0, alt },
                Key::Z => *i = Input::Dep { dir: -1.0, alt },
                Key::X => *i = Input::Dep { dir: 1.0, alt },
                _ => (),
            }
            KeyboardInput {
                state: ElementState::Released,
                virtual_keycode: Some(key),
                ..
            } => match key {
                Key::W | Key::S | Key::A | Key::D | Key::Z | Key::X => *i = Input::Empty,
                _ => (),
            }
            /*
            Event::KeyboardInput(_, _, Some(Key::R)) =>
                self.cam.rot = self.cam.rot * cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_x(), angle),
            Event::KeyboardInput(_, _, Some(Key::F)) =>
                self.cam.rot = self.cam.rot * cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_x(), -angle),
            */
            _ => {}
        }

        true
    }

    fn update(&mut self, delta: f32) {
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
            Input::DepQuant {dir} => {
                let vec = cgmath::Vector3::unit_z();
                self.cam.loc += 1000.0 * delta * dir * vec.normalize();
                self.input = Input::Empty;
            }
            Input::PlaneQuant {x, y} => {
                let mut vec_x = self.cam.rot * cgmath::Vector3::unit_x() * -1.0 * x;
                let mut vec_y = self.cam.rot * cgmath::Vector3::unit_y() * y;

                let mut vec = vec_x + vec_y;

                let norm1 = vec.magnitude();
                let x1 = vec.x;
                let y1 = vec.y;
                vec.z = 0.0;
                let norm = vec.magnitude();
                vec = cgmath::Vector3{x: x1 * norm1/norm, y: y1 * norm1/norm, z: 0.0};

                self.cam.loc += self.cam.loc.z * 0.2  * delta * vec;
                self.input = Input::Empty;
            }
            Input::RotQuant {x, y} => {
                let rot_x = cgmath::Quaternion::from_angle_z(cgmath::Rad(0.3 * 1.0 * delta * x));
                let rot_y = cgmath::Quaternion::from_angle_x(cgmath::Rad(0.3 * 1.0 * delta * y));
                self.cam.rot = rot_x  * self.cam.rot * rot_y;
                self.input = Input::Empty;
            }
            _ => {}
        }
    }

    fn draw<C: gfx::CommandBuffer<R>>(
        &mut self,
        enc: &mut gfx::Encoder<R, C>,
    ) {
        self.render
            .draw_world(enc, None.into_iter(), &self.cam, false);
    }

    fn reload_shaders<F: gfx::Factory<R>>(&mut self, factory: &mut F) {
        self.render.reload(factory);
    }
}
