use std::{
    fs::File,
    io::{BufWriter, Write as _},
    ops::Range,
    path::Path,
};
use vangers::level::{Level, Texel};

const MAX_COLUMN_VERTICES: usize = 4 * 4;
const EXTREME_HEIGHT: i32 = i32::max_value();

#[derive(Debug)]
pub struct Optimization {}

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
    data: [Vertex; MAX_COLUMN_VERTICES + 1],
}
impl VertexColumn {
    fn add(&mut self, height: i32, _optimization: &Optimization) -> &mut Vertex {
        let order_index = self.data.iter().position(|v| v.height > height).unwrap();

        if height < self.data[order_index].height {
            self.data[order_index..].rotate_right(1);
        }

        let v_new = &mut self.data[order_index];
        if v_new.height == EXTREME_HEIGHT {
            v_new.height = height;
        }
        v_new
    }
}

#[derive(Clone, Default)]
struct Quad {
    z: i32,
    indices: [u32; 4],
}
#[derive(Clone, Default)]
struct FaceColumn {
    low: Quad,
    mid: Quad,
    high: Quad,
}

enum Boundary<'a> {
    Bottom,
    Top,
    Other(&'a Quad),
}
impl Default for Boundary<'_> {
    fn default() -> Self {
        Self::Bottom
    }
}
impl Quad {
    fn as_boundary(&self) -> Boundary {
        Boundary::Other(self)
    }
}

#[derive(Default)]
struct Intersection<'a> {
    ranges: [Range<Boundary<'a>>; 2],
    count: usize,
}
impl<'a> Intersection<'a> {
    fn push(&mut self, range: Range<Boundary<'a>>) {
        self.ranges[self.count] = range;
        self.count += 1;
    }

    fn polygonize(
        self,
        mapper: impl Fn(Boundary<'a>) -> (u32, u32) + 'a,
    ) -> impl Iterator<Item = [u32; 4]> + 'a {
        self.ranges
            .into_iter()
            .take(self.count)
            .filter_map(move |br| {
                let (i0, i1) = mapper(br.start);
                let (i3, i2) = mapper(br.end);
                if i0 == !0 || i3 == !0 {
                    None
                } else {
                    Some([i0, i1, i2, i3])
                }
            })
    }
}

const SKIP: (u32, u32) = (!0, !0);

impl FaceColumn {
    fn intersect(&self, range: Range<i32>) -> Intersection {
        let mut i = Intersection::default();
        // consider the interval between the low and the middle
        if range.start < self.mid.z && range.end > self.low.z && self.low.z < self.mid.z {
            let start = if range.start < self.low.z {
                self.low.as_boundary()
            } else {
                Boundary::Bottom
            };
            let end = if range.end > self.mid.z {
                self.mid.as_boundary()
            } else {
                Boundary::Top
            };
            i.push(start..end);
        }
        // consider the interval from the high level and up
        if range.end > self.high.z {
            let start = if range.start < self.high.z {
                self.high.as_boundary()
            } else {
                Boundary::Bottom
            };
            i.push(start..Boundary::Top);
        }
        i
    }

    fn low_indices<'a>(&'a self, start: usize) -> impl Fn(Boundary) -> (u32, u32) + 'a {
        move |b| match b {
            Boundary::Bottom => SKIP,
            Boundary::Top => (self.low.indices[start], self.low.indices[(start + 1) & 3]),
            Boundary::Other(quad) => (quad.indices[(start + 3) & 3], quad.indices[(start + 2) & 3]),
        }
    }
    fn high_indices<'a>(&'a self, start: usize) -> impl Fn(Boundary) -> (u32, u32) + 'a {
        move |b| match b {
            Boundary::Bottom => (self.mid.indices[start], self.mid.indices[(start + 1) & 3]),
            Boundary::Top => (self.high.indices[start], self.high.indices[(start + 1) & 3]),
            Boundary::Other(quad) => (quad.indices[(start + 3) & 3], quad.indices[(start + 2) & 3]),
        }
    }
}

struct VertexCollector<'p> {
    final_vertices: Vec<[i32; 3]>,
    vertex_columns: Vec<Vec<VertexColumn>>,
    face_columns: Vec<Vec<FaceColumn>>,
    optimization: &'p Optimization,
    initial_vertices: usize,
    initial_quads: usize,
}

impl VertexCollector<'_> {
    fn add(&mut self, x: i32, y: i32, z: i32) -> u32 {
        let vertex = self.vertex_columns[y as usize][x as usize].add(z, &self.optimization);
        if vertex.index == !0 {
            vertex.index = self.final_vertices.len() as u32;
            self.final_vertices.push([x, y, z]);
        }
        vertex.index
    }

    fn add_quad(&mut self, x: i32, y: i32, z: i32) -> Quad {
        self.initial_vertices += 4;
        Quad {
            z,
            indices: [
                self.add(x, y, z),
                self.add(x + 1, y, z),
                self.add(x + 1, y + 1, z),
                self.add(x, y + 1, z),
            ],
        }
    }
}

pub fn save(path: &Path, level: &Level, optimization: &Optimization) {
    let mut dest = BufWriter::new(File::create(&path).unwrap());
    let mut groups: [Vec<[u32; 4]>; 16] = Default::default();
    let mut bar = progress::Bar::new();

    bar.set_job_title("Processing top/bottom:");
    let mut c = VertexCollector {
        final_vertices: Vec::new(),
        vertex_columns: (0..=level.size.1)
            .map(|_| vec![VertexColumn::default(); level.size.0 as usize + 1])
            .collect(),
        face_columns: (0..level.size.1)
            .map(|_| vec![FaceColumn::default(); level.size.0 as usize])
            .collect(),
        optimization,
        initial_vertices: 0,
        initial_quads: 0,
    };

    // first, fill out the face columns
    for y in 0..level.size.1 {
        for x in 0..level.size.0 {
            c.face_columns[y as usize][x as usize] = match level.get((x, y)) {
                Texel::Single(p) => {
                    let quad = c.add_quad(x, y, p.0 as i32);
                    FaceColumn {
                        high: quad.clone(),
                        mid: quad.clone(),
                        low: quad,
                    }
                }
                // Cut out unexpected/invalid cases
                Texel::Dual { low, mid, high } if mid > high.0 => {
                    let quad = c.add_quad(x, y, low.0 as i32);
                    FaceColumn {
                        high: quad.clone(),
                        mid: quad.clone(),
                        low: quad,
                    }
                }
                Texel::Dual { low, mid, high } => FaceColumn {
                    high: c.add_quad(x, y, high.0 as i32),
                    mid: c.add_quad(x, y, mid as i32),
                    low: c.add_quad(x, y, low.0 as i32),
                },
            };
        }
        bar.reach_percent(y * 100 / level.size.1);
    }

    bar.set_job_title("Processing sides:");
    bar.reach_percent(0);
    let dummy_quad = Quad {
        z: EXTREME_HEIGHT,
        indices: [0; 4],
    };
    let dummy_face = FaceColumn {
        low: dummy_quad.clone(),
        mid: dummy_quad.clone(),
        high: dummy_quad.clone(),
    };

    for y in 0..level.size.1 {
        for x in 0..level.size.0 {
            let fc = &c.face_columns[y as usize][x as usize];
            let fx0 = if x != 0 {
                &c.face_columns[y as usize][x as usize - 1]
            } else {
                &dummy_face
            };
            let fx1 = if x + 1 != level.size.0 {
                &c.face_columns[y as usize][x as usize + 1]
            } else {
                &dummy_face
            };
            let fy0 = if y != 0 {
                &c.face_columns[y as usize - 1][x as usize]
            } else {
                &dummy_face
            };
            let fy1 = if y + 1 != level.size.1 {
                &c.face_columns[y as usize + 1][x as usize]
            } else {
                &dummy_face
            };

            let p = match level.get((x, y)) {
                Texel::Single(p) => p,
                // Cut out unexpected/invalid cases
                Texel::Dual { low, mid, high } if mid > high.0 => low,
                Texel::Dual { low, mid, high } => {
                    let g = &mut groups[high.1 as usize];
                    // top + bottom
                    g.push(fc.high.indices);
                    if mid >= low.0 {
                        let m = &fc.mid.indices;
                        g.push([m[3], m[2], m[1], m[0]]);
                    }
                    // sides
                    g.extend(
                        fx0.intersect(mid as i32..high.0 as i32)
                            .polygonize(fc.high_indices(3)),
                    );
                    g.extend(
                        fx1.intersect(mid as i32..high.0 as i32)
                            .polygonize(fc.high_indices(1)),
                    );
                    g.extend(
                        fy0.intersect(mid as i32..high.0 as i32)
                            .polygonize(fc.high_indices(0)),
                    );
                    g.extend(
                        fy1.intersect(mid as i32..high.0 as i32)
                            .polygonize(fc.high_indices(2)),
                    );
                    // done
                    c.initial_quads += 6;
                    low
                }
            };

            let g = &mut groups[p.1 as usize];
            g.push(fc.low.indices);
            c.initial_quads += 1;
            let z = p.0 as i32;
            if z > 0 {
                g.extend(fx0.intersect(-1..z).polygonize(fc.low_indices(3)));
                g.extend(fx1.intersect(-1..z).polygonize(fc.low_indices(1)));
                g.extend(fy0.intersect(-1..z).polygonize(fc.low_indices(0)));
                g.extend(fy1.intersect(-1..z).polygonize(fc.low_indices(2)));
                c.initial_quads += 4;
            }
        }
        bar.reach_percent(y * 100 / level.size.1);
    }

    bar.jobs_done();
    let num_quads: usize = groups.iter().map(|g| g.len()).sum();
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

    bar.set_job_title("Vertices:");
    for v in c.final_vertices {
        writeln!(dest, "v {} {} {}", v[0], v[1], v[2]).unwrap();
    }
    bar.set_job_title("Faces:");
    writeln!(dest).unwrap();
    for (i, g) in groups.iter().enumerate() {
        writeln!(dest, "g m{}", i).unwrap();
        for t in g {
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
}
