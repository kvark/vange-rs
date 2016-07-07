use std::io::{Read, Seek};
use byteorder::{LittleEndian as E, ReadBytesExt};
use gfx;
use gfx::format::I8Norm;
use render::{ObjectVertex, NUM_COLOR_IDS, COLOR_ID_BODY};


const SCALE: f32 = 1.0 / 4.0;

pub struct Mesh<R: gfx::Resources> {
    pub slice: gfx::Slice<R>,
    pub buffer: gfx::handle::Buffer<R, ObjectVertex>,
    pub offset: [f32; 3],
}

pub struct Polygon {
    pub middle: [f32; 3],
    pub normal: [f32; 3],
}

pub type Shape = Vec<Polygon>;

pub struct Wheel<R: gfx::Resources> {
    pub mesh: Option<Mesh<R>>,
    pub steer: u32,
    pub width: f32,
    pub radius: f32,
}

pub struct Debrie<R: gfx::Resources> {
    pub mesh: Mesh<R>,
    pub shape: Shape,
}

pub struct Model<R: gfx::Resources> {
    pub body: Mesh<R>,
    pub shape: Shape,
    pub color: [u32; 2],
    pub wheels: Vec<Wheel<R>>,
    pub debris: Vec<Debrie<R>>,
}

fn read_vec<I: ReadBytesExt>(source: &mut I) -> [f32; 3] {
    [
        source.read_i32::<E>().unwrap() as f32 * SCALE,
        source.read_i32::<E>().unwrap() as f32 * SCALE,
        source.read_i32::<E>().unwrap() as f32 * SCALE,
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
    info!("\tBound {:?} to {:?} with offset {:?}", coord_min, coord_max, parent_off);
    let _max_radius = source.read_u32::<E>().unwrap();
    let _parent_rot = [source.read_u32::<E>().unwrap(), source.read_u32::<E>().unwrap(), source.read_u32::<E>().unwrap()];
    for _ in 0 .. (1+3+9) {
        source.read_f64::<E>().unwrap();
    }

    info!("\tReading {} positions...", num_positions);
    let mut positions = Vec::with_capacity(num_positions as usize);
    for _ in 0 .. num_positions {
        read_vec(source); //unknown
        let pos = [
            source.read_i8().unwrap(),
            source.read_i8().unwrap(),
            source.read_i8().unwrap(),
        ];
        let _sort_info = source.read_u32::<E>().unwrap();
        positions.push(pos);
    }

    info!("\tReading {} normals...", num_normals);
    let mut normals = Vec::with_capacity(num_normals as usize);
    for _ in 0 .. num_normals {
        let mut norm = [0u8; 4];
        source.read_exact(&mut norm).unwrap();
        let _sort_info = source.read_u32::<E>().unwrap();
        normals.push(norm);
    }

    info!("\tReading {} polygons...", num_polygons);
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

    let convert = |(p, n, c): ([i8; 3], [u8; 4], [u32; 2])| ObjectVertex {
        pos: [
            coord_min[0] + (p[0] as f32 + 128.0) * (coord_max[0] - coord_min[0]) / 255.0,
            coord_min[1] + (p[1] as f32 + 128.0) * (coord_max[1] - coord_min[1]) / 255.0,
            coord_min[2] + (p[2] as f32 + 128.0) * (coord_max[2] - coord_min[2]) / 255.0,
            1.0],
        color: if c[0] < NUM_COLOR_IDS { c[0] } else { COLOR_ID_BODY },
        normal: [
            I8Norm(n[0] as i8), I8Norm(n[1] as i8),
            I8Norm(n[2] as i8), I8Norm(n[3] as i8),
            ],
    };
    let do_compact = false;

    let mut gpu_verts = Vec::new();
    let (vbuf, slice) = if do_compact {
        info!("\tCompacting...");
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

    info!("\tGot {} GPU vertices...", gpu_verts.len());
    Mesh {
        slice: slice,
        buffer: vbuf,
        offset: parent_off,
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

    let coord_max = read_vec(source);
    let coord_min = read_vec(source);
    info!("\tBound {:?} to {:?}", coord_min, coord_max);

    source.seek(Current(
        (3+1+3) * 4 + // parent offset, max radius, and parent rotation
        (1+3+9) * 8 + // ?
        (num_positions as i64) * (3*4 + 3*1 + 4) + // positions
        (num_normals as i64) * (4*1 + 4) + // normals
        0)).unwrap();

    info!("\tReading {} polygons...", num_polygons);
    let polygons = (0 .. num_polygons).map(|_| {
        let num_corners = source.read_u32::<E>().unwrap();
        assert!(3 <= num_corners && num_corners <= 4);
        source.seek(Current(4 + 4 + 4)).unwrap(); // sort info and color
        let mut d = [0i8; 7];
        for b in d.iter_mut() {
            *b = source.read_i8().unwrap();
        }
        source.seek(Current((num_corners as i64) * (4 + 4))).unwrap(); // vertices
        Polygon {
            middle: [
                coord_min[0] + (d[4] as f32 + 128.0) * (coord_max[0] - coord_min[0]) / 255.0,
                coord_min[1] + (d[5] as f32 + 128.0) * (coord_max[1] - coord_min[1]) / 255.0,
                coord_min[2] + (d[6] as f32 + 128.0) * (coord_max[2] - coord_min[2]) / 255.0,
            ],
            normal: [d[0] as f32 / 128.0, d[1] as f32 / 128.0, d[2] as f32 / 128.0],
        }
    }).collect();

    source.seek(Current(3 * (num_polygons as i64) * 4)).unwrap(); // sorted var polys

    polygons
}

pub fn load_m3d<I, R, F>(source: &mut I, factory: &mut F) -> Model<R> where
    I: ReadBytesExt + Seek,
    R: gfx::Resources,
    F: gfx::traits::FactoryExt<R>,
{
    info!("\tReading the body...");
    let mut model = Model {
        body: load_c3d(source, factory),
        shape: Vec::new(),
        color: [0, 0],
        wheels: Vec::new(),
        debris: Vec::new(),
    };
    let _bounds = [
        source.read_u32::<E>().unwrap(),
        source.read_u32::<E>().unwrap(),
        source.read_u32::<E>().unwrap(),
    ];
    let _max_radius = source.read_u32::<E>().unwrap();
    let num_wheels = source.read_u32::<E>().unwrap();
    let num_debris = source.read_u32::<E>().unwrap();
    model.color = [source.read_u32::<E>().unwrap(), source.read_u32::<E>().unwrap()];
    model.wheels.reserve_exact(num_wheels as usize);
    model.debris.reserve_exact(num_debris as usize);

    info!("\tReading {} wheels...", num_wheels);
    for _ in 0 .. num_wheels {
        let steer = source.read_u32::<E>().unwrap();
        for _ in 0..3 {
            source.read_f64::<E>().unwrap();
        }
        let width = source.read_u32::<E>().unwrap() as f32 * SCALE;
        let radius = source.read_u32::<E>().unwrap() as f32 * SCALE;
        let _bound_index = source.read_u32::<E>().unwrap();
        info!("\tSteer {}, width {}, radius {}", steer, width, radius);
        model.wheels.push(Wheel {
            mesh: if steer != 0 {
                Some(load_c3d(source, factory))
            } else {None},
            steer: steer,
            width: width,
            radius: radius,
        })
    }

    info!("\tReading {} debris...", num_debris);
    for _ in 0 .. num_debris {
        model.debris.push(Debrie {
            mesh: load_c3d(source, factory),
            shape: load_c3d_shape(source),
        })
    }

    info!("\tReading the physical shape...");
    model.shape = load_c3d_shape(source);
    model
}
