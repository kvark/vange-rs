use crate::{
    config::settings,
    level::HEIGHT_SCALE,
    space::{Camera, Projection},
};

use cgmath::{EuclideanSpace as _, InnerSpace as _, Rotation as _};

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;

pub struct Shadow {
    pub(super) view: wgpu::TextureView,
    pub(super) cam: Camera,
    pub(super) size: u32,
    dir: cgmath::Vector3<f32>,
}

impl Shadow {
    pub(super) fn new(light: &settings::Light, device: &wgpu::Device) -> Self {
        let size = light.shadow_size;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Shadow"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsage::SAMPLED | wgpu::TextureUsage::OUTPUT_ATTACHMENT,
        });

        let dir = cgmath::Vector4::from(light.pos).truncate();
        let up = if dir.x == 0.0 && dir.y == 0.0 {
            cgmath::Vector3::unit_y()
        } else {
            cgmath::Vector3::unit_z()
        };

        Shadow {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            cam: Camera {
                loc: cgmath::Zero::zero(),
                rot: cgmath::Quaternion::look_at(dir, up),
                proj: Projection::ortho(1, 1, 0.0..1.0),
            },
            size,
            dir,
        }
    }

    fn get_local_point(&self, world_pt: cgmath::Point3<f32>) -> cgmath::Point3<f32> {
        let diff = world_pt.to_vec() - self.cam.loc;
        let right = self.cam.rot * cgmath::Vector3::unit_x();
        let up = self.cam.rot * cgmath::Vector3::unit_y();
        let backward = self.cam.rot * cgmath::Vector3::unit_z();
        cgmath::Point3::new(diff.dot(right), diff.dot(up), diff.dot(-backward))
    }

    pub(super) fn update_view(&mut self, cam: &Camera) {
        self.cam.loc = cam.intersect_height(0.0).to_vec();
        let mut p = cgmath::Ortho {
            left: 0.0f32,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
            near: 0.0,
            far: 0.0,
        };

        // in addition to the camera bound, we need to include
        // all the potential occluders nearby
        let mut offset = -self.dir * (HEIGHT_SCALE as f32 / self.dir.z);
        offset.z = 0.0;

        let points_lo = cam.bound_points(0.0);
        let points_hi = cam.bound_points(HEIGHT_SCALE as f32);
        for pt in points_lo
            .iter()
            .cloned()
            .chain(points_hi.iter().cloned())
            .chain(points_hi.iter().map(|&p| p + offset))
        {
            let local = self.get_local_point(pt);
            p.left = p.left.min(local.x);
            p.bottom = p.bottom.min(local.y);
            p.near = p.near.min(local.z);
            p.right = p.right.max(local.x);
            p.top = p.top.max(local.y);
            p.far = p.far.max(local.z);
        }

        self.cam.proj = Projection::Ortho {
            p,
            original: (0, 0),
        };
    }
}
