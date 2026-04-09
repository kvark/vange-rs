use crate::{
    config::settings,
    space::{Camera, OrthoParams, Projection},
};

use glam::{Quat, Vec3};

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
            view_formats: &[],
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        });

        Shadow {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            cam: Camera {
                loc: Vec3::ZERO,
                rot: Quat::IDENTITY,
                scale: Vec3::new(1.0, 1.0, 1.0),
                proj: Projection::ortho(1, 1, 0.0..1.0),
            },
            size,
        }
    }

    fn get_local_point(&self, world_pt: Vec3) -> Vec3 {
        let diff = world_pt - self.cam.loc;
        self.cam.rot.inverse() * diff
    }

    pub(super) fn update_view(&mut self, light_pos: &[f32; 4], cam: &Camera, max_height: f32) {
        let dir = Vec3::new(light_pos[0], light_pos[1], light_pos[2]);
        let up = if dir.x == 0.0 && dir.y == 0.0 {
            Vec3::Y
        } else {
            Vec3::Z
        };

        // Build rotation matrix looking along dir
        let forward = dir.normalize();
        let right = forward.cross(up).normalize();
        let up_corrected = right.cross(forward);
        self.cam.rot =
            Quat::from_mat3(&glam::Mat3::from_cols(right, up_corrected, -forward)).inverse();
        self.cam.loc = cam.intersect_height(0.0);

        let mut p = OrthoParams {
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
            .chain(points_hi.iter().map(|&pp| pp + offset))
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
