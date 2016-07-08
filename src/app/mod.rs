use cgmath;
use glutin::Event;
use gfx;

mod car;
mod game;
mod view;

pub use self::game::{Agent, Game};
pub use self::car::CarView;
pub use self::view::ResourceView;


pub type Transform = cgmath::Decomposed<cgmath::Vector3<f32>, cgmath::Quaternion<f32>>;

pub struct Camera {
    pub loc: cgmath::Vector3<f32>,
    pub rot: cgmath::Quaternion<f32>,
    proj: cgmath::PerspectiveFov<f32>,
}

pub struct Follow {
    transform: Transform,
    _move_speed: f32,
    _rot_speed: cgmath::Rad<f32>,
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

    fn follow(&mut self, target: &Transform, follow: &Follow) {
        use cgmath::Transform;
        let result = target.concat(&follow.transform);
        //TODO
        self.loc = result.disp;
        self.rot = result.rot;
    }
}


pub trait App<R: gfx::Resources> {
    fn update<I: Iterator<Item=Event>, F: gfx::Factory<R>>(&mut self, I, f32, &mut F) -> bool;
    fn draw<C: gfx::CommandBuffer<R>>(&mut self, &mut gfx::Encoder<R, C>);
}
