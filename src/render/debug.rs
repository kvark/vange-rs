use crate::{
    config::settings,
    model,
    render::{
        DEPTH_FORMAT, VertexStorageNotSupported,
        global::Context as GlobalContext,
        object::{Context as ObjectContext, Instance as ObjectInstance},
    },
};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt as _;

use std::{collections::HashMap, mem, num::NonZeroU64};

const BLEND_FRONT: wgpu::BlendComponent = wgpu::BlendComponent::REPLACE;
const BLEND_BEHIND: wgpu::BlendComponent = wgpu::BlendComponent {
    src_factor: wgpu::BlendFactor::Constant,
    dst_factor: wgpu::BlendFactor::OneMinusConstant,
    operation: wgpu::BlendOperation::Add,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Visibility {
    Front,
    Behind,
}
type Selector = (Visibility, wgpu::VertexStepMode);

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Position {
    pub pos: [f32; 4],
}
unsafe impl Pod for Position {}
unsafe impl Zeroable for Position {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Color {
    pub color: u32,
}
unsafe impl Pod for Color {}
unsafe impl Zeroable for Color {}

#[repr(C)]
#[derive(Clone, Copy)]
struct Locals {
    color: [f32; 4],
    _pad: [f32; 60],
}
unsafe impl Pod for Locals {}
unsafe impl Zeroable for Locals {}

impl Locals {
    fn new(color: [f32; 4]) -> Self {
        Locals {
            color,
            _pad: [0.0; 60],
        }
    }
}

pub struct LineBuffer {
    vertices: Vec<Position>,
    colors: Vec<Color>,
}

impl LineBuffer {
    pub fn new() -> Self {
        LineBuffer {
            vertices: Vec::new(),
            colors: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.vertices.clear();
        self.colors.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    pub fn len(&self) -> usize {
        self.vertices.len()
    }

    pub fn add(&mut self, from: [f32; 3], to: [f32; 3], color: u32) {
        self.vertices.push(Position {
            pos: [from[0], from[1], from[2], 1.0],
        });
        self.vertices.push(Position {
            pos: [to[0], to[1], to[2], 1.0],
        });
        let color = Color { color };
        self.colors.push(color);
        self.colors.push(color);
    }
}

pub struct Context {
    settings: settings::DebugRender,
    /// Three-group layout for collision-shape draws (globals, debug colour,
    /// shape storage). Line draws use [`Self::line_pipeline_layout`] instead.
    #[allow(dead_code)]
    pipeline_layout: Result<wgpu::PipelineLayout, VertexStorageNotSupported>,
    /// Globals + debug colour. The line shader never reads the shape
    /// storage group, so this layout is only two groups; using the
    /// three-group shape layout would demand a dummy group 2 on every draw.
    line_pipeline_layout: wgpu::PipelineLayout,
    pipelines_line: HashMap<Selector, wgpu::RenderPipeline>,
    pipeline_face: Option<wgpu::RenderPipeline>,
    pipeline_edge: Option<wgpu::RenderPipeline>,
    line_color_buf: wgpu::Buffer,
    bind_group_line: wgpu::BindGroup,
    bind_group_face: wgpu::BindGroup,
    bind_group_edge: wgpu::BindGroup,
    color_format: wgpu::TextureFormat,
    // hold the buffers alive
    vertex_buf: Option<wgpu::Buffer>,
    color_buf: Option<wgpu::Buffer>,
}

impl Context {
    pub fn new(
        device: &wgpu::Device,
        settings: &settings::DebugRender,
        global: &GlobalContext,
        object: &ObjectContext,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Debug"),
            entries: &[
                // locals
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = match object.shape_bind_group_layout {
            Ok(ref shape_bgl) => {
                let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("debug"),
                    bind_group_layouts: &[
                        Some(&global.bind_group_layout),
                        Some(&bind_group_layout),
                        Some(shape_bgl),
                    ],
                    immediate_size: 0,
                });
                Ok(pl)
            }
            Err(e) => Err(e),
        };
        let line_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("debug-line"),
            bind_group_layouts: &[Some(&global.bind_group_layout), Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let line_color_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("debug-line-color"),
            contents: bytemuck::bytes_of(&Color { color: 0xFF000080 }), // line
            usage: wgpu::BufferUsages::VERTEX,
        });
        let locals_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("debug-locals"),
            contents: bytemuck::cast_slice(&[
                Locals::new([1.0; 4]),             // line
                Locals::new([0.0, 1.0, 0.0, 0.2]), // face
                Locals::new([1.0, 1.0, 0.0, 0.2]), // edge
            ]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let locals_size = mem::size_of::<Locals>() as wgpu::BufferAddress;
        let bind_group_line = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Debug line"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &locals_buf,
                    offset: 0 * locals_size,
                    size: NonZeroU64::new(locals_size),
                }),
            }],
        });
        let bind_group_face = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Debug face"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &locals_buf,
                    offset: 1 * locals_size,
                    size: NonZeroU64::new(locals_size),
                }),
            }],
        });
        let bind_group_edge = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Debug edge"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &locals_buf,
                    offset: 2 * locals_size,
                    size: NonZeroU64::new(locals_size),
                }),
            }],
        });

        let mut result = Context {
            settings: *settings,
            pipeline_layout,
            line_pipeline_layout,
            pipelines_line: HashMap::new(),
            pipeline_face: None,
            pipeline_edge: None,
            line_color_buf,
            bind_group_line,
            bind_group_face,
            bind_group_edge,
            color_format: global.color_format,
            vertex_buf: None,
            color_buf: None,
        };
        result.reload(device);
        result
    }

    pub fn reload(&mut self, device: &wgpu::Device) {
        self.pipelines_line.clear();
        let shader = super::load_shader("debug", &[], device).unwrap();
        for &visibility in &[Visibility::Front, Visibility::Behind] {
            for &color_rate in &[wgpu::VertexStepMode::Vertex, wgpu::VertexStepMode::Instance] {
                let pipeline = create_line_pipeline(
                    device,
                    &self.line_pipeline_layout,
                    &shader,
                    self.color_format,
                    visibility,
                    color_rate,
                );
                self.pipelines_line
                    .insert((visibility, color_rate), pipeline);
            }
        }
    }

    fn draw_liner<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        vertex_buf: &'a wgpu::Buffer,
        color_buf: &'a wgpu::Buffer,
        color_rate: wgpu::VertexStepMode,
        num_vert: usize,
        visibilities: &[Visibility],
    ) {
        // Behind uses Constant/OneMinusConstant. A white constant would
        // replace the framebuffer and paint occluded ticks on top.
        pass.set_blend_constant(wgpu::Color {
            r: 0.2,
            g: 0.2,
            b: 0.2,
            a: 0.2,
        });
        pass.set_vertex_buffer(0, vertex_buf.slice(..));
        pass.set_vertex_buffer(1, color_buf.slice(..));
        for &vis in visibilities {
            if let Some(pipeline) = self.pipelines_line.get(&(vis, color_rate)) {
                pass.set_pipeline(pipeline);
                pass.draw(0..num_vert as u32, 0..1);
            }
        }
    }

    pub fn draw_shape<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        shape: &'a model::Shape,
        instance_buf: &'a wgpu::Buffer,
        instance_id: usize,
    ) {
        if !self.settings.collision_shapes {
            return;
        }
        let shape_bg = match shape.bind_group {
            Ok(ref bg) => bg,
            Err(_) => return,
        };

        //TODO: this is broken - both regular rendering and debug one
        // require instancing now, one has to yield and be refactored.
        let instance_offset = instance_id * mem::size_of::<ObjectInstance>();
        pass.set_bind_group(2, shape_bg, &[]);
        pass.set_vertex_buffer(0, shape.polygon_buf.slice(..));
        pass.set_vertex_buffer(
            1,
            instance_buf.slice(
                instance_offset as wgpu::BufferAddress
                    ..mem::size_of::<ObjectInstance>() as wgpu::BufferAddress,
            ),
        );

        // draw collision polygon faces
        if let Some(ref pipeline) = self.pipeline_face {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(1, &self.bind_group_face, &[]);
            pass.draw(0..4, 0..shape.polygons.len() as u32);
        }
        // draw collision polygon edges
        if let Some(ref pipeline) = self.pipeline_edge {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(1, &self.bind_group_edge, &[]);
            pass.draw(0..4, 0..shape.polygons.len() as u32);
        }

        // draw sample normals
        if let Some((ref sample_buf, num_vert)) = shape.sample_buf {
            pass.set_bind_group(1, &self.bind_group_line, &[]);
            self.draw_liner(
                pass,
                sample_buf,
                &self.line_color_buf,
                wgpu::VertexStepMode::Instance,
                num_vert,
                &[Visibility::Front, Visibility::Behind],
            );
        }
    }

    pub fn draw_lines<'a>(
        &'a mut self,
        pass: &mut wgpu::RenderPass<'a>,
        device: &wgpu::Device,
        linebuf: &LineBuffer,
    ) {
        self.vertex_buf = Some(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("debug-vertices"),
                contents: bytemuck::cast_slice(&linebuf.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
        );
        self.color_buf = Some(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("debug-colors"),
                contents: bytemuck::cast_slice(&linebuf.colors),
                usage: wgpu::BufferUsages::VERTEX,
            }),
        );
        assert_eq!(linebuf.vertices.len(), linebuf.colors.len());

        pass.set_bind_group(1, &self.bind_group_line, &[]);
        self.draw_liner(
            pass,
            self.vertex_buf.as_ref().unwrap(),
            self.color_buf.as_ref().unwrap(),
            wgpu::VertexStepMode::Vertex,
            linebuf.vertices.len(),
            particle_line_visibilities(),
        );
    }
}

/// Beebs, dust, and the other ticks. Only the LessEqual pass: a Greater
/// pass would still draw the occluded half, which reads as "no depth".
fn particle_line_visibilities() -> &'static [Visibility] {
    &[Visibility::Front]
}

/// `LineBuffer::add` writes independent 2-vertex segments. A triangle strip
/// of two vertices is empty; a line list draws each pair as a tick.
pub(crate) fn line_primitive() -> wgpu::PrimitiveState {
    wgpu::PrimitiveState {
        topology: wgpu::PrimitiveTopology::LineList,
        front_face: wgpu::FrontFace::Ccw,
        // original was not drawn with rasterizer, used no culling
        ..Default::default()
    }
}

/// Depth state for [`line_primitive`]. wgpu forbids a non-zero depth bias
/// on non-triangle topologies (`LineList` included); ticks stay above the
/// ground by spawning them with [`crate::particle::SURFACE_LIFT`] instead.
pub(crate) fn line_depth_stencil(front: bool) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(front),
        depth_compare: Some(if front {
            wgpu::CompareFunction::LessEqual
        } else {
            wgpu::CompareFunction::Greater
        }),
        stencil: Default::default(),
        bias: wgpu::DepthBiasState::default(),
    }
}

fn create_line_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    color_format: wgpu::TextureFormat,
    visibility: Visibility,
    color_rate: wgpu::VertexStepMode,
) -> wgpu::RenderPipeline {
    let blend = match visibility {
        Visibility::Front => BLEND_FRONT,
        Visibility::Behind => BLEND_BEHIND,
    };
    let name = format!("debug-line-{:?}-{:?}", visibility, color_rate);
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(&name),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("main_vs"),
            compilation_options: Default::default(),
            buffers: &[
                wgpu::VertexBufferLayout {
                    array_stride: mem::size_of::<Position>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        offset: 0,
                        format: wgpu::VertexFormat::Float32x4,
                        shader_location: 0,
                    }],
                },
                wgpu::VertexBufferLayout {
                    array_stride: mem::size_of::<Color>() as wgpu::BufferAddress,
                    step_mode: color_rate,
                    attributes: &[wgpu::VertexAttribute {
                        offset: 0,
                        format: wgpu::VertexFormat::Unorm8x4,
                        shader_location: 1,
                    }],
                },
            ],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("main_fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: color_format,
                blend: Some(wgpu::BlendState {
                    color: blend,
                    alpha: blend,
                }),
                write_mask: wgpu::ColorWrites::all(),
            })],
        }),
        primitive: line_primitive(),
        depth_stencil: Some(line_depth_stencil(visibility == Visibility::Front)),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn line_pipeline_draws_segments_not_triangles() {
        assert_eq!(
            super::line_primitive().topology,
            wgpu::PrimitiveTopology::LineList
        );
    }

    #[test]
    fn particle_lines_only_use_the_front_depth_test() {
        assert_eq!(
            super::particle_line_visibilities(),
            &[super::Visibility::Front]
        );
        let front = super::line_depth_stencil(true);
        assert_eq!(front.depth_compare, Some(wgpu::CompareFunction::LessEqual));
        assert_eq!(front.depth_write_enabled, Some(true));
    }

    #[test]
    fn line_list_does_not_use_depth_bias() {
        // wgpu 29: "Depth bias is not compatible with non-triangle topology LineList"
        let front = super::line_depth_stencil(true);
        let behind = super::line_depth_stencil(false);
        assert_eq!(front.bias, wgpu::DepthBiasState::default());
        assert_eq!(behind.bias, wgpu::DepthBiasState::default());
        assert_eq!(front.bias.constant, 0);
        assert_eq!(front.bias.slope_scale, 0.0);
        assert_eq!(
            super::line_primitive().topology,
            wgpu::PrimitiveTopology::LineList
        );
    }

    fn try_headless_device() -> Option<wgpu::Device> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok()?;
        let (device, _queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("debug-line-test"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
                experimental_features: Default::default(),
            }))
            .ok()?;
        Some(device)
    }

    /// Same `Device::create_render_pipeline` the game hits on boot for
    /// `debug-line-Front-Vertex`. A CPU-only descriptor check does not
    /// run wgpu-core's LineList + depth-bias validator.
    #[test]
    fn debug_line_pipelines_validate_on_device() {
        let Some(device) = try_headless_device() else {
            eprintln!("skipping: no wgpu adapter for debug-line pipeline validation");
            return;
        };

        let global_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("debug-line-test-global"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let debug_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("debug-line-test-color"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("debug-line"),
            bind_group_layouts: &[Some(&global_bgl), Some(&debug_bgl)],
            immediate_size: 0,
        });
        let shader = crate::render::load_shader("debug", &[], &device).unwrap();
        let color_format = wgpu::TextureFormat::Rgba8UnormSrgb;

        for &visibility in &[super::Visibility::Front, super::Visibility::Behind] {
            for &color_rate in &[wgpu::VertexStepMode::Vertex, wgpu::VertexStepMode::Instance] {
                let _pipeline = super::create_line_pipeline(
                    &device,
                    &layout,
                    &shader,
                    color_format,
                    visibility,
                    color_rate,
                );
            }
        }
    }
}
