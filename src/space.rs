use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use std::ops::Range;

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Transform {
    pub scale: f32,
    pub rot: Quat,
    pub disp: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Self = Transform {
        scale: 1.0,
        rot: Quat::IDENTITY,
        disp: Vec3::ZERO,
    };

    pub fn concat(&self, other: &Transform) -> Transform {
        Transform {
            scale: self.scale * other.scale,
            rot: self.rot * other.rot,
            disp: self.rot * (other.disp * self.scale) + self.disp,
        }
    }

    pub fn inverse(&self) -> Transform {
        let inv_scale = 1.0 / self.scale;
        let inv_rot = self.rot.inverse();
        Transform {
            scale: inv_scale,
            rot: inv_rot,
            disp: inv_rot * (-self.disp * inv_scale),
        }
    }

    pub fn transform_point(&self, p: Vec3) -> Vec3 {
        self.rot * (p * self.scale) + self.disp
    }

    pub fn transform_vector(&self, v: Vec3) -> Vec3 {
        self.rot * (v * self.scale)
    }

    pub fn to_mat4(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(Vec3::splat(self.scale), self.rot, self.disp)
    }
}

#[derive(Copy, Clone)]
pub struct OrthoParams {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
    pub near: f32,
    pub far: f32,
}

#[derive(Copy, Clone)]
pub struct PerspectiveParams {
    pub fovy: f32,
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
    /// If `Some`, [`Projection::update`] recomputes `fovy` from this
    /// focal length (in screen pixels) and the new viewport height,
    /// matching the original Vangers `focus_flt = 512` formula in
    /// `road.cpp`. `None` keeps `fovy` fixed across resizes.
    pub focal_px: Option<f32>,
}

impl PerspectiveParams {
    /// Original-Vangers FOV: vertical FOV that places `focal_px` screen
    /// pixels at the same angular size as one world unit at distance
    /// `focal_px`. `focus_flt = 512` in the 1998 source.
    pub fn fov_from_focal_px(focal_px: f32, height_px: f32) -> f32 {
        2.0 * (0.5 * height_px / focal_px).atan()
    }
}

/// Default focal length for camera setup, matching `focus_flt = 512` in
/// the original Vangers `road.cpp`. Together with the window height
/// this gives a vertical FOV close to what the 1998 game showed at
/// equivalent resolutions, which feels less "telephoto" than the prior
/// fixed 45°.
pub const DEFAULT_FOCAL_PX: f32 = 512.0;

#[derive(Copy, Clone)]
pub enum Projection {
    Ortho {
        p: OrthoParams,
        original: (u16, u16),
    },
    Perspective(PerspectiveParams),
}

impl Projection {
    pub fn ortho(w: u16, h: u16, depth: Range<f32>) -> Self {
        Projection::Ortho {
            p: OrthoParams {
                left: -0.5 * w as f32,
                right: 0.5 * w as f32,
                top: -0.5 * h as f32,
                bottom: 0.5 * h as f32,
                near: depth.start,
                far: depth.end,
            },
            original: (w, h),
        }
    }

    pub fn update(&mut self, w: u16, h: u16) {
        match *self {
            Projection::Ortho {
                ref mut p,
                ref mut original,
            } => {
                let scale_x = w as f32 / original.0 as f32;
                let scale_y = h as f32 / original.1 as f32;
                let center_x = 0.5 * p.left + 0.5 * p.right;
                let center_y = 0.5 * p.top + 0.5 * p.bottom;
                *original = (w, h);
                p.left = center_x - scale_x * (center_x - p.left);
                p.right = center_x - scale_x * (center_x - p.right);
                p.top = center_y - scale_y * (center_y - p.top);
                p.bottom = center_y - scale_y * (center_y - p.bottom);
            }
            Projection::Perspective(ref mut p) => {
                p.aspect = w as f32 / h as f32;
                if let Some(focal) = p.focal_px {
                    p.fovy = PerspectiveParams::fov_from_focal_px(focal, h as f32);
                }
            }
        }
    }

    pub fn to_matrix(&self) -> Mat4 {
        match *self {
            Projection::Ortho { p, .. } => {
                // GL-style orthographic projection (depth [-1, 1])
                let rml = p.right - p.left;
                let tmb = p.top - p.bottom;
                let fmn = p.far - p.near;
                Mat4::from_cols(
                    Vec4::new(2.0 / rml, 0.0, 0.0, 0.0),
                    Vec4::new(0.0, 2.0 / tmb, 0.0, 0.0),
                    Vec4::new(0.0, 0.0, -2.0 / fmn, 0.0),
                    Vec4::new(
                        -(p.right + p.left) / rml,
                        -(p.top + p.bottom) / tmb,
                        -(p.far + p.near) / fmn,
                        1.0,
                    ),
                )
            }
            Projection::Perspective(p) => {
                // GL-style perspective projection (depth [-1, 1])
                let f = 1.0 / (p.fovy * 0.5).tan();
                let nf = 1.0 / (p.near - p.far);
                Mat4::from_cols(
                    Vec4::new(f / p.aspect, 0.0, 0.0, 0.0),
                    Vec4::new(0.0, f, 0.0, 0.0),
                    Vec4::new(0.0, 0.0, (p.far + p.near) * nf, -1.0),
                    Vec4::new(0.0, 0.0, 2.0 * p.far * p.near * nf, 0.0),
                )
            }
        }
    }
}

#[derive(Copy, Clone)]
pub struct Camera {
    pub loc: Vec3,
    pub rot: Quat,
    // this non-uniform scale is used to make the camera left-handed
    pub scale: Vec3,
    pub proj: Projection,
}

#[derive(Debug, Copy, Clone)]
pub struct Follow {
    /// Angle in radians
    pub angle_x: f32,
    pub offset: Vec3,
    pub speed: f32,
}

#[derive(Copy, Clone)]
pub struct Direction {
    pub view: Vec3,
    pub height: f32,
}

impl Camera {
    pub fn dir(&self) -> Vec3 {
        self.rot * -Vec3::Z
    }

    pub fn depth_range(&self) -> Range<f32> {
        match self.proj {
            Projection::Ortho { p, .. } => p.near..p.far,
            Projection::Perspective(p) => p.near..p.far,
        }
    }

    fn get_proj_matrix(&self) -> Mat4 {
        let mut proj = self.proj.to_matrix();
        // convert from GL's depth of [-1,1] to wgpu/gfx-rs [0,1]
        let col = proj.col_mut(0);
        let w0 = col[3];
        col[2] = 0.5 * (col[2] + w0);
        let col = proj.col_mut(1);
        let w1 = col[3];
        col[2] = 0.5 * (col[2] + w1);
        let col = proj.col_mut(2);
        let w2 = col[3];
        col[2] = 0.5 * (col[2] + w2);
        let col = proj.col_mut(3);
        let w3 = col[3];
        col[2] = 0.5 * (col[2] + w3);
        proj
    }

    fn view_transform(&self) -> Transform {
        Transform {
            scale: 1.0,
            rot: self.rot,
            disp: self.loc,
        }
    }

    fn scale_matrix(&self) -> Mat4 {
        Mat4::from_scale(self.scale)
    }

    pub fn get_view_proj(&self) -> Mat4 {
        let view = self.view_transform();
        let view_mx = view.inverse().to_mat4();
        self.get_proj_matrix() * self.scale_matrix() * view_mx
    }

    fn intersect_ray_height(&self, dir: Vec3, height: f32) -> Vec3 {
        let t_raw = (height - self.loc.z) / dir.z;
        let range = self.depth_range();
        let t = range.start.max(t_raw).min(range.end);
        self.loc + t * dir
    }

    pub fn intersect_height(&self, height: f32) -> Vec3 {
        let dir = self.dir();
        self.intersect_ray_height(dir, height)
    }

    pub fn visible_bounds_at(&self, height: f32) -> Range<Vec2> {
        let center = self.intersect_height(height).truncate();
        let mut bounds = center..center;

        let proj = self.get_proj_matrix();
        let view = self.view_transform();
        let mx = view.to_mat4() * self.scale_matrix() * proj.inverse();
        // Scale vectors in a way that makes their Z footprint to be -1 in local space.
        let scaler = 1.0 / self.depth_range().end;
        let ndc_points = [
            Vec2::new(-1.0, -1.0),
            Vec2::new(1.0, -1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(-1.0, 1.0),
        ];
        for ndc in &ndc_points {
            let v4 = mx * Vec4::new(ndc.x, ndc.y, 1.0, 1.0);
            let wp = Vec3::new(v4.x / v4.w, v4.y / v4.w, v4.z / v4.w);
            let pt = if wp.z < self.loc.z {
                let dir = scaler * (wp - self.loc);
                self.intersect_ray_height(dir, height)
            } else {
                wp
            };
            bounds.start.x = bounds.start.x.min(pt.x);
            bounds.start.y = bounds.start.y.min(pt.y);
            bounds.end.x = bounds.end.x.max(pt.x);
            bounds.end.y = bounds.end.y.max(pt.y);
        }
        bounds
    }

    pub fn visible_bounds(&self) -> Range<Vec2> {
        let lo = self.visible_bounds_at(0.0);
        let min = Vec2::new(self.loc.x.min(lo.start.x), self.loc.y.min(lo.start.y));
        let max = Vec2::new(self.loc.x.max(lo.end.x), self.loc.y.max(lo.end.y));
        min..max
    }

    pub fn bound_points(&self, height: f32) -> [Vec3; 4] {
        let vb = self.visible_bounds_at(height);
        [
            Vec3::new(vb.start.x, vb.start.y, height),
            Vec3::new(vb.end.x, vb.start.y, height),
            Vec3::new(vb.end.x, vb.end.y, height),
            Vec3::new(vb.start.x, vb.end.y, height),
        ]
    }

    pub fn follow(&mut self, target: &Transform, dt: f32, follow: &Follow) {
        let swing = Quat::from_rotation_x(follow.angle_x);
        let mut front = target.rot * Vec3::Y;
        front.z = 0.0;
        let twist = Quat::from_rotation_arc(Vec3::Y, front);

        let patch = Quat::from_rotation_z(std::f32::consts::PI);
        let rotation = patch * twist * swing;

        let k = (dt * -follow.speed).exp();
        self.rot = rotation.slerp(self.rot, k);

        let location = target.disp + (patch * twist) * follow.offset;
        self.loc = location * (1.0 - k) + self.loc * k;
    }

    /// Push the camera up out of the ground.
    ///
    /// A chase camera trails the car by a fixed offset, which puts it
    /// inside the hillside whenever the car drives up a slope or into a
    /// dip - the view then ends up under the terrain looking at its
    /// backfaces. Raising it to sit `clearance` above whatever surface is
    /// below keeps the framing while never letting that happen.
    ///
    /// Only ever raises: a camera legitimately under a slab, in a tunnel
    /// or cave, is above that space's own floor and is left alone.
    pub fn keep_above_ground(&mut self, level: &crate::level::Level, clearance: f32) {
        let floor = level.floor_below(self.loc);
        self.loc.z = self.loc.z.max(floor + clearance);
    }

    pub fn look_by(&mut self, target: &Transform, dir: &Direction) {
        debug_assert!(dir.view.z < 0.0);
        let k = (target.disp.z - self.loc.z) / -dir.view.z;
        self.loc = target.disp + dir.view * k;
        self.rot = Quat::from_rotation_x(30.0f32.to_radians());
    }

    pub fn focus_on(&mut self, target: &Transform) {
        self.loc = target.disp + Vec3::new(0.0, -64.0, 40.0);
        self.rot = Quat::from_rotation_x(std::f32::consts::FRAC_PI_3);
    }

    pub fn rotate_focus(&mut self, target: &Transform, hor: f32, ver: f32) {
        let mut view = Transform {
            scale: 1.0,
            rot: self.rot,
            disp: self.loc,
        };
        if hor != 0.0 {
            let pre = Transform {
                scale: 1.0,
                rot: Quat::IDENTITY,
                disp: -target.disp,
            };
            let post = Transform {
                scale: 1.0,
                rot: Quat::from_rotation_z(hor),
                disp: target.disp,
            };
            view = post.concat(&pre.concat(&view));
        }
        if ver != 0.0 {
            let target_inv = target.inverse();
            let axis_local = target_inv.rot * self.rot * Vec3::X;
            let mid = Transform {
                scale: 1.0,
                rot: Quat::from_axis_angle(axis_local, -ver),
                disp: Vec3::ZERO,
            };
            view = target.concat(&mid.concat(&target_inv.concat(&view)));
        }
        self.loc = view.disp;
        self.rot = view.rot;
    }

    pub fn front_face(&self) -> wgpu::FrontFace {
        if self.scale.x * self.scale.y > 0.0 {
            wgpu::FrontFace::Cw
        } else {
            wgpu::FrontFace::Ccw
        }
    }

    /// Heading and elevation, in degrees. Yaw is 0 along +Y and grows
    /// clockwise; pitch is 0 at the horizon and positive looking up.
    pub fn angles(&self) -> (f32, f32) {
        let fwd = self.dir();
        (
            fwd.x.atan2(fwd.y).to_degrees().rem_euclid(360.0),
            fwd.z.clamp(-1.0, 1.0).asin().to_degrees(),
        )
    }

    /// Point the camera at a heading and elevation, keeping the horizon
    /// level. Matches the basis `--fp-yaw`/`--fp-pitch` build, so a view
    /// set here reproduces exactly under the snapshot binary.
    pub fn set_angles(&mut self, yaw_deg: f32, pitch_deg: f32) {
        let (yaw, pitch) = (yaw_deg.to_radians(), pitch_deg.to_radians());
        let fwd = Vec3::new(
            yaw.sin() * pitch.cos(),
            yaw.cos() * pitch.cos(),
            pitch.sin(),
        )
        .normalize();
        let right = fwd.cross(Vec3::Z).normalize();
        let up = right.cross(fwd);
        self.rot = Quat::from_mat3(&glam::Mat3::from_cols(right, up, -fwd));
    }

    pub fn draw_ui(&mut self, ui: &mut egui::Ui) {
        // Read the angles back from the rotation every frame so flying
        // with the keyboard keeps these in step, and only write when the
        // user actually drags one - otherwise the round trip through
        // degrees would slowly rewrite the orientation.
        let (mut yaw, mut pitch) = self.angles();
        let mut aimed = false;
        ui.horizontal(|ui| {
            ui.label("Yaw");
            aimed |= ui
                .add(
                    egui::DragValue::new(&mut yaw)
                        .speed(0.5)
                        .range(0.0..=360.0)
                        .suffix("°"),
                )
                .changed();
            ui.label("Pitch");
            aimed |= ui
                .add(
                    egui::DragValue::new(&mut pitch)
                        .speed(0.5)
                        .range(-89.0..=89.0)
                        .suffix("°"),
                )
                .changed();
        });
        // Quarter turns, since that is the interval a bug is most likely
        // to be described in.
        ui.horizontal(|ui| {
            for step in [-90.0f32, -15.0, 15.0, 90.0] {
                if ui.button(format!("{step:+.0}°")).clicked() {
                    yaw = (yaw + step).rem_euclid(360.0);
                    aimed = true;
                }
            }
        });
        if aimed {
            self.set_angles(yaw, pitch);
        }

        match self.proj {
            Projection::Ortho {
                ref mut p,
                original: _,
            } => {
                ui.add(egui::Slider::new(&mut p.near, 0.1..=50.0).text("Depth near"));
                ui.add(egui::Slider::new(&mut p.far, 50.0..=10000.0).text("Depth far"));
            }
            Projection::Perspective(ref mut p) => {
                ui.add(egui::Slider::new(&mut p.near, 0.1..=50.0).text("Depth near"));
                ui.add(egui::Slider::new(&mut p.far, 50.0..=10000.0).text("Depth far"));
            }
        }
    }
}

#[cfg(test)]
mod ground_tests {
    use super::*;
    use crate::{config::settings, level};

    fn test_level() -> level::Level {
        level::load(&level::LevelConfig::new_test(), &settings::Geometry::default())
    }

    fn cam_at(loc: Vec3) -> Camera {
        Camera {
            loc,
            rot: Quat::IDENTITY,
            scale: Vec3::ONE,
            proj: Projection::Perspective(PerspectiveParams {
                fovy: 45f32.to_radians(),
                aspect: 1.0,
                near: 1.0,
                far: 1000.0,
                focal_px: None,
            }),
        }
    }

    /// `set_angles` and `angles` have to be exact inverses, and have to
    /// agree with the basis `--fp-yaw`/`--fp-pitch` build in the snapshot
    /// binary. If they drift, an angle dialled in the viewer renders as a
    /// different view under the harness, which is the one thing the
    /// control exists to prevent.
    #[test]
    fn angles_round_trip() {
        let mut cam = cam_at(Vec3::new(100.0, 100.0, 50.0));
        for yaw_step in 0..24 {
            for pitch_step in -4..=4 {
                let (yaw, pitch) = (yaw_step as f32 * 15.0, pitch_step as f32 * 20.0);
                cam.set_angles(yaw, pitch);
                let (got_yaw, got_pitch) = cam.angles();
                assert!(
                    (got_pitch - pitch).abs() < 1e-3,
                    "pitch {pitch} came back as {got_pitch}"
                );
                // Yaw is meaningless looking straight up or down, and 0
                // and 360 are the same heading.
                if pitch.abs() < 89.0 {
                    let d = (got_yaw - yaw).rem_euclid(360.0);
                    let d = d.min(360.0 - d);
                    assert!(d < 1e-2, "yaw {yaw} came back as {got_yaw}");
                }
            }
        }
    }

    /// The forward vector `set_angles` produces must be the one
    /// `bin/level/headless.rs` builds from the same numbers.
    #[test]
    fn angles_match_the_snapshot_camera() {
        let mut cam = cam_at(Vec3::ZERO);
        for (yaw, pitch) in [(0.0f32, 0.0f32), (122.0, -12.0), (275.0, 10.0), (150.0, -1.0)] {
            cam.set_angles(yaw, pitch);
            let (y, p) = (yaw.to_radians(), pitch.to_radians());
            let expected = Vec3::new(y.sin() * p.cos(), y.cos() * p.cos(), p.sin()).normalize();
            let got = cam.dir();
            assert!(
                (got - expected).length() < 1e-5,
                "yaw {yaw} pitch {pitch}: {got:?} vs {expected:?}"
            );
        }
    }

    /// A camera below the surface is pushed back up to the clearance.
    #[test]
    fn raises_a_camera_that_is_under_the_ground() {
        let level = test_level();
        let (x, y) = (10.0f32, 10.0f32);
        let floor = level.get((x as i32, y as i32)).low();
        let mut cam = cam_at(Vec3::new(x, y, floor - 30.0));
        cam.keep_above_ground(&level, 4.0);
        assert!(
            (cam.loc.z - (floor + 4.0)).abs() < 1e-3,
            "expected {}, got {}",
            floor + 4.0,
            cam.loc.z
        );
    }

    /// ...and one already in the open is left where it is. `keep_above_ground`
    /// only ever raises, so it cannot fight the follow camera.
    #[test]
    fn leaves_a_camera_in_the_open_alone() {
        let level = test_level();
        let (x, y) = (10.0f32, 10.0f32);
        let floor = level.get((x as i32, y as i32)).low();
        let mut cam = cam_at(Vec3::new(x, y, floor + 200.0));
        cam.keep_above_ground(&level, 4.0);
        assert_eq!(cam.loc.z, floor + 200.0);
    }

    /// The case that makes this more than a `max`: under a slab, the
    /// surface below is the cave floor, not the slab top. Clamping to the
    /// slab top would fling the camera through the roof every time the car
    /// drove into a tunnel.
    #[test]
    fn a_camera_inside_a_cave_keeps_the_cave_floor() {
        let level = test_level();
        let dual = (0..level.size.1)
            .flat_map(|y| (0..level.size.0).map(move |x| (x, y)))
            .find_map(|(x, y)| match level.get((x, y)) {
                level::Texel::Dual { low, mid, high } => Some((x, y, low.0, mid, high.0)),
                level::Texel::Single(_) => None,
            })
            .expect("the test level is built with a double-level region");
        let (x, y, low, mid, high) = dual;
        assert!(mid < high, "the cave has to have a ceiling below the slab");

        // Sitting in the cave, comfortably under the ceiling.
        let inside = low + (mid - low) * 0.5;
        let mut cam = cam_at(Vec3::new(x as f32, y as f32, inside));
        cam.keep_above_ground(&level, 4.0);
        assert_eq!(cam.loc.z, inside, "a camera in the cave should not move");

        // Below the cave floor: raised to the floor, still under the slab.
        let mut cam = cam_at(Vec3::new(x as f32, y as f32, low - 10.0));
        cam.keep_above_ground(&level, 4.0);
        assert!((cam.loc.z - (low + 4.0)).abs() < 1e-3);
        assert!(cam.loc.z < mid, "should not be pushed through the ceiling");
    }
}
