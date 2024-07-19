use std::{
    fs::File,
    io::{BufWriter, Write as _},
    ops::Range,
    path::Path,
};
use vangers::level::{Level, Texel};

const EXTREME_HEIGHT: i32 = i32::max_value();

#[derive(Debug)]
pub struct Config<'a> {
    pub xr: Range<i32>,
    pub yr: Range<i32>,
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
struct Vertex {
    index: u32,
    height: i32,
}
impl Default for Vertex {
    fn default() -> Self {
        Self {
            index: !0,
            height: EXTREME_HEIGHT,
        }
    }
}

#[derive(Clone, Default)]
struct VertexColumn {
    data: [Vertex; 32],
}
impl VertexColumn {
    fn add(&mut self, height: i32) -> &mut Vertex {
        let order_index = self.data.iter().position(|v| v.height >= height).unwrap();

        if height < self.data[order_index].height {
            self.data[order_index..].rotate_right(1);
        }

        let v_new = &mut self.data[order_index];
        if v_new.height == EXTREME_HEIGHT {
            v_new.height = height;
        }
        v_new
    }

    fn add_faces(&mut self, fc: &FaceColumn) {
        for z in [fc.low.z, fc.mid.z, fc.high.z] {
            self.add(z);
        }
    }

    fn check(&self) {
        let mut height = 0;
        for v in self.data.iter() {
            assert!(v.height >= height);
            height = v.height;
        }
    }
}

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
        let vertex = self.vertex_columns[(y - self.config.yr.start) as usize]
            [(x - self.config.xr.start) as usize]
            .add(z);
        if vertex.index == !0 {
            vertex.index = self.final_vertices.len() as u32;
            self.final_vertices.push([x, y, z]);
        }
        vertex.index
    }

    fn add_quad_custom(
        &mut self,
        expected_vertices: usize,
        x: Range<i32>,
        y: Range<i32>,
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

// Vertical slicing map:
// -X: 3, 0, 0, 3
// +X: 1, 2, 2, 1
// -Y: 0, 1, 1, 0
// +Y: 2, 3, 3, 2
fn vertical_slice(a: &Quad, b: &Quad, start: usize) -> [u32; 4] {
    let i0 = a.indices[start];
    let i1 = a.indices[(start + 1) & 3];
    let i2 = b.indices[(start + 1) & 3];
    let i3 = b.indices[start];
    [i0, i1, i2, i3]
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

    let mut groups: [Vec<[u32; 4]>; 16] = Default::default();
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
    groups[0].push(bottom_quad.indices);

    for y in config.yr.clone() {
        for x in config.xr.clone() {
            let yrel = (y - config.yr.start) as usize;
            let xrel = (x - config.xr.start) as usize;
            let fc = c.face_columns[yrel][xrel].clone();

            let fx0 = if x != config.xr.start {
                c.face_columns[yrel][xrel - 1].clone()
            } else {
                FaceColumn::from_quad(c.add_quad_custom(2, x..x, y..y + 1, 0))
            };
            let fx1 = if x + 1 != config.xr.end {
                c.face_columns[yrel][xrel + 1].clone()
            } else {
                FaceColumn::from_quad(c.add_quad_custom(2, x + 1..x + 1, y..y + 1, 0))
            };
            let fy0 = if y != config.yr.start {
                c.face_columns[yrel - 1][xrel].clone()
            } else {
                FaceColumn::from_quad(c.add_quad_custom(2, x..x + 1, y..y, 0))
            };
            let fy1 = if y + 1 != config.yr.end {
                c.face_columns[yrel + 1][xrel].clone()
            } else {
                FaceColumn::from_quad(c.add_quad_custom(2, x..x + 1, y + 1..y + 1, 0))
            };

            // Build a list of all Z levels participating in this column
            let mut vc = VertexColumn::default();
            vc.add_faces(&fc);
            vc.add_faces(&fx0);
            vc.add_faces(&fx1);
            vc.add_faces(&fy0);
            vc.add_faces(&fy1);

            let mut base = c.add_quad(x, y, 0);
            for next in vc.data.iter() {
                if next.height == base.z {
                    continue;
                }
                if next.height == EXTREME_HEIGHT {
                    break;
                }
                let cur = c.add_quad(x, y, next.height);
                if fc.contains(base.z) {
                    // first, determine the material type
                    let mat_type = match level.get((x, y)) {
                        Texel::Single(p) => p.1,
                        Texel::Dual { low, mid, high } => {
                            if base.z >= mid as i32 {
                                high.1
                            } else {
                                low.1
                            }
                        }
                    };

                    // generate the side faces
                    let g = &mut groups[mat_type as usize];
                    if !fx0.contains(base.z) {
                        g.push(vertical_slice(&base, &cur, 3));
                    }
                    if !fx1.contains(base.z) {
                        g.push(vertical_slice(&base, &cur, 1));
                    }
                    if !fy0.contains(base.z) {
                        g.push(vertical_slice(&base, &cur, 0));
                    }
                    if !fy1.contains(base.z) {
                        g.push(vertical_slice(&base, &cur, 2));
                    }
                    c.initial_quads += 4;
                }
                base = cur;
            }

            let low = match level.get((x, y)) {
                Texel::Single(p) => p,
                // Cut out unexpected/invalid cases
                Texel::Dual { low, mid, high } if mid > high.0 => low,
                Texel::Dual { low, mid, high } => {
                    let g = &mut groups[high.1 as usize];
                    // top + bottom
                    g.push(fc.high.indices);
                    if mid >= low.0 {
                        g.push(fc.mid.clone().flip().indices);
                    }
                    c.initial_quads += 2;
                    low
                }
            };

            let g = &mut groups[low.1 as usize];
            g.push(fc.low.indices);
            c.initial_quads += 1;
        }
        bar.reach_percent((y - config.yr.start) * 100 / y_total);
    }

    bar.jobs_done();
    let num_quads: usize = groups
        .iter()
        .flat_map(|g| g.iter().map(|plane| plane.len()))
        .sum();
    fn unit(count: usize) -> f32 {
        count as f32 / 1_000_000.0
    }
    println!(
        "Exporting {:.1}M (of {:.1}M) vertices, {:.1}M (of {:.1}M) quads",
        unit(c.final_vertices.len()),
        unit(c.initial_vertices),
        unit(num_quads),
        unit(c.initial_quads),
    );

    let mut dest = BufWriter::new(File::create(&path).unwrap());
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
        for t in group {
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
        bar.reach_percent((i as i32 + 1) * 100 / 16);
    }
    bar.jobs_done();
}
