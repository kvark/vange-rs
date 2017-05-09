use std::io::{self, Seek, Write};
use std::fs::File;
use byteorder::{LittleEndian as E, ReadBytesExt};
use gfx;
use gfx::format::I8Norm;
use render::{ObjectVertex, DebugPos, NUM_COLOR_IDS, COLOR_ID_BODY};


const MAX_SLOTS: u32 = 3;

#[derive(Clone)]
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
    pub sample_range: (u16, u16),
}

#[derive(Clone, Debug)]
pub struct DebugShape<R: gfx::Resources> {
    pub bound_vb: gfx::handle::Buffer<R, DebugPos>,
    pub bound_slice: gfx::Slice<R>,
    pub sample_vb: gfx::handle::Buffer<R, DebugPos>,
}

#[derive(Clone, Debug)]
pub struct Shape<R: gfx::Resources> {
    pub polygons: Vec<Polygon>,
    pub samples: Vec<[i8; 3]>,
    pub debug: Option<DebugShape<R>>,
}

#[derive(Clone)]
pub struct Wheel<R: gfx::Resources> {
    pub mesh: Option<Mesh<R>>,
    pub steer: u32,
    pub pos: [f32; 3],
    pub width: u32,
    pub radius: u32,
}

#[derive(Clone)]
pub struct Debrie<R: gfx::Resources> {
    pub mesh: Mesh<R>,
    pub shape: Shape<R>,
}

#[derive(Clone)]
pub struct Slot<R: gfx::Resources> {
    pub mesh: Option<Mesh<R>>,
    pub scale: f32,
    pub pos: [f32; 3],
    pub angle: i32,
}

#[derive(Clone)]
pub struct Model<R: gfx::Resources> {
    pub body: Mesh<R>,
    pub shape: Shape<R>,
    pub color: [u32; 2],
    pub wheels: Vec<Wheel<R>>,
    pub debris: Vec<Debrie<R>>,
    pub slots: Vec<Slot<R>>,
}

type RawVertex = [i8; 3];

struct Tessellator {
    samples: Vec<RawVertex>,
}

impl Tessellator {
    fn new() -> Tessellator {
        Tessellator { samples: Vec::new() }
    }
    fn tessellate(&mut self, corners: &[DebugPos], middle: RawVertex) -> &[RawVertex] {
        self.samples.clear();
        self.samples.push(middle);
        self.samples.extend(corners.iter().map(|dv| [
            (dv.pos[0]/3*2 + middle[0]/3),
            (dv.pos[1]/3*2 + middle[1]/3),
            (dv.pos[2]/3*2 + middle[2]/3),
            ]));
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

pub struct RawMesh {
    vertices: Vec<ObjectVertex>,
    indices: Vec<u16>,
    coord_min: [f32; 3],
    coord_max: [f32; 3],
    parent_off: [f32; 3],
    _parent_rot: [f32; 3],
    max_radius: f32,
    physics: Physics,
}

impl RawMesh {
    pub fn load<I: ReadBytesExt>(source: &mut I, compact: bool) -> RawMesh {
        let version = source.read_u32::<E>().unwrap();
        assert_eq!(version, 8);
        let num_positions = source.read_u32::<E>().unwrap();
        let num_normals   = source.read_u32::<E>().unwrap();
        let num_polygons  = source.read_u32::<E>().unwrap();
        let _total_verts  = source.read_u32::<E>().unwrap();

        let mut result = RawMesh {
            vertices: Vec::new(),
            indices: Vec::new(),
            coord_min: read_vec(source),
            coord_max: read_vec(source),
            parent_off: read_vec(source),
            max_radius: source.read_u32::<E>().unwrap() as f32,
            _parent_rot: read_vec(source),
            physics: {
                let mut q = [0.0f32; 1+3+9];
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
        debug!("\tBound {:?} to {:?} with offset {:?}",
               result.coord_min, result.coord_max, result.parent_off);

        debug!("\tReading {} positions...", num_positions);
        let mut positions = Vec::with_capacity(num_positions as usize);
        for _ in 0 .. num_positions {
            read_vec(source); //unknown
            let pos = [
                source.read_i8().unwrap(),
                source.read_i8().unwrap(),
                source.read_i8().unwrap(),
                1];
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
            let color = [source.read_u32::<E>().unwrap(), source.read_u32::<E>().unwrap()];
            let mut dummy = [0; 4];
            source.read_exact(&mut dummy[..4]).unwrap(); //skip flat normal
            source.read_exact(&mut dummy[..3]).unwrap(); //skip middle point
            for k in 0..num_corners {
                let pid = source.read_u32::<E>().unwrap();
                let nid = source.read_u32::<E>().unwrap();
                let v = (i*3+k, (positions[pid as usize], normals[nid as usize], color));
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
            color: if c[0] < NUM_COLOR_IDS { c[0] } else { COLOR_ID_BODY },
            normal: [I8Norm(n[0] as i8), I8Norm(n[1] as i8), I8Norm(n[2] as i8), I8Norm(n[3] as i8)],
        };

        if compact {
            debug!("\tCompacting...");
            vertices.sort_by_key(|v| v.1);
            //vertices.dedup();
            result.indices.extend((0..vertices.len()).map(|_| 0));
            let mut last = vertices[0].1;
            last.2[0] ^= 1; //change something
            let mut v_id = 0;
            for v in vertices.into_iter() {
                if v.1 != last {
                    last = v.1;
                    v_id = result.vertices.len() as u16;
                    result.vertices.push(convert(v.1));
                }
                result.indices[v.0 as usize] = v_id;
            }
        } else {
            result.vertices.extend(vertices.into_iter().map(|v| convert(v.1)))
        };

        result
    }

    pub fn save_obj<W: Write>(&self, mut dest: W) -> io::Result<()> {
        for v in self.vertices.iter() {
            try!(writeln!(dest, "v {} {} {}", v.pos[0], v.pos[1], v.pos[2]));
        }
        try!(writeln!(dest, ""));
        for v in self.vertices.iter() {
            try!(writeln!(dest, "vn {} {} {}", v.normal[0].0 as f32 / 124.0,
                v.normal[1].0 as f32 / 124.0, v.normal[2].0 as f32 / 124.0));
        }
        try!(writeln!(dest, ""));
        if self.indices.is_empty() {
            for i in 0 .. self.vertices.len()/3 {
                try!(writeln!(dest, "f {} {} {}", i*3+1, i*3+2, i*3+3));
            }
        } else {
            for c in self.indices.chunks(3) {
                // notice the winding order change
                try!(writeln!(dest, "f {} {} {}", c[0]+1, c[1]+1, c[2]+1));
            }
        }
        Ok(())
    }
}


pub fn load_c3d<I, R, F>(source: &mut I, factory: &mut F) -> Mesh<R> where
    I: ReadBytesExt,
    R: gfx::Resources,
    F: gfx::traits::FactoryExt<R>,
{
    let raw = RawMesh::load(source, true);

    let (vbuf, slice) = if raw.indices.is_empty() {
        factory.create_vertex_buffer_with_slice(&raw.vertices, ())
    } else {
        factory.create_vertex_buffer_with_slice(&raw.vertices, &raw.indices[..])
    };

    debug!("\tGot {} GPU vertices...", raw.vertices.len());
    Mesh {
        slice: slice,
        buffer: vbuf,
        offset: raw.parent_off,
        bbox: (raw.coord_min, raw.coord_max, raw.max_radius),
        physics: raw.physics,
    }
}

pub fn load_c3d_shape<I, R, F>(source: &mut I, factory: Option<&mut F>) -> Shape<R> where
    I: ReadBytesExt + Seek,
    R: gfx::Resources,
    F: gfx::traits::FactoryExt<R>,
{
    use std::io::SeekFrom::Current;

    let version = source.read_u32::<E>().unwrap();
    assert_eq!(version, 8);
    let num_positions = source.read_u32::<E>().unwrap();
    let num_normals   = source.read_u32::<E>().unwrap();
    let num_polygons  = source.read_u32::<E>().unwrap();
    let _total_verts  = source.read_u32::<E>().unwrap();

    let mut shape = Shape {
        polygons: Vec::with_capacity(num_polygons as usize),
        samples: Vec::new(),
        debug: None,
    };
    let coord_max = read_vec(source);
    let coord_min = read_vec(source);
    debug!("\tBound {:?} to {:?}", coord_min, coord_max);

    source.seek(Current(
        (3+1+3) * 4 + // parent offset, max radius, and parent rotation
        (1+3+9) * 8 + // physics
        0)).unwrap();

    let positions: Vec<_> = (0 .. num_positions).map(|_| {
        read_vec(source); //unknown
        let pos = [
            source.read_i8().unwrap(),
            source.read_i8().unwrap(),
            source.read_i8().unwrap(),
            1];
        let _sort_info = source.read_u32::<E>().unwrap();
        DebugPos {
            pos: pos,
        }
    }).collect();

    source.seek(Current(
        (num_normals as i64) * (4*1 + 4) // normals
        )).unwrap();

    let mut debug = if factory.is_some() {
        Some((
            Vec::with_capacity(num_polygons as usize * 6),
            Vec::new(),
         ))
    } else { None };

    debug!("\tReading {} polygons...", num_polygons);
    let mut tess = Tessellator::new();
    for _ in 0 .. num_polygons {
        let num_corners = source.read_u32::<E>().unwrap();
        assert!(3 <= num_corners && num_corners <= 4);
        source.seek(Current(4 + 4 + 4)).unwrap(); // sort info and color
        let mut d = [0i8; 7];
        for b in d.iter_mut() {
            *b = source.read_i8().unwrap();
        }
        let mut pids = [0u32; 4];
        for i in 0 .. num_corners {
            pids[i as usize] = source.read_u32::<E>().unwrap();
            let _ = source.read_u32::<E>().unwrap(); //nid
        }
        let corners = [
            positions[pids[0] as usize], positions[pids[1] as usize],
            positions[pids[2] as usize], positions[pids[3] as usize],
        ];
        let mid = [d[4] as f32, d[5] as f32, d[6] as f32];
        let normal = [d[0] as f32 / 128.0, d[1] as f32 / 128.0, d[2] as f32 / 128.0];
        let samples = tess.tessellate(&corners, [d[4], d[5], d[6]]);
        if let Some((ref mut ind, ref mut samp)) = debug {
            for p in pids[0..3].iter() {
                ind.push(*p);
            }
            if num_corners > 3  {
                ind.push(pids[2]);
                ind.push(pids[3]);
                ind.push(pids[0]);
            }
            for s in samples.iter() {
                samp.push(DebugPos {
                    pos: [s[0], s[1], s[2], 1],
                });
                let nlen = 16.0;
                samp.push(DebugPos {pos: [
                    s[0].saturating_add((normal[0] * nlen) as i8),
                    s[1].saturating_add((normal[1] * nlen) as i8),
                    s[2].saturating_add((normal[2] * nlen) as i8),
                    1,
                ]});
            }
        }
        shape.polygons.push(Polygon {
            middle: mid,
            normal: normal,
            sample_range: (shape.samples.len() as u16,
                (shape.samples.len() + samples.len()) as u16),
        });
        shape.samples.extend_from_slice(samples);
    }

    if let (Some((ind, samp)), Some(f)) = (debug, factory) {
        let (vbo, slice) = f.create_vertex_buffer_with_slice(&positions, &ind[..]);
        shape.debug = Some(DebugShape {
            bound_vb: vbo,
            bound_slice: slice,
            sample_vb: f.create_vertex_buffer(&samp),
        });
    }

    source.seek(Current(3 * (num_polygons as i64) * 4)).unwrap(); // sorted var polys

    shape
}

pub fn load_m3d<I, R, F>(source: &mut I, factory: &mut F) -> Model<R> where
    I: ReadBytesExt + Seek,
    R: gfx::Resources,
    F: gfx::traits::FactoryExt<R>,
{
    debug!("\tReading the body...");
    let mut model = Model {
        body: load_c3d(source, factory),
        shape: Shape {
            polygons: Vec::new(),
            samples: Vec::new(),
            debug: None,
        },
        color: [0, 0],
        wheels: Vec::new(),
        debris: Vec::new(),
        slots: Vec::new(),
    };
    let _bounds = read_vec(source);
    let _max_radius = source.read_u32::<E>().unwrap();
    let num_wheels = source.read_u32::<E>().unwrap();
    let num_debris = source.read_u32::<E>().unwrap();
    model.color = [source.read_u32::<E>().unwrap(), source.read_u32::<E>().unwrap()];
    model.wheels.reserve_exact(num_wheels as usize);
    model.debris.reserve_exact(num_debris as usize);

    debug!("\tReading {} wheels...", num_wheels);
    for _ in 0 .. num_wheels {
        let steer = source.read_u32::<E>().unwrap();
        let pos = [
            source.read_f64::<E>().unwrap() as f32,
            source.read_f64::<E>().unwrap() as f32,
            source.read_f64::<E>().unwrap() as f32,
        ];
        let width = source.read_u32::<E>().unwrap();
        let radius = source.read_u32::<E>().unwrap();
        let _bound_index = source.read_u32::<E>().unwrap();
        debug!("\tSteer {}, width {}, radius {}", steer, width, radius);
        model.wheels.push(Wheel {
            mesh: if steer != 0 {
                Some(load_c3d(source, factory))
            } else {None},
            steer: steer,
            pos: pos,
            width: width,
            radius: radius,
        })
    }

    debug!("\tReading {} debris...", num_debris);
    for _ in 0 .. num_debris {
        model.debris.push(Debrie {
            mesh: load_c3d(source, factory),
            shape: load_c3d_shape(source, None::<&mut F>),
        })
    }

    debug!("\tReading the physical shape...");
    model.shape = load_c3d_shape(source, Some(factory));

    let slot_mask = source.read_u32::<E>().unwrap();
    debug!("\tReading {} slot mask...", slot_mask);
    if slot_mask != 0 {
        for i in 0 .. MAX_SLOTS {
            let pos = read_vec(source);
            let angle = source.read_i32::<E>().unwrap();
            if slot_mask & (1<<i) != 0 {
                debug!("\tSlot {} at pos {:?} and angle of {}", i, pos, angle);
                model.slots.push(Slot {
                    mesh: None,
                    scale: 1.0,
                    pos: [pos[0] as f32, pos[1] as f32, pos[2] as f32],
                    angle: angle,
                });
            }
        }
    }

    model
}

pub fn convert_m3d(mut input: File, out_path: &str) {
    debug!("\tReading the body...");
    let body = RawMesh::load(&mut input, false);
    body.save_obj(File::create(format!("{}/body.obj", out_path)).unwrap()).unwrap();
    let _bounds = read_vec(&mut input);
    let _max_radius = input.read_u32::<E>().unwrap();
    let num_wheels = input.read_u32::<E>().unwrap();
    let num_debris = input.read_u32::<E>().unwrap();
    let _color = [input.read_u32::<E>().unwrap(), input.read_u32::<E>().unwrap()];

    debug!("\tReading {} wheels...", num_wheels);
    for i in 0 .. num_wheels {
        let steer = input.read_u32::<E>().unwrap();
        let _pos = [
            input.read_f64::<E>().unwrap() as f32,
            input.read_f64::<E>().unwrap() as f32,
            input.read_f64::<E>().unwrap() as f32,
        ];
        let _width = input.read_u32::<E>().unwrap();
        let _radius = input.read_u32::<E>().unwrap();
        let _bound_index = input.read_u32::<E>().unwrap();
        if steer != 0 {
            let path = format!("{}/wheel{}.obj", out_path, i);
            let wheel = RawMesh::load(&mut input, false);
            wheel.save_obj(File::create(path).unwrap()).unwrap();
        }
    }

    debug!("\tReading {} debris...", num_debris);
    for i in 0 .. num_debris {
        let path = format!("{}/debrie{}.obj", out_path, i);
        let debrie = RawMesh::load(&mut input, false);
        debrie.save_obj(File::create(path).unwrap()).unwrap();
        let _shape = RawMesh::load(&mut input, false);
    }
}
