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
        // `cam.rot` is local→world (matches the rest of the codebase),
        // so the world→local rotation is its inverse.
        self.cam.rot.inverse() * diff
    }

    pub(super) fn update_view(&mut self, light_pos: &[f32; 4], cam: &Camera, max_height: f32) {
        // Light direction in world space — `light_pos.xyz` is the
        // direction *to* the sun, so the camera looks back along it.
        let to_sun = Vec3::new(light_pos[0], light_pos[1], light_pos[2]).normalize();
        let up = if to_sun.x == 0.0 && to_sun.y == 0.0 {
            Vec3::Y
        } else {
            Vec3::Z
        };

        // Camera convention is local -Z = view direction. The shadow
        // camera looks *from* the sun *toward* the scene, i.e. opposite
        // the to-sun vector.
        let forward = -to_sun;
        let right = forward.cross(up).normalize();
        let up_corrected = right.cross(forward);
        // Mat3 cols are local axes in world coordinates → local→world,
        // which is exactly what `cam.rot` stores. The previous code
        // wrapped this in `.inverse()`, leaving cam.rot as world→local
        // and causing the shadow render to project in the wrong frame.
        self.cam.rot = Quat::from_mat3(&glam::Mat3::from_cols(right, up_corrected, -forward));
        self.cam.loc = cam.intersect_height(0.0);

        let mut p = OrthoParams {
            left: 0.0f32,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
            near: 0.0,
            far: 0.0,
        };

        // Limit the shadow region to a box around the camera center
        // rather than covering the entire view frustum. This keeps
        // shadow texel density high near the player where it matters.
        // The radius is capped so that cameras with very long far
        // planes don't produce continent-sized shadow maps.
        let shadow_radius = 600.0f32;
        let center = self.cam.loc;
        let corners = [
            Vec3::new(center.x - shadow_radius, center.y - shadow_radius, 0.0),
            Vec3::new(center.x + shadow_radius, center.y - shadow_radius, 0.0),
            Vec3::new(center.x + shadow_radius, center.y + shadow_radius, 0.0),
            Vec3::new(center.x - shadow_radius, center.y + shadow_radius, 0.0),
            Vec3::new(center.x - shadow_radius, center.y - shadow_radius, max_height),
            Vec3::new(center.x + shadow_radius, center.y - shadow_radius, max_height),
            Vec3::new(center.x + shadow_radius, center.y + shadow_radius, max_height),
            Vec3::new(center.x - shadow_radius, center.y + shadow_radius, max_height),
        ];

        for pt in corners {
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
