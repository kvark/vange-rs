#[cfg(feature = "obj")]
use std::io::{self, Write};
#[cfg(feature = "obj")]
use std::path::PathBuf;


pub const NORMALIZER: f32 = 124.0;

#[derive(Copy, Clone, Debug)]
pub struct Vertex {
    pub pos: u16,
    pub normal: u16,
}

impl Vertex {
    pub const DUMMY: Self = Vertex {
        pos: !0,
        normal: !0,
    };
}

pub struct DrawTriangle {
    pub vertices: [Vertex; 3],
    pub flat_normal: [i8; 3],
    pub material: [u32; 2],
}

pub struct CollisionQuad {
    pub vertices: [u16; 4],
    pub middle: [i8; 3],
    pub flat_normal: [i8; 3],
}

#[derive(Default)]
pub struct Geometry<P> {
    pub positions: Vec<[i8; 3]>,
    pub normals: Vec<[i8; 3]>,
    pub polygons: Vec<P>,
}

#[cfg(feature = "obj")]
impl<P> Geometry<P> {
    pub fn save_obj(&self, path: PathBuf) -> io::Result<()> {
        use std::fs::File;

        let mut dest = File::create(&path).unwrap();
        for p in self.positions.iter() {
            writeln!(dest, "v {} {} {}", p[0], p[1], p[2])?;
        }
        writeln!(dest, "")?;
        for n in self.normals.iter() {
            writeln!(
                dest,
                "vn {} {} {}",
                n[0] as f32 / NORMALIZER,
                n[1] as f32 / NORMALIZER,
                n[2] as f32 / NORMALIZER
            )?;
        }
        writeln!(dest, "")?;
        for i in 0 .. self.positions.len() / 3 {
            writeln!(
                dest,
                "f {} {} {}",
                i * 3 + 1,
                i * 3 + 2,
                i * 3 + 3
            )?;
        }
        Ok(())
    }

    pub fn load_obj(_path: PathBuf) -> Self {
        /*use obj::{IndexTuple, Obj, SimplePolygon};

        let obj: Obj<SimplePolygon> = Obj::load(&path).unwrap();
        assert_eq!(obj.position.len(), obj.normal.len());
        let mut vertices = Vec::new();
        for object in &obj.objects {
            for group in &object.groups {
                for poly in &group.polys {
                    for &IndexTuple(a, _b, c) in poly {
                        let p = obj.position[a];
                        let n = obj.normal[c.unwrap_or(a)];
                        vertices.push(Vertex {
                            pos: [p[0] as i8, p[1] as i8, p[2] as i8],
                            color: 0, //TODO!
                            normal: [
                                (n[0] * NORMALIZER) as i8,
                                (n[1] * NORMALIZER) as i8,
                                (n[2] * NORMALIZER) as i8,
                            ],
                        });
                    }
                }
            }
        }

        Geometry {
            vertices,
            indices: Vec::new(),
        }*/
        unimplemented!()
    }
}
