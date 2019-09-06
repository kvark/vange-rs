use crate::{
    config::settings,
    level,
    model,
    space::{Camera, Transform},
};

use glsl_to_spirv;
use cgmath::Decomposed;
use wgpu;

use std::{
    io::{BufReader, Read, Write, Error as IoError},
    fs::File,
    mem,
    path::PathBuf,
};

//mod collision; ../TODO
pub mod debug;
pub mod global;
pub mod mipmap;
pub mod object;
pub mod terrain;

//pub use self::collision::{DebugBlit, GpuCollider, ShapeId};


pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

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
        let base_path = PathBuf::from("data").join("shader");
        let path = base_path.join(name).with_extension("glsl");
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
                    let inc_path = base_path
                        .join(include)
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

        let spv_vs = match glsl_to_spirv::compile(&str_vs, glsl_to_spirv::ShaderType::Vertex) {
            Ok(file) => wgpu::read_spirv(file).unwrap(),
            Err(e) => {
                println!("Generated VS shader:\n{}", str_vs);
                panic!("\nUnable to compile '{}': {:?}", name, e);
            }
        };
        let spv_fs = match glsl_to_spirv::compile(&str_fs, glsl_to_spirv::ShaderType::Fragment) {
            Ok(file) => wgpu::read_spirv(file).unwrap(),
            Err(e) => {
                println!("Generated FS shader:\n{}", str_fs);
                panic!("\nUnable to compile '{}': {:?}", name, e);
            }
        };

        Ok(Shaders {
            vs: device.create_shader_module(&spv_vs),
            fs: device.create_shader_module(&spv_fs),
        })
    }

    pub fn new_compute(
        name: &str,
        group_size: [u32; 3],
        specialization: &[&str],
        device: &wgpu::Device,
    ) -> Result<wgpu::ShaderModule, IoError> {
        let base_path = PathBuf::from("data").join("shader");
        let path = base_path.join(name).with_extension("glsl");
        if !path.is_file() {
            panic!("Shader not found: {:?}", path);
        }

        let mut buf = b"#version 450\n".to_vec();
        write!(buf, "layout(local_size_x = {}, local_size_y = {}, local_size_z = {}) in;\n",
            group_size[0], group_size[1], group_size[2])?;
        write!(buf, "#define SHADER_CS\n")?;

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
                        "cs" => &mut buf,
                        other => panic!("Unknown target: {}", other),
                    };
                    let include = temp.next().unwrap();
                    let inc_path = base_path
                        .join(include)
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
                    write!(buf, "#define {} {}\n", define, value)?;
                }
            }
        }

        write!(buf, "\n{}", code)?;
        let str_cs = String::from_utf8_lossy(&buf);
        debug!("cs:\n{}", str_cs);

        let spv = match glsl_to_spirv::compile(&str_cs, glsl_to_spirv::ShaderType::Compute) {
            Ok(file) => wgpu::read_spirv(file).unwrap(),
            Err(e) => {
                println!("Generated CS shader:\n{}", str_cs);
                panic!("\nUnable to compile '{}': {:?}", name, e);
            }
        };

        Ok(device.create_shader_module(&spv))

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
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D1,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsage::SAMPLED | wgpu::TextureUsage::COPY_DST,
        });

        let staging = device
            .create_buffer_mapped(data.len(), wgpu::BufferUsage::COPY_SRC)
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
                mip_level: 0,
                array_layer: 0,
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


pub struct RenderModel<'a> {
    pub model: &'a model::VisualModel,
    pub locals_buf: &'a wgpu::Buffer,
    pub transform: Transform,
    pub debug_shape_scale: Option<f32>,
}

impl<'a> RenderModel<'a> {
    pub fn prepare(&self, encoder: &mut wgpu::CommandEncoder, device: &wgpu::Device) {
        use cgmath::{Deg, One, Quaternion, Rad, Rotation3, Transform, Vector3};

        let count = self.model.mesh_count();
        let mapping = device.create_buffer_mapped(
            count,
            wgpu::BufferUsage::COPY_SRC,
        );

        // body
        mapping.data[0] = object::Locals::new(self.transform);
        // wheels
        for w in self.model.wheels.iter() {
            if let Some(ref mesh) = w.mesh {
                let transform = self.transform.concat(&Decomposed {
                    disp: mesh.offset.into(),
                    rot: Quaternion::one(),
                    scale: 1.0,
                });
                mapping.data[mesh.locals_id] = object::Locals::new(transform);
            }
        }
        // slots
        for s in self.model.slots.iter() {
            if let Some(ref mesh) = s.mesh {
                let mut local = Decomposed {
                    disp: Vector3::new(s.pos[0] as f32, s.pos[1] as f32, s.pos[2] as f32),
                    rot: Quaternion::from_angle_y(Rad::from(Deg(s.angle as f32))),
                    scale: s.scale / self.transform.scale,
                };
                local.disp -= local.transform_vector(Vector3::from(mesh.offset));
                let transform = self.transform.concat(&local);
                mapping.data[mesh.locals_id] = object::Locals::new(transform);
            }
        }

        encoder.copy_buffer_to_buffer(
            &mapping.finish(),
            0,
            &self.locals_buf,
            0,
            (count * mem::size_of::<object::Locals>()) as wgpu::BufferAddress,
        );
    }
}

pub struct Render {
    global: global::Context,
    object: object::Context,
    terrain: terrain::Context,
    pub debug: debug::Context,
    pub light_config: settings::Light,
}

impl Render {
    pub fn new(
        device: &mut wgpu::Device,
        level: &level::Level,
        object_palette: &[[u8; 4]],
        settings: &settings::Render,
        screen_extent: wgpu::Extent3d,
    ) -> Self {
        let mut init_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            todo: 0,
        });
        let global = global::Context::new(device);
        let object = object::Context::new(&mut init_encoder, device, object_palette, &global);
        let terrain = terrain::Context::new(&mut init_encoder, device, level, &global, &settings.terrain, screen_extent);
        let debug = debug::Context::new(device, &settings.debug, &global);
        device.get_queue().submit(&[
            init_encoder.finish(),
        ]);

        Render {
            global,
            object,
            terrain,
            debug,
            light_config: settings.light.clone(),
        }
    }

    pub fn locals_layout(&self) -> &wgpu::BindGroupLayout {
        &self.object.part_bind_group_layout
    }

    pub fn draw_mesh(
        pass: &mut wgpu::RenderPass,
        mesh: &model::Mesh,
    ) {
        pass.set_bind_group(2, &mesh.bind_group, &[]);
        pass.set_vertex_buffers(0, &[(&mesh.vertex_buf, 0)]);
        pass.draw(0 .. mesh.num_vertices as u32, 0 .. 1);
    }

    pub fn draw_model(
        pass: &mut wgpu::RenderPass,
        model: &model::VisualModel,
    ) {
        // body
        Render::draw_mesh(pass, &model.body);
        // wheels
        for w in model.wheels.iter() {
            if let Some(ref mesh) = w.mesh {
                Render::draw_mesh(pass, mesh);
            }
        }
        // slots
        for s in model.slots.iter() {
            if let Some(ref mesh) = s.mesh {
                Render::draw_mesh(pass, mesh);
            }
        }
    }

    pub fn draw_world<'a>(
        &mut self,
        render_models: &[RenderModel<'a>],
        cam: &Camera,
        targets: ScreenTargets,
        device: &wgpu::Device,
    ) -> wgpu::CommandBuffer {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            todo: 0,
        });

        let global_staging = device
            .create_buffer_mapped(1, wgpu::BufferUsage::COPY_SRC)
            .fill_from_slice(&[
                global::Constants::new(cam, &self.light_config),
            ]);
        encoder.copy_buffer_to_buffer(
            &global_staging,
            0,
            &self.global.uniform_buf,
            0,
            mem::size_of::<global::Constants>() as wgpu::BufferAddress,
        );

        for rm in render_models {
            rm.prepare(&mut encoder, device);
        }

        self.terrain.prepare(&mut encoder, device, &self.global, cam);

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

            pass.set_bind_group(0, &self.global.bind_group, &[]);
            self.terrain.draw(&mut pass);

            // draw vehicle models
            pass.set_pipeline(&self.object.pipeline);
            pass.set_bind_group(1, &self.object.bind_group, &[]);
            for rm in render_models {
                Render::draw_model(&mut pass, &rm.model);
            }
            for rm in render_models {
                if let Some(_scale) = rm.debug_shape_scale {
                    self.debug.draw_shape(&mut pass, &rm.model.shape);
                }
            }
        }

        encoder.finish()
    }

    pub fn reload(&mut self, device: &wgpu::Device) {
        info!("Reloading shaders");
        self.object.reload(device);
        self.terrain.reload(device);
    }

    pub fn resize(&mut self, extent: wgpu::Extent3d, device: &wgpu::Device) {
        self.terrain.resize(extent, device);
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
