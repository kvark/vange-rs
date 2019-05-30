use crate::{
    config::settings,
    space::Camera,
};
use wgpu;

use std::mem;


#[derive(Clone, Copy)]
pub struct Constants {
    _camera_pos: [f32; 4],
    _m_vp: [[f32; 4]; 4],
    _m_inv_vp: [[f32; 4]; 4],
    _light_pos: [f32; 4],
    _light_color: [f32; 4],
}

impl Constants {
    pub fn new(cam: &Camera, light: &settings::Light) -> Self {
        use cgmath::SquareMatrix;

        let mx_vp = cam.get_view_proj();
        Constants {
            _camera_pos: cam.loc.extend(1.0).into(),
            _m_vp: mx_vp.into(),
            _m_inv_vp: mx_vp.invert().unwrap().into(),
            _light_pos: light.pos,
            _light_color: light.color,
        }
    }
}

pub struct Context {
    pub uniform_buf: wgpu::Buffer,
    pub palette_sampler: wgpu::Sampler,
    pub bind_group: wgpu::BindGroup,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

impl Context {
    pub fn new(device: &wgpu::Device) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            bindings: &[
                wgpu::BindGroupLayoutBinding {
                    binding: 0,
                    visibility: wgpu::ShaderStage::VERTEX | wgpu::ShaderStage::FRAGMENT,
                    ty: wgpu::BindingType::UniformBuffer,
                },
                wgpu::BindGroupLayoutBinding { // palette sampler
                    binding: 1,
                    visibility: wgpu::ShaderStage::VERTEX | wgpu::ShaderStage::FRAGMENT,
                    ty: wgpu::BindingType::Sampler,
                },
            ],
        });
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            size: mem::size_of::<Constants>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsage::UNIFORM | wgpu::BufferUsage::TRANSFER_DST,
        });
        let palette_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: 0.0,
            compare_function: wgpu::CompareFunction::Always,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
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
            ],
        });

        Context {
            uniform_buf,
            palette_sampler,
            bind_group_layout,
            bind_group,
        }
    }
}
