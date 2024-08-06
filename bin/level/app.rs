use crate::boilerplate::Application;
use vangers::{
    config, level,
    render::{Batcher, GraphicsContext, Render, ScreenTargets},
    space,
};

use log::info;
use winit::event;

#[derive(Debug)]
enum Input {
    Hor { dir: f32, alt: bool, shift: bool },
    Ver { dir: f32, alt: bool, shift: bool },
    Dep { dir: f32, alt: bool },
    DepQuant(f32),
    PlaneQuant(cgmath::Vector2<f32>),
    RotQuant(cgmath::Vector2<f32>),
    Empty,
}

pub struct LevelView {
    render: Render,
    level: level::Level,
    cam: space::Camera,
    input: Input,
    ui: config::settings::Ui,

    last_mouse_pos: cgmath::Vector2<f32>,
    alt_button_pressed: bool,
    mouse_button_pressed: bool,
}

impl LevelView {
    pub fn new(
        override_path: Option<&String>,
        settings: &config::settings::Settings,
        gfx: &GraphicsContext,
    ) -> Self {
        let mut override_palette = None;
        let level_config = if settings.game.level.is_empty() {
            info!("Using test level");
            level::LevelConfig::new_test()
        } else if let Some(ini_path) = override_path {
            info!("Using level at {}", ini_path);
            let full_path = settings.data_path.join(ini_path);
            level::LevelConfig::load(&full_path)
        } else {
            let escaves = config::escaves::load(settings.open_relative("escaves.prm"));
            let worlds = config::worlds::load(settings.open_relative("wrlds.dat"));

            let ini_name = worlds.get(&settings.game.level).unwrap_or_else(|| {
                panic!(
                    "Unable to find the world, supported: {:?}",
                    worlds.keys().collect::<Vec<_>>()
                )
            });
            let ini_path = settings.data_path.join(ini_name);
            info!("Using level {}", ini_name);

            if !settings.game.cycle.is_empty() {
                let escave = escaves
                    .iter()
                    .find(|e| e.world == settings.game.level)
                    .unwrap_or_else(|| {
                        panic!(
                            "Unable to find the escave for this world, supported: {:?}",
                            escaves.iter().map(|e| &e.world).collect::<Vec<_>>()
                        )
                    });
                let bunch = {
                    let file = settings.open_relative("bunches.prm");
                    let mut bunches = config::bunches::load(file);
                    let index = bunches
                        .iter()
                        .position(|b| b.escave == escave.name)
                        .unwrap_or_else(|| {
                            panic!(
                                "Unable to find the bunch, supported: {:?}",
                                bunches.iter().map(|b| &b.escave).collect::<Vec<_>>()
                            )
                        });
                    info!("Found bunch {}", index);
                    bunches.swap_remove(index)
                };
                let cycle = bunch
                    .cycles
                    .iter()
                    .find(|c| c.name == settings.game.cycle)
                    .unwrap_or_else(|| {
                        panic!(
                            "Unknown cycle is provided, supported: {:?}",
                            bunch.cycles.iter().map(|c| &c.name).collect::<Vec<_>>()
                        )
                    });
                override_palette = Some(settings.open_relative(&cycle.palette_path));
            }

            level::LevelConfig::load(&ini_path)
        };

        let depth = settings.game.camera.depth_range;
        let cam = space::Camera {
            loc: cgmath::vec3(0.0, 0.0, 400.0),
            rot: cgmath::One::one(),
            scale: cgmath::vec3(1.0, -1.0, 1.0),
            proj: match settings.game.camera.projection {
                config::settings::Projection::Perspective => {
                    let pf = cgmath::PerspectiveFov {
                        fovy: cgmath::Deg(45.0).into(),
                        aspect: settings.window.size[0] as f32 / settings.window.size[1] as f32,
                        near: depth.0,
                        far: depth.1,
                    };
                    space::Projection::Perspective(pf)
                }
                config::settings::Projection::Flat => space::Projection::ortho(
                    settings.window.size[0] as u16,
                    settings.window.size[1] as u16,
                    depth.0..depth.1,
                ),
            },
        };

        let objects_palette = level::read_palette(settings.open_palette(), None);
        let render = Render::new(
            gfx,
            &level_config,
            &objects_palette,
            &settings.render,
            &settings.game.geometry,
            cam.front_face(),
        );

        let mut level = level::load(&level_config, &settings.game.geometry);
        if let Some(pal_file) = override_palette {
            level.palette = level::read_palette(pal_file, Some(&level_config.terrains));
        }

        LevelView {
            render,
            level,
            cam,
            input: Input::Empty,
            ui: settings.ui,
            last_mouse_pos: cgmath::vec2(-1.0, -1.0),
            alt_button_pressed: false,
            mouse_button_pressed: false,
        }
    }
}

impl Application for LevelView {
    fn on_cursor_move(&mut self, position: (f64, f64)) {
        if !self.mouse_button_pressed {
            return;
        }
        let position_vec = cgmath::vec2(position.0 as f32, position.1 as f32);

        if self.last_mouse_pos.x < 0.0 {
            self.last_mouse_pos = position_vec;
            return;
        }

        let mut shift = position_vec - self.last_mouse_pos;
        shift.x *= self.cam.scale.x;
        shift.y *= self.cam.scale.y;

        self.input = if self.alt_button_pressed {
            Input::RotQuant(shift)
        } else {
            Input::PlaneQuant(shift)
        };
        self.last_mouse_pos = position_vec;
    }

    #[allow(clippy::single_match)]
    fn on_mouse_wheel(&mut self, delta: event::MouseScrollDelta) {
        match delta {
            event::MouseScrollDelta::LineDelta(_, y) => {
                self.input = Input::DepQuant(y);
            }
            _ => {}
        }
    }

    fn on_mouse_button(&mut self, state: event::ElementState, button: event::MouseButton) {
        if button == event::MouseButton::Left {
            self.mouse_button_pressed = state == event::ElementState::Pressed;
            self.last_mouse_pos = cgmath::vec2(-1.0, -1.0);
        }
    }

    fn on_key(&mut self, input: event::KeyboardInput) -> bool {
        use winit::event::{ElementState, KeyboardInput, VirtualKeyCode as Key};

        let i = &mut self.input;
        #[allow(deprecated)]
        match input {
            KeyboardInput {
                state: ElementState::Pressed,
                virtual_keycode: Some(key),
                ref modifiers,
                ..
            } => match key {
                Key::Escape => return false,
                Key::W => {
                    *i = Input::Ver {
                        dir: self.cam.scale.y,
                        alt: modifiers.alt(),
                        shift: modifiers.shift(),
                    }
                }
                Key::S => {
                    *i = Input::Ver {
                        dir: -self.cam.scale.y,
                        alt: modifiers.alt(),
                        shift: modifiers.shift(),
                    }
                }
                Key::A => {
                    *i = Input::Hor {
                        dir: -self.cam.scale.x,
                        alt: modifiers.alt(),
                        shift: modifiers.shift(),
                    }
                }
                Key::D => {
                    *i = Input::Hor {
                        dir: self.cam.scale.x,
                        alt: modifiers.alt(),
                        shift: modifiers.shift(),
                    }
                }
                Key::Z => {
                    *i = Input::Dep {
                        dir: -self.cam.scale.z,
                        alt: modifiers.alt(),
                    }
                }
                Key::X => {
                    *i = Input::Dep {
                        dir: self.cam.scale.z,
                        alt: modifiers.alt(),
                    }
                }
                Key::LAlt => self.alt_button_pressed = true,
                _ => (),
            },
            KeyboardInput {
                state: ElementState::Released,
                virtual_keycode: Some(key),
                ..
            } => match key {
                Key::W | Key::S | Key::A | Key::D | Key::Z | Key::X => *i = Input::Empty,
                Key::LAlt => self.alt_button_pressed = false,
                _ => (),
            },
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

    fn update(&mut self, _device: &wgpu::Device, _queue: &wgpu::Queue, delta: f32) {
        use cgmath::{InnerSpace as _, Rotation3 as _};

        let move_speed = match self.cam.proj {
            space::Projection::Perspective(_) => 100.0,
            space::Projection::Ortho { .. } => 500.0,
        };
        let rotation_speed = 1.0;
        let fast_move_speed = 5.0 * move_speed;
        match self.input {
            Input::Hor {
                dir,
                alt: false,
                shift,
            } if dir != 0.0 => {
                let mut vec = self.cam.rot * cgmath::Vector3::unit_x();
                vec.z = 0.0;
                let speed = if shift { fast_move_speed } else { move_speed };
                self.cam.loc += speed * delta * dir * vec.normalize();
            }
            Input::Ver {
                dir,
                alt: false,
                shift,
            } if dir != 0.0 => {
                let mut vec = self.cam.rot * cgmath::Vector3::unit_y();
                vec.z = 0.0;
                let speed = if shift { fast_move_speed } else { move_speed };
                self.cam.loc += speed * delta * dir * vec.normalize();
            }
            Input::Dep { dir, alt: false } if dir != 0.0 => {
                let vec = cgmath::Vector3::unit_z();
                self.cam.loc += move_speed * delta * dir * vec.normalize();
            }
            Input::Hor { dir, alt: true, .. } if dir != 0.0 => {
                let rot =
                    cgmath::Quaternion::from_angle_z(cgmath::Rad(rotation_speed * delta * dir));
                self.cam.rot = rot * self.cam.rot;
            }
            Input::Ver { dir, alt: true, .. } if dir != 0.0 => {
                let rot =
                    cgmath::Quaternion::from_angle_x(cgmath::Rad(rotation_speed * delta * dir));
                self.cam.rot = self.cam.rot * rot;
            }
            Input::DepQuant(dir) => {
                let vec = cgmath::Vector3::unit_z();
                self.cam.loc += 1000.0 * delta * dir * vec.normalize();
                self.input = Input::Empty;
            }
            Input::PlaneQuant(dir) => {
                let vec_x = self.cam.rot * cgmath::vec3(-dir.x, 0.0, 0.0);
                let vec_y = self.cam.rot * cgmath::vec3(0.0, dir.y, 0.0);

                let mut vec = vec_x + vec_y;

                let norm1 = vec.magnitude();
                if norm1 > 0.0 {
                    vec.z = 0.0;
                    let norm = vec.magnitude();
                    vec *= norm1 / norm;
                    self.cam.loc += self.cam.loc.z * 0.2 * delta * vec;
                }
                self.input = Input::Empty;
            }
            Input::RotQuant(dir) => {
                let speed = 0.3;
                let rot_x = cgmath::Quaternion::from_angle_z(cgmath::Rad(speed * delta * dir.x));
                let rot_y = cgmath::Quaternion::from_angle_x(cgmath::Rad(-speed * delta * dir.y));
                self.cam.rot = rot_x * self.cam.rot * rot_y;
                self.input = Input::Empty;
            }
            _ => {}
        }
    }

    fn resize(&mut self, device: &wgpu::Device, extent: wgpu::Extent3d) {
        self.cam
            .proj
            .update(extent.width as u16, extent.height as u16);
        self.render.resize(extent, device);
    }

    fn reload(&mut self, device: &wgpu::Device) {
        self.render.reload(device);
    }

    fn draw_ui(&mut self, context: &egui::Context) {
        if !self.ui.enabled {
            return;
        }
        egui::SidePanel::right("Tweaks").show(context, |ui| {
            ui.group(|ui| {
                ui.label("Camera:");
                self.cam.draw_ui(ui);
            });
            ui.group(|ui| {
                ui.label("Level:");
                self.level.draw_ui(ui);
            });
            ui.group(|ui| {
                ui.label("Renderer:");
                self.render.draw_ui(ui);
            });
        });
    }

    fn draw(&mut self, device: &wgpu::Device, targets: ScreenTargets) -> wgpu::CommandBuffer {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("World"),
        });

        self.render.draw_world(
            &mut encoder,
            &mut Batcher::new(),
            &self.level,
            &self.cam,
            targets,
            None,
            device,
        );

        encoder.finish()
    }
}
