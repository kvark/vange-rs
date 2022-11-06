use crate::{
    config::settings,
    space::{Camera, Projection},
};

use cgmath::{EuclideanSpace as _, One as _, Rotation as _};

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;

pub struct Shadow {
    pub(super) view: wgpu::TextureView,
    pub(super) cam: Camera,
    pub(super) size: u32,
}

impl Shadow {
    pub(super) fn new(settings: &settings::Shadow, device: &wgpu::Device) -> Self {
        let size = settings.size;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Shadow"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        });

        Shadow {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            cam: Camera {
                loc: cgmath::Zero::zero(),
                rot: cgmath::Quaternion::one(),
                scale: cgmath::Vector3::new(1.0, 1.0, 1.0),
                proj: Projection::ortho(1, 1, 0.0..1.0),
            },
            size,
        }
    }

    fn get_local_point(&self, world_pt: cgmath::Point3<f32>) -> cgmath::Point3<f32> {
        let diff = world_pt.to_vec() - self.cam.loc;
        cgmath::Point3::origin() + self.cam.rot.invert() * diff
    }

    pub(super) fn update_view(&mut self, light_pos: &[f32; 4], cam: &Camera, max_height: f32) {
        let dir = cgmath::Vector4::from(*light_pos).truncate();
        let up = if dir.x == 0.0 && dir.y == 0.0 {
            cgmath::Vector3::unit_y()
        } else {
            cgmath::Vector3::unit_z()
        };

        self.cam.rot = cgmath::Quaternion::look_at(dir, up).invert();
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
        let mut offset = dir * (max_height / dir.z);
        offset.z = 0.0;

        let points_lo = cam.bound_points(0.0);
        let points_hi = cam.bound_points(max_height);
        for pt in points_lo
            .iter()
            .cloned()
            .chain(points_hi.iter().cloned())
            .chain(points_hi.iter().map(|&p| p + offset))
        {
            let local = self.get_local_point(pt);
            p.left = p.left.min(local.x);
            p.bottom = p.bottom.min(local.y);
            p.near = p.near.min(-local.z);
            p.right = p.right.max(local.x);
            p.top = p.top.max(local.y);
            p.far = p.far.max(-local.z);
        }

        self.cam.proj = Projection::Ortho {
            p,
            original: (0, 0),
        };
    }
}
