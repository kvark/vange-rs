use glam::{Mat3, Vec3};

pub struct RigidBody {
    j_inv: Mat3,
    pub vel: Vec3,
    wel_orig: Vec3,
    wel_raw: Vec3,
}

impl RigidBody {
    pub fn new(
        jacobian: &Mat3,
        vel: Vec3,
        wel: Vec3,
    ) -> Self {
        RigidBody {
            j_inv: jacobian.inverse(),
            vel,
            wel_orig: wel,
            wel_raw: Vec3::ZERO,
        }
    }

    fn calc_collision_matrix_inv(&self, r: &Vec3) -> Mat3 {
        let ji = &self.j_inv;
        let ji_col0 = ji.col(0);
        let ji_col1 = ji.col(1);
        let ji_col2 = ji.col(2);
        let t3 = -r.z * ji_col1[1] + r.y * ji_col2[1];
        let t7 = -r.z * ji_col1[2] + r.y * ji_col2[2];
        let t12 = -r.z * ji_col1[0] + r.y * ji_col2[0];
        let t21 = r.z * ji_col0[1] - r.x * ji_col2[1];
        let t25 = r.z * ji_col0[2] - r.x * ji_col2[2];
        let t30 = r.z * ji_col0[0] - r.x * ji_col2[0];
        let t39 = -r.y * ji_col0[1] + r.x * ji_col1[1];
        let t43 = -r.y * ji_col0[2] + r.x * ji_col1[2];
        let t48 = -r.y * ji_col0[0] + r.x * ji_col1[0];
        let cm = Mat3::from_cols(
            Vec3::new(
                1.0 - t3 * r.z + t7 * r.y,
                t12 * r.z - t7 * r.x,
                -t12 * r.y + t3 * r.x,
            ),
            Vec3::new(
                -t21 * r.z + t25 * r.y,
                1.0 + t30 * r.z - t25 * r.x,
                -t30 * r.y + t21 * r.x,
            ),
            Vec3::new(
                -t39 * r.z + t43 * r.y,
                t48 * r.z - t43 * r.x,
                1.0 - t48 * r.y + t39 * r.x,
            ),
        );
        cm.inverse()
    }

    pub fn add_raw(&mut self, vel: Vec3, wel_raw: Vec3) {
        self.vel += vel;
        self.wel_raw += wel_raw;
    }

    pub fn push(
        &mut self,
        point: Vec3,
        vec: Vec3,
    ) -> Vec3 {
        let pulse = self.calc_collision_matrix_inv(&point) * vec;
        self.vel += pulse;
        self.wel_raw += point.cross(pulse);
        pulse
    }

    pub fn velocity_at(&self, point: Vec3) -> Vec3 {
        self.vel + self.wel_orig.cross(point)
    }

    pub fn angular_velocity(&self) -> Vec3 {
        self.wel_orig
    }

    pub fn finish(self) -> (Vec3, Vec3) {
        (self.vel, self.wel_orig + self.j_inv * self.wel_raw)
    }
}
