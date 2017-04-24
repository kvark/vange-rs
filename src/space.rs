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

    pub fn follow(&mut self, target: &Transform, dt: f32, follow: &Follow) {
        use cgmath::Transform;
        let result = target.concat(&follow.transform);
        let k = (dt * -follow.speed).exp();
        //TODO
        self.loc = result.disp * (1.0 - k) + self.loc * k;
        self.rot = result.rot.slerp(self.rot, k);
    }
}
