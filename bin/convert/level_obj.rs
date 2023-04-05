use std::{
    fs::File,
    io::{BufWriter, Write as _},
    path::Path,
};
use vangers::level::{Level, Texel};

const MAX_COLUMN_VERTICES: usize = 4 * 4;
const EXTREME_HEIGHT: i32 = i32::max_value();

#[derive(Debug)]
pub struct Optimization {
    pub weld_height_diff: i32,
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
struct Column {
    vertices: [Vertex; MAX_COLUMN_VERTICES + 1],
}
impl Column {
    fn add(&mut self, height: i32, optimization: &Optimization) -> &mut Vertex {
        let mut order_index = self
            .vertices
            .iter()
            .position(|v| v.height > height)
            .unwrap();

        if self.vertices[order_index].height - height <= optimization.weld_height_diff {
            // do nothing
        } else if order_index != 0
            && height - self.vertices[order_index - 1].height <= optimization.weld_height_diff
        {
            order_index -= 1;
        } else {
            self.vertices[order_index..].rotate_right(1);
        }

        let v_new = &mut self.vertices[order_index];
        if v_new.height == EXTREME_HEIGHT {
            v_new.height = height;
        }
        v_new
    }
}

struct VertexCollector<'p> {
    final_vertices: Vec<[i32; 3]>,
    columns: Vec<Vec<Column>>,
    optimization: &'p Optimization,
}

impl VertexCollector<'_> {
    fn add(&mut self, x: i32, y: i32, z: i32) -> u32 {
        let vertex = self.columns[y as usize][x as usize].add(z, &self.optimization);
        if vertex.index == !0 {
            vertex.index = self.final_vertices.len() as u32;
            self.final_vertices.push([x, y, z]);
        }
        vertex.index
    }

    fn add_quad(&mut self, x: i32, w: i32, y: i32, z: i32) -> [u32; 4] {
        [
            self.add(x, y, z),
            self.add(x + w, y, z),
            self.add(x + w, y + 1, z),
            self.add(x, y + 1, z),
        ]
    }
}

pub fn save(path: &Path, level: &Level, optimization: &Optimization) {
    let mut dest = BufWriter::new(File::create(&path).unwrap());
    let mut groups: [Vec<[u32; 4]>; 16] = Default::default();
    let mut bar = progress::Bar::new();

    // these are metrics for the most naive bar generation
    let mut num_vertices_total = 0;
    let mut num_quads_total = 0;

    bar.set_job_title("Processing:");
    let mut c = VertexCollector {
        final_vertices: Vec::new(),
        columns: (0..=level.size.1)
            .map(|_| vec![Column::default(); level.size.0 as usize + 1])
            .collect(),
        optimization,
    };
    for y in 0..level.size.1 {
        let mut x = 0;
        while x < level.size.0 {
            let threshold = optimization.weld_height_diff as f32;
            let (p, w) = match level.get((x, y)) {
                Texel::Single(p) => (p, 1),
                // Cut out unexpected/invalid cases
                Texel::Dual { low, mid, high } if mid > high.0 => (low, 2),
                Texel::Dual { low, mid, high } => {
                    let g = &mut groups[high.1 as usize];
                    let lo = c.add_quad(x, 2, y, mid as i32);
                    let hi = c.add_quad(x, 2, y, high.0 as i32);
                    // top + bottom
                    g.push(hi);
                    if mid > low.0 + threshold {
                        g.push([lo[3], lo[2], lo[1], lo[0]]);
                    }
                    // left
                    if x == 0 || high.0 > level.get_low_fast((x - 1, y)) + threshold {
                        g.push([lo[3], lo[0], hi[0], hi[3]]);
                    }
                    // right
                    if x + 1 == level.size.0 || high.0 > level.get_low_fast((x + 1, y)) + threshold
                    {
                        g.push([lo[1], lo[2], hi[2], hi[1]]);
                    }
                    // near
                    if y == 0 || high.0 > level.get_low_fast_dual((x, y - 1)) + threshold {
                        g.push([lo[0], lo[1], hi[1], hi[0]]);
                    }
                    // far
                    if y + 1 == level.size.1
                        || high.0 > level.get_low_fast_dual((x, y + 1)) + threshold
                    {
                        g.push([lo[2], lo[3], hi[3], hi[2]]);
                    }
                    // done
                    num_vertices_total += 16;
                    num_quads_total += 10;
                    (low, 2)
                }
            };

            let g = &mut groups[p.1 as usize];
            if p.0 > threshold {
                // determine conditions
                let c_left = x == 0 || p.0 > level.get_low_fast((x - 1, y)) + threshold;
                let c_right =
                    x + 1 == level.size.0 || p.0 > level.get_low_fast((x + 1, y)) + threshold;
                let c_near =
                    y == 0 || p.0 > level.get_low_fast_switch((x, y - 1), w > 1) + threshold;
                let c_far =
                    y + 1 == level.size.1 || p.0 > level.get_low_fast_switch((x, y + 1), w > 1);
                // top and bottom
                let hi = c.add_quad(x, w, y, p.0 as i32);
                let lo = if c_left || c_right || c_near || c_far {
                    c.add_quad(x, w, y, 0)
                } else {
                    [0; 4]
                };
                if false {
                    g.push([lo[3], lo[2], lo[1], lo[0]]);
                }
                g.push(hi);
                // add faces
                if c_left {
                    g.push([lo[3], lo[0], hi[0], hi[3]]);
                }
                if c_right {
                    g.push([lo[1], lo[2], hi[2], hi[1]]);
                }
                if c_near {
                    g.push([lo[0], lo[1], hi[1], hi[0]]);
                }
                if c_far {
                    g.push([lo[2], lo[3], hi[3], hi[2]]);
                }
                // done
                num_vertices_total += 8 * w;
                num_quads_total += 2 + 4 * w;
            } else {
                let lo = c.add_quad(x, w, y, 0);
                g.push(lo);
                num_vertices_total += 4 * w;
                num_quads_total += w;
            }

            x += w;
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
        unit(num_vertices_total as usize),
        unit(num_quads),
        unit(num_quads_total as usize),
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
