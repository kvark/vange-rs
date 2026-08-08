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

    pub(super) fn update_view(&mut self, light_pos: &[f32; 4], cam: &Camera, max_height: f32) {
        let (rot, loc, p) = shadow_view(light_pos, cam, max_height);
        self.cam.rot = rot;
        self.cam.loc = loc;
        self.cam.proj = Projection::Ortho {
            p,
            original: (0, 0),
        };
    }
}

/// Where the shadow camera goes and what it covers.
///
/// Split out of [`Shadow::update_view`] so it can be tested without a
/// device: it is pure, and it is the part that has been wrong.
fn shadow_view(light_pos: &[f32; 4], cam: &Camera, max_height: f32) -> (Quat, Vec3, OrthoParams) {
    // Light direction in world space - `light_pos.xyz` is the direction
    // *to* the sun, so the camera looks back along it.
    let to_sun = Vec3::new(light_pos[0], light_pos[1], light_pos[2]).normalize();
    let up = if to_sun.x == 0.0 && to_sun.y == 0.0 {
        Vec3::Y
    } else {
        Vec3::Z
    };

    // Camera convention is local -Z = view direction. The shadow camera
    // looks *from* the sun *toward* the scene, i.e. opposite the to-sun
    // vector.
    let forward = -to_sun;
    let right = forward.cross(up).normalize();
    let up_corrected = right.cross(forward);
    // Mat3 cols are local axes in world coordinates -> local->world, which
    // is exactly what `cam.rot` stores.
    let rot = Quat::from_mat3(&glam::Mat3::from_cols(right, up_corrected, -forward));

    // Centre the map on the ground ahead of the camera.
    //
    // This used to be `cam.intersect_height(0.0)`, which does not do what
    // its name suggests. It clamps the ray parameter to the depth range
    // and returns a point regardless, so it reports a hit even when the
    // ray never meets the ground: level or looking up it hands back the
    // near plane - the camera itself - and on a shallow downward pitch it
    // hands back a point still tens of units in the air, clipped by the
    // far plane. Measured on a camera 120 up: at -30 degrees it really is
    // on the ground, at -4 degrees it is at z = 50, and at 0 degrees and
    // above it is the camera. Only steep angles worked.
    //
    // The horizontal heading is always defined, so use that instead and
    // step half the radius along it. The covered band then runs from half
    // a radius behind the camera to one and a half ahead, whatever the
    // pitch, and nothing about it can degenerate.
    let flat = {
        let d = cam.dir();
        let f = Vec3::new(d.x, d.y, 0.0);
        if f.length_squared() > 1e-6 {
            f.normalize()
        } else {
            // Straight up or down: any heading covers the same ground.
            Vec3::Y
        }
    };
    let shadow_radius = 600.0f32;
    let loc = Vec3::new(cam.loc.x, cam.loc.y, 0.0) + flat * (0.5 * shadow_radius);

    // A box around that centre rather than the whole view frustum, so
    // texel density stays high near the player and a camera with a very
    // long far plane does not ask for a continent-sized shadow map.
    let corners = [
        Vec3::new(loc.x - shadow_radius, loc.y - shadow_radius, 0.0),
        Vec3::new(loc.x + shadow_radius, loc.y - shadow_radius, 0.0),
        Vec3::new(loc.x + shadow_radius, loc.y + shadow_radius, 0.0),
        Vec3::new(loc.x - shadow_radius, loc.y + shadow_radius, 0.0),
        Vec3::new(loc.x - shadow_radius, loc.y - shadow_radius, max_height),
        Vec3::new(loc.x + shadow_radius, loc.y - shadow_radius, max_height),
        Vec3::new(loc.x + shadow_radius, loc.y + shadow_radius, max_height),
        Vec3::new(loc.x - shadow_radius, loc.y + shadow_radius, max_height),
    ];

    let inv_rot = rot.inverse();
    let mut p = OrthoParams {
        left: 0.0f32,
        right: 0.0,
        top: 0.0,
        bottom: 0.0,
        near: 0.0,
        far: 0.0,
    };
    for pt in corners {
        let local = inv_rot * (pt - loc);
        p.left = p.left.min(local.x);
        p.bottom = p.bottom.min(local.y);
        p.near = p.near.min(-local.z);
        p.right = p.right.max(local.x);
        p.top = p.top.max(local.y);
        p.far = p.far.max(-local.z);
    }
    (rot, loc, p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::space::PerspectiveParams;

    /// Matches the chase camera: a shallow downward pitch, which is what
    /// the vehicle views use.
    fn chase_cam(yaw_deg: f32, pitch_deg: f32) -> Camera {
        let (yaw, pitch) = (yaw_deg.to_radians(), pitch_deg.to_radians());
        let forward = Vec3::new(
            yaw.sin() * pitch.cos(),
            yaw.cos() * pitch.cos(),
            pitch.sin(),
        )
        .normalize();
        let right = forward.cross(Vec3::Z).normalize();
        let up = right.cross(forward);
        Camera {
            loc: Vec3::new(1024.0, 1024.0, 120.0),
            rot: Quat::from_mat3(&glam::Mat3::from_cols(right, up, -forward)),
            scale: Vec3::ONE,
            proj: Projection::Perspective(PerspectiveParams {
                fovy: 60f32.to_radians(),
                aspect: 1.6,
                near: 1.0,
                // The shipped default. The failure window depends on it:
                // a shallow downward pitch only reaches the ground within
                // the far plane at some distances, and that is exactly
                // where the old centre went wrong.
                far: 2000.0,
                focal_px: None,
            }),
        }
    }

    const LIGHT: [f32; 4] = [1.0, 2.0, 4.0, 0.0];

    /// The shadow box has to stay finite and non-empty however the player
    /// turns. Nothing about the light moves with the camera, so a yaw that
    /// produces a degenerate box means the box is being derived from the
    /// wrong thing.
    #[test]
    fn shadow_box_survives_every_heading() {
        for step in 0..72 {
            let yaw = step as f32 * 5.0;
            let (rot, loc, p) = shadow_view(&LIGHT, &chase_cam(yaw, -16.0), 256.0);
            assert!(
                loc.is_finite() && rot.is_finite(),
                "yaw {yaw}: loc {loc:?} rot {rot:?}"
            );
            assert!(
                p.right > p.left && p.top > p.bottom && p.far > p.near,
                "yaw {yaw}: degenerate box {:?}..{:?} x {:?}..{:?} x {:?}..{:?}",
                p.left,
                p.right,
                p.bottom,
                p.top,
                p.near,
                p.far
            );
        }
    }

    /// ...and at every pitch, including exactly level, where the ray the
    /// centre is derived from never meets the ground.
    #[test]
    fn shadow_box_survives_every_pitch() {
        for step in -18..=18 {
            let pitch = step as f32 * 5.0;
            let (rot, loc, p) = shadow_view(&LIGHT, &chase_cam(30.0, pitch), 256.0);
            assert!(
                loc.is_finite() && rot.is_finite(),
                "pitch {pitch}: loc {loc:?} rot {rot:?}"
            );
            assert!(
                p.right > p.left && p.top > p.bottom && p.far > p.near,
                "pitch {pitch}: degenerate box"
            );
        }
    }

    /// What the fragment shader actually does: project a world point with
    /// the light's view-projection and look it up. If that lands outside
    /// the shadow map, the pixel is unshadowed - so a heading where it
    /// leaves the volume is a heading with no shadows, whatever the box
    /// bounds say.
    #[test]
    fn ground_under_the_camera_projects_into_the_shadow_map() {
        for step in 0..72 {
            let yaw = step as f32 * 5.0;
            let cam = chase_cam(yaw, -16.0);
            let (rot, loc, p) = shadow_view(&LIGHT, &cam, 256.0);
            let shadow_cam = Camera {
                loc,
                rot,
                scale: Vec3::ONE,
                proj: Projection::Ortho {
                    p,
                    original: (0, 0),
                },
            };
            let vp = shadow_cam.get_view_proj();
            let probe = Vec3::new(cam.loc.x, cam.loc.y, 0.0);
            let c = vp * probe.extend(1.0);
            let ndc = c.truncate() / c.w;
            assert!(
                ndc.x >= -1.0 && ndc.x <= 1.0 && ndc.y >= -1.0 && ndc.y <= 1.0,
                "yaw {yaw}: ground under the camera projects to {ndc:?}, \
                 outside the shadow map"
            );
            assert!(
                ndc.z >= 0.0 && ndc.z <= 1.0,
                "yaw {yaw}: ground under the camera projects to depth {}, \
                 outside the clip range",
                ndc.z
            );
        }
    }

    /// The box has to cover the ground the camera is *looking at*, not
    /// merely the ground beneath it.
    ///
    /// This is the check the earlier tests were missing. They asserted the
    /// box was finite, non-empty and contained its own centre, all of
    /// which stayed true while the centre itself was wrong - derived from
    /// a ray/ground intersection that does not exist at level or upward
    /// pitch, and that gets clamped into mid-air on a shallow one.
    #[test]
    fn shadow_box_covers_what_the_camera_looks_at() {
        // -1 is the case seen in play: shallow enough that the ground
        // intersection is real but nearly a kilometre away, so a box
        // centred on it leaves the player standing outside their own
        // shadow map.
        for pitch in [-30.0f32, -12.0, -4.0, -2.0, -1.0, -0.5, 0.0, 4.0, 10.0] {
            for step in 0..12 {
                let yaw = step as f32 * 30.0;
                let cam = chase_cam(yaw, pitch);
                let (rot, loc, p) = shadow_view(&LIGHT, &cam, 256.0);
                let inv = rot.inverse();

                // The ground under the camera, and the ground ahead along
                // the heading at a few distances a player would notice.
                let flat = {
                    let d = cam.dir();
                    Vec3::new(d.x, d.y, 0.0).normalize()
                };
                for reach in [0.0f32, 100.0, 250.0, 400.0] {
                    let probe = Vec3::new(cam.loc.x, cam.loc.y, 0.0) + flat * reach;
                    let l = inv * (probe - loc);
                    assert!(
                        l.x >= p.left && l.x <= p.right && l.y >= p.bottom && l.y <= p.top,
                        "pitch {pitch} yaw {yaw}: ground {reach} units ahead is \
                         outside the shadow box"
                    );
                }
            }
        }
    }

    /// The box must actually contain the ground under the camera. A box
    /// that is finite, non-empty and somewhere else casts no shadows where
    /// the player is looking, which is indistinguishable from having none.
    #[test]
    fn shadow_box_covers_the_ground_ahead() {
        for step in 0..24 {
            let yaw = step as f32 * 15.0;
            let cam = chase_cam(yaw, -16.0);
            let (rot, loc, p) = shadow_view(&LIGHT, &cam, 256.0);
            let inv = rot.inverse();
            for probe in [cam.loc.truncate().extend(0.0), loc] {
                let l = inv * (probe - loc);
                assert!(
                    l.x >= p.left && l.x <= p.right && l.y >= p.bottom && l.y <= p.top,
                    "yaw {yaw}: {probe:?} falls outside the shadow box"
                );
            }
        }
    }
}
