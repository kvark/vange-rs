use crate::{
    config::settings,
    render::{global::Context as GlobalContext, terrain::Context as TerrainContext, DEPTH_FORMAT},
    space::Camera,
};
use bytemuck::{Pod, Zeroable};
use std::ops;
use wgpu::util::DeviceExt as _;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub flood_id: u32,
}
unsafe impl Pod for Vertex {}
unsafe impl Zeroable for Vertex {}

pub struct Context {
    pub pipeline_layout: wgpu::PipelineLayout,
    pub pipeline: wgpu::RenderPipeline,
    pub color_format: wgpu::TextureFormat,
    texture_size: u32,
    section_size: (u32, u32),
    vertex_buf: wgpu::Buffer,
    vertices: Vec<Vertex>,
}

impl Context {
    fn create_pipeline(
        layout: &wgpu::PipelineLayout,
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        let vertex_descriptor = wgpu::VertexBufferLayout {
            array_stride: size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Sint32],
        };
        let shader = super::load_shader("water", &[], device).unwrap();

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("water"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "main_vs",
                buffers: &[vertex_descriptor],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "main_fs",
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::all(),
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                // original was not drawn with rasterizer, used no culling
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        })
    }

    pub fn new(
        device: &wgpu::Device,
        _settings: &settings::Water,
        global: &GlobalContext,
        terrain: &TerrainContext,
    ) -> Self {
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("water"),
            bind_group_layouts: &[&global.bind_group_layout, &terrain.bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = Self::create_pipeline(&pipeline_layout, device, global.color_format);

        let max_vertices = 1000;
        let vertex_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("water-vertex"),
            size: (max_vertices * size_of::<Vertex>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Context {
            pipeline_layout,
            pipeline,
            color_format: global.color_format,
            texture_size: terrain.flood.texture_size,
            section_size: terrain.flood.section_size,
            vertex_buf,
            vertices: Vec::with_capacity(max_vertices),
        }
    }

    pub fn reload(&mut self, device: &wgpu::Device) {
        self.pipeline = Self::create_pipeline(&self.pipeline_layout, device, self.color_format);
    }

    pub fn prepare(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
        cam: &Camera,
    ) {
        fn tile_range(start: f32, end: f32, tile_size: u32) -> ops::Range<i32> {
            (start / tile_size as f32).floor() as i32..(end / tile_size as f32).ceil() as i32
        }

        self.vertices.clear();
        let bounds = cam.visible_bounds();

        'outer: for tile_y in tile_range(bounds.start.y, bounds.end.y, self.section_size.1) {
            let flood_id_signed = tile_y % self.texture_size as i32;
            let flood_id = if flood_id_signed < 0 {
                (flood_id_signed + self.texture_size as i32) as u32
            } else {
                flood_id_signed as u32
            };

            let start_y = tile_y as f32 * self.section_size.1 as f32;
            let end_y = start_y + self.section_size.1 as f32;
            for tile_x in tile_range(bounds.start.x, bounds.end.x, self.section_size.0) {
                if self.vertices.len() + 6 > self.vertices.capacity() {
                    log::error!("Too many flood tiles are visible!");
                    break 'outer;
                }
                let start_x = tile_x as f32 * self.section_size.0 as f32;
                let end_x = start_x + self.section_size.0 as f32;
                let positions = &[
                    [start_x, start_y],
                    [start_x, end_y],
                    [end_x, end_y],
                    [end_x, end_y],
                    [end_x, start_y],
                    [start_x, start_y],
                ];
                self.vertices.extend(
                    positions
                        .iter()
                        .cloned()
                        .map(|pos| Vertex { pos, flood_id }),
                );
            }
        }

        let staging = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("staging flood update"),
            contents: bytemuck::cast_slice(&self.vertices),
            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::MAP_WRITE,
        });
        let total_size = self.vertices.len() * size_of::<Vertex>();
        encoder.copy_buffer_to_buffer(
            &staging,
            0,
            &self.vertex_buf,
            0,
            total_size as wgpu::BufferAddress,
        );
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        // needs to be set up by the terrain context
        //pass.set_bind_group(1, &self.bind_group, &[]);
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.vertex_buf.slice(..));
        pass.draw(0..self.vertices.len() as u32, 0..1);
    }
}
