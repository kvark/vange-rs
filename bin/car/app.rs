use crate::boilerplate::Application;
use m3d::Mesh;
use vangers::{config, level, model, render, space};

use cgmath;
use log::info;
use wgpu;


pub struct CarView {
    model: model::RenderModel,
    transform: space::Transform,
    physics: config::car::CarPhysics,
    debug_render: render::DebugRender,
    global: render::GlobalContext,
    object: render::object::Context,
    cam: space::Camera,
    rotation: (cgmath::Rad<f32>, cgmath::Rad<f32>),
    light_config: config::settings::Light,
}

impl CarView {
    pub fn new(
        settings: &config::Settings,
        device: &mut wgpu::Device,
    ) -> Self {
        info!("Loading car registry");
        let game_reg = config::game::Registry::load(settings);
        let car_reg = config::car::load_registry(settings, &game_reg, device);
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
            let raw = Mesh::load(&mut settings.open_relative(&info.path));
            ms.mesh = Some(model::load_c3d(raw, device));
            ms.scale = info.scale;
        }

        let pal_data = level::read_palette(settings.open_palette(), None);
        let mut init_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            todo: 0,
        });
        let global = render::GlobalContext::new(device);
        let object = render::object::Context::new(&mut init_encoder, device, &pal_data, &global);
        device.get_queue().submit(&[
            init_encoder.finish(),
        ]);

        CarView {
            model,
            transform: cgmath::Decomposed {
                scale: cinfo.scale,
                disp: cgmath::Vector3::unit_z(),
                rot: cgmath::One::one(),
            },
            physics: cinfo.physics.clone(),
            debug_render: render::DebugRender::new(device, &settings.render.debug),
            global,
            object,
            cam: space::Camera {
                loc: cgmath::vec3(0.0, -64.0, 32.0),
                rot: cgmath::Rotation3::from_angle_x::<cgmath::Rad<_>>(
                    cgmath::Angle::turn_div_6(),
                ),
                proj: space::Projection::Perspective(cgmath::PerspectiveFov {
                    fovy: cgmath::Deg(45.0).into(),
                    aspect: settings.window.size[0] as f32 / settings.window.size[1] as f32,
                    near: 1.0,
                    far: 100.0,
                }),
            },
            rotation: (cgmath::Rad(0.), cgmath::Rad(0.)),
            light_config: settings.render.light.clone(),
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

impl Application for CarView {
    fn on_key(&mut self, input: wgpu::winit::KeyboardInput) -> bool {
        use wgpu::winit::{ElementState, KeyboardInput, VirtualKeyCode as Key};

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

    fn resize(&mut self, _device: &wgpu::Device, extent: wgpu::Extent3d) {
        self.cam.proj.update(extent.width as u16, extent.height as u16);
    }

    fn reload(&mut self, device: &wgpu::Device) {
        self.object.reload(device);
    }

    fn draw(
        &mut self,
        device: &wgpu::Device,
        targets: render::ScreenTargets,
    ) -> Vec<wgpu::CommandBuffer> {
        let mx_vp = self.cam.get_view_proj();

        let mut updater = render::Updater::new(device);
        updater.update(&self.global.uniform_buf, &[
            render::GlobalConstants::new(&self.cam, &self.light_config),
        ]);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            todo: 0,
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[
                    wgpu::RenderPassColorAttachmentDescriptor {
                        attachment: targets.color,
                        load_op: wgpu::LoadOp::Clear,
                        store_op: wgpu::StoreOp::Store,
                        clear_color: wgpu::Color {
                            r: 0.1, g: 0.2, b: 0.3, a: 1.0,
                        },
                    },
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachmentDescriptor {
                    attachment: targets.depth,
                    depth_load_op: wgpu::LoadOp::Clear,
                    depth_store_op: wgpu::StoreOp::Store,
                    clear_depth: 1.0,
                    stencil_load_op: wgpu::LoadOp::Clear,
                    stencil_store_op: wgpu::StoreOp::Store,
                    clear_stencil: 0,
                }),
            });

            pass.set_pipeline(&self.object.pipeline);
            pass.set_bind_group(0, &self.global.bind_group);
            pass.set_bind_group(1, &self.object.bind_group);
            render::Render::draw_model(
                &mut pass,
                &mut updater,
                &self.model,
                self.transform,
                &self.object.uniform_buf,
                Some((&mut self.debug_render, self.physics.scale_bound, &mx_vp)),
            );
        }

        vec![
            updater.finish(),
            encoder.finish(),
        ]
    }
}
