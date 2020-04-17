use crate::{
    config::settings,
    space::Camera,
};
use bytemuck::{Pod, Zeroable};
use std::mem;


#[repr(C)]
#[derive(Clone, Copy)]
pub struct Constants {
    camera_pos: [f32; 4],
    m_vp: [[f32; 4]; 4],
    m_inv_vp: [[f32; 4]; 4],
    light_pos: [f32; 4],
    light_color: [f32; 4],
}
unsafe impl Pod for Constants {}
unsafe impl Zeroable for Constants {}

impl Constants {
    pub fn new(cam: &Camera, light: &settings::Light) -> Self {
        use cgmath::SquareMatrix;

        let mx_vp = cam.get_view_proj();
        Constants {
            camera_pos: cam.loc.extend(1.0).into(),
            m_vp: mx_vp.into(),
            m_inv_vp: mx_vp.invert().unwrap().into(),
            light_pos: light.pos,
            light_color: light.color,
        }
    }
}

pub struct Context {
    pub uniform_buf: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

impl Context {
    pub fn new(device: &wgpu::Device, store_buffer: wgpu::BindingResource) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Global"),
            bindings: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStage::all(),
                    ty: wgpu::BindingType::UniformBuffer { dynamic: false },
                },
                wgpu::BindGroupLayoutEntry { // palette sampler
                    binding: 1,
                    visibility: wgpu::ShaderStage::all(),
                    ty: wgpu::BindingType::Sampler { comparison: false },
                },
                wgpu::BindGroupLayoutEntry { // GPU store
                    binding: 2,
                    visibility: wgpu::ShaderStage::VERTEX,
                    ty: wgpu::BindingType::StorageBuffer {
                        dynamic: false,
                        readonly: true,
                    },
                },
            ],
        });
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uniform"),
            size: mem::size_of::<Constants>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsage::UNIFORM | wgpu::BufferUsage::COPY_DST,
        });
        let palette_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: 0.0,
            compare: wgpu::CompareFunction::Always,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Global"),
            layout: &bind_group_layout,
            bindings: &[
                wgpu::Binding {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer {
                        buffer: &uniform_buf,
                        range: 0 .. mem::size_of::<Constants>() as wgpu::BufferAddress,
                    },
                },
                wgpu::Binding {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&palette_sampler),
                },
                wgpu::Binding {
                    binding: 2,
                    resource: store_buffer,
                },
            ],
        });

        Context {
            uniform_buf,
            bind_group_layout,
            bind_group,
        }
    }
}
