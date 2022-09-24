use crate::boilerplate::Application;
use vangers::{config, level, model, render, space};

use futures::executor::LocalSpawner;
use log::info;
use wgpu::util::DeviceExt as _;

use std::mem;

pub struct ResourceView {
    model: model::VisualModel,
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
        queue: &wgpu::Queue,
        downlevel_caps: &wgpu::DownlevelCapabilities,
        color_format: wgpu::TextureFormat,
    ) -> Self {
        let cam = space::Camera {
            loc: cgmath::vec3(0.0, -200.0, 100.0),
            rot: cgmath::Rotation3::from_angle_x::<cgmath::Rad<_>>(cgmath::Angle::turn_div_6()),
            scale: cgmath::vec3(1.0, -1.0, 1.0),
            proj: space::Projection::Perspective(cgmath::PerspectiveFov {
                fovy: cgmath::Deg(45.0).into(),
                aspect: settings.window.size[0] as f32 / settings.window.size[1] as f32,
                near: 5.0,
                far: 400.0,
            }),
        };

        info!("Initializing the render");
        let pal_data = level::read_palette(settings.open_palette(), None);
        let global = render::global::Context::new(device, queue, color_format, None);
        let object = render::object::Context::new(
            device,
            queue,
            downlevel_caps,
            cam.front_face(),
            &pal_data,
            &global,
        );

        info!("Loading model {}", path);
        let file = settings.open_relative(path);
        let model = model::load_m3d(file, device, &object, settings.game.physics.shape_sampling);

        ResourceView {
            model,
            global,
            object,
            transform: cgmath::Decomposed {
                scale: 1.0,
                disp: cgmath::Vector3::unit_z(),
                rot: cgmath::One::one(),
            },
            cam,
            rotation: cgmath::Rad(0.),
            light_config: settings.render.light,
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
            },
            KeyboardInput {
                state: ElementState::Released,
                virtual_keycode: Some(key),
                ..
            } => match key {
                Key::A | Key::D => self.rotation = cgmath::Rad(0.0),
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
            None,
            render::object::BodyColor::Dummy,
        );
        batcher.prepare(device);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Draw"),
        });
        let global_data = render::global::Constants::new(&self.cam, &self.light_config, None);
        let global_staging = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::bytes_of(&global_data),
            usage: wgpu::BufferUsages::COPY_SRC,
        });
        encoder.copy_buffer_to_buffer(
            &global_staging,
            0,
            &self.global.uniform_buf,
            0,
            mem::size_of::<render::global::Constants>() as wgpu::BufferAddress,
        );

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
