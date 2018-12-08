use byteorder::{LittleEndian as E, ReadBytesExt, WriteBytesExt};
use gfx;
use gfx::format::I8Norm;

use render::{
    COLOR_ID_BODY, NUM_COLOR_IDS,
    DebugPos, ObjectVertex, ShapeVertex, ShapePolygon,
};

use std::fs::File;
use std::io::{self, Seek, Write};
use std::ops::Range;
use std::path::PathBuf;


const MAX_SLOTS: usize = 3;

#[derive(Clone, Serialize, Deserialize)]
pub struct Physics {
    pub volume: f32,
    pub rcm: [f32; 3],
    pub jacobi: [[f32; 3]; 3], // column-major
}

#[derive(Clone)]
pub struct Mesh<R: gfx::Resources> {
    pub slice: gfx::Slice<R>,
    pub buffer: gfx::handle::Buffer<R, ObjectVertex>,
    pub offset: [f32; 3],
    pub bbox: ([f32; 3], [f32; 3], f32),
    pub physics: Physics,
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
    pub bounds: Bounds,
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

#[derive(Clone, Serialize, Deserialize)]
pub struct Wheel<M> {
    pub mesh: Option<M>,
    pub steer: u32,
    pub pos: [f32; 3],
    pub width: u32,
    pub radius: u32,
    bound_index: u32,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Debrie<M, S> {
    pub mesh: M,
    pub shape: S,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Slot<M> {
    pub mesh: Option<M>,
    pub scale: f32,
    pub pos: [i32; 3],
    pub angle: i32,
}

impl<M> Slot<M> {
    fn empty() -> Self {
        Slot {
            mesh: None,
            scale: 0.0,
            pos: [0; 3],
            angle: 0,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Model<M, S> {
    pub body: M,
    pub shape: S,
    dimensions: [u32; 3],
    max_radius: u32,
    pub color: [u32; 2],
    pub wheels: Vec<Wheel<M>>,
    pub debris: Vec<Debrie<M, S>>,
    pub slots: [Slot<M>; MAX_SLOTS],
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bounds {
    pub coord_min: [i32; 3],
    pub coord_max: [i32; 3],
}

impl Bounds {
    fn read<I: ReadBytesExt>(source: &mut I) -> Self {
        let mut b = [0i32; 6];
        for b in &mut b {
            *b = source.read_i32::<E>().unwrap();
        }
        Bounds {
            coord_min: [b[3], b[4], b[5]],
            coord_max: [b[0], b[1], b[2]],
        }
    }
}

#[derive(Default)]
pub struct Geometry {
    vertices: Vec<ObjectVertex>,
    indices: Vec<u16>,
}

impl Geometry {
    pub fn save_obj<W: Write>(
        &self,
        mut dest: W,
    ) -> io::Result<()> {
        for v in self.vertices.iter() {
            try!(writeln!(dest, "v {} {} {}", v.pos[0], v.pos[1], v.pos[2]));
        }
        try!(writeln!(dest, ""));
        for v in self.vertices.iter() {
            try!(writeln!(
                dest,
                "vn {} {} {}",
                v.normal[0].0 as f32 / 124.0,
                v.normal[1].0 as f32 / 124.0,
                v.normal[2].0 as f32 / 124.0
            ));
        }
        try!(writeln!(dest, ""));
        if self.indices.is_empty() {
            for i in 0 .. self.vertices.len() / 3 {
                try!(writeln!(
                    dest,
                    "f {} {} {}",
                    i * 3 + 1,
                    i * 3 + 2,
                    i * 3 + 3
                ));
            }
        } else {
            for c in self.indices.chunks(3) {
                // notice the winding order change
                try!(writeln!(dest, "f {} {} {}", c[0] + 1, c[1] + 1, c[2] + 1));
            }
        }
        Ok(())
    }

    #[cfg(feature = "obj")]
    fn load_obj(path: PathBuf) -> Self {
        use obj::{IndexTuple, Obj, SimplePolygon};
        let obj: Obj<SimplePolygon> = Obj::load(&path).unwrap();
        assert_eq!(obj.position.len(), obj.normal.len());
        let mut vertices = Vec::new();
        for object in &obj.objects {
            for group in &object.groups {
                for poly in &group.polys {
                    for &IndexTuple(a, _b, c) in poly {
                        let p = obj.position[a];
                        let n = obj.normal[c.unwrap_or(a)];
                        vertices.push(ObjectVertex {
                            pos: [p[0] as i8, p[1] as i8, p[2] as i8, 1],
                            color: 0, //TODO!
                            normal: [
                                gfx::format::I8Norm((n[0] * 127.5) as i8),
                                gfx::format::I8Norm((n[1] * 127.5) as i8),
                                gfx::format::I8Norm((n[2] * 127.5) as i8),
                                gfx::format::I8Norm(0),
                            ],
                        });
                    }
                }
            }
        }
        Geometry {
            vertices,
            indices: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct RawMesh<G> {
    geometry: G,
    bounds: Bounds,
    parent_off: [f32; 3],
    parent_rot: [f32; 3],
    max_radius: f32,
    physics: Physics,
}

impl RawMesh<Geometry> {
    fn with_geometry<T>(self, geometry: T) -> RawMesh<T> {
        RawMesh {
            geometry,
            bounds: self.bounds,
            parent_off: self.parent_off,
            parent_rot: self.parent_rot,
            max_radius: self.max_radius,
            physics: self.physics,
        }
    }

    pub fn load<I: ReadBytesExt>(
        source: &mut I,
        compact: bool,
    ) -> Self {
        let version = source.read_u32::<E>().unwrap();
        assert_eq!(version, 8);
        let num_positions = source.read_u32::<E>().unwrap();
        let num_normals = source.read_u32::<E>().unwrap();
        let num_polygons = source.read_u32::<E>().unwrap();
        let _total_verts = source.read_u32::<E>().unwrap();

        let mut result = RawMesh {
            geometry: Geometry::default(),
            bounds: Bounds::read(source),
            parent_off: read_vec(source),
            max_radius: source.read_u32::<E>().unwrap() as f32,
            parent_rot: read_vec(source),
            physics: {
                let mut q = [0.0f32; 1 + 3 + 9];
                for qel in q.iter_mut() {
                    *qel = source.read_f64::<E>().unwrap() as f32;
                }
                Physics {
                    volume: q[0],
                    rcm: [q[1], q[2], q[3]],
                    jacobi: [
                        [q[4], q[7], q[10]],
                        [q[5], q[8], q[11]],
                        [q[6], q[9], q[12]],
                    ],
                }
            },
        };
        debug!(
            "\tBounds {:?} with offset {:?}",
            result.bounds, result.parent_off
        );

        debug!("\tReading {} positions...", num_positions);
        let mut positions = Vec::with_capacity(num_positions as usize);
        for _ in 0 .. num_positions {
            read_vec(source); //unknown
            let pos = [
                source.read_i8().unwrap(),
                source.read_i8().unwrap(),
                source.read_i8().unwrap(),
                1,
            ];
            let _sort_info = source.read_u32::<E>().unwrap();
            positions.push(pos);
        }

        debug!("\tReading {} normals...", num_normals);
        let mut normals = Vec::with_capacity(num_normals as usize);
        for _ in 0 .. num_normals {
            let mut norm = [0u8; 4];
            source.read_exact(&mut norm).unwrap();
            let _sort_info = source.read_u32::<E>().unwrap();
            normals.push(norm);
        }

        debug!("\tReading {} polygons...", num_polygons);
        let mut vertices = Vec::with_capacity(num_polygons as usize * 3);
        for i in 0 .. num_polygons {
            let num_corners = source.read_u32::<E>().unwrap();
            assert!(num_corners == 3 || num_corners == 4);
            let _sort_info = source.read_u32::<E>().unwrap();
            let color = [
                source.read_u32::<E>().unwrap(),
                source.read_u32::<E>().unwrap(),
            ];
            let mut flat_normal = [0; 4];
            source.read_exact(&mut flat_normal).unwrap();
            let mut middle = [0; 3];
            source.read_exact(&mut middle).unwrap();
            for k in 0 .. num_corners {
                let pid = source.read_u32::<E>().unwrap();
                let nid = source.read_u32::<E>().unwrap();
                let v = (
                    i * 3 + k,
                    (positions[pid as usize], normals[nid as usize], color),
                );
                vertices.push(v);
            }
        }

        // sorted variable polygons
        for _ in 0 .. 3 {
            for _ in 0 .. num_polygons {
                let _poly_ind = source.read_u32::<E>().unwrap();
            }
        }

        let convert = |(p, n, c): ([i8; 4], [u8; 4], [u32; 2])| ObjectVertex {
            pos: p,
            color: if c[0] < NUM_COLOR_IDS {
                c[0]
            } else {
                COLOR_ID_BODY
            },
            normal: [
                I8Norm(n[0] as i8),
                I8Norm(n[1] as i8),
                I8Norm(n[2] as i8),
                I8Norm(n[3] as i8),
            ],
        };

        if compact {
            debug!("\tCompacting...");
            vertices.sort_by_key(|v| v.1);
            //vertices.dedup();
            result.geometry.indices.extend((0 .. vertices.len()).map(|_| 0));
            let mut last = vertices[0].1;
            last.2[0] ^= 1; //change something
            let mut v_id = 0;
            for v in vertices.into_iter() {
                if v.1 != last {
                    last = v.1;
                    v_id = result.geometry.vertices.len() as u16;
                    result.geometry.vertices.push(convert(v.1));
                }
                result.geometry.indices[v.0 as usize] = v_id;
            }
        } else {
            result.geometry.vertices
                .extend(vertices.into_iter().map(|v| convert(v.1)))
        };

        result
    }

    fn save<W: Write>(&self, _dest: W) {
        unimplemented!()
    }
}

fn vec_i2f(v: [i32; 3]) -> [f32; 3] {
    [v[0] as f32, v[1] as f32, v[2] as f32]
}

pub fn load_c3d<I, R, F>(
    source: &mut I, factory: &mut F
) -> Mesh<R>
where
    I: ReadBytesExt,
    R: gfx::Resources,
    F: gfx::traits::FactoryExt<R>,
{
    let raw = RawMesh::load(source, true);

    let (vbuf, slice) = if raw.geometry.indices.is_empty() {
        factory.create_vertex_buffer_with_slice(&raw.geometry.vertices, ())
    } else {
        factory.create_vertex_buffer_with_slice(&raw.geometry.vertices, &raw.geometry.indices[..])
    };

    debug!("\tGot {} GPU vertices...", raw.geometry.vertices.len());
    Mesh {
        slice: slice,
        buffer: vbuf,
        offset: raw.parent_off,
        bbox: (
            vec_i2f(raw.bounds.coord_min),
            vec_i2f(raw.bounds.coord_max),
            raw.max_radius,
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
    let bounds = Bounds::read(source);
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

pub fn load_m3d<I, R, F>(
    source: &mut I, factory: &mut F
) -> Model<Mesh<R>, Shape<R>>
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

        wheels.push(Wheel {
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
        debris.push(Debrie {
            mesh: load_c3d(source, factory),
            shape: load_c3d_shape(source, factory, false),
        })
    }

    debug!("\tReading the physical shape...");
    let shape = load_c3d_shape(source, factory, true);

    let mut slots = [Slot::empty(), Slot::empty(), Slot::empty()];
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

    Model {
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

type RefModel = Model<RawMesh<String>, RawMesh<String>>;
pub type FullModel = Model<RawMesh<Geometry>, RawMesh<Geometry>>;
pub type RenderModel<R> = Model<Mesh<R>, Shape<R>>;

const BODY_PATH: &str = "body.obj";
const SHAPE_PATH: &str = "body-shape.obj";
const MODEL_PATH: &str = "model.ron";

pub fn convert_m3d(
    mut input: File,
    out_path: &PathBuf,
) {
    use ron;

    if !out_path.is_dir() {
        panic!("The output path must be an existing directory!");
    }

    debug!("\tReading the body...");
    let body = RawMesh::load(&mut input, false);
    body.geometry.save_obj(File::create(out_path.join(BODY_PATH)).unwrap())
        .unwrap();

    let dimensions = [
        input.read_u32::<E>().unwrap(),
        input.read_u32::<E>().unwrap(),
        input.read_u32::<E>().unwrap(),
    ];
    let max_radius = input.read_u32::<E>().unwrap();
    let num_wheels = input.read_u32::<E>().unwrap();
    let num_debris = input.read_u32::<E>().unwrap();
    let color = [
        input.read_u32::<E>().unwrap(),
        input.read_u32::<E>().unwrap(),
    ];

    let mut wheels = Vec::with_capacity(num_wheels as usize);
    debug!("\tReading {} wheels...", num_wheels);
    for i in 0 .. num_wheels {
        let steer = input.read_u32::<E>().unwrap();
        let pos = [
            input.read_f64::<E>().unwrap() as f32,
            input.read_f64::<E>().unwrap() as f32,
            input.read_f64::<E>().unwrap() as f32,
        ];
        let width = input.read_u32::<E>().unwrap();
        let radius = input.read_u32::<E>().unwrap();
        let bound_index = input.read_u32::<E>().unwrap();
        let mesh = if steer != 0 {
            let name = format!("wheel{}.obj", i);
            let path = out_path.join(&name);
            let wheel = RawMesh::load(&mut input, false);
            wheel.geometry.save_obj(File::create(path).unwrap()).unwrap();
            Some(wheel.with_geometry(name))
        } else {
            None
        };

        wheels.push(Wheel {
            mesh,
            steer,
            pos,
            width,
            radius,
            bound_index,
        });
    }

    let mut debris = Vec::with_capacity(num_debris as usize);
    debug!("\tReading {} debris...", num_debris);
    for i in 0 .. num_debris {
        let name = format!("debrie{}.obj", i);
        let debrie = RawMesh::load(&mut input, false);
        debrie.geometry.save_obj(File::create(out_path.join(&name)).unwrap()).unwrap();
        let shape_name = format!("debrie{}-shape.obj", i);
        let shape = RawMesh::load(&mut input, false);
        shape.geometry.save_obj(File::create(out_path.join(&shape_name)).unwrap()).unwrap();
        debris.push(Debrie {
            mesh: debrie.with_geometry(name),
            shape: shape.with_geometry(shape_name),
        });
    }

    debug!("\tReading the shape...");
    let shape = RawMesh::load(&mut input, false);
    shape.geometry.save_obj(File::create(out_path.join(SHAPE_PATH)).unwrap())
        .unwrap();

    let mut slots = [Slot::empty(), Slot::empty(), Slot::empty()];
    let slot_mask = input.read_u32::<E>().unwrap();
    debug!("\tReading {} slot mask...", slot_mask);
    for slot in &mut slots {
        for p in &mut slot.pos {
            *p = input.read_i32::<E>().unwrap();
        }
        slot.angle = input.read_i32::<E>().unwrap();
        slot.scale = 1.0;
    }

    let model = RefModel {
        body: body.with_geometry(BODY_PATH.to_string()),
        shape: shape.with_geometry(SHAPE_PATH.to_string()),
        dimensions,
        max_radius,
        color,
        wheels,
        debris,
        slots,
    };
    let string = ron::ser::to_string_pretty(&model, ron::ser::PrettyConfig::default()).unwrap();
    let mut model_file = File::create(out_path.join(MODEL_PATH)).unwrap();
    write!(model_file, "{}", string).unwrap();
}

impl RawMesh<String> {
    #[cfg(feature = "obj")]
    fn resolve(&self, source_dir: &PathBuf) -> RawMesh<Geometry> {
        RawMesh {
            geometry: Geometry::load_obj(source_dir.join(&self.geometry)),
            bounds: self.bounds.clone(),
            parent_off: self.parent_off,
            parent_rot: self.parent_rot,
            max_radius: self.max_radius,
            physics: self.physics.clone(),
        }
    }
}

impl Slot<RawMesh<String>> {
    #[cfg(feature = "obj")]
    fn resolve(&self, source_dir: &PathBuf) -> Slot<RawMesh<Geometry>> {
        Slot {
            mesh: self.mesh.as_ref().map(|m| m.resolve(source_dir)),
            scale: self.scale,
            pos: self.pos,
            angle: self.angle,
        }
    }
}

impl FullModel {
    #[cfg(feature = "obj")]
    pub fn import(dir_path: &PathBuf) -> Self {
        let model_file = File::open(dir_path.join(MODEL_PATH)).unwrap();
        let model = ron::de::from_reader::<_, RefModel>(model_file).unwrap();
        FullModel {
            body: model.body.resolve(dir_path),
            shape: model.shape.resolve(dir_path),
            dimensions: model.dimensions,
            max_radius: model.max_radius,
            color: model.color,
            wheels: model.wheels
                .into_iter()
                .map(|wheel| Wheel {
                    mesh: wheel.mesh.map(|m| m.resolve(dir_path)),
                    steer: wheel.steer,
                    pos: wheel.pos,
                    width: wheel.width,
                    radius: wheel.radius,
                    bound_index: wheel.bound_index,
                })
                .collect(),
            debris: model.debris
                .into_iter()
                .map(|debrie| Debrie {
                    mesh: debrie.mesh.resolve(dir_path),
                    shape: debrie.shape.resolve(dir_path),
                })
                .collect(),
            slots: [
                model.slots[0].resolve(dir_path),
                model.slots[1].resolve(dir_path),
                model.slots[2].resolve(dir_path),
            ],
        }
    }

    pub fn save(&self, out_path: &PathBuf) {
        let mut output = File::create(out_path).unwrap();
        self.body.save(&mut output);
        for d in &self.dimensions {
            output.write_u32::<E>(*d).unwrap();
        }
        output.write_u32::<E>(self.max_radius).unwrap();
        output.write_u32::<E>(self.wheels.len() as u32).unwrap();
        output.write_u32::<E>(self.debris.len() as u32).unwrap();
        for c in &self.color {
            output.write_u32::<E>(*c).unwrap();
        }

        for wheel in &self.wheels {
            output.write_u32::<E>(wheel.steer).unwrap();
            for p in &wheel.pos {
                output.write_f64::<E>(*p as f64).unwrap();
            }
            output.write_u32::<E>(wheel.width).unwrap();
            output.write_u32::<E>(wheel.radius).unwrap();
            output.write_u32::<E>(wheel.bound_index).unwrap();
            if let Some(ref mesh) = wheel.mesh {
                mesh.save(&mut output);
            }
        }

        for debrie in &self.debris {
            debrie.mesh.save(&mut output);
            debrie.shape.save(&mut output);
        }

        self.shape.save(&mut output);

        let slot_mask = 0; //TODO?
        output.write_u32::<E>(slot_mask).unwrap();
        for slot in &self.slots {
            for p in &slot.pos {
                output.write_i32::<E>(*p).unwrap();
            }
            output.write_i32::<E>(slot.angle).unwrap()
        }
    }
}
