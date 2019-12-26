use crate::render::{
    ShapePolygon,
    debug::Position as DebugPos,
    object::{
        Context as ObjectContext,
        Vertex as ObjectVertex,
    },
};
use m3d;

use zerocopy::AsBytes as _;

use std::{
    mem,
    fs::File,
    ops::Range,
    slice,
    sync::Arc,
};


pub struct Mesh {
    pub num_vertices: usize,
    pub vertex_buf: wgpu::Buffer,
    pub offset: [f32; 3],
    pub bbox: ([f32; 3], [f32; 3], f32),
    pub physics: m3d::Physics,
}

#[derive(Clone, Debug)]
pub struct Polygon {
    pub middle: [f32; 3],
    pub normal: [f32; 3],
    pub samples: Range<usize>,
}

pub struct Shape {
    pub polygons: Vec<Polygon>,
    pub samples: Vec<RawVertex>,
    pub vertex_buf: wgpu::Buffer,
    pub polygon_buf: wgpu::Buffer,
    pub sample_buf: Option<(wgpu::Buffer, usize)>,
    pub bind_group: wgpu::BindGroup,
    pub bounds: m3d::Bounds,
}

pub type RawVertex = [i8; 3];
pub type ShapeVertex = [i8; 4];

struct Tessellator {
    samples: Vec<RawVertex>,
    depth: u8,
}

impl Tessellator {
    fn new(depth: u8) -> Self {
        Tessellator {
            samples: Vec::new(),
            depth,
        }
    }
    fn tessellate(
        &mut self,
        corners: &[RawVertex],
        _middle: RawVertex,
    ) -> &[RawVertex] {
        self.samples.clear();
        //self.samples.push(middle);
        let mid_sum = corners
            .iter()
            .fold([0f32; 3], |sum, cur| [
                sum[0] + cur[0] as f32,
                sum[1] + cur[1] as f32,
                sum[2] + cur[2] as f32,
            ]);
        if self.depth != 0 {
            let corner_ratio = 0.66f32;
            let div = (1.0 - corner_ratio) / corners.len() as f32;
            let mid_rationed = [
                (mid_sum[0] * div) as i8,
                (mid_sum[1] * div) as i8,
                (mid_sum[2] * div) as i8,
            ];
            let ring1 = corners.iter().map(|c| {
                [
                    (corner_ratio * c[0] as f32) as i8 + mid_rationed[0],
                    (corner_ratio * c[1] as f32) as i8 + mid_rationed[1],
                    (corner_ratio * c[2] as f32) as i8 + mid_rationed[2],
                ]
            }).collect::<Vec<_>>();
            self.samples.extend((0 .. corners.len()).map(|i| {
                let c0 = &ring1[i];
                let c1 = &ring1[(i+1)%corners.len()];
                [
                    c0[0] / 2 + c1[0] / 2,
                    c0[1] / 2 + c1[1] / 2,
                    c0[2] / 2 + c1[2] / 2,
                ]
            }));
            self.samples.extend(ring1);
            self.samples.push([
                (mid_sum[0] / corners.len() as f32) as i8,
                (mid_sum[1] / corners.len() as f32) as i8,
                (mid_sum[2] / corners.len() as f32) as i8,
            ]);
        } else {
            let div = 0.5 / corners.len() as f32;
            let mid_half = [
                (mid_sum[0] * div) as i8,
                (mid_sum[1] * div) as i8,
                (mid_sum[2] * div) as i8,
            ];
            self.samples.extend(corners.iter().map(|c| {
                [
                    c[0] / 2 + mid_half[0],
                    c[1] / 2 + mid_half[1],
                    c[2] / 2 + mid_half[2],
                ]
            }));
        }
        &self.samples
    }
}


fn vec_i2f(v: [i32; 3]) -> [f32; 3] {
    [v[0] as f32, v[1] as f32, v[2] as f32]
}

pub fn load_c3d(
    raw: m3d::Mesh<m3d::Geometry<m3d::DrawTriangle>>,
    device: &wgpu::Device,
) -> Arc<Mesh> {
    let num_vertices = raw.geometry.polygons.len() * 3;
    debug!("\tGot {} GPU vertices...", num_vertices);
    let vertex_size = mem::size_of::<ObjectVertex>();
    let mapping = device.create_buffer_mapped(
        num_vertices * vertex_size,
        wgpu::BufferUsage::VERTEX,
    );
    for (chunk, tri) in mapping.data
        .chunks_mut(3 * vertex_size)
        .zip(&raw.geometry.polygons)
    {
        let out_vertices = unsafe {
            slice::from_raw_parts_mut(chunk.as_mut_ptr() as *mut ObjectVertex, 3)
        };
        for (vo, v) in out_vertices.iter_mut().zip(&tri.vertices) {
            let p = raw.geometry.positions[v.pos as usize];
            let n = raw.geometry.normals[v.normal as usize];
            *vo = ObjectVertex {
                pos: [p[0], p[1], p[2], 1],
                color: tri.material[0],
                normal: [n[0], n[1], n[2], 0],
            };
        }
    }

    Arc::new(Mesh {
        num_vertices,
        vertex_buf: mapping.finish(),
        offset: vec_i2f(raw.parent_off),
        bbox: (
            vec_i2f(raw.bounds.coord_min),
            vec_i2f(raw.bounds.coord_max),
            raw.max_radius as f32,
        ),
        physics: raw.physics,
    })
}

pub fn load_c3d_shape(
    raw: m3d::Mesh<m3d::Geometry<m3d::CollisionQuad>>,
    device: &wgpu::Device,
    shape_sampling: u8,
    with_sample_buf: bool,
    object: &ObjectContext,
) -> Arc<Shape> {
    debug!("\tTessellating polygons...");
    let mut polygons = Vec::new();
    let mut polygon_data = Vec::with_capacity(raw.geometry.polygons.len());
    let mut samples = Vec::new();
    let mut sample_data = Vec::new();
    let mut tess = Tessellator::new(shape_sampling);

    for quad in &raw.geometry.polygons {
        let corners = [
            raw.geometry.positions[quad.vertices[0] as usize],
            raw.geometry.positions[quad.vertices[1] as usize],
            raw.geometry.positions[quad.vertices[2] as usize],
            raw.geometry.positions[quad.vertices[3] as usize],
        ];
        let square = 1.0; //TODO: compute polygon square
        let middle = [
            quad.middle[0] as f32,
            quad.middle[1] as f32,
            quad.middle[2] as f32,
        ];
        polygon_data.push(ShapePolygon {
            indices: quad.vertices,
            normal: [
                quad.flat_normal[0],
                quad.flat_normal[1],
                quad.flat_normal[2],
                0,
            ],
            origin_square: [ middle[0], middle[1], middle[2], square ],
        });
        let normal = [
            quad.flat_normal[0] as f32 / m3d::NORMALIZER,
            quad.flat_normal[1] as f32 / m3d::NORMALIZER,
            quad.flat_normal[2] as f32 / m3d::NORMALIZER,
        ];
        let cur_samples = tess.tessellate(&corners[..], quad.middle);

        if with_sample_buf {
            let mut nlen = 16.0;
            sample_data.push(DebugPos {
                pos: [ middle[0], middle[1], middle[2], 1.0],
            });
            sample_data.push(DebugPos {
                pos: [
                    middle[0] + normal[0] * nlen,
                    middle[1] + normal[1] * nlen,
                    middle[2] + normal[2] * nlen,
                    1.0,
                ],
            });
            nlen = 4.0;
            for s in cur_samples {
                sample_data.push(DebugPos {
                    pos: [s[0] as f32, s[1] as f32, s[2] as f32, 1.0],
                });
                sample_data.push(DebugPos {
                    pos: [
                        s[0] as f32 + normal[0] * nlen,
                        s[1] as f32 + normal[1] * nlen,
                        s[2] as f32 + normal[2] * nlen,
                        1.0,
                    ],
                });
            }
        }

        polygons.push(Polygon {
            middle,
            normal,
            samples: samples.len() .. samples.len() + cur_samples.len(),
        });
        samples.extend(cur_samples);
    }

    let vertex_buf = {
        let mapping = device.create_buffer_mapped(
            raw.geometry.positions.len() * mem::size_of::<ShapeVertex>(),
            wgpu::BufferUsage::VERTEX | wgpu::BufferUsage::STORAGE_READ,
        );
        for (vo, p) in mapping.data.chunks_mut(4).zip(&raw.geometry.positions) {
            vo[..3].copy_from_slice(unsafe {
                slice::from_raw_parts(p.as_ptr() as *const u8, 3)
            });
            vo[3] = 1;
        }
        mapping.finish()
    };
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &object.shape_bind_group_layout,
        bindings: &[
            wgpu::Binding {
                binding: 0,
                resource: wgpu::BindingResource::Buffer {
                    buffer: &vertex_buf,
                    range: 0 .. (raw.geometry.positions.len() * 4) as wgpu::BufferAddress,
                },
            },
        ],
    });

    Arc::new(Shape {
        polygons,
        samples,
        vertex_buf,
        bind_group,
        polygon_buf: device
            .create_buffer_with_data(
                polygon_data.as_bytes(),
                wgpu::BufferUsage::VERTEX,
            ),
        sample_buf: if with_sample_buf {
            let buffer = device.create_buffer_with_data(
                sample_data.as_bytes(),
                wgpu::BufferUsage::VERTEX,
            );
            Some((buffer, sample_data.len()))
        } else {
            None
        },
        bounds: raw.bounds,
    })
}

pub type VisualModel = m3d::Model<Arc<Mesh>, Arc<Shape>>;

pub fn load_m3d(
    file: File,
    device: &wgpu::Device,
    object: &ObjectContext,
    shape_sampling: u8,
) -> VisualModel {
    let raw = m3d::FullModel::load(file);

    let model = VisualModel {
        body: load_c3d(raw.body, device),
        shape: load_c3d_shape(raw.shape, device, shape_sampling, true, object),
        dimensions: raw.dimensions,
        max_radius: raw.max_radius,
        color: raw.color,
        wheels: raw.wheels
            .into_iter()
            .map(|wheel| wheel.map(|mesh| {
                load_c3d(mesh, device)
            }))
            .collect(),
        debris: raw.debris
            .into_iter()
            .map(|debrie| m3d::Debrie {
                mesh: load_c3d(debrie.mesh, device),
                shape: load_c3d_shape(debrie.shape, device, 0, false, object),
            })
            .collect(),
        slots: m3d::Slot::map_all(raw.slots, |_, _| unreachable!()),
    };

    model
}
