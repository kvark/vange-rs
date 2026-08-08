use crate::boilerplate::Application;
use vangers::{
    config, level,
    render::{Batcher, GraphicsContext, Render, ScreenTargets},
    space,
};

use log::info;
use winit::{event, keyboard::KeyCode};

#[derive(Debug)]
enum Input {
    Hor { dir: f32, alt: bool, shift: bool },
    Ver { dir: f32, alt: bool, shift: bool },
    Dep { dir: f32, alt: bool },
    DepQuant(f32),
    PlaneQuant(glam::Vec2),
    RotQuant(glam::Vec2),
    Empty,
}

pub struct LevelView {
    render: Render,
    level: level::Level,
    cam: space::Camera,
    input: Input,
    ui: config::settings::Ui,

    last_mouse_pos: glam::Vec2,
    alt_button_pressed: bool,
    /// Whether the tweaks panel is showing. Collapsed it is one button, so
    /// a framed shot is not half controls.
    ui_expanded: bool,
    mouse_button_pressed: bool,
}

impl LevelView {
    /// Log the camera as a `tools/compare-terrain.py --view` spec and as a
    /// runnable `level` command line.
    ///
    /// `--fp` stands the camera on the surface at an XY and looks along a
    /// heading, so this reports eye height above the local ground rather
    /// than an absolute Z, and sets `--fp-under` when the camera is
    /// beneath a slab. That conversion is the whole point: it is the
    /// fiddly step between flying somewhere worth measuring and being able
    /// to measure it.
    fn dump_camera(&self) {
        let loc = self.cam.loc;
        let fwd = self.cam.dir();
        let yaw = fwd.x.atan2(fwd.y).to_degrees().rem_euclid(360.0);
        let pitch = fwd.z.clamp(-1.0, 1.0).asin().to_degrees();

        // Wrap into the level's own range. The viewer's camera roams a
        // continuous plane and the level repeats under it, so a position
        // found by flying is routinely negative or past the width - which
        // `Level::get` handles, but a `--view` spec does not: the reference
        // ray cast indexes the height array directly, where a negative
        // wraps by luck and an oversized one is an index error.
        let texel = (
            (loc.x as i32).rem_euclid(self.level.size.0),
            (loc.y as i32).rem_euclid(self.level.size.1),
        );
        let ground = self.level.get(texel);
        let under = match ground {
            level::Texel::Dual { mid, .. } => loc.z < mid,
            level::Texel::Single(_) => false,
        };
        let base = match ground {
            level::Texel::Single(p) => p.0,
            level::Texel::Dual { low, high, .. } => {
                if under {
                    low.0
                } else {
                    high.0
                }
            }
        };
        let eye = (loc.z - base).max(1.0);

        println!(
            "\n--view \"spot:{},{}:{:.0}{}\"   --pitch {:.0}",
            texel.0,
            texel.1,
            yaw,
            if under { ":under" } else { "" },
            pitch
        );
        println!(
            "cargo run --release --bin level -- --terrain Mesh \\\n  \
             --fp {},{} --fp-height {:.0} --fp-yaw={:.0} --fp-pitch={:.0}{} \\\n  \
             --near 1 --far 600 --snapshot view.png\n",
            texel.0,
            texel.1,
            eye,
            yaw,
            pitch,
            if under { " --fp-under" } else { "" },
        );
    }

    pub fn new(
        override_path: Option<&str>,
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
            loc: glam::Vec3::new(0.0, 0.0, 400.0),
            rot: glam::Quat::IDENTITY,
            scale: glam::Vec3::new(1.0, -1.0, 1.0),
            proj: match settings.game.camera.projection {
                config::settings::Projection::Perspective => {
                    let h = settings.window.size[1] as f32;
                    let focal = space::DEFAULT_FOCAL_PX;
                    let pf = space::PerspectiveParams {
                        fovy: space::PerspectiveParams::fov_from_focal_px(focal, h),
                        aspect: settings.window.size[0] as f32 / h,
                        near: depth.0,
                        far: depth.1,
                        focal_px: Some(focal),
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
            last_mouse_pos: glam::Vec2::new(-1.0, -1.0),
            alt_button_pressed: false,
            ui_expanded: true,
            mouse_button_pressed: false,
        }
    }
}

impl Application for LevelView {
    fn on_cursor_move(&mut self, position: (f64, f64)) {
        if !self.mouse_button_pressed {
            return;
        }
        let position_vec = glam::Vec2::new(position.0 as f32, position.1 as f32);

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
            self.last_mouse_pos = glam::Vec2::new(-1.0, -1.0);
        }
    }

    fn on_key(&mut self, key: KeyCode, state: event::ElementState) -> bool {
        use winit::event::ElementState;

        let i = &mut self.input;
        let alt = self.alt_button_pressed;
        match state {
            ElementState::Pressed => match key {
                KeyCode::Escape => return false,
                KeyCode::KeyW => {
                    *i = Input::Ver {
                        dir: self.cam.scale.y,
                        alt,
                        shift: false,
                    }
                }
                KeyCode::KeyS => {
                    *i = Input::Ver {
                        dir: -self.cam.scale.y,
                        alt,
                        shift: false,
                    }
                }
                KeyCode::KeyA => {
                    *i = Input::Hor {
                        dir: -self.cam.scale.x,
                        alt,
                        shift: false,
                    }
                }
                KeyCode::KeyD => {
                    *i = Input::Hor {
                        dir: self.cam.scale.x,
                        alt,
                        shift: false,
                    }
                }
                KeyCode::KeyZ => {
                    *i = Input::Dep {
                        dir: -self.cam.scale.z,
                        alt,
                    }
                }
                KeyCode::KeyX => {
                    *i = Input::Dep {
                        dir: self.cam.scale.z,
                        alt,
                    }
                }
                // Print the current camera as something the comparison
                // harness accepts, so a viewpoint found by flying around
                // can be reproduced exactly rather than described.
                KeyCode::F9 => self.dump_camera(),
                KeyCode::AltLeft => self.alt_button_pressed = true,
                _ => (),
            },
            ElementState::Released => match key {
                KeyCode::KeyW
                | KeyCode::KeyS
                | KeyCode::KeyA
                | KeyCode::KeyD
                | KeyCode::KeyZ
                | KeyCode::KeyX => *i = Input::Empty,
                KeyCode::AltLeft => self.alt_button_pressed = false,
                _ => (),
            },
        }

        true
    }

    fn update(&mut self, _device: &wgpu::Device, _queue: &wgpu::Queue, delta: f32) {
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
                let mut vec = self.cam.rot * glam::Vec3::X;
                vec.z = 0.0;
                let speed = if shift { fast_move_speed } else { move_speed };
                self.cam.loc += speed * delta * dir * vec.normalize();
            }
            Input::Ver {
                dir,
                alt: false,
                shift,
            } if dir != 0.0 => {
                let mut vec = self.cam.rot * glam::Vec3::Y;
                vec.z = 0.0;
                let speed = if shift { fast_move_speed } else { move_speed };
                self.cam.loc += speed * delta * dir * vec.normalize();
            }
            Input::Dep { dir, alt: false } if dir != 0.0 => {
                let vec = glam::Vec3::Z;
                self.cam.loc += move_speed * delta * dir * vec.normalize();
            }
            Input::Hor { dir, alt: true, .. } if dir != 0.0 => {
                let rot = glam::Quat::from_rotation_z(rotation_speed * delta * dir);
                self.cam.rot = rot * self.cam.rot;
            }
            Input::Ver { dir, alt: true, .. } if dir != 0.0 => {
                let rot = glam::Quat::from_rotation_x(rotation_speed * delta * dir);
                self.cam.rot *= rot;
            }
            Input::DepQuant(dir) => {
                let vec = glam::Vec3::Z;
                self.cam.loc += 1000.0 * delta * dir * vec.normalize();
                self.input = Input::Empty;
            }
            Input::PlaneQuant(dir) => {
                let vec_x = self.cam.rot * glam::Vec3::new(-dir.x, 0.0, 0.0);
                let vec_y = self.cam.rot * glam::Vec3::new(0.0, dir.y, 0.0);

                let mut vec = vec_x + vec_y;

                let norm1 = vec.length();
                if norm1 > 0.0 {
                    vec.z = 0.0;
                    let norm = vec.length();
                    vec *= norm1 / norm;
                    self.cam.loc += self.cam.loc.z * 0.2 * delta * vec;
                }
                self.input = Input::Empty;
            }
            Input::RotQuant(dir) => {
                let speed = 0.3;
                let rot_x = glam::Quat::from_rotation_z(speed * delta * dir.x);
                let rot_y = glam::Quat::from_rotation_x(-speed * delta * dir.y);
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
        // Collapsed, the panel is a single button, so a view can be framed
        // and captured without the controls covering the thing being
        // looked at. The state lives on the app rather than in egui's
        // memory so it survives a UI rebuild.
        if !self.ui_expanded {
            egui::Area::new("Tweaks toggle".into())
                .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, 8.0))
                .show(context, |ui| {
                    if ui.button("<").on_hover_text("Show controls").clicked() {
                        self.ui_expanded = true;
                    }
                });
            return;
        }

        #[allow(deprecated)]
        egui::SidePanel::right("Tweaks").show(context, |ui| {
            ui.horizontal(|ui| {
                if ui.button(">").on_hover_text("Hide controls").clicked() {
                    self.ui_expanded = false;
                }
                ui.label("Tweaks");
            });
            ui.group(|ui| {
                ui.label("Camera:");
                self.cam.draw_ui(ui);
                if ui
                    .button("Copy view spec (F9)")
                    .on_hover_text("Print this camera as a --view spec and a \
                                    runnable level command line")
                    .clicked()
                {
                    self.dump_camera();
                }
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

    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        targets: ScreenTargets,
    ) -> wgpu::CommandBuffer {
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
            queue,
        );

        encoder.finish()
    }
}
