use std::{
    fs::File,
    io::{BufWriter, Write as _},
    path::Path,
};
use vangers::level::{Level, Texel};

struct VertexCollector<O> {
    dest: O,
    vertices: fnv::FnvHashMap<(i32, i32, i32), u32>,
}

impl<O: std::io::Write> VertexCollector<O> {
    fn add(&mut self, x: i32, y: i32, z: i32) -> u32 {
        let next = self.vertices.len();
        let dest = &mut self.dest;
        *self.vertices.entry((x, y, z)).or_insert_with(|| {
            writeln!(dest, "v {} {} {}", x, y, z).unwrap();
            next as u32
        })
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

pub fn save(path: &Path, level: &Level) {
    let mut dest = BufWriter::new(File::create(&path).unwrap());
    let mut groups: [Vec<[u32; 4]>; 16] = Default::default();
    let mut bar = progress::Bar::new();

    // these are metrics for the most naive bar generation
    let mut num_vertices_total = 0;
    let mut num_quads_total = 0;

    bar.set_job_title("Vertices:");
    let mut c = VertexCollector {
        dest: &mut dest,
        vertices: Default::default(),
    };
    for y in 0..level.size.1 {
        let mut x = 0;
        while x < level.size.0 {
            let (p, w) = match level.get((x, y)) {
                Texel::Single(p) => (p, 1),
                Texel::Dual { low, mid, high } if mid >= high.0 => (low, 2),
                Texel::Dual { low, mid, high } => {
                    let g = &mut groups[high.1 as usize];
                    assert_ne!(mid, high.0);
                    let lo = c.add_quad(x, 2, y, mid as i32);
                    let hi = c.add_quad(x, 2, y, high.0 as i32);
                    // top + bottom
                    g.push(hi);
                    if low.0 != mid {
                        g.push([lo[3], lo[2], lo[1], lo[0]]);
                    }
                    // left
                    if x == 0 || high.0 > level.get_low_fast((x - 1, y)) {
                        g.push([lo[3], lo[0], hi[0], hi[3]]);
                    }
                    // right
                    if x + 1 == level.size.0 || high.0 > level.get_low_fast((x + 1, y)) {
                        g.push([lo[1], lo[2], hi[2], hi[1]]);
                    }
                    // near
                    if y == 0 || high.0 > level.get_low_fast_dual((x, y - 1)) {
                        g.push([lo[0], lo[1], hi[1], hi[0]]);
                    }
                    // far
                    if y + 1 == level.size.1 || high.0 > level.get_low_fast_dual((x, y + 1)) {
                        g.push([lo[2], lo[3], hi[3], hi[2]]);
                    }
                    // done
                    num_vertices_total += 16;
                    num_quads_total += 10;
                    (low, 2)
                }
            };

            let g = &mut groups[p.1 as usize];
            if p.0 >= 1.0 {
                let lo = c.add_quad(x, w, y, 0);
                let hi = c.add_quad(x, w, y, p.0 as i32);
                // top + bottom
                g.push(hi);
                if false {
                    g.push([lo[3], lo[2], lo[1], lo[0]]);
                }
                // left
                if x == 0 || p.0 > level.get_low_fast((x - 1, y)) {
                    g.push([lo[3], lo[0], hi[0], hi[3]]);
                }
                // right
                if x + 1 == level.size.0 || p.0 > level.get_low_fast((x + 1, y)) {
                    g.push([lo[1], lo[2], hi[2], hi[1]]);
                }
                // near
                if y == 0 || p.0 > level.get_low_fast_switch((x, y - 1), w > 1) {
                    g.push([lo[0], lo[1], hi[1], hi[0]]);
                }
                // far
                if y + 1 == level.size.1 || p.0 > level.get_low_fast_switch((x, y + 1), w > 1) {
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
        unit(c.vertices.len()),
        unit(num_vertices_total as usize),
        unit(num_quads),
        unit(num_quads_total as usize),
    );

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
