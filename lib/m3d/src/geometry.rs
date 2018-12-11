use std::io::{self, Write};
use std::path::PathBuf;

use Polygon;


pub const NORMALIZER: f32 = 124.0;
pub const NUM_COLOR_IDS: u32 = 25;

#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum ColorId {
    Reserved = 0,
    Body = 1,
    Window = 2,
    Wheel = 3,
    Defence = 4,
    Weapon = 5,
    Tube = 6,
    BodyRed = 7,
    BodyBlue = 8,
    BodyYellow = 9,
    BodyGray = 10,
    YellowCharged = 11,
    Custom0 = 12,
    Custom1 = 13,
    Custom2 = 14,
    Custom3 = 15,
    Custom4 = 16,
    Custom5 = 17,
    Custom6 = 18,
    Custom7 = 19,
    Black = 20,
    BodyGreen = 21,
    SkyFarmerKernboo = 22,
    SkyFarmerPipetka = 23,
    RottenItem = 24,
}

impl ColorId {
    pub fn new(id: u32) -> Self {
        use std::mem;
        if id < NUM_COLOR_IDS {
            unsafe { mem::transmute(id) }
        } else {
            error!("Unknown ColorId {:?}", id);
            ColorId::Reserved
        }
    }
}

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

fn flatten_normal(poly: &[obj::IndexTuple], normals: &[[f32; 3]]) -> [i8; 3] {
    let n = poly.iter().fold([0f32; 3], |u, obj::IndexTuple(_, _, ni)| {
        let n = match ni {
            Some(index) => normals[*index],
            None => [0.0, 0.0, 0.0],
        };
        [u[0] + n[0], u[1] + n[1], u[2] + n[2]]
    });
    let m2 = n.iter().fold(0f32, |u, v| u + v*v);
    let scale = if m2 == 0.0 { 0.0 } else { NORMALIZER / m2.sqrt() };
    [(n[0] * scale) as i8, (n[1] * scale) as i8, (n[2] * scale) as i8]
}

fn flatten_pos(poly: &[obj::IndexTuple], positions: &[[f32; 3]]) -> [i8; 3] {
    let m = poly.iter().fold([0f32; 3], |u, obj::IndexTuple(pi, _, _)| {
        let p = positions[*pi];
        [u[0] + p[0], u[1] + p[1], u[2] + p[2]]
    });
    [ (m[0] * 0.25) as i8, (m[1] * 0.25) as i8, (m[2] * 0.25) as i8 ]
}

impl<P: Polygon> Geometry<P> {
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

        let mut mask = 0u32;
        for p in &self.polygons {
            mask |= 1 << p.color_id();
        }
        let mut vertices = Vec::new();
        for color_id in 0 .. NUM_COLOR_IDS {
            if mask & (1 << color_id) == 0 {
                continue
            }
            writeln!(dest, "g {:?}", ColorId::new(color_id))?;
            for p in &self.polygons {
                if p.color_id() != color_id {
                    continue
                }
                write!(dest, "f")?;
                let (_, _, _) = p.dump(&mut vertices);
                for v in vertices.drain(..) {
                    write!(dest, " {}//{}", v.pos + 1, v.normal + 1)?;
                }
                writeln!(dest, "");
            }
        }
        Ok(())
    }

    pub fn load_obj(path: PathBuf) -> Self {
        use obj::{Obj, SimplePolygon};

        let obj: Obj<SimplePolygon> = Obj::load(&path).unwrap();

        let positions = obj.position
            .iter()
            .map(|p| [p[0] as i8, p[1] as i8, p[2] as i8])
            .collect();
        let normals = obj.normal
            .iter()
            .map(|n| [
                (n[0] * NORMALIZER) as i8,
                (n[1] * NORMALIZER) as i8,
                (n[2] * NORMALIZER) as i8,
            ])
            .collect();

        let color_names = (0 .. NUM_COLOR_IDS)
            .map(|id| format!("{:?}", ColorId::new(id)))
            .collect::<Vec<_>>();

        let obj::Obj { ref objects, ref position, ref normal, .. } = obj;
        let polygons = objects
            .iter()
            .flat_map(|object| {
                object.groups.iter().flat_map(|group| {
                    let mut vertices = Vec::new();
                    let color_id = color_names
                        .iter()
                        .position(|c| c == &group.name)
                        .unwrap_or(0);
                    group.polys
                        .iter()
                        .map(move |poly| {
                            vertices.clear();
                            for &obj::IndexTuple(pi, _, ni) in poly {
                                vertices.push(Vertex {
                                    pos: pi as u16,
                                    normal: ni.unwrap_or(0) as u16,
                                })
                            }
                            P::new(
                                flatten_pos(poly, position),
                                flatten_normal(poly, normal),
                                [color_id as u32, 0],
                                &vertices
                            )
                        })
                })
            })
            .collect();

        Geometry {
            positions,
            normals,
            polygons,
        }
    }
}
