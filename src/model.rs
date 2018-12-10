use byteorder::{LittleEndian as E, ReadBytesExt};
use gfx;
use gfx::format::I8Norm;

use m3d;
use render::{
    DebugPos, ObjectVertex, ShapeVertex, ShapePolygon,
};

use std::io::Seek;
use std::ops::Range;


pub const NUM_COLOR_IDS: u32 = 25;
const COLOR_ID_BODY: u32 = 1;

#[derive(Clone)]
pub struct Mesh<R: gfx::Resources> {
    pub slice: gfx::Slice<R>,
    pub buffer: gfx::handle::Buffer<R, ObjectVertex>,
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

#[derive(Clone, Debug)]
pub struct Shape<R: gfx::Resources> {
    pub polygons: Vec<Polygon>,
    pub samples: Vec<RawVertex>,
    pub vertex_buf: gfx::handle::Buffer<R, ShapeVertex>,
    pub vertex_view: gfx::handle::ShaderResourceView<R, [f32; 4]>,
    pub polygon_buf: gfx::handle::Buffer<R, ShapePolygon>,
    pub sample_buf: Option<gfx::handle::Buffer<R, DebugPos>>,
    pub bounds: m3d::Bounds,
}

impl<R: gfx::Resources> Shape<R> {
    pub fn make_draw_slice(&self) -> gfx::Slice<R> {
        gfx::Slice {
            start: 0,
            end: 4,
            base_vertex: 0,
            instances: Some((self.polygons.len() as _, 0)),
            buffer: gfx::IndexBuffer::Auto,
        }
    }
}

pub type RawVertex = [i8; 3];

struct Tessellator {
    samples: Vec<RawVertex>,
}

impl Tessellator {
    fn new() -> Self {
        Tessellator {
            samples: Vec::new(),
        }
    }
    fn tessellate(
        &mut self,
        corners: &[ShapeVertex],
        _middle: RawVertex,
    ) -> &[RawVertex] {
        let go_deeper = false;
        self.samples.clear();
        //self.samples.push(middle);
        let mid_sum = corners
            .iter()
            .fold([0f32; 3], |sum, cur| [
                sum[0] + cur[0],
                sum[1] + cur[1],
                sum[2] + cur[2],
            ]);
        if go_deeper {
            let corner_ratio = 0.66f32;
            let div = (1.0 - corner_ratio) / corners.len() as f32;
            let mid_rationed = [
                (mid_sum[0] * div) as i8,
                (mid_sum[1] * div) as i8,
                (mid_sum[2] * div) as i8,
            ];
            let ring1 = corners.iter().map(|c| {
                [
                    (corner_ratio * c[0]) as i8 + mid_rationed[0],
                    (corner_ratio * c[1]) as i8 + mid_rationed[1],
                    (corner_ratio * c[2]) as i8 + mid_rationed[2],
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
                    (0.5 * c[0]) as i8 + mid_half[0],
                    (0.5 * c[1]) as i8 + mid_half[1],
                    (0.5 * c[2]) as i8 + mid_half[2],
                ]
            }));
        }
        &self.samples
    }
}

fn read_vec<I: ReadBytesExt>(source: &mut I) -> [f32; 3] {
    [
        source.read_i32::<E>().unwrap() as f32,
        source.read_i32::<E>().unwrap() as f32,
        source.read_i32::<E>().unwrap() as f32,
    ]
}

fn vec_i2f(v: [i32; 3]) -> [f32; 3] {
    [v[0] as f32, v[1] as f32, v[2] as f32]
}

fn color(material: u32) -> u32 {
    if material < NUM_COLOR_IDS {
        material
    } else {
        COLOR_ID_BODY
    }
}

pub fn load_c3d<I, R, F>(
    source: &mut I, factory: &mut F
) -> Mesh<R>
where
    I: ReadBytesExt,
    R: gfx::Resources,
    F: gfx::traits::FactoryExt<R>,
{
    let raw = m3d::Mesh::<m3d::Geometry<m3d::DrawTriangle>>::load(source);

    let m3d::Geometry { ref positions, ref normals, ref polygons } = raw.geometry;
    let vertices = polygons
        .iter()
        .flat_map(|tri| {
            tri.vertices.into_iter().map(move |v| {
                let p = positions[v.pos as usize];
                let n = normals[v.normal as usize];
                ObjectVertex {
                    pos: [p[0], p[1], p[2], 1],
                    color: color(tri.material[0]),
                    normal: [I8Norm(n[0]), I8Norm(n[1]), I8Norm(n[2]), I8Norm(0)],
                }
            })
        })
        .collect::<Vec<_>>();

    let (buffer, slice) = factory.create_vertex_buffer_with_slice(&vertices, ());

    debug!("\tGot {} GPU vertices...", vertices.len());
    Mesh {
        slice,
        buffer,
        offset: vec_i2f(raw.parent_off),
        bbox: (
            vec_i2f(raw.bounds.coord_min),
            vec_i2f(raw.bounds.coord_max),
            raw.max_radius as f32,
        ),
        physics: raw.physics,
    }
}

pub fn load_c3d_shape<I, R, F>(
    source: &mut I, factory: &mut F, with_sample_buf: bool,
) -> Shape<R>
where
    I: ReadBytesExt + Seek,
    R: gfx::Resources,
    F: gfx::traits::FactoryExt<R>,
{
    use std::io::SeekFrom::Current;

    let version = source.read_u32::<E>().unwrap();
    assert_eq!(version, 8);
    let num_positions = source.read_u32::<E>().unwrap();
    let num_normals = source.read_u32::<E>().unwrap();
    let num_polygons = source.read_u32::<E>().unwrap();
    let _total_verts = source.read_u32::<E>().unwrap();

    let mut polygons = Vec::with_capacity(num_polygons as _);
    let mut samples = Vec::new();
    let bounds = m3d::Bounds::read(source);
    debug!("\tBounds {:?}", bounds);
    let mut polygon_data = Vec::with_capacity(num_polygons as _);
    let mut sample_data = Vec::new();

    source
        .seek(Current(
            (3+1+3) * 4 + // parent offset, max radius, and parent rotation
        (1+3+9) * 8 + // physics
        0,
        ))
        .unwrap();

    let positions: Vec<_> = (0 .. num_positions)
        .map(|_| {
            read_vec(source); //unknown "ti"
            let pos = [
                source.read_i8().unwrap() as f32,
                source.read_i8().unwrap() as f32,
                source.read_i8().unwrap() as f32,
                1.0,
            ];
            let _sort_info = source.read_u32::<E>().unwrap();
            pos
        })
        .collect();

    source
        .seek(Current(
            (num_normals as i64) * (4 * 1 + 4), // normals
        ))
        .unwrap();

    debug!("\tReading {} polygons...", num_polygons);
    let mut tess = Tessellator::new();
    for _ in 0 .. num_polygons {
        let num_corners = source.read_u32::<E>().unwrap() as usize;
        assert_eq!(num_corners, 4);
        source.seek(Current(4 + 4 + 4)).unwrap(); // sort info and color
        let mut d = [0i8; 7];
        for b in d.iter_mut() {
            *b = source.read_i8().unwrap();
        }
        let mut indices = [0u16; 4];
        for i in 0 .. num_corners {
            indices[i] = source.read_u32::<E>().unwrap() as _;
            let _ = source.read_u32::<E>().unwrap(); //nid
        }
        let corners = [
            positions[indices[0] as usize],
            positions[indices[1] as usize],
            positions[indices[2] as usize],
            positions[indices[3] as usize],
        ];
        let square = 1.0; //TODO: compute polygon square
        polygon_data.push(ShapePolygon {
            indices,
            normal: [I8Norm(d[0]), I8Norm(d[1]), I8Norm(d[2]), I8Norm(0)],
            origin_square: [d[4] as f32, d[5] as f32, d[6] as f32, square],
        });
        let middle = [d[4] as f32, d[5] as f32, d[6] as f32];
        let normal = [
            d[0] as f32 / 128.0,
            d[1] as f32 / 128.0,
            d[2] as f32 / 128.0,
        ];
        let cur_samples = tess.tessellate(
            &corners[.. num_corners],
            [d[4], d[5], d[6]],
        );
        if with_sample_buf {
            let mut nlen = 16.0;
            sample_data.push(DebugPos {
                pos: [middle[0], middle[1], middle[2], 1.0],
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

    source.seek(Current(3 * (num_polygons as i64) * 4)).unwrap(); // sorted var polys

    let vertex_buf = factory
        .create_buffer_immutable(
            &positions,
            gfx::buffer::Role::Vertex,
            gfx::memory::Bind::SHADER_RESOURCE,
        )
        .unwrap();

    Shape {
        polygons,
        samples,
        vertex_view: factory
            .view_buffer_as_shader_resource(&vertex_buf)
            .unwrap(),
        vertex_buf,
        polygon_buf: factory.create_vertex_buffer(&polygon_data),
        sample_buf: if with_sample_buf {
            Some(factory.create_vertex_buffer(&sample_data))
        } else {
            None
        },
        bounds,
    }
}

//TODO: convert to use m3d::Model as a source

pub type RenderModel<R> = m3d::Model<Mesh<R>, Shape<R>>;

pub fn load_m3d<I, R, F>(
    source: &mut I, factory: &mut F
) -> RenderModel<R>
where
    I: ReadBytesExt + Seek,
    R: gfx::Resources,
    F: gfx::traits::FactoryExt<R>,
{
    debug!("\tReading the body...");
    let body = load_c3d(source, factory);
    let dimensions = [
        source.read_u32::<E>().unwrap(),
        source.read_u32::<E>().unwrap(),
        source.read_u32::<E>().unwrap(),
    ];

    let max_radius = source.read_u32::<E>().unwrap();
    let num_wheels = source.read_u32::<E>().unwrap();
    let num_debris = source.read_u32::<E>().unwrap();
    let color = [
        source.read_u32::<E>().unwrap(),
        source.read_u32::<E>().unwrap(),
    ];

    debug!("\tReading {} wheels...", num_wheels);
    let mut wheels = Vec::with_capacity(num_wheels as _);
    for _ in 0 .. num_wheels {
        let steer = source.read_u32::<E>().unwrap();
        let pos = [
            source.read_f64::<E>().unwrap() as f32,
            source.read_f64::<E>().unwrap() as f32,
            source.read_f64::<E>().unwrap() as f32,
        ];
        let width = source.read_u32::<E>().unwrap();
        let radius = source.read_u32::<E>().unwrap();
        let bound_index = source.read_u32::<E>().unwrap();
        debug!("\tSteer {}, width {}, radius {}", steer, width, radius);

        wheels.push(m3d::Wheel {
            mesh: if steer != 0 {
                Some(load_c3d(source, factory))
            } else {
                None
            },
            steer,
            pos,
            width,
            radius,
            bound_index,
        })
    }

    debug!("\tReading {} debris...", num_debris);
    let mut debris = Vec::with_capacity(num_debris as _);
    for _ in 0 .. num_debris {
        debris.push(m3d::Debrie {
            mesh: load_c3d(source, factory),
            shape: load_c3d_shape(source, factory, false),
        })
    }

    debug!("\tReading the physical shape...");
    let shape = load_c3d_shape(source, factory, true);

    let mut slots = [m3d::Slot::EMPTY, m3d::Slot::EMPTY, m3d::Slot::EMPTY];
    let slot_mask = source.read_u32::<E>().unwrap();
    debug!("\tReading {} slot mask...", slot_mask);
    if slot_mask != 0 {
        for (i, slot) in slots.iter_mut().enumerate() {
            for p in &mut slot.pos {
                *p = source.read_i32::<E>().unwrap();
            }
            slot.angle = source.read_i32::<E>().unwrap();
            if slot_mask & (1 << i as i32) != 0 {
                debug!("\tSlot {} at pos {:?} and angle of {}", i, slot.pos, slot.angle);
                slot.scale = 1.0;
            }
        }
    }

    RenderModel {
        body,
        shape,
        dimensions,
        max_radius,
        color,
        wheels,
        debris,
        slots,
    }
}
