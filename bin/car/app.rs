use crate::boilerplate::Application;
use m3d::Mesh;
use vangers::{config, level, model, render, space};

use futures::executor::LocalSpawner;
use log::info;

use std::mem;

pub struct CarView {
    model: model::VisualModel,
    transform: space::Transform,
    physics: config::car::CarPhysics,
    color: render::object::BodyColor,
    debug_render: render::debug::Context,
    global: render::global::Context,
    object: render::object::Context,
    cam: space::Camera,
    rotation: (cgmath::Rad<f32>, cgmath::Rad<f32>),
    light_config: config::settings::Light,
}

impl CarView {
    pub fn new(settings: &config::Settings, device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        info!("Initializing the render");
        let pal_data = level::read_palette(settings.open_palette(), None);
        let store_init = render::body::GpuStoreInit::new_dummy(device);
        let global = render::global::Context::new(device, queue, store_init.resource(), None);
        let object = render::object::Context::new(device, queue, &pal_data, &global);

        info!("Loading car registry");
        let game_reg = config::game::Registry::load(settings);
        let car_reg = config::car::load_registry(settings, &game_reg, device, &object);
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

        CarView {
            model,
            transform: cgmath::Decomposed {
                scale: cinfo.scale,
                disp: cgmath::Vector3::unit_z(),
                rot: cgmath::One::one(),
            },
            physics: cinfo.physics.clone(),
            color: settings.car.color,
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
                rot: cgmath::Rotation3::from_angle_x::<cgmath::Rad<_>>(cgmath::Angle::turn_div_6()),
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

    fn rotate_z(&mut self, angle: cgmath::Rad<f32>) {
        use cgmath::Transform;
        let other = cgmath::Decomposed {
            scale: 1.0,
            rot: cgmath::Rotation3::from_angle_z(angle),
            disp: cgmath::Zero::zero(),
        };
        self.transform = other.concat(&self.transform);
    }

    fn rotate_x(&mut self, angle: cgmath::Rad<f32>) {
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
            },
            KeyboardInput {
                state: ElementState::Released,
                virtual_keycode: Some(key),
                ..
            } => match key {
                Key::A | Key::D => self.rotation.0 = cgmath::Rad(0.),
                Key::W | Key::S => self.rotation.1 = cgmath::Rad(0.),
                _ => (),
            },
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
        self.cam
            .proj
            .update(extent.width as u16, extent.height as u16);
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
        let mut batcher = render::Batcher::new();
        batcher.add_model(
            &self.model,
            &self.transform,
            Some(self.physics.scale_bound),
            &render::body::GpuBody::ZERO,
            self.color,
        );
        batcher.prepare(device);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Draw"),
        });
        let global_data = render::global::Constants::new(&self.cam, &self.light_config, None);
        let global_staging = device.create_buffer_with_data(
            bytemuck::bytes_of(&global_data),
            wgpu::BufferUsage::COPY_SRC,
        );
        encoder.copy_buffer_to_buffer(
            &global_staging,
            0,
            &self.global.uniform_buf,
            0,
            mem::size_of::<render::global::Constants>() as wgpu::BufferAddress,
        );

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[wgpu::RenderPassColorAttachmentDescriptor {
                    attachment: targets.color,
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
                }],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachmentDescriptor {
                    attachment: targets.depth,
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

            let _ = &self.debug_render;
            /*TODO:
            self.debug_render.draw_shape(
                &mut pass,
                &self.model.shape,
                &self.instance_buf,
                0,
            );*/
        }

        encoder.finish()
    }
}
