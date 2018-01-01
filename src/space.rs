use cgmath;
use cgmath::{Rotation3 as Rotation3_, Transform as Transform_};
use std::ops::Range;

pub type Transform = cgmath::Decomposed<cgmath::Vector3<f32>, cgmath::Quaternion<f32>>;

pub enum Projection {
    Ortho {
        p: cgmath::Ortho<f32>,
        original: (u16, u16),
    },
    Perspective(cgmath::PerspectiveFov<f32>),
}

impl Projection {
    pub fn ortho(w: u16, h: u16, depth: Range<f32>) -> Self {
        Projection::Ortho {
            p: cgmath::Ortho {
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
            Projection::Ortho { ref mut p, ref mut original } => {
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

    pub fn to_matrix(&self) -> cgmath::Matrix4<f32> {
        match *self {
            Projection::Ortho { p, .. } => p.into(),
            Projection::Perspective(p) => p.into(),
        }
    }
}

pub struct Camera {
    pub loc: cgmath::Vector3<f32>,
    pub rot: cgmath::Quaternion<f32>,
    pub proj: Projection,
}

pub struct Follow {
    pub transform: Transform,
    pub speed: f32,
    pub fix_z: bool,
}

pub struct Direction {
    pub view: cgmath::Vector3<f32>,
    pub height: f32,
}

impl Camera {
    pub fn get_view_proj(&self) -> cgmath::Matrix4<f32> {
        use cgmath::{Decomposed, Matrix4};
        let view = Decomposed {
            scale: 1.0,
            rot: self.rot,
            disp: self.loc,
        };
        let view_mx = Matrix4::from(view.inverse_transform().unwrap());
        self.proj.to_matrix() * view_mx
    }

    pub fn follow(
        &mut self,
        target: &Transform,
        dt: f32,
        follow: &Follow,
    ) {
        let new_target = if follow.fix_z {
            let z_axis = target.rot * cgmath::vec3(0.0, 0.0, 1.0);
            let adjust_quat = cgmath::Quaternion::from_arc(z_axis, cgmath::vec3(0.0, 0.0, 1.0), None);
            Transform {
                disp: target.disp,
                rot: adjust_quat * target.rot,
                scale: 1.0,
            }
        } else {
            target.clone()
        };

        let result = new_target.concat(&follow.transform);
        let k = (dt * -follow.speed).exp();
        //TODO
        self.loc = result.disp * (1.0 - k) + self.loc * k;
        self.rot = result.rot.slerp(self.rot, k);
    }

    pub fn look_by(
        &mut self,
        target: &Transform,
        dir: &Direction,
    ) {
        debug_assert!(dir.view.z < 0.0);
        let k = (target.disp.z - self.loc.z) / -dir.view.z;
        self.loc = target.disp + dir.view * k;
        //self.rot = cgmath::Quaternion::look_at(dir.view, cgmath::Vector3::unit_y()).invert();
        self.rot =
            cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_x(), cgmath::Deg(30.0));
    }

    pub fn focus_on(
        &mut self,
        target: &Transform,
    ) {
        use cgmath::{Angle};
        self.loc = target.disp + cgmath::vec3(0.0, -64.0, 40.0);
        self.rot = cgmath::Quaternion::from_axis_angle(
            cgmath::Vector3::unit_x(),
            cgmath::Rad::turn_div_6(),
        );
    }

    pub fn rotate_focus(
        &mut self,
        target: &Transform,
        hor: cgmath::Rad<f32>,
        ver: cgmath::Rad<f32>,
    ) {
        use cgmath::{Decomposed, One, Zero};
        let mut view = Decomposed {
            scale: 1.0,
            rot: self.rot,
            disp: self.loc,
        };
        // old mv: inv(view) * model
        if hor.0 != 0.0 {
            // inv(new) = inv(view) * post * mid * pre
            // new = pre^ * mid^ post^ * view = post * mid^ * pre * view
            let pre = Decomposed {
                scale: 1.0,
                rot: cgmath::Quaternion::one(),
                disp: -target.disp,
            };
            let post = Decomposed {
                scale: 1.0,
                rot: cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), hor),
                disp: target.disp,
            };
            view = post.concat(&pre.concat(&view));
        }
        if ver.0 != 0.0 {
            // inv(new) * model = inv(view) * model * mid
            // model^ * new = mid^ * model^ * view
            // new = model * mid^ * model^ * view
            let target_inv = target.inverse_transform().unwrap();
            let axis_local = target_inv.rot * self.rot * cgmath::Vector3::unit_x();
            let mid = Decomposed {
                scale: 1.0,
                rot: cgmath::Quaternion::from_axis_angle(axis_local, -ver),
                disp: cgmath::Vector3::zero(),
            };
            view = target.concat(&mid.concat(&target_inv.concat(&view)));
        }
        self.loc = view.disp;
        self.rot = view.rot;
    }
}
