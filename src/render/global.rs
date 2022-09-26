use crate::{config::settings, space::Camera};
use bytemuck::{Pod, Zeroable};
use std::mem;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Constants {
    camera_pos: [f32; 4],
    m_vp: [[f32; 4]; 4],
    m_inv_vp: [[f32; 4]; 4],
    m_light_vp: [[f32; 4]; 4],
    light_pos: [f32; 4],
    light_color: [f32; 3],
    pad: f32,
}
unsafe impl Pod for Constants {}
unsafe impl Zeroable for Constants {}

impl Constants {
    pub fn new(cam: &Camera, light: &settings::Light, shadow_cam: Option<&Camera>) -> Self {
        use cgmath::{SquareMatrix, Zero};

        //Note: zero matrix is handled specially in `fetch_shadow` shader
        let m_light_vp = shadow_cam
            .map_or_else(cgmath::Matrix4::zero, |sc| sc.get_view_proj())
            .into();
        let mx_vp = cam.get_view_proj();
        Constants {
            camera_pos: cam.loc.extend(1.0).into(),
            m_vp: mx_vp.into(),
            m_inv_vp: mx_vp.invert().unwrap().into(),
            m_light_vp,
            light_pos: light.pos,
            light_color: light.color,
            pad: 1.0,
        }
    }
}

pub struct Context {
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub uniform_buf: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    pub shadow_bind_group: wgpu::BindGroup,
    pub color_format: wgpu::TextureFormat,
}

impl Context {
    pub fn new(gfx: &super::GraphicsContext, shadow_view: Option<&wgpu::TextureView>) -> Self {
        let bind_group_layout =
            gfx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Global"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::all(),
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        // palette sampler
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::all(),
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                        // shadow texture
                        wgpu::BindGroupLayoutEntry {
                            binding: 3,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Depth,
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        // shadow sampler
                        wgpu::BindGroupLayoutEntry {
                            binding: 4,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                            count: None,
                        },
                    ],
                });
        let uniform_buf = gfx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uniform"),
            size: mem::size_of::<Constants>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let palette_sampler = gfx.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let shadow_sampler = gfx.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let dummy_shadow_view = {
            let size = wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            };
            let texture = gfx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("DummyShadow"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            let mut cmd_encoder = gfx.device.create_command_encoder(&Default::default());
            cmd_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow clear"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: true,
                    }),
                    stencil_ops: None,
                }),
            });
            gfx.queue.submit(Some(cmd_encoder.finish()));
            view
        };

        let bind_group = gfx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Global"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&palette_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(
                        shadow_view.unwrap_or(&dummy_shadow_view),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
            ],
        });
        let shadow_bind_group = gfx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("GlobalShadow"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&palette_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&dummy_shadow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
            ],
        });

        Context {
            bind_group_layout,
            uniform_buf,
            bind_group,
            shadow_bind_group,
            color_format: gfx.color_format,
        }
    }
}
