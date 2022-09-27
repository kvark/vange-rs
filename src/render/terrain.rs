use crate::{
    config::settings,
    level,
    render::{
        global::Context as GlobalContext, mipmap::MaxMipper, Palette, PipelineKind, DEPTH_FORMAT,
        SHADOW_FORMAT,
    },
    space::Camera,
};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt as _;

use std::{mem, num::NonZeroU32, ops::Range};

const SCATTER_GROUP_SIZE: [u32; 3] = [16, 16, 1];

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    _pos: [i8; 4],
}
unsafe impl Pod for Vertex {}
unsafe impl Zeroable for Vertex {}

#[repr(C)]
#[derive(Clone, Copy)]
struct SurfaceConstants {
    _tex_scale: [f32; 4],
    _terrain_bits: [u32; 4],
}
unsafe impl Pod for SurfaceConstants {}
unsafe impl Zeroable for SurfaceConstants {}

#[repr(C)]
#[derive(Clone, Copy)]
struct Constants {
    screen_rect: [u32; 4], // x, y, w, h
    params: [u32; 4],
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

//Note: this is very similar to `visible_bounds_at()`
// but it searches in a different parameter space
fn compute_scatter_constants(cam: &Camera) -> ScatterConstants {
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
        let pb = intersect(&cam.loc, wp, level::HEIGHT_SCALE);
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

enum Kind {
    Ray {
        pipeline: wgpu::RenderPipeline,
    },
    RayMip {
        pipeline: wgpu::RenderPipeline,
        mipper: MaxMipper,
        params: [u32; 4],
    },
    Slice {
        pipeline: wgpu::RenderPipeline,
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
    shadow_kind: Kind,
    terrain_buf: wgpu::Buffer,
    palette_texture: wgpu::Texture,
    pub flood: Flood,
    pub dirty_rects: Vec<super::Rect>,
    pub dirty_palette: Range<u32>,
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

        let shader = super::load_shader(name, device).unwrap();
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

    fn create_slice_pipeline(
        layout: &wgpu::PipelineLayout,
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        let shader = super::load_shader("terrain/slice", device).unwrap();
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
        let shader = super::load_shader("terrain/paint", device).unwrap();
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
        let shader = super::load_shader("terrain/scatter", device).unwrap();
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
        level: &level::Level,
        global: &GlobalContext,
        config: &settings::Terrain,
        shadow_config: &settings::ShadowTerrain,
    ) -> Self {
        profiling::scope!("Init Terrain");

        let extent = wgpu::Extent3d {
            width: level.size.0 as u32,
            height: level.size.1 as u32,
            depth_or_array_layers: 1,
        };
        let flood_section_count = level.size.1 as u32 >> level.flood_section_power;
        let flood_extent = wgpu::Extent3d {
            width: flood_section_count,
            height: 1,
            depth_or_array_layers: 1,
        };
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
            mapped_at_creation: true,
        });
        {
            let mut mapping = terrain_buf.slice(..).get_mapped_range_mut();
            for y in 0..extent.height {
                for x in 0..extent.width {
                    let index = (y * extent.width + x) as usize;
                    mapping[2 * index + 0] = level.height[index];
                    mapping[2 * index + 1] = level.meta[index];
                }
            }
        }
        terrain_buf.unmap();

        let flood_texture = gfx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Terrain flood"),
            size: flood_extent,
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
            flood_texture.as_image_copy(),
            bytemuck::cast_slice(&level.flood_map),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: NonZeroU32::new(flood_extent.width),
                rows_per_image: None,
            },
            flood_extent,
        );
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
                            visibility: wgpu::ShaderStages::FRAGMENT,
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

        let bits = level.terrain_bits();
        let surface_uni_buf = gfx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("surface-uniforms"),
                contents: bytemuck::bytes_of(&SurfaceConstants {
                    _tex_scale: [
                        level.size.0 as f32,
                        level.size.1 as f32,
                        level::HEIGHT_SCALE as f32,
                        0.0,
                    ],
                    _terrain_bits: [bits.shift as u32 | ((bits.mask as u32) << 4), 0, 0, 0],
                }),
                usage: wgpu::BufferUsages::UNIFORM,
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
            #[allow(unreachable_code, unused_variables, dead_code)]
            settings::Terrain::RayMipTraced {
                mip_count,
                max_jumps,
                max_steps,
                debug,
            } => {
                let pipeline = Self::create_ray_pipeline(
                    &pipeline_layout,
                    &gfx.device,
                    gfx.color_format,
                    "terrain/ray",
                    PipelineKind::Main,
                    "ray_mip_color",
                );
                let mipper = MaxMipper::new(unimplemented!(), extent, mip_count, &gfx.device);

                Kind::RayMip {
                    pipeline,
                    mipper,
                    params: [
                        mip_count - 1,
                        max_jumps,
                        max_steps,
                        if debug { 1 } else { 0 },
                    ],
                }
            }
            settings::Terrain::Sliced => {
                let pipeline =
                    Self::create_slice_pipeline(&pipeline_layout, &gfx.device, gfx.color_format);

                Kind::Slice { pipeline }
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
                    "ray",
                );
                Kind::Ray { pipeline }
            }
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
                section_size: (level.size.0 as u32, 1 << level.flood_section_power),
            },
            dirty_rects: Vec::new(),
            dirty_palette: 0..0x100,
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
            Kind::RayMip {
                ref mut pipeline,
                ref mut mipper,
                ..
            } => {
                *pipeline = Self::create_ray_pipeline(
                    &self.pipeline_layout,
                    device,
                    self.color_format,
                    "terrain/ray",
                    PipelineKind::Main,
                    "ray_mip_color",
                );
                mipper.reload(device);
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
            Kind::Ray {
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
            _ => unreachable!(),
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
        if !self.dirty_rects.is_empty() {
            for rect in self.dirty_rects.iter() {
                let total_size =
                    rect.h as wgpu::BufferAddress * level.size.0 as wgpu::BufferAddress * 2;
                let staging_buf = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("staging level update"),
                    size: total_size,
                    usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::MAP_WRITE,
                    mapped_at_creation: true,
                });
                {
                    let mut mapping = staging_buf.slice(..).get_mapped_range_mut();
                    for (y_off, line) in mapping.chunks_mut(level.size.0 as usize * 2).enumerate() {
                        let base = (rect.y as usize + y_off) * level.size.0 as usize;
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
                    rect.y as wgpu::BufferAddress * level.size.0 as wgpu::BufferAddress * 2,
                    total_size,
                );
            }

            if let Kind::RayMip { ref mipper, .. } = self.kind {
                mipper.update(encoder, &self.dirty_rects, device);
            }
            self.dirty_rects.clear();
        }

        if self.dirty_palette.start != self.dirty_palette.end {
            let staging_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
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
        cam: &Camera,
        screen_rect: super::Rect,
    ) {
        let params = match self.kind {
            Kind::RayMip { params, .. } => params,
            _ => [0; 4],
        };

        let sc = if let Kind::Scatter { .. } = self.kind {
            compute_scatter_constants(cam)
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
                    params,
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
                mem::size_of::<Constants>() as wgpu::BufferAddress,
            );
        }

        match self.kind {
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
        let params = match self.shadow_kind {
            Kind::RayMip { params, .. } => params,
            _ => [0; 4],
        };

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
                    params,
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
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_bind_group(1, &self.bind_group, &[]);
        // draw terrain
        match self.kind {
            Kind::Ray { ref pipeline } | Kind::RayMip { ref pipeline, .. } => {
                let geo = &self.raytrace_geo;
                pass.set_pipeline(pipeline);
                pass.set_index_buffer(geo.index_buf.slice(..), wgpu::IndexFormat::Uint16);
                pass.set_vertex_buffer(0, geo.vertex_buf.slice(..));
                pass.draw_indexed(0..geo.num_indices, 0, 0..1);
            }
            Kind::Slice { ref pipeline } => {
                pass.set_pipeline(pipeline);
                pass.draw(0..4, 0..level::HEIGHT_SCALE);
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
            Kind::Ray { ref pipeline } | Kind::RayMip { ref pipeline, .. } => {
                let geo = &self.raytrace_geo;
                pass.set_pipeline(pipeline);
                pass.set_index_buffer(geo.index_buf.slice(..), wgpu::IndexFormat::Uint16);
                pass.set_vertex_buffer(0, geo.vertex_buf.slice(..));
                pass.draw_indexed(0..geo.num_indices, 0, 0..1);
            }
            _ => unreachable!(),
        }
    }
}
