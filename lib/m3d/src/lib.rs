extern crate byteorder;
#[macro_use]
extern crate log;
#[cfg(feature = "obj")]
extern crate obj;
extern crate ron;
extern crate serde;
#[macro_use]
extern crate serde_derive;

mod geometry;

pub use self::geometry::{CollisionQuad, DrawTriangle, Geometry, Vertex, NORMALIZER};

use byteorder::{LittleEndian as E, ReadBytesExt, WriteBytesExt};
#[cfg(feature = "obj")]
use std::fs::File;
use std::io::Write;
#[cfg(feature = "obj")]
use std::path::PathBuf;


const MAX_SLOTS: usize = 3;
const MAGIC_VERSION: u32 = 8;

#[derive(Clone, Serialize, Deserialize)]
pub struct Physics {
    pub volume: f32,
    pub rcm: [f32; 3],
    pub jacobi: [[f32; 3]; 3], // column-major
}

impl Physics {
    fn load<I: ReadBytesExt>(source: &mut I) -> Self {
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
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Wheel<M> {
    pub mesh: Option<M>,
    pub steer: u32,
    pub pos: [f32; 3],
    pub width: u32,
    pub radius: u32,
    pub bound_index: u32,
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
    pub const EMPTY: Self = Slot {
        mesh: None,
        scale: 0.0,
        pos: [0; 3],
        angle: 0,
    };
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Model<M, S> {
    pub body: M,
    pub shape: S,
    pub dimensions: [u32; 3],
    pub max_radius: u32,
    pub color: [u32; 2],
    pub wheels: Vec<Wheel<M>>,
    pub debris: Vec<Debrie<M, S>>,
    pub slots: [Slot<M>; MAX_SLOTS],
}


#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bounds {
    pub coord_min: [i32; 3],
    pub coord_max: [i32; 3],
}

impl Bounds {
    pub fn read<I: ReadBytesExt>(source: &mut I) -> Self {
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

fn read_vec_i32<I: ReadBytesExt>(source: &mut I) -> [i32; 3] {
    [
        source.read_i32::<E>().unwrap(),
        source.read_i32::<E>().unwrap(),
        source.read_i32::<E>().unwrap(),
    ]
}

fn read_vec_i8<I: ReadBytesExt>(source: &mut I) -> [i8; 3] {
    [
        source.read_i8().unwrap(),
        source.read_i8().unwrap(),
        source.read_i8().unwrap(),
    ]
}


#[derive(Serialize, Deserialize)]
pub struct Mesh<G> {
    pub geometry: G,
    pub bounds: Bounds,
    pub parent_off: [i32; 3],
    pub parent_rot: [i32; 3],
    pub max_radius: u32,
    pub physics: Physics,
}

#[cfg(feature = "obj")]
impl<G> Mesh<G> {
    fn with_geometry<T>(self, geometry: T) -> Mesh<T> {
        Mesh {
            geometry,
            bounds: self.bounds,
            parent_off: self.parent_off,
            parent_rot: self.parent_rot,
            max_radius: self.max_radius,
            physics: self.physics,
        }
    }
}

pub trait Polygon: Sized {
    fn new(
        middle: [i8; 3], flat_normal: [i8; 3], material: [u32; 2], vertices: &[Vertex]
    ) -> Self;
}
impl Polygon for DrawTriangle {
    fn new(
        _middle: [i8; 3], flat_normal: [i8; 3], material: [u32; 2], v: &[Vertex]
    ) -> Self {
        assert_eq!(v.len(), 3);
        DrawTriangle {
            vertices: [v[0], v[1], v[2]],
            flat_normal,
            material,
        }
    }
}
impl Polygon for CollisionQuad {
    fn new(
        middle: [i8; 3], flat_normal: [i8; 3], _material: [u32; 2], v: &[Vertex]
    ) -> Self {
        assert_eq!(v.len(), 4);
        CollisionQuad {
            vertices: [v[0].pos, v[1].pos, v[2].pos, v[3].pos],
            middle,
            flat_normal,
        }
    }
}

impl<P: Polygon> Mesh<Geometry<P>> {
    pub fn load<I: ReadBytesExt>(source: &mut I) -> Self {
        let version = source.read_u32::<E>().unwrap();
        assert_eq!(version, MAGIC_VERSION);
        let num_positions = source.read_u32::<E>().unwrap();
        let num_normals = source.read_u32::<E>().unwrap();
        let num_polygons = source.read_u32::<E>().unwrap();
        let _total_verts = source.read_u32::<E>().unwrap();

        let mut result = Mesh {
            geometry: Geometry {
                positions: Vec::with_capacity(num_positions as usize),
                normals: Vec::with_capacity(num_normals as usize),
                polygons: Vec::with_capacity(num_polygons as usize),
            },
            bounds: Bounds::read(source),
            parent_off: read_vec_i32(source),
            max_radius: source.read_u32::<E>().unwrap(),
            parent_rot: read_vec_i32(source),
            physics: Physics::load(source),
        };
        debug!(
            "\tBounds {:?} with offset {:?}",
            result.bounds, result.parent_off
        );

        debug!("\tReading {} positions...", num_positions);
        for _ in 0 .. num_positions {
            read_vec_i32(source); //unknown
            let pos = read_vec_i8(source);
            let _sort_info = source.read_u32::<E>().unwrap();
            result.geometry.positions.push(pos);
        }

        debug!("\tReading {} normals...", num_normals);
        for _ in 0 .. num_normals {
            let norm = read_vec_i8(source);
            let _something = source.read_i8().unwrap();
            let _sort_info = source.read_u32::<E>().unwrap();
            result.geometry.normals.push(norm);
        }

        debug!("\tReading {} polygons...", num_polygons);
        let mut vertices = Vec::with_capacity(4);
        for _ in 0 .. num_polygons {
            let num_corners = source.read_u32::<E>().unwrap();
            let _sort_info = source.read_u32::<E>().unwrap();
            let material = [
                source.read_u32::<E>().unwrap(),
                source.read_u32::<E>().unwrap(),
            ];
            let flat_normal = read_vec_i8(source);
            let _something = source.read_i8().unwrap();
            let middle = read_vec_i8(source);

            vertices.clear();
            for _ in 0 .. num_corners {
                vertices.push(Vertex {
                    pos: source.read_u32::<E>().unwrap() as u16,
                    normal: source.read_u32::<E>().unwrap() as u16,
                });
            }

            result.geometry.polygons.push(
                P::new(middle, flat_normal, material, &vertices),
            );
        }

        // sorted variable polygons
        for _ in 0 .. 3 {
            for _ in 0 .. num_polygons {
                let _poly_ind = source.read_u32::<E>().unwrap();
            }
        }

        result
    }

    pub fn save<W: Write>(&self, dest: &mut W) {
        dest.write_u32::<E>(MAGIC_VERSION).unwrap();
        /*
        let num_positions = dest.write_u32::<E>().unwrap();
        let num_normals = dest.write_u32::<E>().unwrap();
        let num_polygons = dest.write_u32::<E>().unwrap();
        let _total_verts = dest.write_u32::<E>().unwrap();

        let mut result = Mesh {
            geometry: Geometry::default(),
            bounds: Bounds::read(source),
            parent_off: read_vec(source),
            max_radius: dest.write_u32::<E>().unwrap() as f32,
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
            let _sort_info = dest.write_u32::<E>().unwrap();
            positions.push(pos);
        }

        debug!("\tReading {} normals...", num_normals);
        let mut normals = Vec::with_capacity(num_normals as usize);
        for _ in 0 .. num_normals {
            let mut norm = [0u8; 4];
            source.read_exact(&mut norm).unwrap();
            let _sort_info = dest.write_u32::<E>().unwrap();
            normals.push(norm);
        }

        debug!("\tReading {} polygons...", num_polygons);
        let mut vertices = Vec::with_capacity(num_polygons as usize * 3);
        for i in 0 .. num_polygons {
            let num_corners = dest.write_u32::<E>().unwrap();
            assert!(num_corners == 3 || num_corners == 4);
            let _sort_info = dest.write_u32::<E>().unwrap();
            let color = [
                dest.write_u32::<E>().unwrap(),
                dest.write_u32::<E>().unwrap(),
            ];
            let mut flat_normal = [0; 4];
            source.read_exact(&mut flat_normal).unwrap();
            let mut middle = [0; 3];
            source.read_exact(&mut middle).unwrap();
            for k in 0 .. num_corners {
                let pid = dest.write_u32::<E>().unwrap();
                let nid = dest.write_u32::<E>().unwrap();
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
                let _poly_ind = dest.write_u32::<E>().unwrap();
            }
        }*/
        unimplemented!()
    }
}


pub type DrawMesh = Mesh<Geometry<DrawTriangle>>;
pub type CollisionMesh = Mesh<Geometry<CollisionQuad>>;

#[cfg(feature = "obj")]
pub type FullModel = Model<DrawMesh, CollisionMesh>;

#[cfg(feature = "obj")]
type RefModel = Model<Mesh<String>, Mesh<String>>;

#[cfg(feature = "obj")]
const MODEL_PATH: &str = "model.ron";

#[cfg(feature = "obj")]
pub fn convert_m3d(
    mut input: File,
    out_path: &PathBuf,
) {
    use ron;
    const BODY_PATH: &str = "body.obj";
    const SHAPE_PATH: &str = "body-shape.obj";

    if !out_path.is_dir() {
        panic!("The output path must be an existing directory!");
    }

    debug!("\tReading the body...");
    let body: DrawMesh = Mesh::load(&mut input);
    body.geometry.save_obj(out_path.join(BODY_PATH))
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
            let wheel: DrawMesh = Mesh::load(&mut input);
            wheel.geometry.save_obj(out_path.join(&name)).unwrap();
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
        let debrie: DrawMesh = Mesh::load(&mut input);
        debrie.geometry.save_obj(out_path.join(&name)).unwrap();
        let shape_name = format!("debrie{}-shape.obj", i);
        let shape: CollisionMesh = Mesh::load(&mut input);
        shape.geometry.save_obj(out_path.join(&shape_name)).unwrap();
        debris.push(Debrie {
            mesh: debrie.with_geometry(name),
            shape: shape.with_geometry(shape_name),
        });
    }

    debug!("\tReading the shape...");
    let shape: CollisionMesh = Mesh::load(&mut input);
    shape.geometry.save_obj(out_path.join(SHAPE_PATH))
        .unwrap();

    let mut slots = [Slot::EMPTY, Slot::EMPTY, Slot::EMPTY];
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



#[cfg(feature = "obj")]
impl Mesh<String> {
    fn resolve<P>(&self, source_dir: &PathBuf) -> Mesh<Geometry<P>> {
        Mesh {
            geometry: Geometry::load_obj(source_dir.join(&self.geometry)),
            bounds: self.bounds.clone(),
            parent_off: self.parent_off,
            parent_rot: self.parent_rot,
            max_radius: self.max_radius,
            physics: self.physics.clone(),
        }
    }
}

#[cfg(feature = "obj")]
impl Slot<Mesh<String>> {
    fn resolve<P>(&self, source_dir: &PathBuf) -> Slot<Mesh<Geometry<P>>> {
        Slot {
            mesh: self.mesh.as_ref().map(|m| m.resolve(source_dir)),
            scale: self.scale,
            pos: self.pos,
            angle: self.angle,
        }
    }
}

#[cfg(feature = "obj")]
impl FullModel {
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
