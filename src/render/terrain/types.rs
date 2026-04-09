use crate::space::Camera;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt as _;

use std::ops::Range;

pub(super) const SCATTER_GROUP_SIZE: [u32; 3] = [16, 16, 1];
// Has to agree with the shader
pub(super) const VOXEL_TILE_SIZE: u32 = 8;
pub(super) fn count_tiles(size: u32) -> u32 {
    (size - 1) / VOXEL_TILE_SIZE + 1
}

pub(super) const MAXIMUM_UNIFORM_BUFFER_ALIGNMENT: usize = 256;

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct Vertex {
    pub _pos: [i8; 4],
}
unsafe impl Pod for Vertex {}
unsafe impl Zeroable for Vertex {}

#[repr(C)]
#[derive(Clone, Copy, PartialEq)]
pub(super) struct SurfaceConstants {
    pub texture_scale: [f32; 4],
    pub terrain_bits: u32,
    pub delta_mode: u32,
    pub pad0: u32,
    pub pad1: u32,
}
unsafe impl Pod for SurfaceConstants {}
unsafe impl Zeroable for SurfaceConstants {}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct Constants {
    pub screen_rect: [u32; 4], // x, y, w, h
    pub cam_origin_dir: [f32; 4],
    pub sample_range: [f32; 4], // -x, +x, -y, +y
    pub fog_color: [f32; 3],
    pub pad: f32,
    pub fog_params: [f32; 4],
}
unsafe impl Pod for Constants {}
unsafe impl Zeroable for Constants {}

pub(super) struct ScatterConstants {
    pub origin: glam::Vec2,
    pub dir: glam::Vec2,
    pub sample_y: Range<f32>,
    pub sample_x: Range<f32>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VoxelConstants {
    pub voxel_size: [u32; 3],
    pub pad: u32,
    pub max_depth: f32,
    pub debug_alpha: f32,
    pub max_outer_steps: u32,
    pub max_inner_steps: u32,
}
unsafe impl Pod for VoxelConstants {}
unsafe impl Zeroable for VoxelConstants {}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct BakeConstants {
    pub voxel_size: [u32; 3],
    pub pad: u32,
    pub update_start: [i32; 4],
    pub update_end: [i32; 4],
}
unsafe impl Pod for BakeConstants {}
unsafe impl Zeroable for BakeConstants {}

impl BakeConstants {
    pub fn init_workgroups(&self, wg_size: [i32; 3]) -> [u32; 3] {
        let mut wg_count = [0u32; 3];
        for i in 0..3 {
            let first = self.update_start[i] / wg_size[i];
            let last = (self.update_end[i] - 1) / wg_size[i];
            wg_count[i] = (last + 1 - first) as u32;
        }
        wg_count
    }
    pub fn mip_workgroups(&self, wg_size: [i32; 3], dst_lod: u32) -> [u32; 3] {
        let mut wg_count = [0u32; 3];
        for i in 0..3 {
            let first =
                ((self.update_start[i] / self.voxel_size[i] as i32) >> dst_lod) / wg_size[i];
            let last =
                (((self.update_end[i] - 1) / self.voxel_size[i] as i32) >> dst_lod) / wg_size[i];
            wg_count[i] = (last + 1 - first) as u32;
        }
        wg_count
    }
}

//Note: this is very similar to `visible_bounds_at()`
// but it searches in a different parameter space
pub(super) fn compute_scatter_constants(cam: &Camera, height_scale: u32) -> ScatterConstants {
    use glam::{Vec2, Vec3};

    let cam_origin = Vec2::new(cam.loc.x, cam.loc.y);
    let cam_dir = {
        let vec = cam.dir();
        let v2 = Vec2::new(vec.x, vec.y);
        if v2.length_squared() > 0.0 {
            v2.normalize()
        } else {
            Vec2::new(0.0, 1.0)
        }
    };

    fn intersect(base: &Vec3, target: Vec3, height: u32) -> Vec2 {
        let dir = target - *base;
        let t = if dir.z == 0.0 {
            0.0
        } else {
            (height as f32 - base.z) / dir.z
        };
        let end = *base + dir * t.max(0.0);
        Vec2::new(end.x, end.y)
    }

    let mx_invp = cam.get_view_proj().inverse();
    let y_center = {
        let center = mx_invp.project_point3(Vec3::new(0.0, 0.0, 0.0));
        let center_base = intersect(&cam.loc, center, 0);
        (center_base - cam_origin).dot(cam_dir)
    };
    let mut y_range = y_center..y_center;
    let mut x0 = 0f32..0.0;
    let mut x1 = 0f32..0.0;
    let v = 1.0; // set to smaller for debugging

    let local_positions = [
        Vec3::new(v, v, 0.0),
        Vec3::new(-v, v, 0.0),
        Vec3::new(v, -v, 0.0),
        Vec3::new(-v, -v, 0.0),
    ];

    for &lp in &local_positions {
        let wp = mx_invp.project_point3(lp);
        let pa = intersect(&cam.loc, wp, 0);
        let pb = intersect(&cam.loc, wp, height_scale);
        for p in &[pa, pb] {
            let dir = *p - cam_origin;
            let y = dir.dot(cam_dir);
            y_range.start = y_range.start.min(y);
            y_range.end = y_range.end.max(y);
            let x = dir.x * cam_dir.y - dir.y * cam_dir.x;
            let range = if y > y_center { &mut x1 } else { &mut x0 };
            range.start = range.start.min(x);
            range.end = range.end.max(x);
        }
    }

    ScatterConstants {
        origin: cam_origin,
        dir: cam_dir,
        sample_y: y_range,
        sample_x: x0.end.max(-x0.start)..x1.end.max(-x1.start),
    }
}

pub(super) struct Geometry {
    pub vertex_buf: wgpu::Buffer,
    pub index_buf: wgpu::Buffer,
    pub num_indices: u32,
}

impl Geometry {
    pub fn new(vertices: &[Vertex], indices: &[u16], device: &wgpu::Device) -> Self {
        Geometry {
            vertex_buf: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("terrain-vertex"),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            index_buf: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("terrain-index"),
                contents: bytemuck::cast_slice(indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            num_indices: indices.len() as u32,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct VoxelMip {
    pub extent: wgpu::Extent3d,
    pub data_offset_in_words: u32,
}
unsafe impl Pod for VoxelMip {}
unsafe impl Zeroable for VoxelMip {}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VoxelHeader {
    pub lod_count: u32,
    pub pad: [u32; 3],
    pub mips: [VoxelMip; 16],
}
unsafe impl Pod for VoxelHeader {}
unsafe impl Zeroable for VoxelHeader {}

pub(super) struct VoxelDebugRender {
    pub pipeline: wgpu::RenderPipeline,
    pub geo: Geometry,
    pub lod_range: Option<Range<usize>>,
}
