use crate::boilerplate::Application;
use m3d::Mesh;
use vangers::{config, level, model, render, space};

use futures::executor::LocalSpawner;
use log::info;
use zerocopy::AsBytes as _;

use std::mem;


pub struct CarView {
    model: model::VisualModel,
    instance_buf: wgpu::Buffer,
    transform: space::Transform,
    physics: config::car::CarPhysics,
    debug_render: render::debug::Context,
    global: render::global::Context,
    object: render::object::Context,
    cam: space::Camera,
    rotation: (cgmath::Rad<f32>, cgmath::Rad<f32>),
    light_config: config::settings::Light,
}

impl CarView {
    pub fn new(
        settings: &config::Settings,
        device: &wgpu::Device,
        queue: &mut wgpu::Queue,
    ) -> Self {
        info!("Initializing the render");
        let pal_data = level::read_palette(settings.open_palette(), None);
        let mut init_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            todo: 0,
        });
        let store_init = render::body::GpuStoreInit::new_dummy(device);
        let global = render::global::Context::new(device, store_init.resource());
        let object = render::object::Context::new(&mut init_encoder, device, &pal_data, &global);
        queue.submit(&[
            init_encoder.finish(),
        ]);

        info!("Loading car registry");
        let game_reg = config::game::Registry::load(settings);
        let car_reg = config::car::load_registry(
            settings,
            &game_reg,
            device,
            &object,
        );
        let cinfo = match car_reg.get(&settings.car.id) {
            Some(ci) => ci,
            None => {
                let names = car_reg.keys().collect::<Vec<_>>();
                panic!("Unable to find `{}` in {:?}", settings.car.id, names);
            }
        };
        let mut model = cinfo.model.clone();
        let slot_locals_id = model.mesh_count() - model.slots.len();
        for (i, (ms, sid)) in model.slots
            .iter_mut()
            .zip(settings.car.slots.iter())
            .enumerate()
        {
            let info = &game_reg.model_infos[sid];
            let raw = Mesh::load(&mut settings.open_relative(&info.path));
            ms.mesh = Some(model::load_c3d(
                raw,
                device,
                slot_locals_id + i,
            ));
            ms.scale = info.scale;
        }

        let instance_buf = render::instantiate_visual_model(&model, device);

        CarView {
            model,
            instance_buf,
            transform: cgmath::Decomposed {
                scale: cinfo.scale,
                disp: cgmath::Vector3::unit_z(),
                rot: cgmath::One::one(),
            },
            physics: cinfo.physics.clone(),
            debug_render: render::debug::Context::new(
                device,
                &settings.render.debug,
                &global,
                &object,
            ),
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
        _device: &wgpu::Device,
        delta: f32,
        _spawner: &LocalSpawner,
    ) -> Vec<wgpu::CommandBuffer> {
        if self.rotation.0 != cgmath::Rad(0.) {
            let rot = self.rotation.0 * delta;
            self.rotate_z(rot);
        }
        if self.rotation.1 != cgmath::Rad(0.) {
            let rot = self.rotation.1 * delta;
            self.rotate_x(rot);
        }
        Vec::new()
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
        _spawner: &LocalSpawner,
    ) -> wgpu::CommandBuffer {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            todo: 0,
        });
        let global_data = render::global::Constants::new(&self.cam, &self.light_config);
        let global_staging = device.create_buffer_with_data(
            &[global_data].as_bytes(),
            wgpu::BufferUsage::COPY_SRC,
        );
        encoder.copy_buffer_to_buffer(
            &global_staging,
            0,
            &self.global.uniform_buf,
            0,
            mem::size_of::<render::global::Constants>() as wgpu::BufferAddress,
        );

        render::RenderModel {
            model: &self.model,
            gpu_body: &render::body::GpuBody::ZERO,
            instance_buf: &self.instance_buf,
            transform: self.transform,
            debug_shape_scale: Some(self.physics.scale_bound),
        }.prepare(&mut encoder, device);

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[
                    wgpu::RenderPassColorAttachmentDescriptor {
                        attachment: targets.color,
                        resolve_target: None,
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
            pass.set_bind_group(0, &self.global.bind_group, &[]);
            pass.set_bind_group(1, &self.object.bind_group, &[]);
            render::Render::draw_model(
                &mut pass,
                &self.model,
                &self.instance_buf,
            );

            self.debug_render.draw_shape(
                &mut pass,
                &self.model.shape,
                &self.instance_buf,
            );
        }

        encoder.finish()
    }
}
