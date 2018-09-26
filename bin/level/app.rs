use cgmath;
use gfx;

use boilerplate::{Application, KeyboardInput, MouseScrollDelta, MouseButton, ElementState};
use vangers::{config, level, render, space};

#[derive(Debug)]
enum Input {
    Hor { dir: f32, alt: bool },
    Ver { dir: f32, alt: bool },
    Dep { dir: f32, alt: bool },
    DepQuant(f32),
    PlaneQuant(cgmath::Vector2<f32>),
    RotQuant(cgmath::Vector2<f32>),
    Empty,
}

pub struct LevelView<R: gfx::Resources> {
    render: render::Render<R>,
    _level: level::Level,
    cam: space::Camera,
    input: Input,

    last_mouse_pos: cgmath::Vector2<f32>,
    alt_button_pressed: bool,
    mouse_button_pressed: bool,
}

impl<R: gfx::Resources> LevelView<R> {
    pub fn new<F: gfx::Factory<R>>(
        settings: &config::settings::Settings,
        targets: render::MainTargets<R>,
        factory: &mut F,
    ) -> Self {
        let level = if settings.game.level.is_empty() {
            info!("Using test level");
            level::Level::new_test()
        } else {
            let escaves = config::escaves::load(settings.open_relative("escaves.prm"));
            let worlds = config::worlds::load(settings.open_relative("wrlds.dat"));

            let ini_name = worlds.get(&settings.game.level)
                .expect(&format!("Unable to find the world, supported: {:?}",
                    worlds.keys().collect::<Vec<_>>()
                ));
            let ini_path = settings.data_path.join(ini_name);
            info!("Using level {}", ini_name);

            let level_config = level::LevelConfig::load(&ini_path);
            let mut override_palette = None;

            if !settings.game.cycle.is_empty() {
                let escave = escaves
                    .iter()
                    .find(|e| e.world == settings.game.level)
                    .expect(&format!("Unable to find the escave for this world, supported: {:?}",
                        escaves.iter().map(|e| &e.world).collect::<Vec<_>>()
                    ));
                let bunch = {
                    let file = settings.open_relative("bunches.prm");
                    let mut bunches = config::bunches::load(file);
                    let index = bunches
                        .iter()
                        .position(|b| b.escave == escave.name)
                        .expect(&format!("Unable to find the bunch, supported: {:?}",
                            bunches.iter().map(|b| &b.escave).collect::<Vec<_>>()
                        ));
                    info!("Found bunch {}", index);
                    bunches.swap_remove(index)
                };
                let cycle = bunch.cycles
                    .iter()
                    .find(|c| c.name == settings.game.cycle)
                    .expect(&format!("Unknown cycle is provided, supported: {:?}",
                        bunch.cycles.iter().map(|c| &c.name).collect::<Vec<_>>()
                    ));
                override_palette = Some(settings.open_relative(&cycle.palette_path));
            }

            let mut level = level::load(&level_config);
            if let Some(pal_file) = override_palette {
                level.palette = level::read_palette(pal_file, Some(&level_config.terrains));
            }
            level
        };

        let objects_palette = level::read_palette(settings.open_palette(), None);
        let (width, height, _, _) = targets.color.get_dimensions();
        let depth = 10f32 .. 10000f32;
        let render = render::init(factory, targets, &level, &objects_palette, &settings.render);

        LevelView {
            render,
            _level: level,
            cam: space::Camera {
                loc: cgmath::vec3(0.0, 0.0, 400.0),
                rot: cgmath::Quaternion::new(1.0, 0.0, 0.0, 0.0),
                proj: match settings.game.view {
                    config::settings::View::Perspective => {
                        let pf = cgmath::PerspectiveFov {
                            fovy: cgmath::Deg(45.0).into(),
                            aspect: width as f32 / height as f32,
                            near: depth.start,
                            far: depth.end,
                        };
                        space::Projection::Perspective(pf)
                    }
                    config::settings::View::Flat => {
                        space::Projection::ortho(width, height, depth)
                    }
                },
            },
            input: Input::Empty,
            last_mouse_pos: cgmath::vec2(-1.0, -1.0),
            alt_button_pressed: false,
            mouse_button_pressed: false,
        }
    }
}

impl<R: gfx::Resources> Application<R> for LevelView<R> {
    fn on_cursor_move(&mut self, position: (f64, f64)){
        if !self.mouse_button_pressed {
            return;
        }
        let position_vec = cgmath::vec2(position.0 as f32, position.1 as f32);

        if self.last_mouse_pos.x < 0.0 {
            self.last_mouse_pos = position_vec;
            return;
        }

        let shift = position_vec - self.last_mouse_pos;
        self.input = if self.alt_button_pressed {
            Input::RotQuant(shift)
        }else {
            Input::PlaneQuant(shift)
        };
        self.last_mouse_pos = position_vec;
    }

    fn on_mouse_wheel(&mut self, delta: MouseScrollDelta) {
        match delta {
            MouseScrollDelta::LineDelta(_, y) => {
                self.input = Input::DepQuant(y);
            }
            _ => {}
        }

    }

    fn on_mouse_button(&mut self, state: ElementState, button: MouseButton) {
        if button == MouseButton::Left {
            self.mouse_button_pressed = state == ElementState::Pressed;
            self.last_mouse_pos = cgmath::vec2(-1.0, -1.0);
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
                Key::LAlt => self.alt_button_pressed = true,
                _ => (),
            }
            KeyboardInput {
                state: ElementState::Released,
                virtual_keycode: Some(key),
                ..
            } => match key {
                Key::W | Key::S | Key::A | Key::D | Key::Z | Key::X => *i = Input::Empty,
                Key::LAlt => self.alt_button_pressed = false,
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
        let move_speed = match self.cam.proj {
            space::Projection::Perspective(_) => 100.0,
            space::Projection::Ortho{..} => 500.0,
        };
        match self.input {
            Input::Hor { dir, alt: false } if dir != 0.0 => {
                let mut vec = self.cam.rot * cgmath::Vector3::unit_x();
                vec.z = 0.0;
                self.cam.loc += move_speed * delta * dir * vec.normalize();
            }
            Input::Ver { dir, alt: false } if dir != 0.0 => {
                let mut vec = self.cam.rot * cgmath::Vector3::unit_z();
                vec.z = 0.0;
                if vec == cgmath::Vector3::zero() {
                    vec = self.cam.rot * -cgmath::Vector3::unit_y();
                    vec.z = 0.0;
                }
                self.cam.loc -= move_speed * delta * dir * vec.normalize();
            }
            Input::Dep { dir, alt: false } if dir != 0.0 => {
                let vec = cgmath::Vector3::unit_z();
                self.cam.loc += move_speed * delta * dir * vec.normalize();
            }
            Input::Hor { dir, alt: true } if dir != 0.0 => {
                let rot = cgmath::Quaternion::from_angle_z(cgmath::Rad(-1.0 * delta * dir));
                self.cam.rot = rot * self.cam.rot;
            }
            Input::Ver { dir, alt: true } if dir != 0.0 => {
                let rot = cgmath::Quaternion::from_angle_x(cgmath::Rad(1.0 * delta * dir));
                self.cam.rot = self.cam.rot * rot;
            }
            Input::DepQuant(dir)=> {
                let vec = cgmath::Vector3::unit_z();
                self.cam.loc += 1000.0 * delta * dir * vec.normalize();
                self.input = Input::Empty;
            }
            Input::PlaneQuant(dir) => {
                let vec_x = self.cam.rot * cgmath::vec3(-dir.x, 0.0, 0.0);
                let vec_y = self.cam.rot * cgmath::vec3(0.0, dir.y, 0.0);

                let mut vec = vec_x + vec_y;

                let norm1 = vec.magnitude();
                vec.z = 0.0;
                let norm = vec.magnitude();
                vec *= norm1/norm;

                self.cam.loc += self.cam.loc.z * 0.2  * delta * vec;
                self.input = Input::Empty;
            }
            Input::RotQuant(dir) => {
                let rot_x = cgmath::Quaternion::from_angle_z(cgmath::Rad(0.3 * 1.0 * delta * dir.x));
                let rot_y = cgmath::Quaternion::from_angle_x(cgmath::Rad(0.3 * 1.0 * delta * dir.y));
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
            .draw_world(enc, &[], &self.cam);
    }

    fn gpu_update<F: gfx::Factory<R>>(
        &mut self, factory: &mut F,
        resized_targets: Option<render::MainTargets<R>>,
        reload_shaders: bool,
    ) {
        if let Some(targets) = resized_targets {
            let (w, h, _, _) = targets.color.get_dimensions();
            self.cam.proj.update(w, h);
            self.render.resize(targets);
        }
        if reload_shaders {
            self.render.reload(factory);
        }
    }
}
