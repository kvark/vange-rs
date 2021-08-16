use crate::render::terrain::{Rect, HEIGHT_FORMAT};
use bytemuck::{Pod, Zeroable};
use std::{mem, num::NonZeroU32};
use wgpu::util::DeviceExt as _;

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    _pos: [f32; 2],
}
unsafe impl Pod for Vertex {}
unsafe impl Zeroable for Vertex {}

struct Mip {
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

pub struct MaxMipper {
    size: wgpu::Extent3d,
    pipeline_layout: wgpu::PipelineLayout,
    pipeline: wgpu::RenderPipeline,
    //data: terrain_mip::Data<R>,
    mips: Vec<Mip>,
}

impl MaxMipper {
    fn create_pipeline(
        layout: &wgpu::PipelineLayout,
        device: &wgpu::Device,
    ) -> wgpu::RenderPipeline {
        let shader = super::load_shader("terrain/mip", device).unwrap();
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mipmap"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vertex",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: mem::size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        offset: 0,
                        format: wgpu::VertexFormat::Float32x2,
                        shader_location: 0,
                    }],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fragment",
                targets: &[HEIGHT_FORMAT.into()],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
        })
    }

    pub fn new(
        texture: &wgpu::Texture,
        size: wgpu::Extent3d,
        mip_count: u32,
        device: &wgpu::Device,
    ) -> Self {
        let bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("MaxMipper"),
            entries: &[
                // texture
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mipmap"),
            bind_group_layouts: &[&bg_layout],
            push_constant_ranges: &[],
        });

        let mut mips = Vec::with_capacity(mip_count as usize);
        for level in 0..mip_count {
            let view = texture.create_view(&wgpu::TextureViewDescriptor {
                label: None,
                format: None,
                dimension: None,
                aspect: wgpu::TextureAspect::All,
                base_mip_level: level,
                mip_level_count: NonZeroU32::new(1),
                base_array_layer: 0,
                array_layer_count: NonZeroU32::new(1),
            });

            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("MaxMipper"),
                layout: &bg_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                }],
            });

            mips.push(Mip { view, bind_group });
        }

        let pipeline = Self::create_pipeline(&pipeline_layout, device);

        MaxMipper {
            size,
            pipeline_layout,
            pipeline,
            mips,
        }
    }

    pub fn update(
        &self,
        rects: &[Rect],
        encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
    ) {
        let mut vertex_data = Vec::with_capacity(rects.len() * 6);
        for r in rects.iter() {
            let v_abs = [
                (r.x, r.y),
                (r.x + r.w, r.y),
                (r.x, r.y + r.h),
                (r.x, r.y + r.h),
                (r.x + r.w, r.y),
                (r.x + r.w, r.y + r.h),
            ];
            for &(x, y) in v_abs.iter() {
                vertex_data.push(Vertex {
                    _pos: [
                        x as f32 / self.size.width as f32,
                        y as f32 / self.size.height as f32,
                    ],
                });
            }
        }
        let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mipmap-vertex"),
            contents: bytemuck::cast_slice(&vertex_data),
            usage: wgpu::BufferUsages::VERTEX,
        });

        for mip in 0..self.mips.len() - 1 {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mipmap"),
                color_attachments: &[wgpu::RenderPassColorAttachment {
                    view: &self.mips[mip + 1].view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: true,
                    },
                }],
                depth_stencil_attachment: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.mips[mip].bind_group, &[]);
            pass.set_vertex_buffer(0, vertex_buf.slice(..));
            pass.draw(0..rects.len() as u32 * 6, 0..1);
        }
    }

    pub fn reload(&mut self, device: &wgpu::Device) {
        self.pipeline = Self::create_pipeline(&self.pipeline_layout, device);
    }
}
