use cgmath::{
    EuclideanSpace as _,
    InnerSpace as _,
    Rotation as _,
    Rotation3 as _,
    Transform as _,
};
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

#[derive(Debug)]
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
        let view = cgmath::Decomposed {
            scale: 1.0,
            rot: self.rot,
            disp: self.loc,
        };
        let view_mx = cgmath::Matrix4::from(view.inverse_transform().unwrap());
        let mut mvp = self.proj.to_matrix() * view_mx;
        // convert from GL to wgpu/gfx-rs
        // 1) depth conversion from [-1,1] to [0,1]
        mvp.x.z = 0.5*(mvp.x.z + mvp.x.w);
        mvp.y.z = 0.5*(mvp.y.z + mvp.y.w);
        mvp.z.z = 0.5*(mvp.z.z + mvp.z.w);
        mvp.w.z = 0.5*(mvp.w.z + mvp.w.w);
        // 2) invert Y
        mvp.x.y *= -1.0;
        mvp.y.y *= -1.0;
        mvp.z.y *= -1.0;
        mvp.w.y *= -1.0;
        mvp
    }

    pub fn intersect_height(&self, height: f32) -> cgmath::Point3<f32> {
        let dir = self.rot * cgmath::Vector3::unit_z();
        let t = (height - self.loc.z) / dir.z;
        cgmath::Point3::from_vec(self.loc) + t * dir
    }

    pub fn follow(
        &mut self,
        target: &Transform,
        dt: f32,
        follow: &Follow,
    ) {
        let new_target = if follow.fix_z {
            let z_axis = target.rot * cgmath::Vector3::unit_z();
            let adjust_quat = cgmath::Quaternion::from_arc(z_axis, cgmath::Vector3::unit_z(), None);
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

        self.loc = result.disp * (1.0 - k) + self.loc * k;
        self.rot = cgmath::Quaternion::look_at(
            (self.loc - target.disp).normalize(),
            cgmath::Vector3::unit_z(),
        ).invert();
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
