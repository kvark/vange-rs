use crate::boilerplate::Application;
use vangers::{config, level, model, render, space};

use futures::executor::LocalSpawner;
use log::info;
use zerocopy::AsBytes as _;

use std::mem;


pub struct ResourceView {
    model: model::VisualModel,
    locals_buf: wgpu::Buffer,
    global: render::global::Context,
    object: render::object::Context,
    transform: space::Transform,
    cam: space::Camera,
    rotation: cgmath::Rad<f32>,
    light_config: config::settings::Light,
}

impl ResourceView {
    pub fn new(
        path: &str,
        settings: &config::settings::Settings,
        device: &wgpu::Device,
        queue: &mut wgpu::Queue,
    ) -> Self {
        info!("Initializing the render");
        let pal_data = level::read_palette(settings.open_palette(), None);
        let mut init_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            todo: 0,
        });
        let global = render::global::Context::new(device);
        let object = render::object::Context::new(&mut init_encoder, device, &pal_data, &global);
        queue.submit(&[
            init_encoder.finish(),
        ]);

        info!("Loading model {}", path);
        let file = settings.open_relative(path);
        let (model, locals_buf) = model::load_m3d(file, device, &object);

        ResourceView {
            model,
            locals_buf,
            global,
            object,
            transform: cgmath::Decomposed {
                scale: 1.0,
                disp: cgmath::Vector3::unit_z(),
                rot: cgmath::One::one(),
            },
            cam: space::Camera {
                loc: cgmath::vec3(0.0, -200.0, 100.0),
                rot: cgmath::Rotation3::from_angle_x::<cgmath::Rad<_>>(
                    cgmath::Angle::turn_div_6(),
                ),
                proj: space::Projection::Perspective(cgmath::PerspectiveFov {
                    fovy: cgmath::Deg(45.0).into(),
                    aspect: settings.window.size[0] as f32 / settings.window.size[1] as f32,
                    near: 5.0,
                    far: 400.0,
                }),
            },
            rotation: cgmath::Rad(0.),
            light_config: settings.render.light.clone(),
        }
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
                Key::A => self.rotation = -angle,
                Key::D => self.rotation = angle,
                _ => (),
            }
            KeyboardInput {
                state: ElementState::Released,
                virtual_keycode: Some(key),
                ..
            } => match key {
                Key::A | Key::D => self.rotation = cgmath::Rad(0.0),
                _ => (),
            }
            _ => {}
        }

        true
    }

    fn update(&mut self, _device: &wgpu::Device, delta: f32) -> Option<wgpu::CommandBuffer> {
        use cgmath::Transform;

        if self.rotation != cgmath::Rad(0.) {
            let angle = self.rotation * delta;
            let other = cgmath::Decomposed {
                scale: 1.0,
                rot: cgmath::Rotation3::from_angle_z(angle),
                disp: cgmath::Zero::zero(),
            };
            self.transform = other.concat(&self.transform);
        }

        None
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
    ) -> Vec<wgpu::CommandBuffer> {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            todo: 0,
        });
        let global_data = render::global::Constants::new(&self.cam, &self.light_config);
        let global_staging = device.create_buffer_with_data(
            [global_data].as_bytes(),
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
            locals_buf: &self.locals_buf,
            transform: self.transform,
            debug_shape_scale: None,
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
            );
        }

        vec![encoder.finish()]
    }
}
