use crate::boilerplate::Application;
use cgmath::Transform as _;
use vangers::{config, level, model, render, space};

use log::info;

struct CarContext {
    color: render::object::BodyColor,
    physics: config::car::CarPhysics,
}

pub struct ResourceView {
    model: model::VisualModel,
    car: Option<CarContext>,
    global: render::global::Context,
    object: render::object::Context,
    transform: space::Transform,
    camera: space::Camera,
    rotation: (cgmath::Rad<f32>, cgmath::Rad<f32>),
    light_config: config::settings::Light,
}

impl ResourceView {
    pub fn new(
        path: Option<&String>,
        settings: &config::settings::Settings,
        gfx: &render::GraphicsContext,
    ) -> Self {
        let camera = space::Camera {
            loc: cgmath::vec3(0.0, -200.0, 100.0),
            rot: cgmath::Rotation3::from_angle_x::<cgmath::Rad<_>>(cgmath::Angle::turn_div_6()),
            scale: cgmath::vec3(1.0, 1.0, 1.0),
            proj: space::Projection::Perspective(cgmath::PerspectiveFov {
                fovy: cgmath::Deg(45.0).into(),
                aspect: settings.window.size[0] as f32 / settings.window.size[1] as f32,
                near: 5.0,
                far: 400.0,
            }),
        };

        info!("Initializing the render");
        let pal_data = level::read_palette(settings.open_palette(), None);
        let global = render::global::Context::new(gfx, None);
        let object = render::object::Context::new(gfx, camera.front_face(), &pal_data, &global);

        let (model, car) = if let Some(path_str) = path {
            info!("Loading model {}", path_str);
            let file = settings.open_relative(path_str);
            let model = model::load_m3d(
                file,
                &gfx.device,
                &object,
                settings.game.physics.shape_sampling,
            );
            (model, None)
        } else {
            info!("Loading car registry");
            let game_reg = config::game::Registry::load(settings);
            let car_reg = config::car::load_registry(settings, &game_reg, &gfx.device, &object);
            let cinfo = match car_reg.get(&settings.car.id) {
                Some(ci) => ci,
                None => {
                    let names = car_reg.keys().collect::<Vec<_>>();
                    panic!("Unable to find `{}` in {:?}", settings.car.id, names);
                }
            };

            let mut model = cinfo.model.clone();
            for (ms, sid) in model.slots.iter_mut().zip(settings.car.slots.iter()) {
                let info = &game_reg.model_infos[sid];
                let raw = m3d::Mesh::load(&mut settings.open_relative(&info.path));
                ms.mesh = Some(model::load_c3d(raw, &gfx.device));
                ms.scale = info.scale;
            }

            let cc = CarContext {
                color: settings.car.color,
                physics: cinfo.physics.clone(),
            };
            (model, Some(cc))
        };

        ResourceView {
            model,
            car,
            global,
            object,
            transform: cgmath::Decomposed {
                scale: 1.0,
                disp: cgmath::Vector3::unit_z(),
                rot: cgmath::One::one(),
            },
            camera,
            rotation: (cgmath::Rad(0.), cgmath::Rad(0.)),
            light_config: settings.render.light,
        }
    }

    fn rotate_z(&mut self, angle: cgmath::Rad<f32>) {
        let other = cgmath::Decomposed {
            scale: 1.0,
            rot: cgmath::Rotation3::from_angle_z(angle),
            disp: cgmath::Zero::zero(),
        };
        self.transform = other.concat(&self.transform);
    }

    fn rotate_x(&mut self, angle: cgmath::Rad<f32>) {
        let other = cgmath::Decomposed {
            scale: 1.0,
            rot: cgmath::Rotation3::from_angle_x(angle),
            disp: cgmath::Zero::zero(),
        };
        self.transform = self.transform.concat(&other);
    }
}

impl Application for ResourceView {
    fn on_key(&mut self, input: winit::event::KeyboardInput) -> bool {
        use winit::event::{ElementState, KeyboardInput, VirtualKeyCode as Key};

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
            },
            KeyboardInput {
                state: ElementState::Released,
                virtual_keycode: Some(key),
                ..
            } => match key {
                Key::A | Key::D => self.rotation.0 = cgmath::Rad(0.0),
                Key::W | Key::S => self.rotation.1 = cgmath::Rad(0.0),
                _ => (),
            },
            _ => {}
        }

        true
    }

    fn update(&mut self, _device: &wgpu::Device, queue: &wgpu::Queue, delta: f32) {
        if self.rotation.0 != cgmath::Rad(0.) {
            let rot = self.rotation.0 * delta;
            self.rotate_z(rot);
        }
        if self.rotation.1 != cgmath::Rad(0.) {
            let rot = self.rotation.1 * delta;
            self.rotate_x(rot);
        }

        let global_data = render::global::Constants::new(&self.camera, &self.light_config, None);
        queue.write_buffer(
            &self.global.uniform_buf,
            0,
            bytemuck::bytes_of(&global_data),
        );
    }

    fn resize(&mut self, _device: &wgpu::Device, extent: wgpu::Extent3d) {
        self.camera
            .proj
            .update(extent.width as u16, extent.height as u16);
    }

    fn reload(&mut self, device: &wgpu::Device) {
        self.object.reload(device);
    }

    fn draw_ui(&mut self, _context: &egui::Context) {}

    fn draw(
        &mut self,
        device: &wgpu::Device,
        targets: render::ScreenTargets,
    ) -> wgpu::CommandBuffer {
        let (bound, color) = if let Some(ref cc) = self.car {
            (Some(cc.physics.scale_bound), cc.color)
        } else {
            (None, render::object::BodyColor::Dummy)
        };
        let mut batcher = render::Batcher::new();
        batcher.add_model(&self.model, &self.transform, bound, color);
        batcher.prepare(device);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Draw"),
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: targets.color,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.1,
                            g: 0.2,
                            b: 0.3,
                            a: 1.0,
                        }),
                        store: true,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: true,
                    }),
                    stencil_ops: None,
                }),
            });

            pass.set_pipeline(self.object.pipelines.select(render::PipelineKind::Main));
            pass.set_bind_group(0, &self.global.bind_group, &[]);
            pass.set_bind_group(1, &self.object.bind_group, &[]);

            batcher.draw(&mut pass);
        }

        encoder.finish()
    }
}
