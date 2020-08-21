mod geometry;

pub use self::geometry::{
    CollisionQuad, ColorId, DrawTriangle, Geometry, Vertex, NORMALIZER, NUM_COLOR_IDS,
};

use byteorder::{LittleEndian as E, ReadBytesExt, WriteBytesExt};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Write};

const MAX_SLOTS: usize = 3;
const MAGIC_VERSION: u32 = 8;

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

fn write_vec_i32<W: WriteBytesExt>(dest: &mut W, v: [i32; 3]) {
    dest.write_i32::<E>(v[0]).unwrap();
    dest.write_i32::<E>(v[1]).unwrap();
    dest.write_i32::<E>(v[2]).unwrap();
}

fn write_vec_i8<W: WriteBytesExt>(dest: &mut W, v: [i8; 3]) {
    dest.write_i8(v[0]).unwrap();
    dest.write_i8(v[1]).unwrap();
    dest.write_i8(v[2]).unwrap();
}

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

    fn write<W: WriteBytesExt>(&self, dest: &mut W) {
        let q = [
            self.volume,
            self.rcm[0],
            self.rcm[1],
            self.rcm[2],
            self.jacobi[0][0],
            self.jacobi[1][0],
            self.jacobi[2][0],
            self.jacobi[0][1],
            self.jacobi[1][1],
            self.jacobi[2][1],
            self.jacobi[0][2],
            self.jacobi[1][2],
            self.jacobi[2][2],
        ];
        for qel in q.iter() {
            dest.write_f64::<E>(*qel as f64).unwrap();
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

impl<M> Wheel<M> {
    pub fn map<T, F: FnMut(M) -> T>(self, fun: F) -> Wheel<T> {
        Wheel {
            mesh: self.mesh.map(fun),
            steer: self.steer,
            pos: self.pos,
            width: self.width,
            radius: self.radius,
            bound_index: self.bound_index,
        }
    }
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

    pub fn map<T, F: FnMut(M) -> T>(self, fun: F) -> Slot<T> {
        Slot {
            mesh: self.mesh.map(fun),
            scale: self.scale,
            pos: self.pos,
            angle: self.angle,
        }
    }

    fn take(&mut self) -> Self {
        Slot {
            mesh: self.mesh.take(),
            scale: self.scale,
            pos: self.pos,
            angle: self.angle,
        }
    }

    pub fn map_all<T, F: FnMut(M, u8) -> T>(
        mut slots: [Self; MAX_SLOTS],
        mut fun: F,
    ) -> [Slot<T>; MAX_SLOTS] {
        [
            slots[0].take().map(|m| fun(m, 0)),
            slots[1].take().map(|m| fun(m, 1)),
            slots[2].take().map(|m| fun(m, 2)),
        ]
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct UpperBound {
    pub dimensions: [u32; 3],
    pub radius: u32,
}

impl UpperBound {
    fn read<I: ReadBytesExt>(source: &mut I) -> Self {
        UpperBound {
            dimensions: [
                source.read_u32::<E>().unwrap(),
                source.read_u32::<E>().unwrap(),
                source.read_u32::<E>().unwrap(),
            ],
            radius: source.read_u32::<E>().unwrap(),
        }
    }

    fn write<W: WriteBytesExt>(&self, dest: &mut W) {
        dest.write_u32::<E>(self.dimensions[0]).unwrap();
        dest.write_u32::<E>(self.dimensions[1]).unwrap();
        dest.write_u32::<E>(self.dimensions[2]).unwrap();
        dest.write_u32::<E>(self.radius).unwrap();
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BodyColor {
    pub offset: u32,
    pub shift: u32,
}

impl BodyColor {
    fn read<I: ReadBytesExt>(source: &mut I) -> Self {
        BodyColor {
            offset: source.read_u32::<E>().unwrap(),
            shift: source.read_u32::<E>().unwrap(),
        }
    }

    fn write<W: WriteBytesExt>(&self, dest: &mut W) {
        dest.write_u32::<E>(self.offset).unwrap();
        dest.write_u32::<E>(self.shift).unwrap();
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Model<M, S> {
    pub body: M,
    pub shape: S,
    pub bound: UpperBound,
    pub color: BodyColor,
    pub wheels: Vec<Wheel<M>>,
    pub debris: Vec<Debrie<M, S>>,
    pub slots: [Slot<M>; MAX_SLOTS],
}

impl<M, S> Model<M, S> {
    pub fn mesh_count(&self) -> usize {
        1 + self.wheels.len() + self.debris.len() + MAX_SLOTS
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bounds {
    pub coord_min: [i32; 3],
    pub coord_max: [i32; 3],
}

impl Bounds {
    fn read<I: ReadBytesExt>(source: &mut I) -> Self {
        Bounds {
            coord_max: read_vec_i32(source),
            coord_min: read_vec_i32(source),
        }
    }

    fn write<W: WriteBytesExt>(&self, dest: &mut W) {
        write_vec_i32(dest, self.coord_max);
        write_vec_i32(dest, self.coord_min);
    }
}

pub trait Polygon: Sized {
    fn new(middle: [i8; 3], flat_normal: [i8; 3], material: [u32; 2], vertices: &[Vertex]) -> Self;
    fn dump(&self, vertices: &mut Vec<Vertex>) -> ([i8; 3], [i8; 3], [u32; 2]);
    fn num_vertices() -> u32;
}
impl Polygon for DrawTriangle {
    fn new(_middle: [i8; 3], flat_normal: [i8; 3], material: [u32; 2], v: &[Vertex]) -> Self {
        assert_eq!(v.len(), 3);
        DrawTriangle {
            vertices: [v[0], v[1], v[2]],
            flat_normal,
            material,
        }
    }
    fn dump(&self, vertices: &mut Vec<Vertex>) -> ([i8; 3], [i8; 3], [u32; 2]) {
        vertices.extend_from_slice(&self.vertices);
        ([0; 3], self.flat_normal, self.material)
    }
    fn num_vertices() -> u32 {
        3
    }
}
impl Polygon for CollisionQuad {
    fn new(middle: [i8; 3], flat_normal: [i8; 3], _material: [u32; 2], v: &[Vertex]) -> Self {
        assert_eq!(v.len(), 4);
        CollisionQuad {
            vertices: [v[0].pos, v[1].pos, v[2].pos, v[3].pos],
            middle,
            flat_normal,
        }
    }
    fn dump(&self, vertices: &mut Vec<Vertex>) -> ([i8; 3], [i8; 3], [u32; 2]) {
        vertices.extend(self.vertices.iter().map(|&pos| Vertex { pos, normal: 0 }));
        (self.middle, self.flat_normal, [0; 2])
    }
    fn num_vertices() -> u32 {
        4
    }
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

impl<G> Mesh<G> {
    pub fn map<T, F: FnOnce(G) -> T>(self, fun: F) -> Mesh<T> {
        Mesh {
            geometry: fun(self.geometry),
            bounds: self.bounds,
            parent_off: self.parent_off,
            parent_rot: self.parent_rot,
            max_radius: self.max_radius,
            physics: self.physics,
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
        log::debug!(
            "\tBounds {:?} with offset {:?}",
            result.bounds,
            result.parent_off
        );

        log::debug!("\tReading {} positions...", num_positions);
        for _ in 0..num_positions {
            read_vec_i32(source); //unknown
            let pos = read_vec_i8(source);
            let _sort_info = source.read_u32::<E>().unwrap();
            result.geometry.positions.push(pos);
        }

        log::debug!("\tReading {} normals...", num_normals);
        for _ in 0..num_normals {
            let norm = read_vec_i8(source);
            let _something = source.read_i8().unwrap();
            let _sort_info = source.read_u32::<E>().unwrap();
            result.geometry.normals.push(norm);
        }

        log::debug!("\tReading {} polygons...", num_polygons);
        let mut vertices = Vec::with_capacity(4);
        for _ in 0..num_polygons {
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
            for _ in 0..num_corners {
                vertices.push(Vertex {
                    pos: source.read_u32::<E>().unwrap() as u16,
                    normal: source.read_u32::<E>().unwrap() as u16,
                });
            }

            result
                .geometry
                .polygons
                .push(P::new(middle, flat_normal, material, &vertices));
        }

        // sorted variable polygons
        for _ in 0..3 {
            for _ in 0..num_polygons {
                let _poly_ind = source.read_u32::<E>().unwrap();
            }
        }

        result
    }

    pub fn save<W: Write>(&self, dest: &mut W) {
        dest.write_u32::<E>(MAGIC_VERSION).unwrap();
        dest.write_u32::<E>(self.geometry.positions.len() as u32)
            .unwrap();
        dest.write_u32::<E>(self.geometry.normals.len() as u32)
            .unwrap();
        dest.write_u32::<E>(self.geometry.polygons.len() as u32)
            .unwrap();
        let total_verts = self.geometry.polygons.len() as u32 * P::num_vertices();
        dest.write_u32::<E>(total_verts).unwrap();

        self.bounds.write(dest);
        write_vec_i32(dest, self.parent_off);
        dest.write_u32::<E>(self.max_radius).unwrap();
        write_vec_i32(dest, self.parent_rot);
        self.physics.write(dest);

        for p in &self.geometry.positions {
            write_vec_i32(dest, [p[0] as i32, p[1] as i32, p[2] as i32]);
            write_vec_i8(dest, *p);
            let sort_info = 0;
            dest.write_u32::<E>(sort_info).unwrap();
        }

        for n in &self.geometry.normals {
            write_vec_i8(dest, *n);
            dest.write_i8(0).unwrap();
            let sort_info = 0;
            dest.write_u32::<E>(sort_info).unwrap();
        }

        let mut vertices = Vec::new();
        for poly in &self.geometry.polygons {
            let (middle, flat_normal, materials) = poly.dump(&mut vertices);
            dest.write_u32::<E>(vertices.len() as u32).unwrap();
            let sort_info = 0;
            dest.write_u32::<E>(sort_info).unwrap();

            for m in &materials {
                dest.write_u32::<E>(*m).unwrap();
            }
            write_vec_i8(dest, flat_normal);
            let something = 0;
            dest.write_i8(something).unwrap();
            write_vec_i8(dest, middle);

            for v in vertices.drain(..) {
                dest.write_u32::<E>(v.pos as u32).unwrap();
                dest.write_u32::<E>(v.normal as u32).unwrap();
            }
        }

        for _ in 0..3 {
            for _ in 0..self.geometry.polygons.len() {
                let poly_ind = 0; //TODO?
                dest.write_u32::<E>(poly_ind).unwrap();
            }
        }
    }
}

pub type DrawMesh = Mesh<Geometry<DrawTriangle>>;
pub type CollisionMesh = Mesh<Geometry<CollisionQuad>>;

#[derive(Serialize, Deserialize)]
pub struct AnimatedMesh<G> {
    pub meshes: Vec<Mesh<G>>,
    pub bound: UpperBound,
    pub color: BodyColor,
}

impl<P: Polygon> AnimatedMesh<Geometry<P>> {
    pub fn load(mut input: File) -> Self {
        let count = input.read_u32::<E>().unwrap();
        AnimatedMesh {
            bound: UpperBound::read(&mut input),
            color: BodyColor::read(&mut input),
            meshes: (0..count).map(|_| Mesh::load(&mut input)).collect(),
        }
    }

    pub fn save(&self, mut output: File) {
        output.write_u32::<E>(self.meshes.len() as u32).unwrap();
        self.bound.write(&mut output);
        self.color.write(&mut output);
        for mesh in self.meshes.iter() {
            mesh.save(&mut output);
        }
    }
}

pub type FullModel = Model<DrawMesh, CollisionMesh>;

impl FullModel {
    pub fn load(mut input: File) -> Self {
        log::debug!("\tReading the body...");
        let body: DrawMesh = Mesh::load(&mut input);

        let bound = UpperBound::read(&mut input);
        let num_wheels = input.read_u32::<E>().unwrap();
        let num_debris = input.read_u32::<E>().unwrap();
        let color = BodyColor::read(&mut input);

        let mut wheels = Vec::with_capacity(num_wheels as usize);
        log::debug!("\tReading {} wheels...", num_wheels);
        for _ in 0..num_wheels {
            let steer = input.read_u32::<E>().unwrap();
            let pos = [
                input.read_f64::<E>().unwrap() as f32,
                input.read_f64::<E>().unwrap() as f32,
                input.read_f64::<E>().unwrap() as f32,
            ];
            let width = input.read_u32::<E>().unwrap();
            let radius = input.read_u32::<E>().unwrap();
            let bound_index = input.read_u32::<E>().unwrap();
            let mesh: Option<DrawMesh> = if steer != 0 {
                Some(Mesh::load(&mut input))
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
        log::debug!("\tReading {} debris...", num_debris);
        for _ in 0..num_debris {
            debris.push(Debrie {
                mesh: Mesh::load(&mut input),
                shape: Mesh::load(&mut input),
            });
        }

        log::debug!("\tReading the shape...");
        let shape: CollisionMesh = Mesh::load(&mut input);

        let mut slots = [Slot::EMPTY, Slot::EMPTY, Slot::EMPTY];
        let slot_mask = input.read_u32::<E>().unwrap();
        log::debug!("\tReading {} slot mask...", slot_mask);
        for slot in &mut slots {
            for p in &mut slot.pos {
                *p = input.read_i32::<E>().unwrap();
            }
            slot.angle = input.read_i32::<E>().unwrap();
            slot.scale = 1.0;
        }

        FullModel {
            body,
            shape,
            bound,
            color,
            wheels,
            debris,
            slots,
        }
    }

    pub fn save(&self, mut output: File) {
        self.body.save(&mut output);
        self.bound.write(&mut output);
        output.write_u32::<E>(self.wheels.len() as u32).unwrap();
        output.write_u32::<E>(self.debris.len() as u32).unwrap();
        self.color.write(&mut output);

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
