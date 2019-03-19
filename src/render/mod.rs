use crate::{
    config::settings,
    level,
    model,
    space::{Camera, Transform},
};

use cgmath::{Decomposed, Matrix4};
use wgpu;

use std::io::Error as IoError;
use std::mem;


//mod collision; ../TODO
mod debug;
pub mod global;
pub mod object;
pub mod terrain;

//pub use self::collision::{DebugBlit, GpuCollider, ShapeId};
pub use self::debug::{DebugPos, DebugRender, LineBuffer};


pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::D32Float;

pub struct ScreenTargets<'a> {
    pub extent: wgpu::Extent3d,
    pub color: &'a wgpu::TextureView,
    pub depth: &'a wgpu::TextureView,
}

pub struct SurfaceData {
    pub constants: wgpu::Buffer,
    pub height: (wgpu::TextureView, wgpu::Sampler),
    pub meta: (wgpu::TextureView, wgpu::Sampler),
}

pub type ShapeVertex = [f32; 4];

#[derive(Clone, Copy)]
pub struct ShapePolygon {
    pub indices: [u16; 4],
    pub normal: [i8; 4],
    pub origin_square: [f32; 4],
}

pub struct Updater<'a> {
    command_encoder: wgpu::CommandEncoder,
    device: &'a wgpu::Device,
}

impl<'a> Updater<'a> {
    pub fn new(device: &'a wgpu::Device) -> Self {
        Updater {
            command_encoder: device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                todo: 0,
            }),
            device,
        }
    }

    pub fn update<T: 'static + Copy>(&mut self, buffer: &wgpu::Buffer, data: &[T]) {
        let staging = self.device
            .create_buffer_mapped(data.len(), wgpu::BufferUsageFlags::TRANSFER_SRC)
            .fill_from_slice(data);
        self.command_encoder.copy_buffer_to_buffer(
            &staging,
            0,
            buffer,
            0,
            mem::size_of::<T>() as u32,
        );
    }

    pub fn finish(self) -> wgpu::CommandBuffer {
        self.command_encoder.finish()
    }
}


pub struct Shaders {
    vs: wgpu::ShaderModule,
    fs: wgpu::ShaderModule,
}

impl Shaders {
    pub fn new(
        name: &str,
        specialization: &[&str],
        device: &wgpu::Device,
    ) -> Result<Self, IoError> {
        use glsl_to_spirv;
        use std::fs::File;
        use std::io::{BufReader, Read, Write};
        use std::path::PathBuf;

        let path = PathBuf::from("data")
            .join("shader")
            .join(name)
            .with_extension("glsl");
        if !path.is_file() {
            panic!("Shader not found: {:?}", path);
        }

        let mut buf_vs = b"#version 450\n#define SHADER_VS\n".to_vec();
        let mut buf_fs = b"#version 450\n#define SHADER_FS\n".to_vec();

        let mut code = String::new();
        BufReader::new(File::open(&path)?)
            .read_to_string(&mut code)?;
        // parse meta-data
        {
            let mut lines = code.lines();
            let first = lines.next().unwrap();
            if first.starts_with("//!include") {
                for include_pair in first.split_whitespace().skip(1) {
                    let mut temp = include_pair.split(':');
                    let target = match temp.next().unwrap() {
                        "vs" => &mut buf_vs,
                        "fs" => &mut buf_fs,
                        other => panic!("Unknown target: {}", other),
                    };
                    let include = temp.next().unwrap();
                    let inc_path = path
                        .with_file_name(include)
                        .with_extension("inc.glsl");
                    BufReader::new(File::open(inc_path)?)
                        .read_to_end(target)?;
                }
            }
            let second = lines.next().unwrap();
            if second.starts_with("//!specialization") {
                for define in second.split_whitespace().skip(1) {
                    let value = if specialization.contains(&define) {
                        1
                    } else {
                        0
                    };
                    write!(buf_vs, "#define {} {}\n", define, value)?;
                    write!(buf_fs, "#define {} {}\n", define, value)?;
                }
            }
        }

        write!(buf_vs, "\n{}", code
            .replace("attribute", "in")
            .replace("varying", "out")
        )?;
        write!(buf_fs, "\n{}", code
            .replace("varying", "in")
        )?;

        let str_vs = String::from_utf8_lossy(&buf_vs);
        let str_fs = String::from_utf8_lossy(&buf_fs);
        debug!("vs:\n{}", str_vs);
        debug!("fs:\n{}", str_fs);

        let mut output_vs = match glsl_to_spirv::compile(&str_vs, glsl_to_spirv::ShaderType::Vertex) {
            Ok(file) => file,
            Err(e) => {
                println!("Generated VS shader:\n{}", str_vs);
                panic!("\nUnable to compile '{}': {:?}", name, e);
            }
        };
        let mut spv_vs = Vec::new();
        output_vs.read_to_end(&mut spv_vs).unwrap();

        let mut output_fs = match glsl_to_spirv::compile(&str_fs, glsl_to_spirv::ShaderType::Fragment) {
            Ok(file) => file,
            Err(e) => {
                println!("Generated FS shader:\n{}", str_fs);
                panic!("\nUnable to compile '{}': {:?}", name, e);
            }
        };
        let mut spv_fs = Vec::new();
        output_fs.read_to_end(&mut spv_fs).unwrap();

        Ok(Shaders {
            vs: device.create_shader_module(&spv_vs),
            fs: device.create_shader_module(&spv_fs),
        })
    }
}


pub struct Palette {
    pub view: wgpu::TextureView,
}

impl Palette {
    pub fn new(
        encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
        data: &[[u8; 4]],
    ) -> Self {
        let extent = wgpu::Extent3d {
            width: 0x100,
            height: 1,
            depth: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            size: extent,
            array_size: 1,
            dimension: wgpu::TextureDimension::D1,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsageFlags::SAMPLED | wgpu::TextureUsageFlags::TRANSFER_DST,
        });

        let staging = device
            .create_buffer_mapped(data.len(), wgpu::BufferUsageFlags::TRANSFER_SRC)
            .fill_from_slice(data);
        encoder.copy_buffer_to_texture(
            wgpu::BufferCopyView {
                buffer: &staging,
                offset: 0,
                row_pitch: 0x100 * 4,
                image_height: 1,
            },
            wgpu::TextureCopyView {
                texture: &texture,
                level: 0,
                slice: 0,
                origin: wgpu::Origin3d {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
            },
            extent,
        );

        Palette {
            view: texture.create_default_view(),
        }
    }
}


pub struct Render {
    global: global::Context,
    object: object::Context,
    terrain: terrain::Context,
    pub light_config: settings::Light,
    pub debug: debug::DebugRender,
}

pub struct RenderModel<'a> {
    pub model: &'a model::RenderModel,
    pub transform: Transform,
    pub debug_shape_scale: Option<f32>,
}

impl Render {
    pub fn new(
        device: &mut wgpu::Device,
        level: &level::Level,
        object_palette: &[[u8; 4]],
        settings: &settings::Render,
    ) -> Self {
        let mut init_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            todo: 0,
        });
        let global = global::Context::new(device);
        let object = object::Context::new(&mut init_encoder, device, object_palette, &global);
        let terrain = terrain::Context::new(&mut init_encoder, device, level, &global);
        device.get_queue().submit(&[
            init_encoder.finish(),
        ]);

        Render {
            global,
            object,
            terrain,
            light_config: settings.light.clone(),
            debug: DebugRender::new(device, &settings.debug),
        }
    }

    pub fn draw_mesh(
        pass: &mut wgpu::RenderPass,
        updater: &mut Updater,
        mesh: &model::Mesh,
        model2world: Transform,
        locals_buf: &wgpu::Buffer,
    ) {
        let mx_world = Matrix4::from(model2world);
        updater.update(locals_buf, &[object::Locals {
            matrix: mx_world.into(),
        }]);
        pass.set_vertex_buffers(&[(&mesh.vertex_buf, 0)]);
        pass.draw(0 .. mesh.num_vertices as u32, 0 .. 1);
    }

    pub fn draw_model(
        pass: &mut wgpu::RenderPass,
        updater: &mut Updater,
        model: &model::RenderModel,
        model2world: Transform,
        locals_buf: &wgpu::Buffer,
        debug_context: Option<(&mut DebugRender, f32, &Matrix4<f32>)>,
    ) {
        use cgmath::{Deg, One, Quaternion, Rad, Rotation3, Transform, Vector3};

        // body
        Render::draw_mesh(pass, updater, &model.body, model2world.clone(), locals_buf);
        // debug render
        if let Some((debug, scale, world2screen)) = debug_context {
            let mut mx_shape =  model2world.clone();
            mx_shape.scale *= scale;
            let transform = world2screen * Matrix4::from(mx_shape);
            debug.draw_shape(pass, &model.shape, transform);
        }
        // wheels
        for w in model.wheels.iter() {
            if let Some(ref mesh) = w.mesh {
                let transform = model2world.concat(&Decomposed {
                    disp: mesh.offset.into(),
                    rot: Quaternion::one(),
                    scale: 1.0,
                });
                Render::draw_mesh(pass, updater, mesh, transform, locals_buf);
            }
        }
        // slots
        for s in model.slots.iter() {
            if let Some(ref mesh) = s.mesh {
                let mut local = Decomposed {
                    disp: Vector3::new(s.pos[0] as f32, s.pos[1] as f32, s.pos[2] as f32),
                    rot: Quaternion::from_angle_y(Rad::from(Deg(s.angle as f32))),
                    scale: s.scale / model2world.scale,
                };
                local.disp -= local.transform_vector(Vector3::from(mesh.offset));
                let transform = model2world.concat(&local);
                Render::draw_mesh(pass, updater, mesh, transform, locals_buf);
            }
        }
    }

    pub fn draw_world<'a>(
        &mut self,
        render_models: &[RenderModel<'a>],
        cam: &Camera,
        targets: ScreenTargets,
        device: &wgpu::Device,
    ) -> Vec<wgpu::CommandBuffer> {
        let mx_vp = cam.get_view_proj();

        let mut updater = Updater::new(device);
        updater.update(&self.global.uniform_buf, &[
            global::Constants::new(cam, &self.light_config),
        ]);
        updater.update(&self.terrain.uniform_buf, &[
            terrain::Constants::new(&targets.extent)
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

            pass.set_bind_group(0, &self.global.bind_group);
            pass.set_bind_group(1, &self.terrain.bind_group);
            // draw terrain
            match self.terrain.kind {
                terrain::Kind::Ray { ref pipeline, ref index_buf, ref vertex_buf, num_indices } => {
                    pass.set_pipeline(pipeline);
                    pass.set_index_buffer(index_buf, 0);
                    pass.set_vertex_buffers(&[(vertex_buf, 0)]);
                    pass.draw_indexed(0 .. num_indices as u32, 0, 0 .. 1);
                }
                /*
                Terrain::Tess { ref low, ref high, .. } => {
                    encoder.draw(&self.terrain_slice, low, &self.terrain_data);
                    encoder.draw(&self.terrain_slice, high, &self.terrain_data);
                }*/
            }

            pass.set_pipeline(&self.object.pipeline);
            pass.set_bind_group(1, &self.object.bind_group);
            // draw vehicle models
            for rm in render_models {
                Render::draw_model(
                    &mut pass,
                    &mut updater,
                    &rm.model,
                    rm.transform,
                    &self.object.uniform_buf,
                    //&mut self.object_data,
                    match rm.debug_shape_scale {
                        Some(scale) => Some((&mut self.debug, scale, &mx_vp)),
                        None => None,
                    },
                );
            }
        }

        vec![
            updater.finish(),
            encoder.finish(),
        ]
    }

    pub fn reload(&mut self, device: &wgpu::Device) {
        info!("Reloading shaders");
        self.object.reload(device);
        self.terrain.reload(device);
    }

    /*
    pub fn surface_data(&self) -> SurfaceData {
        SurfaceData {
            constants: self.terrain_data.suf_constants.clone(),
            height: self.terrain_data.height.clone(),
            meta: self.terrain_data.meta.clone(),
        }
    }*/

    /*
    pub fn target_color(&self) -> gfx::handle::RenderTargetView<R, ColorFormat> {
        self.terrain_data.out_color.clone()
    }*/
}
