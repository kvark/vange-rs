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
    light_color: [f32; 4],
}
unsafe impl Pod for Constants {}
unsafe impl Zeroable for Constants {}

impl Constants {
    pub fn new(cam: &Camera, light: &settings::Light, shadow_cam: Option<&Camera>) -> Self {
        use cgmath::SquareMatrix;

        let m_light_vp = shadow_cam
            .map_or_else(cgmath::Matrix4::identity, |sc| sc.get_view_proj())
            .into();
        let mx_vp = cam.get_view_proj();
        Constants {
            camera_pos: cam.loc.extend(1.0).into(),
            m_vp: mx_vp.into(),
            m_inv_vp: mx_vp.invert().unwrap().into(),
            m_light_vp,
            light_pos: light.pos,
            light_color: light.color,
        }
    }
}

pub struct Context {
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub uniform_buf: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    pub shadow_bind_group: wgpu::BindGroup,
}

impl Context {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        store_buffer: wgpu::BindingResource,
        shadow_view: Option<&wgpu::TextureView>,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Global"),
            bindings: &[
                wgpu::BindGroupLayoutEntry::new(
                    0,
                    wgpu::ShaderStage::all(),
                    wgpu::BindingType::UniformBuffer {
                        dynamic: false,
                        min_binding_size: None,
                    },
                ),
                // palette sampler
                wgpu::BindGroupLayoutEntry::new(
                    1,
                    wgpu::ShaderStage::all(),
                    wgpu::BindingType::Sampler { comparison: false },
                ),
                // GPU store
                wgpu::BindGroupLayoutEntry::new(
                    2,
                    wgpu::ShaderStage::VERTEX,
                    wgpu::BindingType::StorageBuffer {
                        dynamic: false,
                        readonly: true,
                        min_binding_size: None,
                    },
                ),
                // shadow texture
                wgpu::BindGroupLayoutEntry::new(
                    3,
                    wgpu::ShaderStage::FRAGMENT,
                    wgpu::BindingType::SampledTexture {
                        dimension: wgpu::TextureViewDimension::D2,
                        component_type: wgpu::TextureComponentType::Float,
                        multisampled: false,
                    },
                ),
                // shadow sampler
                wgpu::BindGroupLayoutEntry::new(
                    4,
                    wgpu::ShaderStage::FRAGMENT,
                    wgpu::BindingType::Sampler { comparison: true },
                ),
            ],
        });
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uniform"),
            size: mem::size_of::<Constants>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsage::UNIFORM | wgpu::BufferUsage::COPY_DST,
            mapped_at_creation: false,
        });
        let palette_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
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
                depth: 1,
            };
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("DummyShadow"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsage::SAMPLED
                    | wgpu::TextureUsage::OUTPUT_ATTACHMENT
                    | wgpu::TextureUsage::COPY_DST,
            });
            queue.write_texture(
                wgpu::TextureCopyView {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                },
                &[0, 0, 128, 63], // f32 1.0
                wgpu::TextureDataLayout {
                    offset: 0,
                    bytes_per_row: 4,
                    rows_per_image: 0,
                },
                size,
            );
            texture.create_default_view()
        };

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Global"),
            layout: &bind_group_layout,
            bindings: &[
                wgpu::Binding {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(uniform_buf.slice(..)),
                },
                wgpu::Binding {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&palette_sampler),
                },
                wgpu::Binding {
                    binding: 2,
                    //TODO: just clone
                    resource: match store_buffer {
                        wgpu::BindingResource::Buffer(slice) => {
                            wgpu::BindingResource::Buffer(slice.clone())
                        }
                        _ => unreachable!(),
                    },
                },
                wgpu::Binding {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(
                        shadow_view.unwrap_or(&dummy_shadow_view),
                    ),
                },
                wgpu::Binding {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
            ],
        });
        let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("GlobalShadow"),
            layout: &bind_group_layout,
            bindings: &[
                wgpu::Binding {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(uniform_buf.slice(..)),
                },
                wgpu::Binding {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&palette_sampler),
                },
                wgpu::Binding {
                    binding: 2,
                    resource: store_buffer,
                },
                wgpu::Binding {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&dummy_shadow_view),
                },
                wgpu::Binding {
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
        }
    }
}
