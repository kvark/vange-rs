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
}

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

    pub fn draw_ui(&mut self, ui: &mut egui::Ui) {
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
