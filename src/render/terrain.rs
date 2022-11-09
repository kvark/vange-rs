use crate::{
    config::settings,
    level,
    render::{
        global::Context as GlobalContext, Palette, PipelineKind, DEPTH_FORMAT, SHADOW_FORMAT,
    },
    space::Camera,
};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt as _;

use std::{mem, num::NonZeroU32, ops::Range};

const SCATTER_GROUP_SIZE: [u32; 3] = [16, 16, 1];
// Has to agree with the shader
const VOXEL_TILE_SIZE: u32 = 8;
fn count_tiles(size: u32) -> u32 {
    (size - 1) / VOXEL_TILE_SIZE + 1
}

const MAXIMUM_UNIFORM_BUFFER_ALIGNMENT: usize = 256;

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
}
unsafe impl Pod for Constants {}
unsafe impl Zeroable for Constants {}

struct ScatterConstants {
    origin: cgmath::Point2<f32>,
    dir: cgmath::Vector2<f32>,
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
    use cgmath::{prelude::*, Point2, Point3, Vector2, Vector3};

    let cam_origin = Point2::new(cam.loc.x, cam.loc.y);
    let cam_dir = {
        let vec = cam.dir();
        let v2 = Vector2::new(vec.x, vec.y);
        if v2.magnitude2() > 0.0 {
            v2.normalize()
        } else {
            Vector2::new(0.0, 1.0)
        }
    };

    fn intersect(base: &Vector3<f32>, target: Point3<f32>, height: u32) -> Point2<f32> {
        let dir = target.to_vec() - *base;
        let t = if dir.z == 0.0 {
            0.0
        } else {
            (height as f32 - base.z) / dir.z
        };
        let end = base + dir * t.max(0.0);
        Point2::new(end.x, end.y)
    }

    let mx_invp = cam.get_view_proj().invert().unwrap();
    let y_center = {
        let center = mx_invp.transform_point(Point3::new(0.0, 0.0, 0.0));
        let center_base = intersect(&cam.loc, center, 0);
        (center_base - cam_origin).dot(cam_dir)
    };
    let mut y_range = y_center..y_center;
    let mut x0 = 0f32..0.0;
    let mut x1 = 0f32..0.0;
    let v = 1.0; // set to smaller for debugging

    let local_positions = [
        Point3::new(v, v, 0.0),
        Point3::new(-v, v, 0.0),
        Point3::new(v, -v, 0.0),
        Point3::new(-v, -v, 0.0),
    ];

    for &lp in &local_positions {
        let wp = mx_invp.transform_point(lp);
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

enum Kind {
    Ray {
        pipeline: wgpu::RenderPipeline,
    },
    RayVoxel {
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
    },
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
    },
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
    surface_uni_buf: wgpu::Buffer,
    pub uniform_buf: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    color_format: wgpu::TextureFormat,
    raytrace_geo: Geometry,
    kind: Kind,
    shadow_kind: ShadowKind,
    terrain_buf: wgpu::Buffer,
    palette_texture: wgpu::Texture,
    pub flood: Flood,
    pub dirty_rects: Vec<super::DirtyRect>,
    pub dirty_flood: bool,
    pub dirty_palette: Range<u32>,
    active_surface_constants: SurfaceConstants,
}

impl Context {
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
                entry_point: "main",
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
                entry_point,
                targets,
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
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
            entry_point: "init",
        });
        let mip_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Voxel mip"),
            layout: Some(bake_layout),
            module: &bake_shader,
            entry_point: "mip",
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
                entry_point: "main",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &draw_shader,
                entry_point: "draw_color",
                targets: &color_descs,
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
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
                module: &draw_shader,
                entry_point: "main",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &draw_shader,
                entry_point: "draw_depth",
                targets: &[],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
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
                entry_point: "vert_bound",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: draw_shader,
                entry_point: "draw_bound",
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
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
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
                entry_point: "main_vs",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "main_fs",
                targets: &[Some(color_format.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
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
                entry_point: "vertex",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fragment",
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
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        })
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
            entry_point: "main",
        });
        let clear_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("terrain-scatter-clear"),
            layout: Some(layout),
            module: &shader,
            entry_point: "clear",
        });

        let copy_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain-scatter-copy"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "copy_vs",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "copy_fs",
                targets: &[Some(color_format.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
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
    ) -> Self {
        profiling::scope!("Init Terrain");

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

        let terrain_buf = gfx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Terrain data"),
            size: (extent.width * extent.height) as wgpu::BufferAddress * 2,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
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
            dimension: wgpu::TextureDimension::D1,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        });
        let table_texture = gfx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Terrain table"),
            size: table_extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D1,
            format: wgpu::TextureFormat::Rgba8Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        });

        gfx.queue.write_texture(
            table_texture.as_image_copy(),
            bytemuck::cast_slice(&terrain_table),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: NonZeroU32::new(table_extent.width * 4),
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
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let table_sampler = gfx.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
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
                            visibility: wgpu::ShaderStages::all(),
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
                            visibility: wgpu::ShaderStages::all(),
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
                            visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: true },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        // flood map
                        wgpu::BindGroupLayoutEntry {
                            binding: 4,
                            visibility: wgpu::ShaderStages::all(),
                            ty: wgpu::BindingType::Texture {
                                view_dimension: wgpu::TextureViewDimension::D1,
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                multisampled: false,
                            },
                            count: None,
                        },
                        // table map
                        wgpu::BindGroupLayoutEntry {
                            binding: 5,
                            visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Texture {
                                view_dimension: wgpu::TextureViewDimension::D1,
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
                                view_dimension: wgpu::TextureViewDimension::D1,
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                multisampled: false,
                            },
                            count: None,
                        },
                        // flood sampler
                        wgpu::BindGroupLayoutEntry {
                            binding: 8,
                            visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
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
                    resource: terrain_buf.as_entire_binding(),
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
                bind_group_layouts: &[&global.bind_group_layout, &bind_group_layout],
                push_constant_ranges: &[],
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
                            bind_group_layouts: &[&bake_bg_layout, &bind_group_layout],
                            push_constant_ranges: &[],
                        });

                let supports_debug = gfx
                    .downlevel_caps
                    .flags
                    .contains(wgpu::DownlevelFlags::VERTEX_STORAGE);

                let draw_bg_layout =
                    gfx.device
                        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                            label: Some("Voxel draw"),
                            entries: &[
                                // voxel grid
                                wgpu::BindGroupLayoutEntry {
                                    binding: 0,
                                    visibility: if supports_debug {
                                        wgpu::ShaderStages::all()
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
                                &global.bind_group_layout,
                                &bind_group_layout,
                                &draw_bg_layout,
                            ],
                            push_constant_ranges: &[],
                        });

                let (init_pipeline, mip_pipeline, draw_pipeline, draw_shader) =
                    Self::create_voxel_pipelines(
                        &bake_pipeline_layout,
                        &draw_pipeline_layout,
                        &gfx.device,
                        gfx.color_format,
                    );

                let debug_render = if supports_debug {
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
                    let mip_extent = grid_extent.mip_level_size(base_mip_level, true);
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

                let grid = gfx.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Grid"),
                    size: (mem::size_of::<VoxelHeader>() + data_offset_in_words as usize * 4) as _,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
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
                let mip_buffer = gfx.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Bake mip constant"),
                    size: MAXIMUM_UNIFORM_BUFFER_ALIGNMENT as wgpu::BufferAddress
                        * mip_level_count as wgpu::BufferAddress,
                    usage: wgpu::BufferUsages::UNIFORM,
                    mapped_at_creation: true,
                });
                {
                    let mut mapping = mip_buffer.slice(..).get_mapped_range_mut();
                    for i in 0..mip_level_count {
                        // initializing the least significant byte of the word
                        mapping[i as usize * MAXIMUM_UNIFORM_BUFFER_ALIGNMENT] = i as u8;
                    }
                }
                mip_buffer.unmap();

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

                Kind::RayVoxel {
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
                }
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
                                &global.bind_group_layout,
                                &bind_group_layout,
                                &local_bg_layout,
                            ],
                            push_constant_ranges: &[],
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
                Kind::RayVoxel {
                    ref draw_pipeline_layout,
                    ref draw_shader,
                    ..
                } => ShadowKind::InheritRayVoxel {
                    pipeline: Self::create_voxel_shadow_pipeline(
                        draw_pipeline_layout,
                        draw_shader,
                        &gfx.device,
                    ),
                    max_outer_steps,
                    max_inner_steps,
                },
                _ => panic!("Unable to inherit the voxel context from the main renderer"),
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
            terrain_buf,
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
            Kind::RayVoxel {
                ref bake_pipeline_layout,
                ref draw_pipeline_layout,
                ref mut draw_shader,
                ref mut init_pipeline,
                ref mut mip_pipeline,
                ref mut draw_pipeline,
                ref mut debug_render,
                ..
            } => {
                let (init, mip, draw, shader) = Self::create_voxel_pipelines(
                    bake_pipeline_layout,
                    draw_pipeline_layout,
                    device,
                    self.color_format,
                );
                if let Some(ref mut debug) = *debug_render {
                    debug.pipeline = Self::create_voxel_debug_pipeline(
                        draw_pipeline_layout,
                        &shader,
                        device,
                        self.color_format,
                    );
                }
                *init_pipeline = init;
                *mip_pipeline = mip;
                *draw_pipeline = draw;
                *draw_shader = shader;
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
                Kind::RayVoxel {
                    ref draw_pipeline_layout,
                    ref draw_shader,
                    ..
                } => {
                    *pipeline = Self::create_voxel_shadow_pipeline(
                        draw_pipeline_layout,
                        draw_shader,
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
    ) {
        let surface_constants = {
            let bits = level.terrain_bits();
            let delta_model = match level.geometry.delta_model {
                settings::DeltaModel::Cave => 0,
                settings::DeltaModel::Thickness => 1,
                settings::DeltaModel::Ignored => 2,
            };
            SurfaceConstants {
                texture_scale: [
                    level.size.0 as f32,
                    level.size.1 as f32,
                    level.geometry.height as f32,
                    0.0,
                ],
                terrain_bits: bits.shift as u32 | ((bits.mask as u32) << 4),
                delta_mode: (delta_model << 8) | level.geometry.delta_power as u32,
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
                mem::size_of::<SurfaceConstants>() as _,
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

        if !self.dirty_rects.is_empty() {
            for dr in self.dirty_rects.iter_mut() {
                if !dr.need_upload {
                    continue;
                }

                let total_size =
                    dr.rect.h as wgpu::BufferAddress * level.size.0 as wgpu::BufferAddress * 2;
                let staging_buf = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("staging level update"),
                    size: total_size,
                    usage: wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: true,
                });
                {
                    let mut mapping = staging_buf.slice(..).get_mapped_range_mut();
                    for (y_off, line) in mapping.chunks_mut(level.size.0 as usize * 2).enumerate() {
                        let base = (dr.rect.y as usize + y_off) * level.size.0 as usize;
                        for x in 0..level.size.0 as usize {
                            line[2 * x + 0] = level.height[base + x];
                            line[2 * x + 1] = level.meta[base + x];
                        }
                    }
                }
                staging_buf.unmap();
                encoder.copy_buffer_to_buffer(
                    &staging_buf,
                    0,
                    &self.terrain_buf,
                    dr.rect.y as wgpu::BufferAddress * level.size.0 as wgpu::BufferAddress * 2,
                    total_size,
                );

                dr.need_upload = false;
            }

            match self.kind {
                Kind::RayVoxel {
                    ref init_pipeline,
                    ref mip_pipeline,
                    ref mips,
                    ref bake_bind_group,
                    ref update_buffer,
                    voxel_size,
                    max_update_rects,
                    max_update_texels,
                    ..
                } => {
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
                            // split into 2 rectangles
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
                            texels_to_update -= texels_to_update;
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
                            (i * mem::size_of::<BakeConstants>()) as _,
                            update_buffer,
                            (i * MAXIMUM_UNIFORM_BUFFER_ALIGNMENT) as _,
                            mem::size_of::<BakeConstants>() as _,
                        );
                    }

                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("Voxel bake"),
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
                wgpu::ImageCopyBuffer {
                    buffer: &staging_buf,
                    layout: wgpu::ImageDataLayout {
                        offset: 0,
                        bytes_per_row: NonZeroU32::new(0x100),
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
                wgpu::ImageCopyBuffer {
                    buffer: &staging_buf,
                    layout: wgpu::ImageDataLayout::default(),
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
            use cgmath::EuclideanSpace;
            let bounds = cam.visible_bounds();
            ScatterConstants {
                origin: cgmath::Point2::from_vec(cam.loc.truncate()),
                dir: cam.dir().truncate(),
                sample_x: bounds.start.x..bounds.end.x,
                sample_y: bounds.start.y..bounds.end.y,
            }
        };

        {
            // constants update
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
                }),
                usage: wgpu::BufferUsages::COPY_SRC,
            });
            encoder.copy_buffer_to_buffer(
                &staging,
                0,
                &self.uniform_buf,
                0,
                mem::size_of::<Constants>() as _,
            );
        }

        match self.kind {
            Kind::RayVoxel {
                ref constant_buffer,
                voxel_size,
                max_outer_steps,
                max_inner_steps,
                debug_alpha,
                ..
            } => {
                let constants = VoxelConstants {
                    voxel_size,
                    pad: 0,
                    max_depth: cam.depth_range().end,
                    debug_alpha,
                    max_outer_steps,
                    max_inner_steps,
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
                    constant_buffer,
                    0,
                    mem::size_of::<VoxelConstants>() as _,
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
        use cgmath::EuclideanSpace;

        let bounds = cam.visible_bounds();
        let sc = ScatterConstants {
            origin: cgmath::Point2::from_vec(cam.loc.truncate()),
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
                Kind::RayVoxel {
                    ref constant_buffer,
                    voxel_size,
                    debug_alpha,
                    ..
                } => {
                    let constants = VoxelConstants {
                        voxel_size,
                        pad: 0,
                        max_depth: cam.depth_range().end,
                        debug_alpha,
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
                        constant_buffer,
                        0,
                        mem::size_of::<VoxelConstants>() as _,
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
            Kind::RayVoxel {
                ref draw_pipeline,
                ref draw_bind_group,
                ref debug_render,
                ref mips,
                ..
            } => {
                pass.set_pipeline(draw_pipeline);
                pass.set_bind_group(2, draw_bind_group, &[]);
                pass.draw(0..3, 0..1);
                if let Some(VoxelDebugRender {
                    ref pipeline,
                    ref geo,
                    lod_range: Some(ref lod_range),
                }) = *debug_render
                {
                    pass.set_pipeline(pipeline);
                    let mut instances = 0..0;
                    for (i, mip) in mips[..lod_range.end.min(mips.len())].iter().enumerate() {
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
                Kind::RayVoxel {
                    ref draw_bind_group,
                    ..
                } => {
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(2, draw_bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
                _ => unreachable!(),
            },
        }
    }

    pub fn draw_ui(&mut self, ui: &mut egui::Ui) {
        match self.kind {
            Kind::RayVoxel {
                ref mut max_outer_steps,
                ref mut max_inner_steps,
                ref mut debug_alpha,
                ref mut debug_render,
                ..
            } => {
                ui.add(egui::Slider::new(max_outer_steps, 0..=100).text("Max outer steps"));
                ui.add(egui::Slider::new(max_inner_steps, 0..=100).text("Max inner steps"));
                ui.add(egui::Slider::new(debug_alpha, 0.0..=1.0).text("Debug alpha"));
                if let Some(ref mut debug) = *debug_render {
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
            _ => {}
        }
    }
}
