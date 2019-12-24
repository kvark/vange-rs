use cgmath::prelude::*;

pub struct RigidBody {
    j_inv: cgmath::Matrix3<f32>,
    pub vel: cgmath::Vector3<f32>,
    wel_orig: cgmath::Vector3<f32>,
    wel_raw: cgmath::Vector3<f32>,
}

impl RigidBody {
    pub fn new(
        jacobian: &cgmath::Matrix3<f32>,
        vel: cgmath::Vector3<f32>,
        wel: cgmath::Vector3<f32>,
    ) -> Self {
        RigidBody {
            j_inv: jacobian.invert().unwrap(),
            vel,
            wel_orig: wel,
            wel_raw: cgmath::Vector3::zero(),
        }
    }

    fn calc_collision_matrix_inv(
        &self,
        r: &cgmath::Vector3<f32>,
    ) -> cgmath::Matrix3<f32> {
        let ji = &self.j_inv;
        let t3 = -r.z * ji[1][1] + r.y * ji[2][1];
        let t7 = -r.z * ji[1][2] + r.y * ji[2][2];
        let t12 = -r.z * ji[1][0] + r.y * ji[2][0];
        let t21 = r.z * ji[0][1] - r.x * ji[2][1];
        let t25 = r.z * ji[0][2] - r.x * ji[2][2];
        let t30 = r.z * ji[0][0] - r.x * ji[2][0];
        let t39 = -r.y * ji[0][1] + r.x * ji[1][1];
        let t43 = -r.y * ji[0][2] + r.x * ji[1][2];
        let t48 = -r.y * ji[0][0] + r.x * ji[1][0];
        let cm = cgmath::Matrix3::new(
            1.0 - t3 * r.z + t7 * r.y,
            t12 * r.z - t7 * r.x,
            -t12 * r.y + t3 * r.x,
            -t21 * r.z + t25 * r.y,
            1.0 + t30 * r.z - t25 * r.x,
            -t30 * r.y + t21 * r.x,
            -t39 * r.z + t43 * r.y,
            t48 * r.z - t43 * r.x,
            1.0 - t48 * r.y + t39 * r.x,
        );
        cm.invert().unwrap()
    }

    pub fn add_raw(&mut self, vel: cgmath::Vector3<f32>, wel_raw: cgmath::Vector3<f32>) {
        self.vel += vel;
        self.wel_raw += wel_raw;
    }

    pub fn push(
        &mut self,
        point: cgmath::Vector3<f32>,
        vec: cgmath::Vector3<f32>,
    ) -> cgmath::Vector3<f32> {
        let pulse = self.calc_collision_matrix_inv(&point) * vec;
        self.vel += pulse;
        self.wel_raw += point.cross(pulse);
        pulse
    }

    pub fn velocity_at(&self, point: cgmath::Vector3<f32>) -> cgmath::Vector3<f32> {
        self.vel + self.wel_orig.cross(point)
    }

    pub fn angular_velocity(&self) -> cgmath::Vector3<f32> {
        self.wel_orig
    }

    pub fn finish(self) -> (cgmath::Vector3<f32>, cgmath::Vector3<f32>) {
        (self.vel, self.wel_orig + self.j_inv * self.wel_raw)
    }
}
