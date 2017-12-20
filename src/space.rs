use cgmath;

pub type Transform = cgmath::Decomposed<cgmath::Vector3<f32>, cgmath::Quaternion<f32>>;

pub struct Camera {
    pub loc: cgmath::Vector3<f32>,
    pub rot: cgmath::Quaternion<f32>,
    pub proj: cgmath::PerspectiveFov<f32>,
}

pub struct Follow {
    pub transform: Transform,
    pub speed: f32,
}

pub struct Direction {
    pub view: cgmath::Vector3<f32>,
    pub height: f32,
}

impl Camera {
    pub fn get_view_proj(&self) -> cgmath::Matrix4<f32> {
        use cgmath::{Decomposed, Matrix4, Transform};
        let view = Decomposed {
            scale: 1.0,
            rot: self.rot,
            disp: self.loc,
        };
        let view_mx: Matrix4<f32> = view.inverse_transform().unwrap().into();
        let proj_mx: Matrix4<f32> = self.proj.into();
        proj_mx * view_mx
    }

    pub fn follow(
        &mut self,
        target: &Transform,
        dt: f32,
        follow: &Follow,
    ) {
        use cgmath::Transform;
        let result = target.concat(&follow.transform);
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
        use cgmath::Rotation3;
        debug_assert!(dir.view.z < 0.0);
        let k = (target.disp.z - self.loc.z) / -dir.view.z;
        self.loc = target.disp + dir.view * k;
        //self.rot = cgmath::Quaternion::look_at(dir.view, cgmath::Vector3::unit_y()).invert();
        self.rot = cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_x(), cgmath::Deg(30.0));
    }

    pub fn focus_on(
        &mut self,
        target: &Transform,
    ) {
        use cgmath::{Angle, Rotation3};
        self.loc = target.disp + cgmath::vec3(0.0, -64.0, 40.0);
        self.rot = cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_x(), cgmath::Rad::turn_div_6());
    }

    pub fn rotate_focus(
        &mut self,
        target: &Transform,
        hor: cgmath::Rad<f32>,
        ver: cgmath::Rad<f32>,
    ) {
        use cgmath::{Decomposed, One, Rotation3, Transform, Zero};
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
