#[cfg(feature = "obj")]
use std::io::{self, Write};
#[cfg(feature = "obj")]
use std::path::PathBuf;


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
                v.normal[0] as f32 / 124.0,
                v.normal[1] as f32 / 124.0,
                v.normal[2] as f32 / 124.0
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
                        vertices.push(Vertex {
                            pos: [p[0] as i8, p[1] as i8, p[2] as i8],
                            color: 0, //TODO!
                            normal: [
                                (n[0] * 127.5) as i8,
                                (n[1] * 127.5) as i8,
                                (n[2] * 127.5) as i8,
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
