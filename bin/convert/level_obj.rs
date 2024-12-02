use std::{
    fs::File,
    io::{BufWriter, Write as _},
    ops,
    path::Path,
};
use vangers::level::{Level, Texel};

const EXTREME_HEIGHT: i32 = i32::max_value();

#[derive(Debug)]
pub struct Config<'a> {
    pub xr: ops::Range<i32>,
    pub yr: ops::Range<i32>,
    pub palette: Option<&'a [u8]>,
}

#[derive(Clone, Default)]
struct Quad {
    z: i32,
    indices: [u32; 4],
}

impl Quad {
    fn flip(mut self) -> Self {
        self.indices.reverse();
        self
    }
}

#[derive(Clone, Default)]
struct FaceColumn {
    low: Quad,
    mid: Quad,
    high: Quad,
}

impl FaceColumn {
    fn from_quad(quad: Quad) -> Self {
        Self {
            low: quad.clone(),
            mid: quad.clone(),
            high: quad,
        }
    }

    fn contains(&self, z: i32) -> bool {
        z < self.low.z || (z >= self.mid.z && z < self.high.z)
    }
}

#[derive(Clone)]
struct Spot<T> {
    height: i32,
    payload: T,
}
impl<T: Default> Default for Spot<T> {
    fn default() -> Self {
        Self {
            height: EXTREME_HEIGHT,
            payload: T::default(),
        }
    }
}

#[derive(Clone, Default)]
struct Column<T> {
    spots: [Spot<T>; 24],
}
impl<T> Column<T> {
    fn add(&mut self, height: i32) -> (&mut T, bool) {
        let order_index = self.spots.iter().position(|v| v.height >= height).unwrap();

        if height < self.spots[order_index].height {
            self.spots[order_index..].rotate_right(1);
        }

        let v_new = &mut self.spots[order_index];
        let is_new = v_new.height == EXTREME_HEIGHT;
        if is_new {
            v_new.height = height;
        }
        (&mut v_new.payload, is_new)
    }

    fn check(&self) {
        let mut height = 0;
        for v in self.spots.iter() {
            assert!(v.height >= height);
            height = v.height;
        }
    }
}

type VertexColumn = Column<u32>;

struct VertexCollector<'p> {
    final_vertices: Vec<[i32; 3]>,
    vertex_columns: Vec<Vec<VertexColumn>>,
    face_columns: Vec<Vec<FaceColumn>>,
    config: &'p Config<'p>,
    initial_vertices: usize,
    initial_quads: usize,
}

impl VertexCollector<'_> {
    fn add(&mut self, x: i32, y: i32, z: i32) -> u32 {
        let (payload, is_new) = self.vertex_columns[(y - self.config.yr.start) as usize]
            [(x - self.config.xr.start) as usize]
            .add(z);
        if is_new {
            *payload = self.final_vertices.len() as u32;
            self.final_vertices.push([x, y, z]);
        }
        *payload
    }

    fn add_quad_custom(
        &mut self,
        expected_vertices: usize,
        x: ops::Range<i32>,
        y: ops::Range<i32>,
        z: i32,
    ) -> Quad {
        self.initial_vertices += expected_vertices;
        Quad {
            z,
            indices: [
                self.add(x.start, y.start, z),
                self.add(x.end, y.start, z),
                self.add(x.end, y.end, z),
                self.add(x.start, y.end, z),
            ],
        }
    }

    fn add_quad(&mut self, x: i32, y: i32, z: i32) -> Quad {
        self.add_quad_custom(4, x..x + 1, y..y + 1, z)
    }

    fn check(&self) {
        for col_line in self.vertex_columns.iter() {
            for column in col_line.iter() {
                column.check();
            }
        }
    }
}

#[derive(Default)]
struct Group {
    quads: Vec<[u32; 4]>,
    tris: Vec<[u32; 3]>,
}

type ConnectionColumn = Column<u32>;
impl ConnectionColumn {
    fn add_faces(&mut self, fc: &FaceColumn, value: u32) {
        for z in [fc.low.z, fc.mid.z, fc.high.z] {
            let (payload, _) = self.add(z);
            *payload |= value;
        }
    }
}

pub fn save(path: &Path, level: &Level, config: &Config) {
    if let Some(palette) = config.palette {
        let mat_path = path.with_extension("mtl");
        let mut dest = BufWriter::new(File::create(&mat_path).unwrap());
        for (i, color) in palette.chunks(3).enumerate() {
            writeln!(dest, "newmtl t{}", i).unwrap();
            writeln!(
                dest,
                "\tKd {} {} {}",
                color[0] as f32 / 255.0,
                color[1] as f32 / 255.0,
                color[2] as f32 / 255.0,
            )
            .unwrap();
        }
    }

    let mut groups: [Group; 16] = Default::default();
    let mut bar = progress::Bar::new();

    let x_total = config.xr.end - config.xr.start;
    let y_total = config.yr.end - config.yr.start;
    bar.set_job_title("Processing top/bottom:");
    let mut c = VertexCollector {
        final_vertices: Vec::new(),
        vertex_columns: (config.yr.start..=config.yr.end)
            .map(|_| vec![VertexColumn::default(); x_total as usize + 1])
            .collect(),
        face_columns: (config.yr.start..config.yr.end)
            .map(|_| vec![FaceColumn::default(); x_total as usize])
            .collect(),
        config,
        initial_vertices: 0,
        initial_quads: 0,
    };

    // first, fill out the face columns
    for y in config.yr.clone() {
        for x in config.xr.clone() {
            c.face_columns[(y - config.yr.start) as usize][(x - config.xr.start) as usize] =
                match level.get((x, y)) {
                    Texel::Single(p) => FaceColumn::from_quad(c.add_quad(x, y, p.0 as i32)),
                    // Cut out unexpected/invalid cases
                    Texel::Dual { low, mid, high } if mid > high.0 => {
                        FaceColumn::from_quad(c.add_quad(x, y, low.0 as i32))
                    }
                    Texel::Dual { low, mid, high } => FaceColumn {
                        high: c.add_quad(x, y, high.0 as i32),
                        mid: c.add_quad(x, y, mid as i32),
                        low: c.add_quad(x, y, low.0 as i32),
                    },
                };
        }
        bar.reach_percent((y - config.yr.start) * 100 / y_total);
    }

    c.check();

    bar.set_job_title("Processing sides:");
    bar.reach_percent(0);

    // add the bottom
    let bottom_quad = c
        .add_quad_custom(4, config.xr.clone(), config.yr.clone(), 0)
        .flip();
    c.initial_quads += 1;
    groups[0].quads.push(bottom_quad.indices);

    for y in config.yr.clone() {
        for x in config.xr.clone() {
            let yrel = (y - config.yr.start) as usize;
            let xrel = (x - config.xr.start) as usize;
            let fc = c.face_columns[yrel][xrel].clone();

            struct Diagonal {
                corner: u32,
                x_offset: i32,
                y_offset: i32,
            }
            let diagonals = [
                Diagonal {
                    corner: 0x1,
                    x_offset: -1,
                    y_offset: -1,
                },
                Diagonal {
                    corner: 0x2,
                    x_offset: 1,
                    y_offset: -1,
                },
                Diagonal {
                    corner: 0x4,
                    x_offset: 1,
                    y_offset: 1,
                },
                Diagonal {
                    corner: 0x8,
                    x_offset: -1,
                    y_offset: 1,
                },
            ];

            struct Side {
                corners: ops::Range<u32>,
                face_column: FaceColumn,
                x_offset: ops::Range<i32>,
                y_offset: ops::Range<i32>,
            }
            let sides = [
                Side {
                    corners: 0x8..0x1,
                    face_column: if x != config.xr.start {
                        c.face_columns[yrel][xrel - 1].clone()
                    } else {
                        FaceColumn::from_quad(c.add_quad_custom(2, x..x, y..y + 1, 0))
                    },
                    x_offset: 0..0,
                    y_offset: 1..0,
                },
                Side {
                    corners: 0x2..0x4,
                    face_column: if x + 1 != config.xr.end {
                        c.face_columns[yrel][xrel + 1].clone()
                    } else {
                        FaceColumn::from_quad(c.add_quad_custom(2, x + 1..x + 1, y..y + 1, 0))
                    },
                    x_offset: 1..1,
                    y_offset: 0..1,
                },
                Side {
                    corners: 0x1..0x2,
                    face_column: if y != config.yr.start {
                        c.face_columns[yrel - 1][xrel].clone()
                    } else {
                        FaceColumn::from_quad(c.add_quad_custom(2, x..x + 1, y..y, 0))
                    },
                    x_offset: 0..1,
                    y_offset: 0..0,
                },
                Side {
                    corners: 0x4..0x8,
                    face_column: if y + 1 != config.yr.end {
                        c.face_columns[yrel + 1][xrel].clone()
                    } else {
                        FaceColumn::from_quad(c.add_quad_custom(2, x..x + 1, y + 1..y + 1, 0))
                    },
                    x_offset: 1..0,
                    y_offset: 1..1,
                },
            ];

            // Build a list of all Z levels participating in this column
            let mut column = ConnectionColumn::default();
            // Central column breaks all corners
            column.add_faces(&fc, !0);
            // Sides each break 2 corners
            for side in sides.iter() {
                column.add_faces(&side.face_column, side.corners.start | side.corners.end);
            }
            // Each diagonal breaks 1 corner
            for diagonal in diagonals.iter() {
                let dx = x + diagonal.x_offset;
                let dy = y + diagonal.y_offset;
                if dx >= config.xr.start
                    && dx < config.xr.end
                    && dy >= config.yr.start
                    && dy < config.yr.end
                {
                    let diag_column = &c.face_columns[(dy - config.yr.start) as usize]
                        [(dx - config.xr.start) as usize];
                    column.add_faces(diag_column, diagonal.corner);
                }
            }

            // Now that we know the corners triangulate the sides
            for side in sides.iter() {
                let mut base_z = 0..0;
                for next in column.spots.iter() {
                    if next.height == EXTREME_HEIGHT {
                        break;
                    }
                    if (next.payload & (side.corners.start | side.corners.end)) == 0 {
                        continue;
                    }

                    let this_inside = fc.contains(base_z.start);
                    let other_inside = side.face_column.contains(base_z.start);
                    let mat_type = if this_inside && !other_inside {
                        // first, determine the material type
                        Some(match level.get((x, y)) {
                            Texel::Single(p) => p.1,
                            Texel::Dual { low, mid, high } => {
                                if base_z.start >= mid as i32 {
                                    high.1
                                } else {
                                    low.1
                                }
                            }
                        })
                    } else {
                        None
                    };

                    // now, advance along the edges and generate side triangles
                    if (next.payload & side.corners.start) != 0 && base_z.start != next.height {
                        if let Some(mt) = mat_type {
                            groups[mt as usize].tris.push([
                                c.add(
                                    x + side.x_offset.start,
                                    y + side.y_offset.start,
                                    base_z.start,
                                ),
                                c.add(x + side.x_offset.end, y + side.y_offset.end, base_z.end),
                                c.add(
                                    x + side.x_offset.start,
                                    y + side.y_offset.start,
                                    next.height,
                                ),
                            ]);
                        }
                        base_z.start = next.height;
                    }
                    if (next.payload & side.corners.end) != 0 && base_z.end != next.height {
                        if let Some(mt) = mat_type {
                            groups[mt as usize].tris.push([
                                c.add(
                                    x + side.x_offset.start,
                                    y + side.y_offset.start,
                                    base_z.start,
                                ),
                                c.add(x + side.x_offset.end, y + side.y_offset.end, base_z.end),
                                c.add(x + side.x_offset.end, y + side.y_offset.end, next.height),
                            ]);
                        }
                        base_z.end = next.height;
                    }
                }
            }

            // Finally, generate top/down faces
            let low = match level.get((x, y)) {
                Texel::Single(p) => p,
                // Cut out unexpected/invalid cases
                Texel::Dual { low, mid, high } if mid > high.0 => low,
                Texel::Dual { low, mid, high } => {
                    let g = &mut groups[high.1 as usize].quads;
                    // top + bottom
                    g.push(fc.high.indices);
                    if mid >= low.0 {
                        g.push(fc.mid.clone().flip().indices);
                    }
                    c.initial_quads += 2;
                    low
                }
            };

            let g = &mut groups[low.1 as usize].quads;
            g.push(fc.low.indices);
            c.initial_quads += 1;
        }
        bar.reach_percent((y - config.yr.start) * 100 / y_total);
    }

    bar.jobs_done();
    let num_quads: usize = groups.iter().map(|g| g.quads.len()).sum();
    let num_tris: usize = groups.iter().map(|g| g.tris.len()).sum();
    fn unit(count: usize) -> f32 {
        count as f32 / 1_000_000.0
    }
    println!(
        "Exporting {:.1}M (of {:.1}M) vertices, {:.1}M (of {:.1}M) quads, {:.1}M tris",
        unit(c.final_vertices.len()),
        unit(c.initial_vertices),
        unit(num_quads),
        unit(c.initial_quads),
        unit(num_tris),
    );

    let mut dest = BufWriter::new(File::create(path).unwrap());
    bar.set_job_title("Vertices:");
    for v in c.final_vertices {
        writeln!(dest, "v {} {} {}", v[0], v[1], v[2]).unwrap();
    }
    bar.set_job_title("Faces:");
    writeln!(dest).unwrap();
    if config.palette.is_some() {
        let mtl_path = path.with_extension("mtl");
        let mat_name = mtl_path.file_name().unwrap().to_str().unwrap();
        writeln!(dest, "mtllib {}", mat_name).unwrap();
    }
    for (i, group) in groups.iter().enumerate() {
        writeln!(dest, "usemtl t{}", i).unwrap();
        for t in group.quads.iter() {
            writeln!(
                dest,
                "f {} {} {} {}",
                t[0] + 1,
                t[1] + 1,
                t[2] + 1,
                t[3] + 1,
            )
            .unwrap();
        }
        for t in group.tris.iter() {
            writeln!(dest, "f {} {} {}", t[0] + 1, t[1] + 1, t[2] + 1,).unwrap();
        }
        bar.reach_percent(((i + 1) * 100 / groups.len()) as i32);
    }
    bar.jobs_done();
}
