use super::types::*;
use crate::render::{PipelineKind, DEPTH_FORMAT, SHADOW_FORMAT};

use std::mem;

pub(super) fn create_ray_pipeline(
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

    let shader = super::super::load_shader(name, &[], device).unwrap();
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

pub(super) fn create_voxel_pipelines(
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
    let bake_shader =
        super::super::load_shader("terrain/voxel-bake", &substitutions, device).unwrap();
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

    let draw_shader =
        super::super::load_shader("terrain/voxel-draw", &substitutions, device).unwrap();
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

pub(super) fn create_voxel_shadow_pipeline(
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

pub(super) fn create_voxel_debug_pipeline(
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

pub(super) fn create_slice_pipeline(
    layout: &wgpu::PipelineLayout,
    device: &wgpu::Device,
    color_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = super::super::load_shader("terrain/slice", &[], device).unwrap();
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

pub(super) fn create_paint_pipeline(
    layout: &wgpu::PipelineLayout,
    device: &wgpu::Device,
    color_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = super::super::load_shader("terrain/paint", &[], device).unwrap();
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

pub(super) fn create_scatter_pipelines(
    layout: &wgpu::PipelineLayout,
    device: &wgpu::Device,
    color_format: wgpu::TextureFormat,
) -> (
    wgpu::ComputePipeline,
    wgpu::ComputePipeline,
    wgpu::RenderPipeline,
) {
    let shader = super::super::load_shader("terrain/scatter", &[], device).unwrap();
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

pub(super) fn create_scatter_resources(
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
