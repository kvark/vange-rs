#[allow(dead_code)]
mod pipeline;

use crate::{
    config::settings,
    level,
    render::{
        DEPTH_FORMAT, Palette, PipelineKind, SHADOW_FORMAT, global::Context as GlobalContext,
    },
    space::Camera,
};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt as _;

use std::{mem, ops::Range};

const SCATTER_GROUP_SIZE: [u32; 3] = [16, 16, 1];
// Has to agree with the shader
const VOXEL_TILE_SIZE: u32 = 8;
fn count_tiles(size: u32) -> u32 {
    (size - 1) / VOXEL_TILE_SIZE + 1
}

const MAXIMUM_UNIFORM_BUFFER_ALIGNMENT: usize = 256;

/// Six frustum planes from a view-projection matrix, each as
/// `(normal, distance)` with the interior on the positive side.
///
/// Standard Gribb-Hartmann extraction, with `z` rather than `w + z` for the
/// near plane because wgpu's clip space has z in `0..1`, not `-1..1`.
fn frustum_planes(m: &glam::Mat4) -> [glam::Vec4; 6] {
    let c = m.to_cols_array_2d();
    let row = |i: usize| glam::Vec4::new(c[0][i], c[1][i], c[2][i], c[3][i]);
    let (x, y, z, w) = (row(0), row(1), row(2), row(3));
    [w + x, w - x, w + y, w - y, z, w - z]
}

/// True when the box is entirely outside at least one plane. Tests the
/// corner furthest along each normal, so it never culls a box that still
/// straddles the frustum.
fn box_outside(planes: &[glam::Vec4; 6], min: glam::Vec3, max: glam::Vec3) -> bool {
    planes.iter().any(|p| {
        let far = glam::Vec3::new(
            if p.x >= 0.0 { max.x } else { min.x },
            if p.y >= 0.0 { max.y } else { min.y },
            if p.z >= 0.0 { max.z } else { min.z },
        );
        p.truncate().dot(far) + p.w < 0.0
    })
}

/// How many wrapped copies of the level the mesh terrain draws around the
/// camera's own tile, per axis. Has to agree with `c_MaxTileRadius` in
/// `terrain/mesh.wgsl`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    _pos: [i8; 4],
}
unsafe impl Pod for Vertex {}
unsafe impl Zeroable for Vertex {}

#[repr(C)]
#[derive(Clone, Copy, PartialEq)]
struct SurfaceConstants {
    texture_scale: [f32; 4],
    terrain_bits: u32,
    delta_mode: u32,
    pad0: u32,
    pad1: u32,
}
unsafe impl Pod for SurfaceConstants {}
unsafe impl Zeroable for SurfaceConstants {}

#[repr(C)]
#[derive(Clone, Copy)]
struct Constants {
    screen_rect: [u32; 4], // x, y, w, h
    cam_origin_dir: [f32; 4],
    sample_range: [f32; 4], // -x, +x, -y, +y
    fog_color: [f32; 3],
    pad: f32,
    fog_params: [f32; 4],
    /// `[0]` = lighting mode. 0 = baked palette gradient (original
    /// Vangers look), 1 = unbaked albedo + cosine diffuse + shadow.
    /// The remaining slots are reserved.
    lighting_flags: [u32; 4],
    /// `[0]` = vertical spacing between slices, in altitude units
    /// (`Sliced` only, 1.0 otherwise). The remaining slots are reserved.
    terrain_params: [f32; 4],
}
unsafe impl Pod for Constants {}
unsafe impl Zeroable for Constants {}

struct ScatterConstants {
    origin: glam::Vec2,
    dir: glam::Vec2,
    sample_y: Range<f32>,
    sample_x: Range<f32>,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct VoxelConstants {
    voxel_size: [u32; 3],
    pad: u32,
    max_depth: f32,
    debug_alpha: f32,
    max_outer_steps: u32,
    max_inner_steps: u32,
}
unsafe impl Pod for VoxelConstants {}
unsafe impl Zeroable for VoxelConstants {}

#[repr(C)]
#[derive(Clone, Copy)]
struct BakeConstants {
    voxel_size: [u32; 3],
    pad: u32,
    update_start: [i32; 4],
    update_end: [i32; 4],
}
unsafe impl Pod for BakeConstants {}
unsafe impl Zeroable for BakeConstants {}

impl BakeConstants {
    fn init_workgroups(&self, wg_size: [i32; 3]) -> [u32; 3] {
        let mut wg_count = [0u32; 3];
        for i in 0..3 {
            let first = self.update_start[i] / wg_size[i];
            let last = (self.update_end[i] - 1) / wg_size[i];
            wg_count[i] = (last + 1 - first) as u32;
        }
        wg_count
    }
    fn mip_workgroups(&self, wg_size: [i32; 3], dst_lod: u32) -> [u32; 3] {
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
fn compute_scatter_constants(cam: &Camera, height_scale: u32) -> ScatterConstants {
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

struct Geometry {
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    num_indices: u32,
}

impl Geometry {
    fn new(vertices: &[Vertex], indices: &[u16], device: &wgpu::Device) -> Self {
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
struct VoxelMip {
    extent: wgpu::Extent3d,
    data_offset_in_words: u32,
}
unsafe impl Pod for VoxelMip {}
unsafe impl Zeroable for VoxelMip {}

#[repr(C)]
#[derive(Clone, Copy)]
struct VoxelHeader {
    lod_count: u32,
    pad: [u32; 3],
    mips: [VoxelMip; 16],
}
unsafe impl Pod for VoxelHeader {}
unsafe impl Zeroable for VoxelHeader {}

struct VoxelDebugRender {
    pipeline: wgpu::RenderPipeline,
    geo: Geometry,
    lod_range: Option<Range<usize>>,
}

struct RayVoxelData {
    grid: wgpu::Buffer,
    grid_bytes: u64,
    bake_pipeline_layout: wgpu::PipelineLayout,
    draw_pipeline_layout: wgpu::PipelineLayout,
    draw_shader: wgpu::ShaderModule,
    init_pipeline: wgpu::ComputePipeline,
    mip_pipeline: wgpu::ComputePipeline,
    draw_pipeline: wgpu::RenderPipeline,
    bake_bind_group: wgpu::BindGroup,
    draw_bind_group: wgpu::BindGroup,
    constant_buffer: wgpu::Buffer,
    update_buffer: wgpu::Buffer,
    voxel_size: [u32; 3],
    max_outer_steps: u32,
    max_inner_steps: u32,
    max_update_rects: usize,
    max_update_texels: usize,
    debug_alpha: f32,
    debug_render: Option<VoxelDebugRender>,
    mips: Vec<VoxelMip>,
}

/// GPU buffers for the terrain TIN: one pair per chunk.
struct ChunkBufs {
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    lods: Vec<(u32, u32)>,
    center: [f32; 2],
    min: [f32; 3],
    max: [f32; 3],
}

impl ChunkBufs {
    fn new(src: &level::tin::ChunkBuffers, device: &wgpu::Device) -> Self {
        ChunkBufs {
            vertex_buf: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("terrain-mesh-chunk-vertex"),
                contents: bytemuck::cast_slice(&src.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            index_buf: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("terrain-mesh-chunk-index"),
                contents: bytemuck::cast_slice(&src.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            lods: src.lods.clone(),
            center: src.center,
            min: src.min,
            max: src.max,
        }
    }
}

/// The two passes behind the mesh grid view.
struct WirePipelines {
    /// Colour writes masked off; fills depth so hidden edges get culled.
    depth: wgpu::RenderPipeline,
    /// The same triangles rasterised as edges.
    line: wgpu::RenderPipeline,
}

struct MeshGeometry {
    chunks: Vec<ChunkBufs>,
    /// The live triangulation, kept so terrain edits refine the mesh in
    /// place instead of rebuilding it.
    tin: level::tin::Tin,
    /// What the last `prepare` decided to draw each chunk at, which is
    /// what a refit follows. See `level::tin::Drawn`. Starts as "all of
    /// them, at full detail" so the first frame, which has no decision
    /// behind it yet, refits everything.
    drawn: Vec<Option<u8>>,
    /// Chunks the web build scaffolded but has not fitted yet; filled in a
    /// few per tick, nearest the camera first. Empty on native, where
    /// `Tin::build` fits everything up front.
    #[cfg(target_arch = "wasm32")]
    unbuilt: Vec<usize>,
    gpu_bytes: u64,
}

impl MeshGeometry {
    fn new(tin: level::tin::Tin, mesh: &level::tin::Mesh, device: &wgpu::Device) -> Self {
        let gpu_bytes = mesh
            .chunks
            .iter()
            .map(|chunk| {
                (chunk.vertices.len() * mem::size_of::<level::tin::MeshVertex>()
                    + chunk.indices.len() * mem::size_of::<u32>()) as u64
            })
            .sum();
        let chunks = mesh
            .chunks
            .iter()
            .map(|c| ChunkBufs::new(c, device))
            .collect::<Vec<_>>();
        let drawn = vec![Some(0); chunks.len()];
        MeshGeometry {
            chunks,
            tin,
            drawn,
            #[cfg(target_arch = "wasm32")]
            unbuilt: Vec::new(),
            gpu_bytes,
        }
    }

    /// Fits the `budget` unbuilt chunks nearest `origin`, replacing their
    /// empty buffers with real ones. Re-sorts the remaining queue by
    /// distance so the next call keeps the state right around the camera
    /// even after it moves.
    #[cfg(target_arch = "wasm32")]
    fn build_pending(
        &mut self,
        level: &level::Level,
        device: &wgpu::Device,
        origin: glam::Vec2,
        budget: usize,
    ) {
        if self.unbuilt.is_empty() {
            return;
        }
        self.unbuilt.sort_by_key(|&ci| {
            let c = self.chunks[ci].center;
            let d = origin - glam::Vec2::new(c[0], c[1]);
            (d.x * d.x + d.y * d.y) as u32
        });
        let mut built = 0;
        self.unbuilt.retain(|&ci| {
            if built >= budget {
                return true;
            }
            let buffers = self.tin.build_chunk(level, ci);
            self.chunks[ci] = ChunkBufs::new(&buffers, device);
            built += 1;
            false
        });
    }
}

enum Kind {
    Ray {
        pipeline: wgpu::RenderPipeline,
    },
    RayVoxel(Box<RayVoxelData>),
    Slice {
        pipeline: wgpu::RenderPipeline,
        layer_count: u32,
    },
    Paint {
        pipeline: wgpu::RenderPipeline,
        geo: Geometry,
        bar_count: u32,
    },
    Scatter {
        pipeline_layout: wgpu::PipelineLayout,
        bg_layout: wgpu::BindGroupLayout,
        scatter_pipeline: wgpu::ComputePipeline,
        clear_pipeline: wgpu::ComputePipeline,
        copy_pipeline: wgpu::RenderPipeline,
        bind_group: wgpu::BindGroup,
        compute_groups: [u32; 3],
        density: [u32; 3],
        storage_bytes: u64,
    },
    Mesh {
        pipeline: wgpu::RenderPipeline,
        /// For inspecting the triangulation. `None` when the adapter lacks
        /// `POLYGON_MODE_LINE` -- it is optional in WebGPU.
        wire_pipeline: Option<WirePipelines>,
        config: level::tin::Config,
        /// Built on the first `update_dirty`, which is where the level data
        /// first becomes available.
        geo: Option<MeshGeometry>,
        wireframe: bool,
        /// `(first index, index count, wrap tile)` per visible chunk, with
        /// the LOD already chosen. Rebuilt each frame in `prepare`.
        draws: Vec<(u32, u32, u32, u32, f32)>,
        /// Distance, in texels, at which the mesh drops to the next coarser
        /// LOD. Each step doubles it.
        ///
        /// Measured against a forced-finest render on Fostral, view
        /// distance 600: at 96 the coarser levels move the surface by up
        /// to 289 units on 4-6% of the frame, which reads as terrain
        /// changing shape as you drive. At 256 that falls to 0.02-0.27%.
        /// The frame time is the same either way here (12.9 vs 13.7 ms),
        /// so the low value was buying nothing.
        ///
        /// It also sets how often a chunk is refitted after a terrain edit:
        /// one that has dropped to half detail refits half as often. See
        /// `level::tin::detail_steps`.
        lod_distance: f32,
        /// Pin every chunk to one detail level, ignoring distance. For
        /// inspecting what a level actually looks like.
        lod_force: Option<usize>,
        /// Frustum-cull chunks before drawing. Off draws every chunk of
        /// every wrap copy, which is slow but definitionally correct - so
        /// toggling it is how you tell a culling bug apart from anything
        /// else that makes geometry come and go.
        cull: bool,
        /// Let a terrain edit wait for the chunk it lands in to be worth
        /// refitting - see `level::tin::Drawn`. Off refits everything on
        /// the tick the edit lands, which is what an offline render needs:
        /// a snapshot draws a handful of frames, so an edit a chunk is
        /// entitled to sit on would simply never appear.
        defer_refits: bool,
    },
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MemoryStats {
    /// Explicit persistent GPU data unique to the selected terrain method.
    /// Shared terrain textures, targets, pipelines, and driver allocations
    /// are deliberately excluded.
    pub method_gpu_bytes: u64,
    /// Explicit persistent CPU data unique to the selected terrain method.
    pub method_cpu_bytes: u64,
}

/// One chunk's footprint, in level texels.
pub struct MeshDebugChunk {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// One surviving (chunk, wrap copy) pair and the level it drew at.
pub struct MeshDebugDraw {
    pub chunk: u32,
    pub copy: u32,
    pub lod: u32,
    pub distance: f32,
}

pub struct MeshDebug {
    pub chunks: Vec<MeshDebugChunk>,
    pub draws: Vec<MeshDebugDraw>,
    pub lod_distance: f32,
    pub culling: bool,
    pub level_size: [f32; 2],
}

enum ShadowKind {
    Ray {
        pipeline: wgpu::RenderPipeline,
    },
    InheritRayVoxel {
        pipeline: wgpu::RenderPipeline,
        max_outer_steps: u32,
        max_inner_steps: u32,
    },
}

pub struct Flood {
    pub texture: wgpu::Texture,
    pub texture_size: u32,
    pub section_size: (u32, u32),
}

pub struct Context {
    pub surface_uni_buf: wgpu::Buffer,
    pub uniform_buf: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    color_format: wgpu::TextureFormat,
    raytrace_geo: Geometry,
    kind: Kind,
    shadow_kind: ShadowKind,
    pub terrain_texture: wgpu::Texture,
    palette_texture: wgpu::Texture,
    pub flood: Flood,
    pub dirty_rects: Vec<super::DirtyRect>,
    pub dirty_flood: bool,
    pub dirty_palette: Range<u32>,
    active_surface_constants: SurfaceConstants,
    /// `false` (default) → original baked-palette lighting; `true` →
    /// unbaked albedo + explicit cosine diffuse + shadow visibility.
    /// Wired into `evaluate_color` via `Locals::lighting_flags`.
    pub unbaked_lighting: bool,
    /// `true` (default) → smooth the terrain normal from the height-field
    /// gradient so low-poly faces shade continuously; `false` → the flat
    /// per-triangle geometric normal.
    pub smooth_normals: bool,
    ray_steps: u32,
}

impl Context {
    pub fn memory_stats(&self) -> MemoryStats {
        match self.kind {
            Kind::Ray { .. } | Kind::Slice { .. } => MemoryStats::default(),
            Kind::Paint { .. } => MemoryStats {
                method_gpu_bytes: (mem::size_of::<Vertex>() + 36 * mem::size_of::<u16>()) as u64,
                method_cpu_bytes: 0,
            },
            Kind::Scatter { storage_bytes, .. } => MemoryStats {
                method_gpu_bytes: storage_bytes,
                method_cpu_bytes: 0,
            },
            Kind::RayVoxel(ref data) => MemoryStats {
                method_gpu_bytes: data.grid_bytes,
                method_cpu_bytes: (data.mips.capacity() * mem::size_of::<VoxelMip>()) as u64,
            },
            Kind::Mesh {
                geo: Some(ref geo), ..
            } => MemoryStats {
                method_gpu_bytes: geo.gpu_bytes,
                method_cpu_bytes: geo.tin.allocated_bytes() as u64,
            },
            Kind::Mesh { geo: None, .. } => MemoryStats::default(),
        }
    }

    fn create_ray_pipeline(
        layout: &wgpu::PipelineLayout,
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        name: &str,
        kind: PipelineKind,
        entry_point: &str,
    ) -> wgpu::RenderPipeline {
        let color_descs = [Some(wgpu::ColorTargetState {
            format: color_format,
            blend: None,
            write_mask: wgpu::ColorWrites::all(),
        })];
        let (targets, depth_format) = match kind {
            PipelineKind::Main => (&color_descs[..], DEPTH_FORMAT),
            PipelineKind::Shadow => (&[][..], SHADOW_FORMAT),
        };

        let shader = super::load_shader(name, &[], device).unwrap();
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain-ray"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: mem::size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        offset: 0,
                        format: wgpu::VertexFormat::Sint8x4,
                        shader_location: 0,
                    }],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(entry_point),
                compilation_options: Default::default(),
                targets,
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }

    fn create_voxel_pipelines(
        bake_layout: &wgpu::PipelineLayout,
        draw_layout: &wgpu::PipelineLayout,
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
    ) -> (
        wgpu::ComputePipeline,
        wgpu::ComputePipeline,
        wgpu::RenderPipeline,
        wgpu::ShaderModule,
    ) {
        let substitutions = [("morton_tile_size", format!("{}u", VOXEL_TILE_SIZE))];
        let bake_shader = super::load_shader("terrain/voxel-bake", &substitutions, device).unwrap();
        let init_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Voxel init"),
            layout: Some(bake_layout),
            module: &bake_shader,
            entry_point: Some("init"),
            compilation_options: Default::default(),
            cache: None,
        });
        let mip_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Voxel mip"),
            layout: Some(bake_layout),
            module: &bake_shader,
            entry_point: Some("mip"),
            compilation_options: Default::default(),
            cache: None,
        });

        let draw_shader = super::load_shader("terrain/voxel-draw", &substitutions, device).unwrap();
        let color_descs = [Some(wgpu::ColorTargetState {
            format: color_format,
            blend: None,
            write_mask: wgpu::ColorWrites::all(),
        })];

        let draw_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain-ray-voxel"),
            layout: Some(draw_layout),
            vertex: wgpu::VertexState {
                module: &draw_shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &draw_shader,
                entry_point: Some("draw_color"),
                compilation_options: Default::default(),
                targets: &color_descs,
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        (init_pipeline, mip_pipeline, draw_pipeline, draw_shader)
    }

    fn create_voxel_shadow_pipeline(
        pipeline_layout: &wgpu::PipelineLayout,
        draw_shader: &wgpu::ShaderModule,
        device: &wgpu::Device,
    ) -> wgpu::RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain-ray-voxel"),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: draw_shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: draw_shader,
                entry_point: Some("draw_depth"),
                compilation_options: Default::default(),
                targets: &[],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }

    fn create_voxel_debug_pipeline(
        pipeline_layout: &wgpu::PipelineLayout,
        draw_shader: &wgpu::ShaderModule,
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("voxel-visualizer"),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: draw_shader,
                entry_point: Some("vert_bound"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: draw_shader,
                entry_point: Some("draw_bound"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::all(),
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }

    fn create_slice_pipeline(
        layout: &wgpu::PipelineLayout,
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        let shader = super::load_shader("terrain/slice", &[], device).unwrap();
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain-slice"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("main_vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("main_fs"),
                compilation_options: Default::default(),
                targets: &[Some(color_format.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }

    fn create_paint_pipeline(
        layout: &wgpu::PipelineLayout,
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        let shader = super::load_shader("terrain/paint", &[], device).unwrap();
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain-paint"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(color_format.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Cw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }

    /// The shaded mesh, and the same thing rasterised as edges.
    ///
    /// The grid view is purely pipeline state - identical shader, identical
    /// geometry, no extra buffers - so it always shows exactly the triangles
    /// being drawn. It takes two passes because the lines are shaded like
    /// the surface: a depth-only prepass (colour writes masked off) fills
    /// the depth buffer, then `PolygonMode::Line` draws the edges against
    /// it. Without the prepass every back-facing triangle shows through and
    /// the result is unreadable. `POLYGON_MODE_LINE` is optional in WebGPU,
    /// so the pair may be `None`.
    fn create_mesh_pipelines(
        layout: &wgpu::PipelineLayout,
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
    ) -> (wgpu::RenderPipeline, Option<WirePipelines>) {
        let shader = super::load_shader("terrain/mesh", &[], device).unwrap();
        let vertex_buffers = [wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<level::tin::MeshVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32],
        }];
        let make = |label, polygon_mode, write_mask, depth_compare, bias| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &vertex_buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fragment"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: color_format,
                        blend: None,
                        write_mask,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    // `emit_chunk` winds every face so that its outward
                    // side is the front face here. Worth ~23% of the frame
                    // on a first-person Fostral view, pixel-identical.
                    cull_mode: Some(wgpu::Face::Back),
                    polygon_mode,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(depth_compare),
                    stencil: Default::default(),
                    bias,
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let all = wgpu::ColorWrites::ALL;
        let less = wgpu::CompareFunction::Less;
        let pipeline = make(
            "terrain-mesh",
            wgpu::PolygonMode::Fill,
            all,
            less,
            Default::default(),
        );
        let wire_pipelines = if device
            .features()
            .contains(wgpu::Features::POLYGON_MODE_LINE)
        {
            Some(WirePipelines {
                depth: make(
                    "terrain-mesh-depth",
                    wgpu::PolygonMode::Fill,
                    wgpu::ColorWrites::empty(),
                    less,
                    // Line and triangle rasterisation don't interpolate
                    // depth identically, so coincident edges lose the test
                    // in patches and the wireframe comes out dashed. Push
                    // the occluder back by more than that discrepancy.
                    wgpu::DepthBiasState {
                        constant: 16,
                        slope_scale: 2.0,
                        clamp: 0.0,
                    },
                ),
                // The edges sit exactly on the surface the prepass just
                // wrote, so they need `LessEqual` to survive against it.
                line: make(
                    "terrain-mesh-wire",
                    wgpu::PolygonMode::Line,
                    all,
                    wgpu::CompareFunction::LessEqual,
                    Default::default(),
                ),
            })
        } else {
            info!("POLYGON_MODE_LINE unavailable; the mesh grid view is disabled");
            None
        };

        (pipeline, wire_pipelines)
    }

    fn create_scatter_pipelines(
        layout: &wgpu::PipelineLayout,
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
    ) -> (
        wgpu::ComputePipeline,
        wgpu::ComputePipeline,
        wgpu::RenderPipeline,
    ) {
        let shader = super::load_shader("terrain/scatter", &[], device).unwrap();
        let scatter_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("terrain-scatter"),
            layout: Some(layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let clear_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("terrain-scatter-clear"),
            layout: Some(layout),
            module: &shader,
            entry_point: Some("clear"),
            compilation_options: Default::default(),
            cache: None,
        });

        let copy_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain-scatter-copy"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("copy_vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("copy_fs"),
                compilation_options: Default::default(),
                targets: &[Some(color_format.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        (scatter_pipeline, clear_pipeline, copy_pipeline)
    }

    fn create_scatter_resources(
        extent: wgpu::Extent3d,
        layout: &wgpu::BindGroupLayout,
        device: &wgpu::Device,
    ) -> (wgpu::BindGroup, [u32; 3]) {
        let storage_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Scatter"),
            size: 4 * (extent.width * extent.height) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Scatter"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: storage_buffer.as_entire_binding(),
            }],
        });

        let group_count = [
            (extent.width / SCATTER_GROUP_SIZE[0]) + (extent.width % SCATTER_GROUP_SIZE[0]).min(1),
            (extent.height / SCATTER_GROUP_SIZE[1])
                + (extent.height % SCATTER_GROUP_SIZE[1]).min(1),
            1,
        ];
        (bind_group, group_count)
    }

    pub fn new(
        gfx: &super::GraphicsContext,
        level: &level::LevelConfig,
        level_height: u32,
        global: &GlobalContext,
        config: &settings::Terrain,
        shadow_config: &settings::ShadowTerrain,
        ray_steps: u32,
    ) -> Self {
        profiling::scope!("Init Terrain");

        let needs_compute = matches!(
            config,
            settings::Terrain::RayVoxelTraced { .. } | settings::Terrain::Scattered { .. }
        );
        let base_visibility = if needs_compute {
            wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE
        } else {
            wgpu::ShaderStages::VERTEX_FRAGMENT
        };

        let supports_vertex_storage = gfx
            .downlevel_caps
            .flags
            .contains(wgpu::DownlevelFlags::VERTEX_STORAGE);

        let extent = wgpu::Extent3d {
            width: level.size.0.as_value() as u32,
            height: level.size.1.as_value() as u32,
            depth_or_array_layers: 1,
        };
        let flood_section_count = extent.height >> level.section.as_power();
        let table_extent = wgpu::Extent3d {
            width: level.terrains.len() as u32,
            height: 1,
            depth_or_array_layers: 1,
        };

        let terrain_table = level
            .terrains
            .iter()
            .map(|terr| {
                [
                    terr.shadow_offset,
                    terr.height_shift,
                    terr.colors.start,
                    terr.colors.end,
                ]
            })
            .collect::<Vec<_>>();

        let terrain_texture = gfx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Terrain data"),
            size: wgpu::Extent3d {
                width: extent.width / 2,
                ..extent
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Uint,
            view_formats: &[],
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        });

        let flood_texture = gfx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Terrain flood"),
            size: wgpu::Extent3d {
                width: flood_section_count,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            view_formats: &[],
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        });
        let table_texture = gfx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Terrain table"),
            size: table_extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Uint,
            view_formats: &[],
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        });

        gfx.queue.write_texture(
            table_texture.as_image_copy(),
            bytemuck::cast_slice(&terrain_table),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(table_extent.width * 4),
                rows_per_image: None,
            },
            table_extent,
        );

        let palette = Palette::new(&gfx.device);

        let flood_sampler = gfx.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let table_sampler = gfx.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let bind_group_layout =
            gfx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Terrain"),
                    entries: &[
                        // surface uniforms
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: base_visibility,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        // terrain locals
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: base_visibility,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        // terrain data
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: base_visibility,
                            ty: wgpu::BindingType::Texture {
                                view_dimension: wgpu::TextureViewDimension::D2,
                                sample_type: wgpu::TextureSampleType::Uint,
                                multisampled: false,
                            },
                            count: None,
                        },
                        // flood map (D2 with height=1 for WebGPU compat)
                        wgpu::BindGroupLayoutEntry {
                            binding: 4,
                            visibility: base_visibility,
                            ty: wgpu::BindingType::Texture {
                                view_dimension: wgpu::TextureViewDimension::D2,
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                multisampled: false,
                            },
                            count: None,
                        },
                        // table map
                        wgpu::BindGroupLayoutEntry {
                            binding: 5,
                            visibility: base_visibility & !wgpu::ShaderStages::VERTEX,
                            ty: wgpu::BindingType::Texture {
                                view_dimension: wgpu::TextureViewDimension::D2,
                                sample_type: wgpu::TextureSampleType::Uint,
                                multisampled: false,
                            },
                            count: None,
                        },
                        // palette map
                        wgpu::BindGroupLayoutEntry {
                            binding: 6,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                view_dimension: wgpu::TextureViewDimension::D2,
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                multisampled: false,
                            },
                            count: None,
                        },
                        // flood sampler
                        wgpu::BindGroupLayoutEntry {
                            binding: 8,
                            visibility: base_visibility & !wgpu::ShaderStages::VERTEX,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                        // table sampler
                        wgpu::BindGroupLayoutEntry {
                            binding: 9,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });

        let surface_uni_buf = gfx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("surface-uniforms"),
            size: mem::size_of::<SurfaceConstants>() as _,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_buf = gfx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Terrain uniforms"),
            size: mem::size_of::<Constants>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = gfx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Terrain"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: surface_uni_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &terrain_texture.create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(
                        &flood_texture.create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(
                        &table_texture.create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&palette.view),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::Sampler(&flood_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::Sampler(&table_sampler),
                },
            ],
        });

        let pipeline_layout = gfx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("terrain"),
                bind_group_layouts: &[Some(&global.bind_group_layout), Some(&bind_group_layout)],
                immediate_size: 0,
            });

        let raytrace_geo = Geometry::new(
            &[
                Vertex { _pos: [0, 0, 0, 1] },
                Vertex {
                    _pos: [-1, 0, 0, 0],
                },
                Vertex {
                    _pos: [0, -1, 0, 0],
                },
                Vertex { _pos: [1, 0, 0, 0] },
                Vertex { _pos: [0, 1, 0, 0] },
            ],
            &[0u16, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 1],
            &gfx.device,
        );

        let kind = match *config {
            settings::Terrain::RayTraced => {
                let pipeline = Self::create_ray_pipeline(
                    &pipeline_layout,
                    &gfx.device,
                    gfx.color_format,
                    "terrain/ray",
                    PipelineKind::Main,
                    "ray_color",
                );
                Kind::Ray { pipeline }
            }
            settings::Terrain::RayVoxelTraced {
                voxel_size,
                max_outer_steps,
                max_inner_steps,
                max_update_texels,
            } => {
                let bake_bg_layout =
                    gfx.device
                        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                            label: Some("Voxel bake"),
                            entries: &[
                                // voxel grid
                                wgpu::BindGroupLayoutEntry {
                                    binding: 0,
                                    visibility: wgpu::ShaderStages::COMPUTE,
                                    ty: wgpu::BindingType::Buffer {
                                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                                        has_dynamic_offset: false,
                                        min_binding_size: None,
                                    },
                                    count: None,
                                },
                                // update constants
                                wgpu::BindGroupLayoutEntry {
                                    binding: 1,
                                    visibility: wgpu::ShaderStages::COMPUTE,
                                    ty: wgpu::BindingType::Buffer {
                                        ty: wgpu::BufferBindingType::Uniform,
                                        has_dynamic_offset: true,
                                        min_binding_size: wgpu::BufferSize::new(mem::size_of::<
                                            BakeConstants,
                                        >(
                                        )
                                            as _),
                                    },
                                    count: None,
                                },
                                // mip constant
                                wgpu::BindGroupLayoutEntry {
                                    binding: 2,
                                    visibility: wgpu::ShaderStages::COMPUTE,
                                    ty: wgpu::BindingType::Buffer {
                                        ty: wgpu::BufferBindingType::Uniform,
                                        has_dynamic_offset: true,
                                        min_binding_size: wgpu::BufferSize::new(4),
                                    },
                                    count: None,
                                },
                            ],
                        });
                let bake_pipeline_layout =
                    gfx.device
                        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                            label: Some("Voxel bake"),
                            bind_group_layouts: &[Some(&bake_bg_layout), Some(&bind_group_layout)],
                            immediate_size: 0,
                        });

                let draw_bg_layout =
                    gfx.device
                        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                            label: Some("Voxel draw"),
                            entries: &[
                                // voxel grid
                                wgpu::BindGroupLayoutEntry {
                                    binding: 0,
                                    visibility: if supports_vertex_storage {
                                        wgpu::ShaderStages::VERTEX_FRAGMENT
                                            | wgpu::ShaderStages::COMPUTE
                                    } else {
                                        wgpu::ShaderStages::FRAGMENT
                                    },
                                    ty: wgpu::BindingType::Buffer {
                                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                                        has_dynamic_offset: false,
                                        min_binding_size: None,
                                    },
                                    count: None,
                                },
                                // uniform buffer
                                wgpu::BindGroupLayoutEntry {
                                    binding: 1,
                                    visibility: wgpu::ShaderStages::VERTEX
                                        | wgpu::ShaderStages::FRAGMENT,
                                    ty: wgpu::BindingType::Buffer {
                                        ty: wgpu::BufferBindingType::Uniform,
                                        has_dynamic_offset: false,
                                        min_binding_size: wgpu::BufferSize::new(mem::size_of::<
                                            VoxelConstants,
                                        >(
                                        )
                                            as _),
                                    },
                                    count: None,
                                },
                            ],
                        });
                let draw_pipeline_layout =
                    gfx.device
                        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                            label: Some("voxel"),
                            bind_group_layouts: &[
                                Some(&global.bind_group_layout),
                                Some(&bind_group_layout),
                                Some(&draw_bg_layout),
                            ],
                            immediate_size: 0,
                        });

                let (init_pipeline, mip_pipeline, draw_pipeline, draw_shader) =
                    Self::create_voxel_pipelines(
                        &bake_pipeline_layout,
                        &draw_pipeline_layout,
                        &gfx.device,
                        gfx.color_format,
                    );

                let debug_render = if supports_vertex_storage {
                    Some(VoxelDebugRender {
                        pipeline: Self::create_voxel_debug_pipeline(
                            &draw_pipeline_layout,
                            &draw_shader,
                            &gfx.device,
                            gfx.color_format,
                        ),
                        geo: Geometry::new(
                            &[
                                Vertex { _pos: [0; 4] }, //dummy
                            ],
                            &[
                                // Bit 0 = shift in X away from the camera
                                // Bits 1=Y and 2=Z in the same way
                                // lower half
                                0, 1, 3, 3, 2, 0, 0, 2, 6, 6, 4, 0, 0, 4, 5, 5, 1, 0,
                            ],
                            &gfx.device,
                        ),
                        lod_range: None,
                    })
                } else {
                    None
                };

                let grid_extent = wgpu::Extent3d {
                    width: (extent.width - 1) / voxel_size[0] + 1,
                    height: (extent.height - 1) / voxel_size[1] + 1,
                    depth_or_array_layers: (level_height - 1) / voxel_size[2] + 1,
                };
                let mip_level_count = 32
                    - grid_extent
                        .width
                        .min(grid_extent.height)
                        .min(grid_extent.depth_or_array_layers)
                        .leading_zeros();

                assert_eq!(mem::size_of::<VoxelMip>(), 16);
                let mut header = VoxelHeader {
                    lod_count: mip_level_count,
                    pad: [0; 3],
                    mips: [VoxelMip::default(); 16],
                };
                let mut data_offset_in_words = 0;
                let mut mips = Vec::new();
                for base_mip_level in 0..mip_level_count {
                    let mip_extent =
                        grid_extent.mip_level_size(base_mip_level, wgpu::TextureDimension::D3);
                    mips.push(VoxelMip {
                        extent: mip_extent,
                        data_offset_in_words,
                    });
                    header.mips[base_mip_level as usize] = VoxelMip {
                        extent: mip_extent,
                        data_offset_in_words,
                    };
                    let tile_count = count_tiles(mip_extent.width)
                        * count_tiles(mip_extent.height)
                        * count_tiles(mip_extent.depth_or_array_layers);
                    data_offset_in_words += tile_count * VOXEL_TILE_SIZE.pow(3) / 32;
                }
                log::info!(
                    "Allocating {} MB storage buffer for the voxel grid",
                    data_offset_in_words >> 18
                );

                let grid_bytes =
                    (mem::size_of::<VoxelHeader>() + data_offset_in_words as usize * 4) as u64;
                let grid = gfx.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Grid"),
                    size: grid_bytes,
                    // COPY_SRC so `debug_voxel_occupancy` can read the
                    // acceleration structure back and check it against the
                    // height map.
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_DST
                        | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
                gfx.queue
                    .write_buffer(&grid, 0, bytemuck::bytes_of(&header));

                let constant_buffer = gfx.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Voxel constants"),
                    size: mem::size_of::<VoxelConstants>() as _,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let draw_bind_group = gfx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Voxel draw"),
                    layout: &draw_bg_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: grid.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: constant_buffer.as_entire_binding(),
                        },
                    ],
                });

                let max_update_rects = 10usize;
                assert!(mem::size_of::<BakeConstants>() <= MAXIMUM_UNIFORM_BUFFER_ALIGNMENT);
                let update_buffer = gfx.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Bake constants"),
                    size: (MAXIMUM_UNIFORM_BUFFER_ALIGNMENT * max_update_rects)
                        as wgpu::BufferAddress,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let mip_data = {
                    let total = MAXIMUM_UNIFORM_BUFFER_ALIGNMENT * mip_level_count as usize;
                    let mut data = vec![0u8; total];
                    for i in 0..mip_level_count as usize {
                        // initializing the least significant byte of the word
                        data[i * MAXIMUM_UNIFORM_BUFFER_ALIGNMENT] = i as u8;
                    }
                    data
                };
                let mip_buffer = gfx
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Bake mip constant"),
                        contents: &mip_data,
                        usage: wgpu::BufferUsages::UNIFORM,
                    });

                let bake_bind_group = gfx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Bake group"),
                    layout: &bake_bg_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: grid.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &update_buffer,
                                offset: 0,
                                size: wgpu::BufferSize::new(mem::size_of::<BakeConstants>() as _),
                            }),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &mip_buffer,
                                offset: 0,
                                size: wgpu::BufferSize::new(4),
                            }),
                        },
                    ],
                });

                Kind::RayVoxel(Box::new(RayVoxelData {
                    grid,
                    grid_bytes,
                    bake_pipeline_layout,
                    draw_pipeline_layout,
                    draw_shader,
                    init_pipeline,
                    mip_pipeline,
                    draw_pipeline,
                    draw_bind_group,
                    bake_bind_group,
                    constant_buffer,
                    update_buffer,
                    voxel_size,
                    max_outer_steps,
                    max_inner_steps,
                    max_update_rects,
                    max_update_texels,
                    debug_alpha: 0.0,
                    debug_render,
                    mips,
                }))
            }
            settings::Terrain::Sliced => {
                let pipeline =
                    Self::create_slice_pipeline(&pipeline_layout, &gfx.device, gfx.color_format);

                Kind::Slice {
                    pipeline,
                    layer_count: level_height,
                }
            }
            settings::Terrain::Painted => {
                assert!(supports_vertex_storage);

                let geo = Geometry::new(
                    &[
                        Vertex { _pos: [0; 4] }, //dummy
                    ],
                    &[
                        // Bit 0 = shift in X away from the camera
                        // Bits 1=Y and 2=Z in the same way
                        // lower half
                        0, 1, 3, 3, 2, 0, 0, 2, 6, 6, 4, 0, 0, 4, 5, 5, 1, 0,
                        // higher half
                        0x10, 0x11, 0x13, 0x13, 0x12, 0x10, 0x10, 0x12, 0x16, 0x16, 0x14, 0x10,
                        0x10, 0x14, 0x15, 0x15, 0x11, 0x10,
                    ],
                    &gfx.device,
                );

                let pipeline =
                    Self::create_paint_pipeline(&pipeline_layout, &gfx.device, gfx.color_format);

                Kind::Paint {
                    pipeline,
                    geo,
                    bar_count: 0,
                }
            }
            settings::Terrain::Scattered { density } => {
                let local_bg_layout =
                    gfx.device
                        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                            label: Some("Terrain locals"),
                            entries: &[
                                // output map
                                wgpu::BindGroupLayoutEntry {
                                    binding: 0,
                                    visibility: wgpu::ShaderStages::FRAGMENT
                                        | wgpu::ShaderStages::COMPUTE,
                                    ty: wgpu::BindingType::Buffer {
                                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                                        has_dynamic_offset: false,
                                        min_binding_size: None,
                                    },
                                    count: None,
                                },
                            ],
                        });
                let local_pipeline_layout =
                    gfx.device
                        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                            label: Some("scatter"),
                            bind_group_layouts: &[
                                Some(&global.bind_group_layout),
                                Some(&bind_group_layout),
                                Some(&local_bg_layout),
                            ],
                            immediate_size: 0,
                        });

                let (scatter_pipeline, clear_pipeline, copy_pipeline) =
                    Self::create_scatter_pipelines(
                        &local_pipeline_layout,
                        &gfx.device,
                        gfx.color_format,
                    );
                let (local_bg, compute_groups) =
                    Self::create_scatter_resources(gfx.screen_size, &local_bg_layout, &gfx.device);
                Kind::Scatter {
                    pipeline_layout: local_pipeline_layout,
                    bg_layout: local_bg_layout,
                    scatter_pipeline,
                    clear_pipeline,
                    copy_pipeline,
                    bind_group: local_bg,
                    compute_groups,
                    density,
                    storage_bytes: 4 * (gfx.screen_size.width * gfx.screen_size.height) as u64,
                }
            }
            settings::Terrain::Mesh { quality } => {
                let (pipeline, wire_pipeline) =
                    Self::create_mesh_pipelines(&pipeline_layout, &gfx.device, gfx.color_format);
                Kind::Mesh {
                    pipeline,
                    wire_pipeline,
                    config: level::tin::Config { quality },
                    geo: None,
                    wireframe: false,
                    draws: Vec::new(),
                    lod_distance: 256.0,
                    lod_force: None,
                    defer_refits: true,
                    cull: true,
                }
            }
        };

        let shadow_kind = match *shadow_config {
            settings::ShadowTerrain::RayTraced => {
                let pipeline = Self::create_ray_pipeline(
                    &pipeline_layout,
                    &gfx.device,
                    gfx.color_format,
                    "terrain/ray",
                    PipelineKind::Shadow,
                    "ray_depth",
                );
                ShadowKind::Ray { pipeline }
            }
            settings::ShadowTerrain::RayVoxelTraced {
                max_outer_steps,
                max_inner_steps,
            } => match kind {
                Kind::RayVoxel(ref rv) => ShadowKind::InheritRayVoxel {
                    pipeline: Self::create_voxel_shadow_pipeline(
                        &rv.draw_pipeline_layout,
                        &rv.draw_shader,
                        &gfx.device,
                    ),
                    max_outer_steps,
                    max_inner_steps,
                },
                _ => panic!("Unable to inherit the shadow voxel context"),
            },
        };

        Context {
            surface_uni_buf,
            uniform_buf,
            bind_group,
            bind_group_layout,
            pipeline_layout,
            color_format: gfx.color_format,
            raytrace_geo,
            kind,
            shadow_kind,
            terrain_texture,
            palette_texture: palette.texture,
            flood: Flood {
                texture: flood_texture,
                texture_size: flood_section_count,
                section_size: (
                    level.size.0.as_value() as u32,
                    1 << level.section.as_power(),
                ),
            },
            dirty_rects: vec![super::DirtyRect {
                rect: super::Rect {
                    x: 0,
                    y: 0,
                    w: extent.width as _,
                    h: extent.height as _,
                },
                z_range: 0..level_height as _,
                need_upload: true,
            }],
            dirty_flood: true,
            dirty_palette: 0..0x100,
            active_surface_constants: SurfaceConstants {
                texture_scale: [0.0; 4],
                terrain_bits: 0,
                delta_mode: 0,
                pad0: 0,
                pad1: 0,
            },
            // Default to unbaked diffuse + shadow — matches the look
            // we tuned in the cosine-lighting commit. Toggle in the UI
            // to A/B against the original baked-palette path.
            unbaked_lighting: true,
            smooth_normals: true,
            ray_steps,
        }
    }

    pub fn reload(&mut self, device: &wgpu::Device) {
        match self.kind {
            Kind::Ray {
                ref mut pipeline, ..
            } => {
                *pipeline = Self::create_ray_pipeline(
                    &self.pipeline_layout,
                    device,
                    self.color_format,
                    "terrain/ray",
                    PipelineKind::Main,
                    "ray_color",
                );
            }
            Kind::RayVoxel(ref mut rv) => {
                let (init, mip, draw, shader) = Self::create_voxel_pipelines(
                    &rv.bake_pipeline_layout,
                    &rv.draw_pipeline_layout,
                    device,
                    self.color_format,
                );
                if let Some(ref mut debug) = rv.debug_render {
                    debug.pipeline = Self::create_voxel_debug_pipeline(
                        &rv.draw_pipeline_layout,
                        &shader,
                        device,
                        self.color_format,
                    );
                }
                rv.init_pipeline = init;
                rv.mip_pipeline = mip;
                rv.draw_pipeline = draw;
                rv.draw_shader = shader;
            }
            Kind::Slice {
                ref mut pipeline, ..
            } => {
                *pipeline =
                    Self::create_slice_pipeline(&self.pipeline_layout, device, self.color_format);
            }
            Kind::Paint {
                ref mut pipeline, ..
            } => {
                *pipeline =
                    Self::create_paint_pipeline(&self.pipeline_layout, device, self.color_format);
            }
            Kind::Mesh {
                ref mut pipeline,
                ref mut wire_pipeline,
                ..
            } => {
                let (fill, wire) =
                    Self::create_mesh_pipelines(&self.pipeline_layout, device, self.color_format);
                *pipeline = fill;
                *wire_pipeline = wire;
            }
            Kind::Scatter {
                ref pipeline_layout,
                ref mut scatter_pipeline,
                ref mut clear_pipeline,
                ref mut copy_pipeline,
                ..
            } => {
                let (scatter, clear, copy) =
                    Self::create_scatter_pipelines(pipeline_layout, device, self.color_format);
                *scatter_pipeline = scatter;
                *clear_pipeline = clear;
                *copy_pipeline = copy;
            }
        }

        match self.shadow_kind {
            ShadowKind::Ray {
                ref mut pipeline, ..
            } => {
                *pipeline = Self::create_ray_pipeline(
                    &self.pipeline_layout,
                    device,
                    self.color_format,
                    "terrain/ray",
                    PipelineKind::Shadow,
                    "ray",
                );
            }
            ShadowKind::InheritRayVoxel {
                ref mut pipeline, ..
            } => match self.kind {
                Kind::RayVoxel(ref rv) => {
                    *pipeline = Self::create_voxel_shadow_pipeline(
                        &rv.draw_pipeline_layout,
                        &rv.draw_shader,
                        device,
                    );
                }
                _ => unreachable!(),
            },
        }
    }

    pub fn resize(&mut self, extent: wgpu::Extent3d, device: &wgpu::Device) {
        if let Kind::Scatter {
            ref bg_layout,
            ref mut bind_group,
            ref mut compute_groups,
            ..
        } = self.kind
        {
            let (bg, gs) = Self::create_scatter_resources(extent, bg_layout, device);
            *bind_group = bg;
            *compute_groups = gs;
        }
    }

    pub fn update_dirty(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        level: &level::Level,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        // Only the web build's incremental first fit reads this.
        #[cfg_attr(not(target_arch = "wasm32"), allow(unused_variables))] origin: glam::Vec2,
    ) {
        let surface_constants = {
            let bits = level.terrain_bits();
            let delta_mode = (level.geometry.delta_mask << 16)
                | ((level.geometry.delta_const as u32) << 8)
                | level.geometry.delta_power as u32;
            SurfaceConstants {
                texture_scale: [
                    level.size.0 as f32,
                    level.size.1 as f32,
                    level.geometry.height as f32,
                    0.0,
                ],
                terrain_bits: bits.shift as u32 | ((bits.mask as u32) << 4),
                delta_mode,
                pad0: 0,
                pad1: 0,
            }
        };
        if surface_constants != self.active_surface_constants {
            self.active_surface_constants = surface_constants;
            let staging_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("temp-surface-uniforms"),
                contents: bytemuck::bytes_of(&surface_constants),
                usage: wgpu::BufferUsages::COPY_SRC,
            });
            encoder.copy_buffer_to_buffer(
                &staging_buf,
                0,
                &self.surface_uni_buf,
                0,
                mem::size_of::<SurfaceConstants>() as wgpu::BufferAddress,
            );
            // Update acceleration structures
            self.dirty_rects.push(super::DirtyRect {
                rect: super::Rect {
                    x: 0,
                    y: 0,
                    w: level.size.0 as _,
                    h: level.size.1 as _,
                },
                z_range: 0..level.geometry.height as _,
                need_upload: false,
            });
        }

        // The TIN needs the level data, which `new()` never sees, so it is
        // fitted on the first update and refined on every one after.
        if let Kind::Mesh {
            ref config,
            ref mut geo,
            defer_refits,
            ..
        } = self.kind
        {
            match *geo {
                None => {
                    // Build the whole TIN at once on native, where rayon
                    // spreads the chunks over everything cores. On wasm the
                    // single-threaded fit is the seconds-long startup hitch,
                    // so scaffold empty chunks and fill them in a few per
                    // tick, nearest the camera first (`build_pending`).
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        let (tin, mesh) = level::tin::Tin::build(level, config);
                        *geo = Some(MeshGeometry::new(tin, &mesh, device));
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        let (tin, mesh) = level::tin::Tin::scaffold(level, config);
                        let geo = &mut *geo;
                        *geo = Some(MeshGeometry::new(tin, &mesh, device));
                        let loaded = geo.as_mut().unwrap();
                        loaded.unbuilt = (0..loaded.chunks.len()).collect();
                        // The frame that scaffolds also builds the first
                        // few chunks, so the camera seat is not blank.
                        loaded.build_pending(level, device, origin, 4);
                    }
                }
                Some(ref mut geo) => {
                    // One batched refit for the whole frame's edits. Several
                    // dirty rectangles routinely land in the same chunk, and
                    // refitting per rectangle rebuilt that chunk - and its
                    // GPU buffers - once for each of them.
                    let rects = self
                        .dirty_rects
                        .iter()
                        .filter(|dr| dr.need_upload)
                        .map(|dr| level::tin::Rect {
                            x: dr.rect.x as i32,
                            y: dr.rect.y as i32,
                            w: dr.rect.w as u32,
                            h: dr.rect.h as u32,
                        })
                        .collect::<Vec<_>>();
                    // Each refitted chunk just gets fresh buffers -- they
                    // are small enough that rebuilding beats patching.
                    let MeshGeometry {
                        ref mut tin,
                        ref mut chunks,
                        ref drawn,
                        ..
                    } = *geo;
                    let drawn = defer_refits.then_some(drawn.as_slice());
                    for (index, buffers) in tin.update(level, &rects, drawn) {
                        chunks[index] = ChunkBufs::new(&buffers, device);
                    }
                    // The scaffolded chunks still awaiting their first fit:
                    // one a tick, nearest the camera, so the visible area
                    // fills over the opening frames and distant terrain
                    // catches up as the level is driven across.
                    #[cfg(target_arch = "wasm32")]
                    geo.build_pending(level, device, origin, 1);
                }
            }
        }

        if !self.dirty_rects.is_empty() {
            for dr in self.dirty_rects.iter_mut() {
                if !dr.need_upload {
                    continue;
                }
                // Only the sub-rectangle, not the whole row. A moving-land
                // patch is a couple of hundred texels across on a level that
                // can be 2048 wide, and a level animating several of them at
                // once spends the whole frame budget interleaving rows that
                // did not change.
                //
                // One texture texel packs a *pair* of level texels, so the
                // span has to be widened to a pair boundary at both ends.
                // `write_texture` takes care of the staging copy, which also
                // gets rid of a buffer allocation per rectangle.
                let x0 = dr.rect.x as usize & !1;
                let x1 = (((dr.rect.x + dr.rect.w) as usize + 1) & !1).min(level.size.0 as usize);
                if x1 <= x0 || dr.rect.h == 0 {
                    dr.need_upload = false;
                    continue;
                }
                let row_bytes = (x1 - x0) * 2;
                let mut staging_data = vec![0u8; dr.rect.h as usize * row_bytes];
                for (y_off, line) in staging_data.chunks_mut(row_bytes).enumerate() {
                    let base = (dr.rect.y as usize + y_off) * level.size.0 as usize;
                    let heights = &level.height[base + x0..base + x1];
                    let metas = &level.meta[base + x0..base + x1];
                    for (i, (&h, &m)) in heights.iter().zip(metas).enumerate() {
                        line[2 * i] = h;
                        line[2 * i + 1] = m;
                    }
                }

                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.terrain_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: (x0 / 2) as u32,
                            y: dr.rect.y as u32,
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &staging_data,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row_bytes as u32),
                        rows_per_image: None,
                    },
                    wgpu::Extent3d {
                        width: ((x1 - x0) / 2) as u32,
                        height: dr.rect.h as u32,
                        depth_or_array_layers: 1,
                    },
                );

                dr.need_upload = false;
            }

            match self.kind {
                Kind::RayVoxel(ref rv) => {
                    let RayVoxelData {
                        ref init_pipeline,
                        ref mip_pipeline,
                        ref mips,
                        ref bake_bind_group,
                        ref update_buffer,
                        voxel_size,
                        max_update_rects,
                        max_update_texels,
                        ..
                    } = **rv;
                    fn align_down(v: u16, tile: u32) -> i32 {
                        assert!(tile.is_power_of_two());
                        (v as u32 & !(tile - 1)) as i32
                    }
                    fn align_up(v: u16, tile: u32) -> i32 {
                        ((v as u32 + tile - 1) & !(tile - 1)) as i32
                    }

                    let mut texels_to_update = max_update_texels;
                    let mut update_buffer_contents = Vec::new();
                    while let Some(dr) = self.dirty_rects.pop() {
                        let num_texels = dr.rect.w as usize * dr.rect.h as usize;
                        if num_texels > max_update_texels {
                            // split into 4 quadrants
                            let mid_x = dr.rect.x + dr.rect.w / 2;
                            let mid_y = dr.rect.y + dr.rect.h / 2;
                            for (xb, yb) in
                                [(false, false), (true, false), (false, true), (true, true)]
                            {
                                self.dirty_rects.push(super::DirtyRect {
                                    rect: super::Rect {
                                        x: if xb { mid_x } else { dr.rect.x },
                                        y: if yb { mid_y } else { dr.rect.y },
                                        w: if xb {
                                            dr.rect.x + dr.rect.w - mid_x
                                        } else {
                                            mid_x - dr.rect.x
                                        },
                                        h: if yb {
                                            dr.rect.y + dr.rect.h - mid_y
                                        } else {
                                            mid_y - dr.rect.y
                                        },
                                    },
                                    z_range: dr.z_range.clone(),
                                    need_upload: false,
                                });
                            }
                        } else if num_texels > texels_to_update
                            || update_buffer_contents.len() == max_update_rects
                        {
                            self.dirty_rects.push(dr);
                            break;
                        } else {
                            update_buffer_contents.push(BakeConstants {
                                voxel_size,
                                pad: 0,
                                update_start: [
                                    align_down(dr.rect.x, voxel_size[0]),
                                    align_down(dr.rect.y, voxel_size[1]),
                                    align_down(dr.z_range.start, voxel_size[2]),
                                    0,
                                ],
                                update_end: [
                                    align_up(dr.rect.x + dr.rect.w, voxel_size[0]),
                                    align_up(dr.rect.y + dr.rect.h, voxel_size[1]),
                                    align_up(dr.z_range.end, voxel_size[2]),
                                    0,
                                ],
                            });
                            // Decrement the per-frame budget by the rect we
                            // just enqueued, so further small rects can pack
                            // into the same dispatch up to `max_update_rects`.
                            // The previous `-= texels_to_update` zeroed the
                            // budget after the first push and capped us to
                            // one rect per frame.
                            texels_to_update -= num_texels;
                        }
                    }

                    let staging_buf =
                        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("Voxel bake update"),
                            contents: bytemuck::cast_slice(&update_buffer_contents),
                            usage: wgpu::BufferUsages::COPY_SRC,
                        });
                    for i in 0..update_buffer_contents.len() {
                        encoder.copy_buffer_to_buffer(
                            &staging_buf,
                            (i * mem::size_of::<BakeConstants>()) as wgpu::BufferAddress,
                            update_buffer,
                            (i * MAXIMUM_UNIFORM_BUFFER_ALIGNMENT) as wgpu::BufferAddress,
                            mem::size_of::<BakeConstants>() as wgpu::BufferAddress,
                        );
                    }

                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("Voxel bake"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(init_pipeline);
                    pass.set_bind_group(1, &self.bind_group, &[]);
                    for (i, update) in update_buffer_contents.iter().enumerate() {
                        let groups = update.init_workgroups([8, 8, 1]);
                        let offset = i * MAXIMUM_UNIFORM_BUFFER_ALIGNMENT;
                        pass.set_bind_group(0, bake_bind_group, &[offset as u32, 0]);
                        pass.dispatch_workgroups(groups[0], groups[1], 1);
                    }
                    pass.set_pipeline(mip_pipeline);
                    for dst_lod in 1..mips.len() {
                        for (i, update) in update_buffer_contents.iter().enumerate() {
                            let groups = update.mip_workgroups([4, 4, 4], dst_lod as u32);
                            let offset = i * MAXIMUM_UNIFORM_BUFFER_ALIGNMENT;
                            let mip_data_offset = (dst_lod - 1) * MAXIMUM_UNIFORM_BUFFER_ALIGNMENT;
                            pass.set_bind_group(
                                0,
                                bake_bind_group,
                                &[offset as u32, mip_data_offset as u32],
                            );
                            pass.dispatch_workgroups(groups[0], groups[1], groups[2]);
                        }
                    }
                }
                _ => {
                    self.dirty_rects.clear();
                }
            }
        }

        if self.dirty_flood {
            let staging_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("staging flood update"),
                contents: &level.flood_map,
                usage: wgpu::BufferUsages::COPY_SRC,
            });

            encoder.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging_buf,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(0x100),
                        rows_per_image: None,
                    },
                },
                self.flood.texture.as_image_copy(),
                wgpu::Extent3d {
                    width: self.flood.texture_size,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );

            self.dirty_flood = false;
        }

        if self.dirty_palette.start != self.dirty_palette.end {
            let staging_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("staging palette update"),
                contents: bytemuck::cast_slice(&level.palette[self.dirty_palette.start as usize..]),
                usage: wgpu::BufferUsages::COPY_SRC,
            });
            let mut img_copy = self.palette_texture.as_image_copy();
            img_copy.origin.x = self.dirty_palette.start;

            encoder.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging_buf,
                    layout: wgpu::TexelCopyBufferLayout::default(),
                },
                img_copy,
                wgpu::Extent3d {
                    width: self.dirty_palette.end - self.dirty_palette.start,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            self.dirty_palette = 0..0;
        }
    }

    pub fn prepare(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
        global: &GlobalContext,
        fog: &settings::Fog,
        level_height: u32,
        cam: &Camera,
        screen_rect: super::Rect,
    ) {
        let sc = if let Kind::Scatter { .. } = self.kind {
            compute_scatter_constants(cam, level_height)
        } else {
            let bounds = cam.visible_bounds();
            ScatterConstants {
                origin: cam.loc.truncate(),
                dir: cam.dir().truncate(),
                sample_x: bounds.start.x..bounds.end.x,
                sample_y: bounds.start.y..bounds.end.y,
            }
        };

        {
            // constants update — shadow + main passes read the same
            // uniform_buf, so the in-encoder copy ordering matters.
            // See the TODO above re: switching to write_buffer.
            let depth_range = cam.depth_range();
            let staging = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("temp-constants"),
                contents: bytemuck::bytes_of(&Constants {
                    screen_rect: [
                        screen_rect.x as u32,
                        screen_rect.y as u32,
                        screen_rect.w as u32,
                        screen_rect.h as u32,
                    ],
                    cam_origin_dir: [sc.origin.x, sc.origin.y, sc.dir.x, sc.dir.y],
                    sample_range: [
                        sc.sample_x.start,
                        sc.sample_x.end,
                        sc.sample_y.start,
                        sc.sample_y.end,
                    ],
                    fog_color: fog.color,
                    pad: 1.0,
                    fog_params: [depth_range.end - fog.depth, depth_range.end, 0.0, 0.0],
                    lighting_flags: [
                        self.unbaked_lighting as u32,
                        self.smooth_normals as u32,
                        0,
                        0,
                    ],
                    terrain_params: [self.slice_spacing(), self.ray_steps as f32, 0.0, 0.0],
                }),
                usage: wgpu::BufferUsages::COPY_SRC,
            });
            encoder.copy_buffer_to_buffer(
                &staging,
                0,
                &self.uniform_buf,
                0,
                mem::size_of::<Constants>() as wgpu::BufferAddress,
            );
        }

        match self.kind {
            Kind::RayVoxel(ref rv) => {
                let constants = VoxelConstants {
                    voxel_size: rv.voxel_size,
                    pad: 0,
                    max_depth: cam.depth_range().end,
                    debug_alpha: rv.debug_alpha,
                    max_outer_steps: rv.max_outer_steps,
                    max_inner_steps: rv.max_inner_steps,
                };
                let constant_update =
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("ray-voxel constants"),
                        contents: bytemuck::bytes_of(&constants),
                        usage: wgpu::BufferUsages::COPY_SRC,
                    });
                encoder.copy_buffer_to_buffer(
                    &constant_update,
                    0,
                    &rv.constant_buffer,
                    0,
                    mem::size_of::<VoxelConstants>() as wgpu::BufferAddress,
                );
            }
            Kind::Paint {
                ref mut bar_count, ..
            } => {
                let rows = (sc.sample_y.end - sc.sample_y.start).ceil() as u32;
                let columns = (sc.sample_x.end - sc.sample_x.start).ceil() as u32;
                let count = rows * columns;
                const MAX_INSTANCES: u32 = 1_000_000;
                *bar_count = if count > MAX_INSTANCES {
                    log::error!("Too many instances: {}", count);
                    MAX_INSTANCES
                } else {
                    count
                };
            }
            Kind::Mesh {
                ref mut draws,
                ref mut geo,
                lod_distance,
                lod_force,
                cull,
                ..
            } => {
                // The TIN covers the level once; the level wraps. Instance
                // it across the tiles the camera can see, deriving the grid
                // from the same visible bounds the shader reads out of
                // `u_Locals.sample_range` so the two cannot disagree.
                let size = self.active_surface_constants.texture_scale;
                draws.clear();
                if let Some(geo) = geo.as_mut() {
                    let MeshGeometry {
                        ref chunks,
                        ref mut drawn,
                        ..
                    } = *geo;
                    // A refit reads this a frame later, by which time the
                    // camera has moved on, so a chunk counts as drawn from
                    // a chunk's width outside the frustum. That is far more
                    // than a frame of turning, and it means terrain is
                    // already being kept current by the time it rotates
                    // into view rather than snapping to it afterwards.
                    let margin = glam::Vec3::new(
                        level::tin::CHUNK_SIZE as f32,
                        level::tin::CHUNK_SIZE as f32,
                        0.0,
                    );
                    drawn.iter_mut().for_each(|slot| *slot = None);
                    let cam_tile = glam::Vec2::new(
                        (sc.origin.x / size[0]).floor(),
                        (sc.origin.y / size[1]).floor(),
                    );
                    // Frustum-cull every (chunk, wrapped copy) pair and pick
                    // a LOD from its distance. Without this the whole level
                    // mesh is redrawn per copy: on Fostral that is millions
                    // of triangles a frame regardless of where you look.
                    //
                    // A 3x3 neighbourhood of copies around the camera's own
                    // tile; anything further is a whole level away and deep
                    // in the fog. The instance index carries which copy, and
                    // `tile_offset` in the shader decodes it with the same
                    // arithmetic - deriving the grid separately on each side
                    // is what previously put wrapped terrain at the wrong
                    // distance.
                    let planes = frustum_planes(&cam.get_view_proj());
                    for copy in 0..9u32 {
                        let offset = glam::Vec2::new(
                            (cam_tile.x + (copy % 3) as f32 - 1.0) * size[0],
                            (cam_tile.y + (copy / 3) as f32 - 1.0) * size[1],
                        );
                        for (ci, chunk) in chunks.iter().enumerate() {
                            // Not fitted yet (web build): nothing to cull
                            // or draw, and nothing for a refit to keep up.
                            if chunk.lods.is_empty() {
                                continue;
                            }
                            let min = glam::Vec3::new(
                                chunk.min[0] + offset.x,
                                chunk.min[1] + offset.y,
                                chunk.min[2],
                            );
                            let max = glam::Vec3::new(
                                chunk.max[0] + offset.x,
                                chunk.max[1] + offset.y,
                                chunk.max[2],
                            );
                            let hidden = cull && box_outside(&planes, min, max);
                            let center = glam::Vec2::new(
                                chunk.center[0] + offset.x,
                                chunk.center[1] + offset.y,
                            );
                            let dist = (center - sc.origin).length();
                            let level = match lod_force {
                                Some(forced) => forced.min(chunk.lods.len() - 1),
                                None => (level::tin::detail_steps(dist, lod_distance) as usize)
                                    .min(chunk.lods.len() - 1),
                            };
                            // The finest level any copy of this chunk is
                            // drawn at is the one a refit has to keep up
                            // with.
                            if !hidden || !box_outside(&planes, min - margin, max + margin) {
                                let slot = &mut drawn[ci];
                                *slot = Some(slot.map_or(level as u8, |l| l.min(level as u8)));
                            }
                            if hidden {
                                continue;
                            }
                            let (base, count) = chunk.lods[level];
                            if count != 0 {
                                draws.push((ci as u32, base, count, copy, dist));
                            }
                        }
                    }
                    // Near to far: the fragment shader is not cheap (it
                    // re-derives the surface gradient per pixel), so letting
                    // the depth test reject occluded chunks before shading
                    // them is worth more than the sort costs.
                    draws.sort_unstable_by(|a, b| a.4.total_cmp(&b.4));
                }
            }
            Kind::Scatter {
                ref clear_pipeline,
                ref scatter_pipeline,
                ref bind_group,
                compute_groups,
                density,
                ..
            } => {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("scatter"),
                    timestamp_writes: None,
                });
                pass.set_bind_group(0, &global.bind_group, &[]);
                pass.set_bind_group(1, &self.bind_group, &[]);
                pass.set_bind_group(2, bind_group, &[]);
                pass.set_pipeline(clear_pipeline);
                pass.dispatch_workgroups(compute_groups[0], compute_groups[1], compute_groups[2]);
                pass.set_pipeline(scatter_pipeline);
                pass.dispatch_workgroups(
                    compute_groups[0] * density[0],
                    compute_groups[1] * density[1],
                    density[2],
                );
            }
            _ => {}
        }
    }

    pub fn prepare_shadow(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
        cam: &Camera,
        screen_size: wgpu::Extent3d,
    ) {
        let bounds = cam.visible_bounds();
        let sc = ScatterConstants {
            origin: cam.loc.truncate(),
            dir: cam.dir().truncate(),
            sample_x: bounds.start.x..bounds.end.x,
            sample_y: bounds.start.y..bounds.end.y,
        };

        {
            // constants update
            let staging = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("temp-constants"),
                contents: bytemuck::bytes_of(&Constants {
                    screen_rect: [0, 0, screen_size.width, screen_size.height],
                    cam_origin_dir: [sc.origin.x, sc.origin.y, sc.dir.x, sc.dir.y],
                    sample_range: [
                        sc.sample_x.start,
                        sc.sample_x.end,
                        sc.sample_y.start,
                        sc.sample_y.end,
                    ],
                    fog_color: [0.0; 3],
                    pad: 1.0,
                    fog_params: [10000000.0, 10000000.0, 0.0, 0.0],
                    lighting_flags: [
                        self.unbaked_lighting as u32,
                        self.smooth_normals as u32,
                        0,
                        0,
                    ],
                    terrain_params: [self.slice_spacing(), self.ray_steps as f32, 0.0, 0.0],
                }),
                usage: wgpu::BufferUsages::COPY_SRC,
            });
            encoder.copy_buffer_to_buffer(
                &staging,
                0,
                &self.uniform_buf,
                0,
                mem::size_of::<Constants>() as wgpu::BufferAddress,
            );
        }

        match self.shadow_kind {
            ShadowKind::InheritRayVoxel {
                max_outer_steps,
                max_inner_steps,
                ..
            } => match self.kind {
                Kind::RayVoxel(ref rv) => {
                    let constants = VoxelConstants {
                        voxel_size: rv.voxel_size,
                        pad: 0,
                        max_depth: cam.depth_range().end,
                        debug_alpha: rv.debug_alpha,
                        max_outer_steps,
                        max_inner_steps,
                    };
                    let constant_update =
                        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("ray-voxel shadow constants"),
                            contents: bytemuck::bytes_of(&constants),
                            usage: wgpu::BufferUsages::COPY_SRC,
                        });
                    encoder.copy_buffer_to_buffer(
                        &constant_update,
                        0,
                        &rv.constant_buffer,
                        0,
                        mem::size_of::<VoxelConstants>() as wgpu::BufferAddress,
                    );
                }
                _ => unreachable!(),
            },
            _ => {}
        }
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_bind_group(1, &self.bind_group, &[]);
        // draw terrain
        match self.kind {
            Kind::Ray { ref pipeline } => {
                let geo = &self.raytrace_geo;
                pass.set_pipeline(pipeline);
                pass.set_index_buffer(geo.index_buf.slice(..), wgpu::IndexFormat::Uint16);
                pass.set_vertex_buffer(0, geo.vertex_buf.slice(..));
                pass.draw_indexed(0..geo.num_indices, 0, 0..1);
            }
            Kind::RayVoxel(ref rv) => {
                pass.set_pipeline(&rv.draw_pipeline);
                pass.set_bind_group(2, &rv.draw_bind_group, &[]);
                pass.draw(0..3, 0..1);
                if let Some(VoxelDebugRender {
                    ref pipeline,
                    ref geo,
                    lod_range: Some(ref lod_range),
                }) = rv.debug_render
                {
                    pass.set_pipeline(pipeline);
                    let mut instances = 0..0;
                    for (i, mip) in rv.mips[..lod_range.end.min(rv.mips.len())]
                        .iter()
                        .enumerate()
                    {
                        let count =
                            mip.extent.width * mip.extent.height * mip.extent.depth_or_array_layers;
                        if i < lod_range.start {
                            instances.start += count;
                        }
                        instances.end += count;
                    }
                    pass.set_index_buffer(geo.index_buf.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..geo.num_indices, 0, instances);
                }
            }
            Kind::Slice {
                ref pipeline,
                layer_count,
            } => {
                pass.set_pipeline(pipeline);
                pass.draw(0..4, 0..layer_count);
            }
            Kind::Paint {
                ref pipeline,
                ref geo,
                bar_count,
            } => {
                pass.set_pipeline(pipeline);
                pass.set_index_buffer(geo.index_buf.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..geo.num_indices, 0, 0..bar_count);
            }
            Kind::Scatter {
                ref copy_pipeline,
                ref bind_group,
                ..
            } => {
                pass.set_pipeline(copy_pipeline);
                pass.set_bind_group(2, bind_group, &[]);
                pass.draw(0..4, 0..1);
            }
            Kind::Mesh {
                ref pipeline,
                ref wire_pipeline,
                geo: Some(ref geo),
                wireframe,
                ref draws,
                ..
            } => {
                // One draw per visible chunk: each owns its buffers, and the
                // LOD is chosen per chunk. With `MAX_TILE_RADIUS` bounding
                // the wrap tiles and the frustum test in `prepare`, this is
                // a handful of calls per frame even on a level-sized mesh.
                let emit = |pass: &mut wgpu::RenderPass<'a>| {
                    for &(ci, base, count, tile, _) in draws.iter() {
                        let chunk = &geo.chunks[ci as usize];
                        pass.set_vertex_buffer(0, chunk.vertex_buf.slice(..));
                        pass.set_index_buffer(chunk.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(base..base + count, 0, tile..tile + 1);
                    }
                };
                if let Some(wire) = wire_pipeline.as_ref().filter(|_| wireframe) {
                    pass.set_pipeline(&wire.depth);
                    emit(pass);
                    pass.set_pipeline(&wire.line);
                    emit(pass);
                } else {
                    pass.set_pipeline(pipeline);
                    emit(pass);
                }
            }
            // Not built yet: the first `update_dirty` has not run.
            Kind::Mesh { geo: None, .. } => {}
        }
    }

    pub fn draw_shadow<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_bind_group(1, &self.bind_group, &[]);
        // draw terrain
        match self.shadow_kind {
            ShadowKind::Ray { ref pipeline } => {
                let geo = &self.raytrace_geo;
                pass.set_pipeline(pipeline);
                pass.set_index_buffer(geo.index_buf.slice(..), wgpu::IndexFormat::Uint16);
                pass.set_vertex_buffer(0, geo.vertex_buf.slice(..));
                pass.draw_indexed(0..geo.num_indices, 0, 0..1);
            }
            ShadowKind::InheritRayVoxel { ref pipeline, .. } => match self.kind {
                Kind::RayVoxel(ref rv) => {
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(2, &rv.draw_bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
                _ => unreachable!(),
            },
        }
    }

    /// Overlay the TIN's triangle edges on the shaded surface. No-op unless
    /// the mesh terrain mode is active.
    /// Read the occupancy bit for one world position at every LOD.
    ///
    /// Replicates `linearize()` from `terrain/voxel.inc.wgsl` on the CPU so
    /// the acceleration structure can be checked against the height map
    /// directly, without going through the ray marcher.
    pub fn debug_voxel_occupancy(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        world: [i32; 3],
    ) -> Vec<bool> {
        fn morton_part(arg: u32) -> u32 {
            let mut x = arg & 0x3ff;
            x = (x ^ (x << 16)) & 0xff0000ff;
            x = (x ^ (x << 8)) & 0x0300f00f;
            x = (x ^ (x << 4)) & 0x030c30c3;
            x = (x ^ (x << 2)) & 0x09249249;
            x
        }

        let rv = match self.kind {
            Kind::RayVoxel(ref rv) => rv,
            _ => return Vec::new(),
        };
        // `VoxelHeader` sits in front of the occupancy words.
        let header_words = mem::size_of::<VoxelHeader>() as u32 / 4;
        let size = rv.grid.size();
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxel readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_buffer_to_buffer(&rv.grid, 0, &staging, 0, size);
        queue.submit(Some(encoder.finish()));
        staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::Wait {
            timeout: None,
            submission_index: Default::default(),
        });
        let view = staging.slice(..).get_mapped_range();
        let words: &[u32] = bytemuck::cast_slice(&view);
        let base = [
            world[0] / rv.voxel_size[0] as i32,
            world[1] / rv.voxel_size[1] as i32,
            world[2] / rv.voxel_size[2] as i32,
        ];
        let mut out = Vec::with_capacity(rv.mips.len());
        for (lod, mip) in rv.mips.iter().enumerate() {
            let dim = [
                mip.extent.width,
                mip.extent.height,
                mip.extent.depth_or_array_layers,
            ];
            let coords = [
                (base[0] >> lod) as u32,
                (base[1] >> lod) as u32,
                (base[2] >> lod) as u32,
            ];
            if (0..3).any(|i| coords[i] >= dim[i]) {
                out.push(false);
                continue;
            }
            let tile = VOXEL_TILE_SIZE;
            let tile_counts = [
                (dim[0] - 1) / tile + 1,
                (dim[1] - 1) / tile + 1,
                (dim[2] - 1) / tile + 1,
            ];
            let bit_index = (morton_part(coords[2] % tile) << 2)
                + (morton_part(coords[1] % tile) << 1)
                + morton_part(coords[0] % tile);
            let tile_coord = [coords[0] / tile, coords[1] / tile, coords[2] / tile];
            let tile_index =
                (tile_coord[2] * tile_counts[1] + tile_coord[1]) * tile_counts[0] + tile_coord[0];
            let words_per_tile = tile.pow(3) / 32;
            let offset = header_words
                + mip.data_offset_in_words
                + tile_index * words_per_tile
                + bit_index / 32;
            let word = words.get(offset as usize).copied().unwrap_or(0);
            out.push(word & (1 << (bit_index & 31)) != 0);
        }
        drop(view);
        staging.unmap();
        out
    }

    /// Draw the voxel occupancy grid itself, for the given LOD range.
    /// No-op unless the voxel terrain mode is active.
    pub fn set_voxel_debug_lods(&mut self, range: Option<Range<usize>>) {
        if let Kind::RayVoxel(ref mut rv) = self.kind {
            // Only tint the frame when the debug view is actually wanted:
            // `debug_alpha` recolours the whole terrain, so setting it
            // unconditionally silently corrupts every ordinary render.
            rv.debug_alpha = if range.is_some() { 1.0 } else { 0.0 };
            if let Some(ref mut debug) = rv.debug_render {
                debug.lod_range = range;
            }
        }
    }

    pub fn set_mesh_wireframe(&mut self, enabled: bool) {
        if let Kind::Mesh {
            ref mut wireframe, ..
        } = self.kind
        {
            *wireframe = enabled;
        }
    }

    /// What the last `prepare` decided, for a top-down debug plot: every
    /// chunk's footprint, and for the ones that survived the frustum test
    /// which wrap copy and detail level they were drawn at.
    ///
    /// The LOD is recovered by matching the draw's first index against the
    /// chunk's per-level ranges rather than being carried in the draw
    /// list, so this costs the render path nothing.
    pub fn mesh_debug(&self) -> Option<MeshDebug> {
        let Kind::Mesh {
            geo: Some(ref geo),
            ref draws,
            lod_distance,
            cull,
            ..
        } = self.kind
        else {
            return None;
        };
        Some(MeshDebug {
            chunks: geo
                .chunks
                .iter()
                .map(|c| MeshDebugChunk {
                    min: c.min,
                    max: c.max,
                })
                .collect(),
            draws: draws
                .iter()
                .map(|&(ci, base, _, copy, dist)| {
                    let lods = &geo.chunks[ci as usize].lods;
                    let lod = lods.iter().position(|&(b, _)| b == base).unwrap_or(0);
                    MeshDebugDraw {
                        chunk: ci,
                        copy,
                        lod: lod as u32,
                        distance: dist,
                    }
                })
                .collect(),
            lod_distance,
            culling: cull,
            level_size: [
                self.active_surface_constants.texture_scale[0],
                self.active_surface_constants.texture_scale[1],
            ],
        })
    }

    /// Turn frustum culling off, so every chunk of every wrap copy is
    /// drawn. Slow, but definitionally correct - the reference to check
    /// the culled render against.
    /// See `Kind::Mesh::defer_refits`.
    pub fn set_deferred_refits(&mut self, enabled: bool) {
        if let Kind::Mesh {
            ref mut defer_refits,
            ..
        } = self.kind
        {
            *defer_refits = enabled;
        }
    }

    pub fn set_mesh_culling(&mut self, enabled: bool) {
        if let Kind::Mesh { ref mut cull, .. } = self.kind {
            *cull = enabled;
        }
    }

    /// Vertical distance between consecutive slices, chosen so the whole
    /// `0..height` range is always covered whatever the count.
    fn slice_spacing(&self) -> f32 {
        match self.kind {
            Kind::Slice { layer_count, .. } => {
                self.active_surface_constants.texture_scale[2] / layer_count as f32
            }
            _ => 1.0,
        }
    }

    /// How many horizontal slices the sliced terrain draws, spread evenly
    /// over the level's height range. Defaults to the level's height in
    /// units, i.e. one per altitude step; fewer is cheaper and coarser,
    /// and this is the only quality knob the method has.
    ///
    /// The spread matters: an earlier version kept unit spacing and let a
    /// smaller count truncate slices off the *bottom* of the range, so
    /// "fewer slices" deleted the low terrain outright instead of
    /// sampling all of it more coarsely — and a tuning sweep read that
    /// as the method collapsing.
    pub fn set_slice_layers(&mut self, layers: u32) {
        if let Kind::Slice {
            ref mut layer_count,
            ..
        } = self.kind
        {
            *layer_count = layers.max(1);
        }
    }

    /// Pin every chunk to one detail level, or `None` to pick by distance.
    pub fn set_mesh_lod(&mut self, level: Option<usize>, distance: Option<f32>) {
        if let Kind::Mesh {
            ref mut lod_force,
            ref mut lod_distance,
            ..
        } = self.kind
        {
            *lod_force = level;
            if let Some(d) = distance {
                *lod_distance = d;
            }
        }
    }

    pub fn draw_ui(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(
            &mut self.unbaked_lighting,
            "Unbaked diffuse + shadow (terrain)",
        );
        ui.checkbox(&mut self.smooth_normals, "Smooth normals (terrain)");
        match self.kind {
            Kind::RayVoxel(ref mut rv) => {
                ui.add(egui::Slider::new(&mut rv.max_outer_steps, 0..=100).text("Max outer steps"));
                ui.add(egui::Slider::new(&mut rv.max_inner_steps, 0..=100).text("Max inner steps"));
                ui.add(egui::Slider::new(&mut rv.debug_alpha, 0.0..=1.0).text("Debug alpha"));
                if let Some(ref mut debug) = rv.debug_render {
                    let mut debug_voxels = debug.lod_range.is_some();
                    ui.checkbox(&mut debug_voxels, "Debug voxels");
                    let mut lod_start = debug.lod_range.clone().map_or(4, |r| r.start);
                    let mut lod_count = debug.lod_range.clone().map_or(1, |r| r.end - r.start);
                    ui.add_enabled_ui(debug_voxels, |ui| {
                        ui.add(egui::Slider::new(&mut lod_start, 1..=8).text("LOD start"));
                        ui.add(egui::Slider::new(&mut lod_count, 1..=8).text("LOD count"));
                    });
                    debug.lod_range = if debug_voxels {
                        Some(lod_start..lod_start + lod_count)
                    } else {
                        None
                    };
                }
            }
            Kind::Mesh {
                geo: Some(ref geo),
                ref mut wireframe,
                ref mut lod_distance,
                ref mut lod_force,
                ref mut cull,
                ref draws,
                config,
                ..
            } => {
                let draw_count = draws.len();
                let stats = &geo.tin.stats;
                ui.label(format!(
                    "TIN: {} verts, {} tris",
                    stats.vertices, stats.triangles
                ));
                ui.label(format!(
                    "{:.1}x fewer than a full grid mesh",
                    2.0 * stats.source_texels as f32 / stats.triangles.max(1) as f32
                ));
                ui.label(format!(
                    "quality {:.2} -> max error {:.2}",
                    config.quality, stats.max_error
                ));
                ui.label(format!(
                    "{} chunks drawn, {} levels each",
                    draw_count,
                    level::tin::LOD_COUNT
                ));
                ui.checkbox(wireframe, "Show mesh grid");
                ui.add(
                    egui::Slider::new(lod_distance, 16.0..=512.0)
                        .logarithmic(true)
                        .text("LOD distance"),
                )
                .on_hover_text(
                    "Terrain further than this drops a detail level for \
                     every doubling of the distance, and refits after an \
                     edit that much less often with it. Only the geometry \
                     waits - collision reads the level.",
                );
                let mut forced = lod_force.is_some();
                ui.checkbox(&mut forced, "Force one LOD");
                let mut level = lod_force.unwrap_or(0);
                ui.add_enabled_ui(forced, |ui| {
                    ui.add(
                        egui::Slider::new(&mut level, 0..=level::tin::LOD_COUNT - 1)
                            .text("Forced LOD (0 = finest)"),
                    );
                });
                *lod_force = if forced { Some(level) } else { None };
                ui.checkbox(cull, "Frustum culling");
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod cull_tests {
    use super::{box_outside, frustum_planes};

    /// A box is *visible* under the reference test when any of its eight
    /// corners projects inside the clip volume. That is not the same set
    /// the plane test rejects - a box can straddle the frustum with every
    /// corner outside - but it is a subset, which is what makes it usable
    /// as an oracle: anything the reference calls visible must not be
    /// culled.
    fn any_corner_inside(m: &glam::Mat4, min: glam::Vec3, max: glam::Vec3) -> bool {
        for i in 0..8 {
            let p = glam::Vec3::new(
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            );
            let c = *m * p.extend(1.0);
            if c.w > 0.0
                && c.x >= -c.w
                && c.x <= c.w
                && c.y >= -c.w
                && c.y <= c.w
                && c.z >= 0.0
                && c.z <= c.w
            {
                return true;
            }
        }
        false
    }

    /// wgpu clip space is z in `0..w`, so the near plane is the `z` row and
    /// the far plane is `w - z`. Getting that pair wrong is the classic
    /// Gribb-Hartmann mistake and it culls geometry the camera is looking
    /// straight at.
    #[test]
    fn never_culls_a_box_with_a_visible_corner() {
        let proj = glam::Mat4::perspective_rh(1.0, 1.6, 1.0, 2000.0);
        let mut checked = 0;
        for yaw_step in 0..16 {
            let yaw = yaw_step as f32 * std::f32::consts::TAU / 16.0;
            let eye = glam::Vec3::new(500.0, 500.0, 120.0);
            let view = glam::Mat4::look_at_rh(
                eye,
                eye + glam::Vec3::new(yaw.cos(), yaw.sin(), -0.2),
                glam::Vec3::Z,
            );
            let m = proj * view;
            let planes = frustum_planes(&m);
            for gx in -6..=6 {
                for gy in -6..=6 {
                    for gz in -1..=2 {
                        let min = glam::Vec3::new(
                            500.0 + gx as f32 * 128.0,
                            500.0 + gy as f32 * 128.0,
                            gz as f32 * 128.0,
                        );
                        let max = min + glam::Vec3::splat(128.0);
                        if any_corner_inside(&m, min, max) {
                            assert!(
                                !box_outside(&planes, min, max),
                                "culled a box with a visible corner: {:?}..{:?} at yaw {}",
                                min,
                                max,
                                yaw
                            );
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert!(checked > 100, "the sweep never produced visible boxes");
    }

    /// The other direction: a box squarely behind the camera has to go.
    /// Without this the test above passes for a `box_outside` that always
    /// returns false.
    #[test]
    fn culls_a_box_behind_the_camera() {
        let proj = glam::Mat4::perspective_rh(1.0, 1.6, 1.0, 2000.0);
        let eye = glam::Vec3::new(0.0, 0.0, 0.0);
        let view = glam::Mat4::look_at_rh(eye, glam::Vec3::new(0.0, 1.0, 0.0), glam::Vec3::Z);
        let planes = frustum_planes(&(proj * view));
        assert!(box_outside(
            &planes,
            glam::Vec3::new(-50.0, -500.0, -50.0),
            glam::Vec3::new(50.0, -400.0, 50.0),
        ));
    }
}
