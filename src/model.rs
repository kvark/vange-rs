use std::io::{Seek};
use byteorder::{LittleEndian as E, ReadBytesExt};
use gfx;
use gfx::format::I8Norm;
use render::{ObjectVertex, NUM_COLOR_IDS, COLOR_ID_BODY};


const MAX_SLOTS: u32 = 3;

#[derive(Clone)]
pub struct Physics {
    pub volume: f32,
    pub rcm: [f32; 3],
    pub jacobi: [[f32; 3]; 3],
}

#[derive(Clone)]
pub struct Mesh<R: gfx::Resources> {
    pub slice: gfx::Slice<R>,
    pub buffer: gfx::handle::Buffer<R, ObjectVertex>,
    pub offset: [f32; 3],
    pub bbox: ([f32; 3], [f32; 3], f32),
    pub physics: Physics,
}

#[derive(Clone)]
pub struct Polygon {
    pub middle: [f32; 3],
    pub normal: [f32; 3],
    pub sample_range: (u16, u16),
}

#[derive(Clone)]
pub struct Shape {
    pub polygons: Vec<Polygon>,
    pub samples: Vec<[f32; 3]>,
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
    pub shape: Shape,
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
    pub shape: Shape,
    pub color: [u32; 2],
    pub wheels: Vec<Wheel<R>>,
    pub debris: Vec<Debrie<R>>,
    pub slots: Vec<Slot<R>>,
}

fn read_vec<I: ReadBytesExt>(source: &mut I) -> [f32; 3] {
    [
        source.read_i32::<E>().unwrap() as f32,
        source.read_i32::<E>().unwrap() as f32,
        source.read_i32::<E>().unwrap() as f32,
    ]
}

pub fn load_c3d<I, R, F>(source: &mut I, factory: &mut F) -> Mesh<R> where
    I: ReadBytesExt,
    R: gfx::Resources,
    F: gfx::traits::FactoryExt<R>,
{
    let version = source.read_u32::<E>().unwrap();
    assert_eq!(version, 8);
    let num_positions = source.read_u32::<E>().unwrap();
    let num_normals   = source.read_u32::<E>().unwrap();
    let num_polygons  = source.read_u32::<E>().unwrap();
    let _total_verts  = source.read_u32::<E>().unwrap();

    let coord_max = read_vec(source);
    let coord_min = read_vec(source);
    let parent_off = read_vec(source);
    debug!("\tBound {:?} to {:?} with offset {:?}", coord_min, coord_max, parent_off);
    let max_radius = source.read_u32::<E>().unwrap() as f32;
    let _parent_rot = read_vec(source);
    let physics = {
        let mut q = [0.0f32; 1+3+9];
        for qel in q.iter_mut() {
            *qel = source.read_f64::<E>().unwrap() as f32;
        }
        Physics {
            volume: q[0],
            rcm: [q[1], q[2], q[3]],
            jacobi: [
                [q[4], q[5], q[6]],
                [q[7], q[8], q[9]],
                [q[10], q[11], q[12]],
            ],
        }
    };

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
        assert_eq!(num_corners, 3);
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
    let do_compact = true;

    let mut gpu_verts = Vec::new();
    let (vbuf, slice) = if do_compact {
        debug!("\tCompacting...");
        vertices.sort_by_key(|v| v.1);
        //vertices.dedup();
        let mut indices = vec![0; vertices.len()];
        let mut last = vertices[0].1;
        last.2[0] ^= 1; //change something
        let mut v_id = 0;
        for v in vertices.into_iter() {
            if v.1 != last {
                last = v.1;
                v_id = gpu_verts.len() as u16;
                gpu_verts.push(convert(v.1));
            }
            indices[v.0 as usize] = v_id;
        }
        factory.create_vertex_buffer_with_slice(&gpu_verts, &indices[..])
    }else {
        for v in vertices.into_iter() {
            gpu_verts.push(convert(v.1));
        }
        factory.create_vertex_buffer_with_slice(&gpu_verts, ())
    };

    debug!("\tGot {} GPU vertices...", gpu_verts.len());
    Mesh {
        slice: slice,
        buffer: vbuf,
        offset: parent_off,
        bbox: (coord_min, coord_max, max_radius),
        physics: physics,
    }
}

pub fn load_c3d_shape<I>(source: &mut I) -> Shape where
    I: ReadBytesExt + Seek,
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
    };
    let coord_max = read_vec(source);
    let coord_min = read_vec(source);
    debug!("\tBound {:?} to {:?}", coord_min, coord_max);

    source.seek(Current(
        (3+1+3) * 4 + // parent offset, max radius, and parent rotation
        (1+3+9) * 8 + // ?
        (num_positions as i64) * (3*4 + 3*1 + 4) + // positions
        (num_normals as i64) * (4*1 + 4) + // normals
        0)).unwrap();

    debug!("\tReading {} polygons...", num_polygons);
    for _ in 0 .. num_polygons {
        let num_corners = source.read_u32::<E>().unwrap();
        assert!(3 <= num_corners && num_corners <= 4);
        source.seek(Current(4 + 4 + 4)).unwrap(); // sort info and color
        let mut d = [0i8; 7];
        for b in d.iter_mut() {
            *b = source.read_i8().unwrap();
        }
        source.seek(Current((num_corners as i64) * (4 + 4))).unwrap(); // vertices
        //TODO: more samples!
        let mid = [d[4] as f32, d[5] as f32, d[6] as f32];
        shape.polygons.push(Polygon {
            middle: mid,
            normal: [d[0] as f32 / 128.0, d[1] as f32 / 128.0, d[2] as f32 / 128.0],
            sample_range: (shape.samples.len() as u16, shape.samples.len() as u16 + 1),
        });
        shape.samples.push(mid);
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
            shape: load_c3d_shape(source),
        })
    }

    debug!("\tReading the physical shape...");
    model.shape = load_c3d_shape(source);

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
