use std::{
    fs::File,
    io::{BufWriter, Write as _},
    path::Path,
};
use vangers::level::{Level, Texel};

struct VertexCollector<O> {
    dest: O,
    vertices: fnv::FnvHashMap<(i32, i32, u8), u32>,
}

impl<O: std::io::Write> VertexCollector<O> {
    fn add(&mut self, x: i32, y: i32, z: u8) -> u32 {
        let next = self.vertices.len();
        let dest = &mut self.dest;
        *self.vertices.entry((x, y, z)).or_insert_with(|| {
            writeln!(dest, "v {} {} {}", x, y, z).unwrap();
            next as u32
        })
    }

    fn add_quad(&mut self, x: i32, y: i32, z: u8) -> [u32; 4] {
        [
            self.add(x, y, z),
            self.add(x + 1, y, z),
            self.add(x + 1, y + 1, z),
            self.add(x, y + 1, z),
        ]
    }
}

trait TexelExt {
    fn lowest(&self) -> u8;
}
impl TexelExt for Texel {
    fn lowest(&self) -> u8 {
        match *self {
            Self::Single(ref p) => p.0,
            Self::Dual { ref low, .. } => low.0,
        }
    }
}

pub fn save(path: &Path, level: &Level) {
    let mut dest = BufWriter::new(File::create(&path).unwrap());
    let mut groups: [Vec<[u32; 4]>; 16] = Default::default();
    let mut bar = progress::Bar::new();

    bar.set_job_title("Vertices:");
    let mut c = VertexCollector {
        dest: &mut dest,
        vertices: Default::default(),
    };
    for y in 0..level.size.1 {
        bar.reach_percent(y * 100 / level.size.1);
        for x in 0..level.size.0 {
            let p = match level.get((x, y)) {
                Texel::Single(p) => p,
                Texel::Dual { low, high, delta } => {
                    let g = &mut groups[high.1 as usize];
                    let lo = c.add_quad(x, y, low.0 + delta);
                    let hi = c.add_quad(x, y, high.0);
                    // top + bottom
                    g.push(hi);
                    g.push([lo[3], lo[2], lo[1], lo[0]]);
                    // left
                    g.push([lo[3], lo[0], hi[0], hi[3]]);
                    // right
                    g.push([lo[1], lo[2], hi[2], hi[1]]);
                    // near
                    g.push([lo[0], lo[1], hi[1], hi[0]]);
                    // far
                    g.push([lo[2], lo[3], hi[3], hi[2]]);
                    low
                }
            };

            let g = &mut groups[p.1 as usize];
            if p.0 > 0 {
                let lo = c.add_quad(x, y, 0);
                let hi = c.add_quad(x, y, p.0);
                // top + bottom
                g.push(hi);
                g.push([lo[3], lo[2], lo[1], lo[0]]);
                // left
                if x > 0 && p.0 > level.get((x - 1, y)).lowest() {
                    g.push([lo[3], lo[0], hi[0], hi[3]]);
                }
                // right
                if x + 1 < level.size.0 && p.0 > level.get((x + 1, y)).lowest() {
                    g.push([lo[1], lo[2], hi[2], hi[1]]);
                }
                // near
                if y > 0 && p.0 > level.get((x, y - 1)).lowest() {
                    g.push([lo[0], lo[1], hi[1], hi[0]]);
                }
                // far
                if y + 1 < level.size.1 && p.0 > level.get((x, y + 1)).lowest() {
                    g.push([lo[2], lo[3], hi[3], hi[2]]);
                }
            } else {
                let lo = c.add_quad(x, y, 0);
                g.push(lo);
            }
        }
    }

    bar.jobs_done();
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
