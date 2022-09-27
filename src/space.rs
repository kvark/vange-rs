use cgmath::{Angle as _, EuclideanSpace as _, Rotation as _, Rotation3 as _, Transform as _};
use std::ops::Range;

pub type Transform = cgmath::Decomposed<cgmath::Vector3<f32>, cgmath::Quaternion<f32>>;

#[derive(Copy, Clone)]
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

    pub fn to_matrix(&self) -> cgmath::Matrix4<f32> {
        match *self {
            Projection::Ortho { p, .. } => p.into(),
            Projection::Perspective(p) => p.into(),
        }
    }
}

#[derive(Copy, Clone)]
pub struct Camera {
    pub loc: cgmath::Vector3<f32>,
    pub rot: cgmath::Quaternion<f32>,
    // this non-uniform scale is used to make the camera left-handed
    pub scale: cgmath::Vector3<f32>,
    pub proj: Projection,
}

#[derive(Debug, Copy, Clone)]
pub struct Follow {
    pub angle_x: cgmath::Deg<f32>,
    pub offset: cgmath::Vector3<f32>,
    pub speed: f32,
}

#[derive(Copy, Clone)]
pub struct Direction {
    pub view: cgmath::Vector3<f32>,
    pub height: f32,
}

impl Camera {
    fn _scale_vec(&self, vec: cgmath::Vector3<f32>) -> cgmath::Vector3<f32> {
        cgmath::Vector3::new(
            self.scale.x * vec.x,
            self.scale.y * vec.y,
            self.scale.z * vec.z,
        )
    }

    pub fn dir(&self) -> cgmath::Vector3<f32> {
        self.rot * -cgmath::Vector3::unit_z()
    }

    pub fn depth_range(&self) -> Range<f32> {
        match self.proj {
            Projection::Ortho { p, .. } => p.near..p.far,
            Projection::Perspective(p) => p.near..p.far,
        }
    }

    fn get_proj_matrix(&self) -> cgmath::Matrix4<f32> {
        let mut proj = self.proj.to_matrix();
        // convert from GL's depth of [-1,1] to wgpu/gfx-rs [0,1]
        proj.x.z = 0.5 * (proj.x.z + proj.x.w);
        proj.y.z = 0.5 * (proj.y.z + proj.y.w);
        proj.z.z = 0.5 * (proj.z.z + proj.z.w);
        proj.w.z = 0.5 * (proj.w.z + proj.w.w);
        proj
    }

    fn view_transform(&self) -> Transform {
        cgmath::Decomposed {
            scale: 1.0,
            rot: self.rot,
            disp: self.loc,
        }
    }

    fn scale_matrix(&self) -> cgmath::Matrix4<f32> {
        cgmath::Matrix4::from_nonuniform_scale(self.scale.x, self.scale.y, self.scale.z)
    }

    pub fn get_view_proj(&self) -> cgmath::Matrix4<f32> {
        let view = self.view_transform();
        let view_mx = cgmath::Matrix4::from(view.inverse_transform().unwrap());
        self.get_proj_matrix() * self.scale_matrix() * view_mx
    }

    fn intersect_ray_height(&self, dir: cgmath::Vector3<f32>, height: f32) -> cgmath::Point3<f32> {
        let t_raw = (height - self.loc.z) / dir.z;
        let range = self.depth_range();
        let t = range.start.max(t_raw).min(range.end);
        cgmath::Point3::from_vec(self.loc) + t * dir
    }

    pub fn intersect_height(&self, height: f32) -> cgmath::Point3<f32> {
        let dir = self.dir();
        self.intersect_ray_height(dir, height)
    }

    pub fn visible_bounds_at(&self, height: f32) -> Range<cgmath::Vector2<f32>> {
        let center = self.intersect_height(height).to_vec().truncate();
        let mut bounds = center..center;

        let proj = self.get_proj_matrix();
        let view = self.view_transform();
        let mx =
            cgmath::Matrix4::from(view) * self.scale_matrix() * proj.inverse_transform().unwrap();
        // Scale vectors in a way that makes their Z footprint to be -1 in local space.
        let scaler = 1.0 / self.depth_range().end;
        let ndc_points = [
            cgmath::vec2(-1.0, -1.0),
            cgmath::vec2(1.0, -1.0),
            cgmath::vec2(1.0, 1.0),
            cgmath::vec2(-1.0, 1.0),
        ];
        for ndc in &ndc_points {
            let wp = cgmath::Point3::from_homogeneous(mx * cgmath::vec4(ndc.x, ndc.y, 1.0, 1.0));
            let pt = if wp.z < self.loc.z {
                let dir = scaler * (wp - cgmath::Point3::from_vec(self.loc));
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

    pub fn visible_bounds(&self) -> Range<cgmath::Vector2<f32>> {
        let lo = self.visible_bounds_at(0.0);
        let min = cgmath::vec2(self.loc.x.min(lo.start.x), self.loc.y.min(lo.start.y));
        let max = cgmath::vec2(self.loc.x.max(lo.end.x), self.loc.y.max(lo.end.y));
        min..max
    }

    pub fn bound_points(&self, height: f32) -> [cgmath::Point3<f32>; 4] {
        let vb = self.visible_bounds_at(height);
        [
            cgmath::Point3::new(vb.start.x, vb.start.y, height),
            cgmath::Point3::new(vb.end.x, vb.start.y, height),
            cgmath::Point3::new(vb.end.x, vb.end.y, height),
            cgmath::Point3::new(vb.start.x, vb.end.y, height),
        ]
    }

    pub fn follow(&mut self, target: &Transform, dt: f32, follow: &Follow) {
        // Determine the Z axis rotation around the target
        let swing = cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_x(), follow.angle_x);
        let mut front = target.rot.rotate_vector(cgmath::Vector3::unit_y());
        front.z = 0.0;
        let twist = cgmath::Quaternion::from_arc(cgmath::Vector3::unit_y(), front, None);

        let patch = cgmath::Quaternion::from_axis_angle(
            cgmath::Vector3::unit_z(),
            cgmath::Deg::turn_div_2(),
        );
        let rotation = patch * twist * swing;

        let k = (dt * -follow.speed).exp();
        self.rot = rotation.slerp(self.rot, k);

        let location = target.disp + (patch * twist).rotate_vector(follow.offset);
        self.loc = location * (1.0 - k) + self.loc * k;
    }

    pub fn look_by(&mut self, target: &Transform, dir: &Direction) {
        debug_assert!(dir.view.z < 0.0);
        let k = (target.disp.z - self.loc.z) / -dir.view.z;
        self.loc = target.disp + dir.view * k;
        //self.rot = cgmath::Quaternion::look_at(dir.view, cgmath::Vector3::unit_y()).invert();
        self.rot =
            cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_x(), cgmath::Deg(30.0));
    }

    pub fn focus_on(&mut self, target: &Transform) {
        use cgmath::Angle;
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

    pub fn front_face(&self) -> wgpu::FrontFace {
        if self.scale.x * self.scale.y > 0.0 {
            wgpu::FrontFace::Cw
        } else {
            wgpu::FrontFace::Ccw
        }
    }
}
