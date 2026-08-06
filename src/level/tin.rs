//! Triangulated irregular network (TIN) approximation of the level.
//!
//! Instead of ray-marching the height map, this builds an actual triangle
//! mesh that approximates it, following Garland & Heckbert's greedy
//! insertion scheme ("Fast Polygonal Approximation of Terrains and Height
//! Fields", 1995): start from a coarse triangulation, then repeatedly
//! insert the grid point with the largest vertical error until the whole
//! surface is within a target tolerance.
//!
//! The multi-layer terrain is handled with a *single* planar Delaunay
//! triangulation in XY. Each texel carries three surfaces - the `low`
//! floor, the `mid` cave ceiling and the `high` slab top - and the error
//! metric is the largest deviation across all of them, so a point gets
//! inserted when *any* layer needs it. Every vertex then stores all three
//! altitudes at once.
//!
//! That shared triangulation is what makes the double-level regions cheap:
//! where a region ends, the layers collapse onto each other, and we close
//! the slab off with a vertical wall along the triangle edges that straddle
//! the boundary (see `emit_chunk`).
//!
//! The triangulation runs on chunk-local integer coordinates so that the
//! `orient2d` / `in_circle` predicates are exact in `i64` - the grid is
//! massively cocircular (every axis-aligned square of four samples), which
//! is exactly the case floating-point predicates get wrong.

use crate::level::{Level, Texel};
use bytemuck::{Pod, Zeroable};

/// Sentinel for "no triangle" in adjacency links and slot indices.
const NONE: u32 = u32::MAX;

/// A render-ready mesh vertex.
///
/// Lighting is derived in the shader from the height map gradient (see
/// `evaluate_color` in `terrain/color.inc.wgsl`), and so is the terrain
/// type - looking it up per fragment keeps the type boundaries at full
/// texel resolution even where the triangles are coarse, which is what
/// makes this match the ray traced output. All the vertex has to say is
/// *which* of the stacked surfaces it belongs to.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MeshVertex {
    pub pos: [f32; 3],
    /// 0 for the `low` floor, 1 for the `mid`/`high` slab.
    pub layer: u32,
}
unsafe impl Pod for MeshVertex {}
unsafe impl Zeroable for MeshVertex {}

/// Side of a build chunk, in texels.
///
/// Large enough that the border simplification stays a small fraction of
/// the vertex budget, small enough to keep the greedy rasterisation cheap
/// and give rayon plenty of independent work.
const CHUNK_SIZE: u32 = 128;

/// Discrete level-of-detail steps kept per chunk.
///
/// Greedy insertion makes these nearly free to produce: LOD `k` is just the
/// same fit stopped at a coarser tolerance, so the coarse levels cost a
/// fraction of the fine one to build and a fraction of its triangles to
/// draw. Each step doubles the tolerance, which roughly halves the
/// triangles - the same curve the `quality` knob rides.
pub const LOD_COUNT: usize = 3;

#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// The single quality knob, in `0..=1`. Higher fits the terrain more
    /// closely and spends more triangles.
    ///
    /// It maps to a vertical tolerance measured in *height quantisation
    /// steps* (`geometry.height / 256`), which makes it independent of the
    /// level's height scale. `1.0` asks for one step - the finest detail
    /// 8-bit samples can carry, and asking for less just makes the greedy
    /// pass chase quantisation stairs for no visual gain. Every 0.25 below
    /// that doubles the tolerance, bottoming out at 16 steps.
    pub quality: f32,
}

impl Default for Config {
    fn default() -> Self {
        Config { quality: 0.75 }
    }
}

impl Config {
    /// Vertical tolerance in world altitude units, for this level's height
    /// scale.
    fn max_error(&self, level: &Level) -> f32 {
        let step = level.geometry.height as f32 / 256.0;
        step * (4.0 * (1.0 - self.quality.clamp(0.0, 1.0))).exp2()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub vertices: usize,
    pub triangles: usize,
    /// Grid samples the mesh was built from, for a reduction ratio.
    pub source_texels: usize,
    /// Vertical tolerance the mesh was actually fitted to.
    pub max_error: f32,
}

pub struct Mesh {
    pub chunks: Vec<ChunkBuffers>,
    pub stats: Stats,
}

/// All three surfaces of one texel.
///
/// Terrain *types* deliberately aren't here: the fragment shader reads them
/// straight from the terrain texture, so they stay at full texel resolution
/// no matter how coarse the triangles get.
#[derive(Clone, Copy, Default)]
struct Sample {
    low: f32,
    mid: f32,
    high: f32,
    is_dual: bool,
}

impl Sample {
    fn at(level: &Level, x: i32, y: i32) -> Self {
        match level.get((x, y)) {
            Texel::Single(p) => Sample {
                low: p.0,
                mid: p.0,
                high: p.0,
                is_dual: false,
            },
            Texel::Dual { low, mid, high } => Sample {
                low: low.0,
                mid,
                high: high.0,
                is_dual: true,
            },
        }
    }

    /// Largest deviation of any layer from the given interpolated values.
    /// This is the "insert if *any* layer needs it" criterion.
    fn deviation(&self, low: f32, mid: f32, high: f32) -> f32 {
        (self.low - low)
            .abs()
            .max((self.mid - mid).abs())
            .max((self.high - high).abs())
    }
}

/// Twice the signed area of the triangle `abc`; positive when CCW.
///
/// Exact: chunk-local coordinates are bounded by `chunk_size`, so the
/// products stay far inside `i64` for any sane chunk.
fn orient2d(a: [i32; 2], b: [i32; 2], c: [i32; 2]) -> i64 {
    let (ax, ay) = (a[0] as i64, a[1] as i64);
    let (bx, by) = (b[0] as i64, b[1] as i64);
    let (cx, cy) = (c[0] as i64, c[1] as i64);
    (bx - ax) * (cy - ay) - (by - ay) * (cx - ax)
}

/// Positive when `d` lies strictly inside the circumcircle of the CCW
/// triangle `abc`. Exact for the same reason as `orient2d`.
fn in_circle(a: [i32; 2], b: [i32; 2], c: [i32; 2], d: [i32; 2]) -> i64 {
    let adx = (a[0] - d[0]) as i64;
    let ady = (a[1] - d[1]) as i64;
    let bdx = (b[0] - d[0]) as i64;
    let bdy = (b[1] - d[1]) as i64;
    let cdx = (c[0] - d[0]) as i64;
    let cdy = (c[1] - d[1]) as i64;

    let alift = adx * adx + ady * ady;
    let blift = bdx * bdx + bdy * bdy;
    let clift = cdx * cdx + cdy * cdy;

    adx * (bdy * clift - cdy * blift) - ady * (bdx * clift - cdx * blift)
        + alift * (bdx * cdy - cdx * bdy)
}

#[derive(Clone)]
struct Tri {
    v: [u32; 3],
    /// `n[i]` is the triangle across edge `(v[i], v[(i + 1) % 3])`.
    n: [u32; 3],
    /// Grid index of the worst-approximated sample inside this triangle.
    cand: u32,
    err: f32,
    alive: bool,
}

/// One chunk's sample grid. `nx * ny` samples covering texels
/// `[x0 ..= x0 + w]` x `[y0 ..= y0 + h]`, i.e. neighbouring chunks overlap
/// by exactly one row/column so their boundary vertices coincide.
struct Grid {
    samples: Vec<Sample>,
    nx: u32,
    ny: u32,
    x0: i32,
    y0: i32,
}

impl Grid {
    fn new(level: &Level, x0: i32, y0: i32, w: u32, h: u32) -> Self {
        let nx = w + 1;
        let ny = h + 1;
        let mut samples = Vec::with_capacity((nx * ny) as usize);
        for ly in 0..ny {
            for lx in 0..nx {
                samples.push(Sample::at(level, x0 + lx as i32, y0 + ly as i32));
            }
        }
        Grid {
            samples,
            nx,
            ny,
            x0,
            y0,
        }
    }

    fn index(&self, lx: u32, ly: u32) -> u32 {
        ly * self.nx + lx
    }

    fn coord(&self, index: u32) -> [i32; 2] {
        [(index % self.nx) as i32, (index / self.nx) as i32]
    }

    fn sample(&self, index: u32) -> &Sample {
        &self.samples[index as usize]
    }

    /// Lowest floor and highest slab top over the whole chunk.
    fn altitude_range(&self) -> (f32, f32) {
        self.samples
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), s| {
                (lo.min(s.low), hi.max(s.high))
            })
    }
}

/// Greedy TIN over a single chunk.
#[cfg_attr(test, derive(Clone))]
struct Chunk {
    /// Grid index of each triangulation vertex.
    verts: Vec<u32>,
    tris: Vec<Tri>,
    /// Freed slots from collapsed cavities, reused before growing `tris`.
    free: Vec<u32>,
}

impl Chunk {
    fn pos(&self, grid: &Grid, v: u32) -> [i32; 2] {
        grid.coord(self.verts[v as usize])
    }

    fn sample<'g>(&self, grid: &'g Grid, v: u32) -> &'g Sample {
        grid.sample(self.verts[v as usize])
    }

    /// Seed with the two triangles spanning the chunk rectangle.
    fn new(grid: &Grid) -> Self {
        let (mx, my) = (grid.nx - 1, grid.ny - 1);
        let corners = [
            grid.index(0, 0),
            grid.index(mx, 0),
            grid.index(mx, my),
            grid.index(0, my),
        ];
        let tris = vec![
            Tri {
                v: [0, 1, 2],
                n: [NONE, NONE, 1],
                cand: NONE,
                err: 0.0,
                alive: true,
            },
            Tri {
                v: [0, 2, 3],
                n: [0, NONE, NONE],
                cand: NONE,
                err: 0.0,
                alive: true,
            },
        ];
        Chunk {
            verts: corners.to_vec(),
            tris,
            free: Vec::new(),
        }
    }

    /// Walk from `hint` toward `p` until we land in the triangle containing
    /// it. The domain stays convex (it is always the chunk rectangle), so a
    /// straight walk terminates; the cap is pure paranoia.
    ///
    /// A point lying exactly *on* an edge yields `orient2d == 0`, which is
    /// not `< 0`, so we stop in one of the two triangles sharing it - which
    /// is what `insert` wants.
    fn locate(&self, grid: &Grid, p: [i32; 2], hint: u32) -> u32 {
        let mut t = hint;
        for _ in 0..self.tris.len() + 8 {
            let tri = &self.tris[t as usize];
            let mut moved = false;
            for i in 0..3 {
                let a = self.pos(grid, tri.v[i]);
                let b = self.pos(grid, tri.v[(i + 1) % 3]);
                if orient2d(a, b, p) < 0 && tri.n[i] != NONE {
                    t = tri.n[i];
                    moved = true;
                    break;
                }
            }
            if !moved {
                return t;
            }
        }
        t
    }

    /// Bowyer-Watson insertion of the grid point `gi`, starting the cavity
    /// search from `seed` (which must contain the point). Returns the slots
    /// of the triangles that were created.
    fn insert(&mut self, grid: &Grid, gi: u32, seed: u32) -> Vec<u32> {
        let p = grid.coord(gi);
        let vid = self.verts.len() as u32;
        self.verts.push(gi);

        // 1. Collect the cavity: every triangle whose circumcircle contains
        // `p`. It is connected, so a flood fill through neighbours suffices.
        let mut cavity = vec![seed];
        let mut in_cavity = vec![false; self.tris.len()];
        in_cavity[seed as usize] = true;
        let mut cursor = 0;
        while cursor < cavity.len() {
            let t = cavity[cursor];
            cursor += 1;
            for i in 0..3 {
                let nb = self.tris[t as usize].n[i];
                if nb == NONE || in_cavity[nb as usize] {
                    continue;
                }
                let tri = &self.tris[nb as usize];
                let (a, b, c) = (
                    self.pos(grid, tri.v[0]),
                    self.pos(grid, tri.v[1]),
                    self.pos(grid, tri.v[2]),
                );
                if in_circle(a, b, c, p) > 0 {
                    in_cavity[nb as usize] = true;
                    cavity.push(nb);
                }
            }
        }

        // 2. The cavity boundary: edges not shared with another cavity
        // triangle. They come out oriented CCW around the cavity, so
        // `(a, b, p)` is CCW for an interior `p`.
        let mut boundary = Vec::new();
        for &t in &cavity {
            let tri = self.tris[t as usize].clone();
            for i in 0..3 {
                let outer = tri.n[i];
                if outer != NONE && in_cavity[outer as usize] {
                    continue;
                }
                boundary.push((tri.v[i], tri.v[(i + 1) % 3], outer));
            }
        }

        // 3. Retire the cavity and re-fan it from the new vertex. An edge
        // collinear with `p` would make a zero-area triangle - that is the
        // hull edge `p` landed on, and skipping it simply splits the hull
        // in two, which is exactly right.
        for &t in &cavity {
            self.tris[t as usize].alive = false;
            self.free.push(t);
        }

        let mut created = Vec::with_capacity(boundary.len());
        let mut fan = Vec::with_capacity(boundary.len());
        for &(a, b, outer) in &boundary {
            if orient2d(self.pos(grid, a), self.pos(grid, b), p) == 0 {
                continue;
            }
            let tri = Tri {
                v: [a, b, vid],
                n: [outer, NONE, NONE],
                cand: NONE,
                err: 0.0,
                alive: true,
            };
            let slot = match self.free.pop() {
                Some(slot) => {
                    self.tris[slot as usize] = tri;
                    slot
                }
                None => {
                    self.tris.push(tri);
                    self.tris.len() as u32 - 1
                }
            };
            created.push(slot);
            fan.push((a, b, slot));
        }

        // 4. Relink adjacency. Edge 0 of a new triangle faces the outer
        // neighbour, edge 1 `(b, vid)` faces the fan triangle starting at
        // `b`, and edge 2 `(vid, a)` the one ending at `a`. Anything left
        // unmatched is a hull edge.
        for &(a, b, slot) in &fan {
            let n1 = fan
                .iter()
                .find(|&&(oa, _, os)| oa == b && os != slot)
                .map_or(NONE, |&(_, _, os)| os);
            let n2 = fan
                .iter()
                .find(|&&(_, ob, os)| ob == a && os != slot)
                .map_or(NONE, |&(_, _, os)| os);
            self.tris[slot as usize].n[1] = n1;
            self.tris[slot as usize].n[2] = n2;

            // Point the outer neighbour back at us.
            let outer = self.tris[slot as usize].n[0];
            if outer != NONE {
                let otri = &mut self.tris[outer as usize];
                for i in 0..3 {
                    if otri.v[i] == b && otri.v[(i + 1) % 3] == a {
                        otri.n[i] = slot;
                        break;
                    }
                }
            }
        }

        created
    }

    /// Find the worst-approximated grid sample inside triangle `t`.
    ///
    /// Straightforward bounding-box rasterisation with the edge functions
    /// doubling as barycentric weights. Cost is proportional to the
    /// triangle's area, which shrinks quickly as insertion proceeds.
    fn compute_candidate(&mut self, grid: &Grid, t: u32) {
        let tri = &self.tris[t as usize];
        let (va, vb, vc) = (tri.v[0], tri.v[1], tri.v[2]);
        let (a, b, c) = (self.pos(grid, va), self.pos(grid, vb), self.pos(grid, vc));
        let area2 = orient2d(a, b, c);
        if area2 <= 0 {
            let tri = &mut self.tris[t as usize];
            tri.cand = NONE;
            tri.err = 0.0;
            return;
        }
        let (sa, sb, sc) = (
            self.sample(grid, va),
            self.sample(grid, vb),
            self.sample(grid, vc),
        );
        let inv = 1.0 / area2 as f64;

        let min_x = a[0].min(b[0]).min(c[0]).max(0);
        let max_x = a[0].max(b[0]).max(c[0]).min(grid.nx as i32 - 1);
        let min_y = a[1].min(b[1]).min(c[1]).max(0);
        let max_y = a[1].max(b[1]).max(c[1]).min(grid.ny as i32 - 1);

        let mut best = NONE;
        let mut best_err = 0.0f32;
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let q = [x, y];
                let w0 = orient2d(b, c, q);
                if w0 < 0 {
                    continue;
                }
                let w1 = orient2d(c, a, q);
                if w1 < 0 {
                    continue;
                }
                let w2 = orient2d(a, b, q);
                if w2 < 0 {
                    continue;
                }
                let (f0, f1, f2) = (w0 as f64 * inv, w1 as f64 * inv, w2 as f64 * inv);
                let lerp = |pa: f32, pb: f32, pc: f32| {
                    (f0 * pa as f64 + f1 * pb as f64 + f2 * pc as f64) as f32
                };
                let gi = grid.index(x as u32, y as u32);
                let err = grid.sample(gi).deviation(
                    lerp(sa.low, sb.low, sc.low),
                    lerp(sa.mid, sb.mid, sc.mid),
                    lerp(sa.high, sb.high, sc.high),
                );
                if err > best_err {
                    best_err = err;
                    best = gi;
                }
            }
        }

        let tri = &mut self.tris[t as usize];
        tri.cand = best;
        tri.err = best_err;
    }
}

/// Douglas-Peucker over a line of samples, using the same "worst layer"
/// metric. Splitting on the largest error and tie-breaking on the lowest
/// index makes this a pure function of the level data, so the two chunks
/// sharing a border independently derive the *same* vertex set - which is
/// what keeps the seam crack-free.
fn simplify_line(samples: &[Sample], max_error: f32, out: &mut Vec<u32>) {
    fn recurse(samples: &[Sample], lo: u32, hi: u32, max_error: f32, out: &mut Vec<u32>) {
        if hi <= lo + 1 {
            return;
        }
        let (a, b) = (&samples[lo as usize], &samples[hi as usize]);
        let span = (hi - lo) as f32;
        let mut best = 0u32;
        let mut best_err = 0.0f32;
        for i in lo + 1..hi {
            let t = (i - lo) as f32 / span;
            let s = &samples[i as usize];
            let err = s.deviation(
                a.low + (b.low - a.low) * t,
                a.mid + (b.mid - a.mid) * t,
                a.high + (b.high - a.high) * t,
            );
            if err > best_err {
                best_err = err;
                best = i;
            }
        }
        if best_err > max_error {
            out.push(best);
            recurse(samples, lo, best, max_error, out);
            recurse(samples, best, hi, max_error, out);
        }
    }
    if samples.len() >= 2 {
        recurse(samples, 0, samples.len() as u32 - 1, max_error, out);
    }
}

/// Per-chunk output, later concatenated into the final mesh.
#[derive(Default)]
struct ChunkMesh {
    vertices: Vec<MeshVertex>,
    indices: Vec<u32>,
}

/// Which of the three surfaces an emitted vertex sits on.
#[derive(Clone, Copy, PartialEq)]
enum Layer {
    Low = 0,
    Mid = 1,
    High = 2,
}

struct Emitter<'a> {
    chunk: &'a Chunk,
    grid: &'a Grid,
    out: ChunkMesh,
    /// `[vertex][layer]` -> index in `out.vertices`, or `NONE`.
    cache: Vec<[u32; 3]>,
}

impl Emitter<'_> {
    fn vertex(&mut self, v: u32, layer: Layer) -> u32 {
        let slot = &mut self.cache[v as usize][layer as usize];
        if *slot != NONE {
            return *slot;
        }
        let s = self.chunk.sample(self.grid, v);
        let local = self.chunk.pos(self.grid, v);
        let grid = self.grid;
        let z = match layer {
            Layer::Low => s.low,
            Layer::Mid => s.mid,
            Layer::High => s.high,
        };
        // Samples describe texel *cells*, so the vertex belongs at the cell
        // centre - matching how the shaders index the terrain texture.
        let index = self.out.vertices.len() as u32;
        self.out.vertices.push(MeshVertex {
            pos: [
                (grid.x0 + local[0]) as f32 + 0.5,
                (grid.y0 + local[1]) as f32 + 0.5,
                z,
            ],
            layer: u32::from(layer != Layer::Low),
        });
        *slot = index;
        index
    }

    /// Emit one triangle, given its corners in outward-facing order.
    ///
    /// The indices go out reversed. Callers describe faces the natural
    /// way, counter-clockwise seen from outside in world space, but the
    /// camera folds a scale of (1, -1, 1) into its view matrix to be
    /// left-handed (see `Camera::get_view_proj`), which flips handedness on
    /// the way to the screen. Reversing here means the rasterizer sees
    /// outward faces as front-facing, so plain back-face culling works.
    fn tri(&mut self, a: (u32, Layer), b: (u32, Layer), c: (u32, Layer)) {
        let ia = self.vertex(a.0, a.1);
        let ib = self.vertex(b.0, b.1);
        let ic = self.vertex(c.0, c.1);
        self.out.indices.extend_from_slice(&[ic, ib, ia]);
    }
}

/// Turn a finished triangulation into renderable geometry.
///
/// * the `low` floor is emitted for every triangle;
/// * a triangle whose three vertices are all double-level also gets the
///   `high` slab top and the `mid` cave ceiling;
/// * where such a triangle borders one that is *not* double-level, the slab
///   would otherwise end in mid-air, so we close it with a vertical wall
///   from `mid` up to `high` along that edge. Only the slab side emits the
///   wall, so it is never duplicated.
fn emit_chunk(chunk: &Chunk, grid: &Grid) -> ChunkMesh {
    let is_slab = |tri: &Tri| -> bool {
        tri.v.iter().all(|&v| chunk.sample(grid, v).is_dual)
            // A zero-thickness slab has nothing to show.
            && tri.v.iter().any(|&v| {
                let s = chunk.sample(grid, v);
                s.high > s.low
            })
    };

    let mut emitter = Emitter {
        chunk,
        grid,
        out: ChunkMesh::default(),
        cache: vec![[NONE; 3]; chunk.verts.len()],
    };

    for t in 0..chunk.tris.len() {
        let tri = &chunk.tris[t];
        if !tri.alive {
            continue;
        }
        let [a, b, c] = tri.v;
        emitter.tri((a, Layer::Low), (b, Layer::Low), (c, Layer::Low));

        if !is_slab(tri) {
            continue;
        }
        // Slab top, and the ceiling underneath it wound the other way.
        emitter.tri((a, Layer::High), (b, Layer::High), (c, Layer::High));
        emitter.tri((c, Layer::Mid), (b, Layer::Mid), (a, Layer::Mid));

        for i in 0..3 {
            let nb = tri.n[i];
            if nb != NONE && chunk.tris[nb as usize].alive && is_slab(&chunk.tris[nb as usize]) {
                continue;
            }
            let (e0, e1) = (tri.v[i], tri.v[(i + 1) % 3]);
            emitter.tri((e0, Layer::Mid), (e1, Layer::Mid), (e1, Layer::High));
            emitter.tri((e1, Layer::High), (e0, Layer::High), (e0, Layer::Mid));
        }
    }

    emitter.out
}

/// Bring a chunk's triangulation within tolerance of its (possibly just
/// re-sampled) grid.
///
/// Vertices are only ever *added* - nothing is removed and nothing moves in
/// XY - so this works equally well on a fresh chunk and on one being
/// refined after a terrain edit. Heights are not stored in the
/// triangulation at all; they are read from the grid at emit time, so an
/// edit that only moves the surface up or down needs no new vertices, just
/// a re-emit.
fn refine(chunk: &mut Chunk, grid: &Grid, max_error: f32) {
    use std::collections::{BinaryHeap, HashSet};

    let existing: HashSet<u32> = chunk.verts.iter().copied().collect();

    // Border vertices first. Both chunks sharing a border derive the same
    // set from the same samples, so the seam matches exactly - and because
    // we only add, re-deriving after an edit keeps them in step: both sides
    // end up with the union of their old picks and the identical new ones.
    let (mx, my) = (grid.nx - 1, grid.ny - 1);
    let mut border = Vec::new();
    let mut line = Vec::with_capacity(grid.nx.max(grid.ny) as usize);
    for (fixed, horizontal) in [(0, true), (my, true), (0, false), (mx, false)] {
        line.clear();
        let count = if horizontal { grid.nx } else { grid.ny };
        for i in 0..count {
            let gi = if horizontal {
                grid.index(i, fixed)
            } else {
                grid.index(fixed, i)
            };
            line.push(*grid.sample(gi));
        }
        let mut picks = Vec::new();
        simplify_line(&line, max_error, &mut picks);
        for i in picks {
            border.push(if horizontal {
                grid.index(i, fixed)
            } else {
                grid.index(fixed, i)
            });
        }
    }
    // Deterministic order keeps the whole build reproducible.
    border.sort_unstable();
    border.dedup();
    for gi in border {
        if existing.contains(&gi) {
            continue;
        }
        let seed = chunk.locate(grid, grid.coord(gi), 0);
        chunk.insert(grid, gi, seed);
    }

    // Every triangle remembers its own worst sample, so popping the global
    // worst needs no point location - we already know which triangle
    // contains it. After an edit the cached candidates are stale, so they
    // all get recomputed; that is one pass over the chunk's samples.
    for t in 0..chunk.tris.len() as u32 {
        if chunk.tris[t as usize].alive {
            chunk.compute_candidate(grid, t);
        }
    }
    let mut heap = BinaryHeap::new();
    for (t, tri) in chunk.tris.iter().enumerate() {
        if tri.alive && tri.cand != NONE {
            heap.push((tri.err.to_bits(), t as u32));
        }
    }

    // The TIN can never usefully exceed the source grid, so this is only a
    // runaway guard, not a quality knob.
    let max_vertices = grid.nx * grid.ny;
    while (chunk.verts.len() as u32) < max_vertices {
        let (bits, t) = match heap.pop() {
            Some(entry) => entry,
            None => break,
        };
        {
            // Stale entry: the slot was rewritten since it was queued.
            let tri = &chunk.tris[t as usize];
            if !tri.alive || tri.cand == NONE || tri.err.to_bits() != bits {
                continue;
            }
            if tri.err <= max_error {
                break;
            }
        }
        let cand = chunk.tris[t as usize].cand;
        for slot in chunk.insert(grid, cand, t) {
            chunk.compute_candidate(grid, slot);
            let tri = &chunk.tris[slot as usize];
            if tri.cand != NONE {
                heap.push((tri.err.to_bits(), slot));
            }
        }
    }
}

/// One chunk's geometry, in its own pair of buffers.
///
/// Per-chunk buffers rather than one buffer for the level: a level-sized
/// index buffer runs past `max_buffer_size` on WebGL-class limits long
/// before the geometry itself is unreasonable (Fostral at quality 0.75 is
/// 119 MB of vertices and 177 MB of indices, but only ~57 KB + ~84 KB per
/// chunk per LOD). It also makes a refit after a terrain edit trivial -
/// rebuild two small buffers - so there is no slot capacity, no degenerate
/// padding, and no relayout when a chunk outgrows its space.
pub struct ChunkBuffers {
    pub vertices: Vec<MeshVertex>,
    /// Indices into this chunk's own vertex buffer.
    pub indices: Vec<u32>,
    /// `(first index, index count)` per LOD, finest first.
    pub lods: Vec<(u32, u32)>,
    /// Chunk centre in texels, for distance-based LOD selection.
    pub center: [f32; 2],
    /// World bounding box, for frustum culling. Covers every layer, so it
    /// holds whatever the chunk emits.
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl ChunkBuffers {
    fn new(state: &ChunkState, per_lod: Vec<ChunkMesh>) -> Self {
        let mut buffers = ChunkBuffers {
            vertices: Vec::new(),
            indices: Vec::new(),
            lods: Vec::with_capacity(per_lod.len()),
            center: [
                state.x0 as f32 + state.w as f32 * 0.5,
                state.y0 as f32 + state.h as f32 * 0.5,
            ],
            // Vertices sit at texel centres, so the chunk spans
            // `[x0 + 0.5, x0 + w + 0.5]`; pad by half a texel either way
            // rather than tracking that exactly.
            min: [state.x0 as f32, state.y0 as f32, state.alt.0],
            max: [
                (state.x0 + state.w as i32) as f32 + 1.0,
                (state.y0 + state.h as i32) as f32 + 1.0,
                state.alt.1,
            ],
        };
        for mesh in per_lod {
            let base = buffers.vertices.len() as u32;
            let first = buffers.indices.len() as u32;
            buffers
                .indices
                .extend(mesh.indices.iter().map(|i| i + base));
            buffers.vertices.extend(mesh.vertices);
            buffers
                .lods
                .push((first, buffers.indices.len() as u32 - first));
        }
        buffers
    }
}

struct ChunkState {
    x0: i32,
    y0: i32,
    w: u32,
    h: u32,
    /// Altitude span over the chunk, for the culling bounding box.
    alt: (f32, f32),
    lods: Vec<LodState>,
}

struct LodState {
    tri: Chunk,
}

impl ChunkState {
    /// Chunks own the texels `[x0 ..= x0 + w]`, i.e. they share their border
    /// row/column with the neighbour. That overlap is what makes an edit on
    /// a border line dirty *both* chunks, which is exactly what keeps the
    /// seam consistent.
    fn overlaps(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        self.x0 <= x + w
            && x <= self.x0 + self.w as i32
            && self.y0 <= y + h
            && y <= self.y0 + self.h as i32
    }
}

/// Chunks whose geometry changed, by index, with their new buffers.
pub type Update = Vec<(usize, ChunkBuffers)>;

/// A live TIN: the triangulations stay resident so terrain edits can refine
/// the mesh instead of rebuilding it.
///
/// The sample grids are *not* kept - at 129x129 samples per chunk they would
/// dwarf everything else on a full level - so they are re-derived from the
/// `Level` for whichever chunks an edit touches. The triangulations
/// themselves are small (grid indices plus adjacency).
pub struct Tin {
    max_error: f32,
    quality: f32,
    chunks: Vec<ChunkState>,
    pub stats: Stats,
}

impl Tin {
    pub fn build(level: &Level, config: &Config) -> (Self, Mesh) {
        profiling::scope!("Build Terrain TIN");

        let max_error = config.max_error(level);
        let step = CHUNK_SIZE as i32;
        // Chunks are clipped to the level: a level smaller than `CHUNK_SIZE`,
        // or a trailing chunk, must not extend past the edge. `Level::get`
        // wraps, so an oversized chunk would quietly mesh the level twice.
        let spans_of = |total: i32| {
            let mut spans = Vec::new();
            let mut at = 0;
            while at < total {
                spans.push((at, (total - at).min(step) as u32));
                at += step;
            }
            spans
        };
        let origins = {
            let mut v = Vec::new();
            for &(y, h) in &spans_of(level.size.1) {
                for &(x, w) in &spans_of(level.size.0) {
                    v.push((x, y, w, h));
                }
            }
            v
        };

        let build_one = |&(x, y, w, h): &(i32, i32, u32, u32)| {
            let grid = Grid::new(level, x, y, w, h);
            // Each LOD is an independent fit at a doubled tolerance. They
            // could share work - the coarse vertex sets are prefixes of the
            // fine one - but refitting from scratch is cheap (the coarse
            // levels converge in a fraction of the insertions) and keeps
            // every level a genuine Delaunay triangulation.
            let alt = grid.altitude_range();
            let mut lods = Vec::with_capacity(LOD_COUNT);
            let mut meshes = Vec::with_capacity(LOD_COUNT);
            for k in 0..LOD_COUNT {
                let mut chunk = Chunk::new(&grid);
                refine(&mut chunk, &grid, max_error * (1 << k) as f32);
                meshes.push(emit_chunk(&chunk, &grid));
                lods.push(LodState { tri: chunk });
            }
            (
                ChunkState {
                    x0: x,
                    y0: y,
                    w,
                    h,
                    alt,
                    lods,
                },
                meshes,
            )
        };

        #[cfg(not(target_arch = "wasm32"))]
        let built: Vec<(ChunkState, Vec<ChunkMesh>)> = {
            use rayon::prelude::*;
            origins.par_iter().map(build_one).collect()
        };
        #[cfg(target_arch = "wasm32")]
        let built: Vec<(ChunkState, Vec<ChunkMesh>)> = origins.iter().map(build_one).collect();

        let (chunks, meshes): (Vec<_>, Vec<_>) = built.into_iter().unzip();
        let mesh = assemble(&chunks, meshes, level, max_error);

        let tin = Tin {
            max_error,
            quality: config.quality,
            chunks,
            stats: mesh.stats,
        };
        tin.log("built");
        (tin, mesh)
    }

    fn log(&self, what: &str) {
        info!(
            "Terrain TIN {} at quality {}: {} vertices, {} triangles from {} texels \
             ({:.1}x fewer triangles, max error {:.2})",
            what,
            self.quality,
            self.stats.vertices,
            self.stats.triangles,
            self.stats.source_texels,
            2.0 * self.stats.source_texels as f32 / self.stats.triangles.max(1) as f32,
            self.stats.max_error,
        );
    }

    /// Re-fit the mesh after the level changed inside the given texel rect.
    ///
    /// Only vertices are added: the triangulation is planar in XY and
    /// altitudes are plain attributes, so an edit that merely raises or
    /// lowers the surface re-emits with no topology change at all. New
    /// vertices appear only where the edit introduced detail the existing
    /// triangles cannot represent.
    pub fn update(&mut self, level: &Level, x: i32, y: i32, w: i32, h: i32) -> Update {
        profiling::scope!("Update Terrain TIN");

        let max_error = self.max_error;
        let mut changed = Vec::new();
        for (index, state) in self.chunks.iter_mut().enumerate() {
            if !state.overlaps(x, y, w, h) {
                continue;
            }
            let grid = Grid::new(level, state.x0, state.y0, state.w, state.h);
            state.alt = grid.altitude_range();
            let mut per_lod = Vec::with_capacity(state.lods.len());
            for (k, lod) in state.lods.iter_mut().enumerate() {
                refine(&mut lod.tri, &grid, max_error * (1 << k) as f32);
                per_lod.push(emit_chunk(&lod.tri, &grid));
            }
            changed.push((index, ChunkBuffers::new(state, per_lod)));
        }
        changed
    }
}

/// Pack every chunk into its own buffers and total up the statistics.
fn assemble(
    chunks: &[ChunkState],
    meshes: Vec<Vec<ChunkMesh>>,
    level: &Level,
    max_error: f32,
) -> Mesh {
    let mut stats = Stats {
        source_texels: (level.size.0 as usize) * (level.size.1 as usize),
        max_error,
        ..Default::default()
    };
    let packed = chunks
        .iter()
        .zip(meshes)
        .map(|(state, per_lod)| {
            // Stats describe LOD 0 - the mesh as actually drawn up close.
            if let Some(fine) = per_lod.first() {
                stats.vertices += fine.vertices.len();
                stats.triangles += fine.indices.len() / 3;
            }
            ChunkBuffers::new(state, per_lod)
        })
        .collect();
    Mesh {
        chunks: packed,
        stats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_grid(nx: u32, ny: u32) -> Grid {
        Grid {
            samples: vec![Sample::default(); (nx * ny) as usize],
            nx,
            ny,
            x0: 0,
            y0: 0,
        }
    }

    /// Every alive triangle must be CCW, and adjacency must be symmetric:
    /// if `t` names `n` across an edge, `n` must name `t` back across the
    /// same (reversed) edge.
    fn check_invariants(chunk: &Chunk, grid: &Grid) {
        for (t, tri) in chunk.tris.iter().enumerate() {
            if !tri.alive {
                continue;
            }
            let (a, b, c) = (
                chunk.pos(grid, tri.v[0]),
                chunk.pos(grid, tri.v[1]),
                chunk.pos(grid, tri.v[2]),
            );
            assert!(orient2d(a, b, c) > 0, "triangle {} is not CCW", t);

            for i in 0..3 {
                let nb = tri.n[i];
                if nb == NONE {
                    continue;
                }
                let other = &chunk.tris[nb as usize];
                assert!(other.alive, "triangle {} links to a dead neighbour", t);
                let (e0, e1) = (tri.v[i], tri.v[(i + 1) % 3]);
                let found = (0..3).any(|j| {
                    other.v[j] == e1 && other.v[(j + 1) % 3] == e0 && other.n[j] == t as u32
                });
                assert!(found, "adjacency of {} across edge {} is not mutual", t, i);
            }
        }
    }

    /// Delaunay's defining property: no vertex inside any circumcircle.
    fn check_delaunay(chunk: &Chunk, grid: &Grid) {
        for tri in chunk.tris.iter().filter(|t| t.alive) {
            let (a, b, c) = (
                chunk.pos(grid, tri.v[0]),
                chunk.pos(grid, tri.v[1]),
                chunk.pos(grid, tri.v[2]),
            );
            for v in 0..chunk.verts.len() as u32 {
                if tri.v.contains(&v) {
                    continue;
                }
                assert!(
                    in_circle(a, b, c, chunk.pos(grid, v)) <= 0,
                    "vertex {} violates the empty-circumcircle property",
                    v
                );
            }
        }
    }

    /// Rolling hills with a rectangular double-level slab punched into the
    /// middle, so the emitted mesh has to exercise the ceiling and the
    /// walls closing the slab off.
    fn make_level(size: i32) -> Level {
        make_level_wh(size, size)
    }

    fn make_level_wh(w: i32, h: i32) -> Level {
        use crate::level::{DOUBLE_LEVEL, TerrainConfig};

        let total = (w * h) as usize;
        let mut height = vec![0u8; total];
        let mut meta = vec![0u8; total];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                let alt = 100.0 + 40.0 * (x as f32 * 0.2).sin() + 30.0 * (y as f32 * 0.15).sin();
                height[i] = alt as u8;
                meta[i] = 2 << 3;
            }
        }
        // Dual data lives in texel *pairs*: the even texel holds `low`, the
        // odd one `high`, and the delta bits of both make up `mid`.
        let (x_lo, x_hi) = (w / 4, w / 2);
        let (y_lo, y_hi) = (h / 4, h / 2);
        for y in y_lo..y_hi {
            let mut x = x_lo & !1;
            while x < x_hi {
                let i0 = (y * w + x) as usize;
                height[i0] = 60;
                height[i0 + 1] = 180;
                meta[i0] = DOUBLE_LEVEL | (2 << 3) | 0x2;
                meta[i0 + 1] = DOUBLE_LEVEL | (5 << 3) | 0x1;
                x += 2;
            }
        }

        Level {
            size: (w, h),
            flood_map: vec![0; 1].into_boxed_slice(),
            height: height.into_boxed_slice(),
            meta: meta.into_boxed_slice(),
            palette: [[0; 4]; 0x100],
            terrains: vec![TerrainConfig::default(); 8].into_boxed_slice(),
            geometry: crate::config::settings::Geometry {
                height: 0x100,
                delta_mask: 0xFFFF,
                delta_power: 3,
                delta_const: 1,
            },
        }
    }

    #[test]
    fn build_approximates_a_level_with_far_fewer_triangles() {
        let size = 64;
        let level = make_level(size);
        let config = Config { quality: 1.0 };
        let (_tin, mesh) = Tin::build(&level, &config);

        assert!(mesh.stats.triangles > 0);
        for c in &mesh.chunks {
            assert_eq!(c.indices.len() % 3, 0);
            assert!(c.indices.iter().all(|&i| (i as usize) < c.vertices.len()));
        }
        // A full grid mesh would be 2 triangles per texel; we must beat it
        // comfortably even on this deliberately bumpy input.
        let full = 2 * mesh.stats.source_texels;
        assert!(
            mesh.stats.triangles < full / 4,
            "{} triangles vs {} for a full grid",
            mesh.stats.triangles,
            full
        );

        // This level is smaller than `CHUNK_SIZE`, so it also pins down that
        // chunks get clipped to the level. `Level::get` wraps, so an
        // unclipped chunk would quietly mesh the level several times over.
        assert!(size < CHUNK_SIZE as i32);
        let span = size as f32 + 1.0;
        assert!(
            mesh.chunks
                .iter()
                .flat_map(|c| c.vertices.iter())
                .all(|v| v.pos[0] < span && v.pos[1] < span),
            "the mesh spilled outside the level bounds"
        );
    }

    #[test]
    fn dual_regions_emit_a_slab_ceiling_and_walls() {
        let level = make_level(64);
        let config = Config { quality: 1.0 };
        let (_tin, mesh) = Tin::build(&level, &config);

        let verts: Vec<MeshVertex> = mesh
            .chunks
            .iter()
            .flat_map(|c| c.vertices.clone())
            .collect();
        let has_z = |z: f32| verts.iter().any(|v| (v.pos[2] - z).abs() < 0.5);
        // low = 60, mid = 60 + (9 << 3) = 132, high = 180.
        assert!(has_z(60.0), "missing the cave floor");
        assert!(has_z(132.0), "missing the slab ceiling");
        assert!(has_z(180.0), "missing the slab top");

        // A wall needs the same XY to appear at both `mid` and `high`.
        let wall = verts.iter().any(|a| {
            (a.pos[2] - 180.0).abs() < 0.5
                && verts.iter().any(|b| {
                    b.pos[0] == a.pos[0] && b.pos[1] == a.pos[1] && (b.pos[2] - 132.0).abs() < 0.5
                })
        });
        assert!(wall, "the slab was never closed off with a vertical wall");
    }

    #[test]
    fn adjacent_chunks_agree_on_their_shared_border() {
        // The seam is crack-free only if both chunks independently derive
        // the same vertices along the column they share.
        let level = make_level(64);
        let size = 16u32;
        let left = Grid::new(&level, 0, 0, size, size);
        let right = Grid::new(&level, size as i32, 0, size, size);

        let column = |grid: &Grid, lx: u32| -> Vec<Sample> {
            (0..grid.ny)
                .map(|y| *grid.sample(grid.index(lx, y)))
                .collect()
        };
        let a = column(&left, size); // left chunk's right edge
        let b = column(&right, 0); // right chunk's left edge
        assert!(
            a.iter().zip(&b).all(|(x, y)| x.low == y.low
                && x.mid == y.mid
                && x.high == y.high
                && x.is_dual == y.is_dual),
            "the shared column must sample identically from both sides"
        );

        let mut picks_a = Vec::new();
        simplify_line(&a, 1.0, &mut picks_a);
        let mut picks_b = Vec::new();
        simplify_line(&b, 1.0, &mut picks_b);
        assert_eq!(picks_a, picks_b);
    }

    #[test]
    fn build_is_deterministic() {
        let level = make_level(48);
        let config = Config { quality: 1.0 };
        let (_, a) = Tin::build(&level, &config);
        let (_, b) = Tin::build(&level, &config);
        assert_eq!(a.chunks.len(), b.chunks.len());
        for (ca, cb) in a.chunks.iter().zip(&b.chunks) {
            assert_eq!(ca.indices, cb.indices);
            assert_eq!(ca.vertices.len(), cb.vertices.len());
            assert!(
                ca.vertices
                    .iter()
                    .zip(&cb.vertices)
                    .all(|(x, y)| x.pos == y.pos && x.layer == y.layer)
            );
        }
    }

    /// Lower a round pit into the level, returning the dirty rect. Steep
    /// enough that the existing triangulation cannot represent it.
    fn dig(level: &mut Level, cx: i32, cy: i32, r: i32) -> (i32, i32, i32, i32) {
        for y in (cy - r)..(cy + r) {
            for x in (cx - r)..(cx + r) {
                let (dx, dy) = ((x - cx) as f32, (y - cy) as f32);
                let d2 = dx * dx + dy * dy;
                if d2 > (r * r) as f32 {
                    continue;
                }
                let i = (y * level.size.0 + x) as usize;
                let depth = (60.0 * (1.0 - d2 / (r * r) as f32)) as u8;
                level.height[i] = level.height[i].saturating_sub(depth);
            }
        }
        (cx - r, cy - r, 2 * r, 2 * r)
    }

    /// Worst residual error of the whole TIN against the current level.
    fn worst_error(tin: &Tin, level: &Level) -> f32 {
        let mut worst = 0.0f32;
        for st in &tin.chunks {
            let grid = Grid::new(level, st.x0, st.y0, st.w, st.h);
            let mut c = st.lods[0].tri.clone();
            for t in 0..c.tris.len() as u32 {
                if c.tris[t as usize].alive {
                    c.compute_candidate(&grid, t);
                    worst = worst.max(c.tris[t as usize].err);
                }
            }
        }
        worst
    }

    /// Every chunk's vertices, in global texel coordinates.
    fn vertex_sets(tin: &Tin) -> Vec<std::collections::HashSet<(i32, i32)>> {
        tin.chunks
            .iter()
            .map(|st| {
                let nx = st.w + 1;
                st.lods[0]
                    .tri
                    .verts
                    .iter()
                    .map(|&gi| (st.x0 + (gi % nx) as i32, st.y0 + (gi / nx) as i32))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn edits_only_ever_add_vertices() {
        let mut level = make_level(96);
        let config = Config { quality: 0.5 };
        let (mut tin, _) = Tin::build(&level, &config);
        let before = vertex_sets(&tin);

        let (x, y, w, h) = dig(&mut level, 70, 70, 12);
        tin.update(&level, x, y, w, h);
        let after = vertex_sets(&tin);

        for (i, (old, new)) in before.iter().zip(&after).enumerate() {
            assert!(
                old.is_subset(new),
                "chunk {} lost vertices: {:?}",
                i,
                old.difference(new).collect::<Vec<_>>()
            );
        }
        assert!(
            after.iter().map(|s| s.len()).sum::<usize>()
                > before.iter().map(|s| s.len()).sum::<usize>(),
            "the pit should have needed new vertices"
        );
    }

    #[test]
    fn edits_refit_the_surface_to_tolerance() {
        let mut level = make_level(96);
        let config = Config { quality: 0.5 };
        let (mut tin, _) = Tin::build(&level, &config);
        let tolerance = tin.max_error;
        assert!(worst_error(&tin, &level) <= tolerance);

        let (x, y, w, h) = dig(&mut level, 70, 70, 12);
        // Without the update the mesh is now badly wrong ...
        assert!(worst_error(&tin, &level) > tolerance * 4.0);
        // ... and refitting brings it back inside tolerance.
        tin.update(&level, x, y, w, h);
        let worst = worst_error(&tin, &level);
        assert!(worst <= tolerance, "residual error {} after update", worst);
    }

    #[test]
    fn a_pure_height_shift_needs_no_new_geometry() {
        // The triangulation is planar in XY and altitudes are plain vertex
        // attributes, so translating the surface must re-emit identically.
        let mut level = make_level(96);
        let config = Config { quality: 0.5 };
        let (mut tin, mesh) = Tin::build(&level, &config);
        let before = mesh.stats.triangles;

        for h in level.height.iter_mut() {
            *h = h.saturating_sub(3);
        }
        let changed = tin.update(&level, 0, 0, level.size.0, level.size.1);
        assert!(!changed.is_empty());
        assert_eq!(tin.stats.triangles, before);
    }

    #[test]
    fn edits_keep_chunk_seams_consistent() {
        // Two chunks wide, with the edit straddling the shared column.
        let mut level = make_level(CHUNK_SIZE as i32 + 32);
        let config = Config { quality: 0.5 };
        let (mut tin, _) = Tin::build(&level, &config);
        assert!(tin.chunks.len() > 1, "need more than one chunk");

        let seam = CHUNK_SIZE as i32;
        let (x, y, w, h) = dig(&mut level, seam, 60, 14);
        tin.update(&level, x, y, w, h);

        // Compare only chunks that actually abut: same row, and the left
        // one's right border is the right one's left border. The seam is
        // crack-free exactly when they agree on that column.
        let sets = vertex_sets(&tin);
        let mut compared = 0;
        for (i, a) in tin.chunks.iter().enumerate() {
            for (j, b) in tin.chunks.iter().enumerate() {
                if a.y0 != b.y0 || b.x0 != a.x0 + a.w as i32 {
                    continue;
                }
                let column = b.x0;
                let on_seam = |set: &std::collections::HashSet<(i32, i32)>| {
                    set.iter()
                        .filter(|&&(vx, _)| vx == column)
                        .map(|&(_, vy)| vy)
                        .collect::<std::collections::BTreeSet<_>>()
                };
                assert_eq!(
                    on_seam(&sets[i]),
                    on_seam(&sets[j]),
                    "chunks disagree on the shared column {} - that is a crack",
                    column
                );
                compared += 1;
            }
        }
        assert!(compared > 0, "no abutting chunks were compared");
    }

    #[test]
    fn a_drastic_edit_relays_out_and_stays_correct() {
        let mut level = make_level(96);
        let config = Config { quality: 0.5 };
        let (mut tin, _) = Tin::build(&level, &config);

        // Shred a region into per-texel noise. That needs far more detail
        // than the slot headroom can absorb, so it must fall back to a
        // relayout rather than silently truncating.
        for y in 20..80 {
            for x in 20..80 {
                let i = (y * level.size.0 + x) as usize;
                level.height[i] = if (x + y) % 2 == 0 { 40 } else { 200 };
            }
        }
        let changed = tin.update(&level, 20, 20, 60, 60);
        assert!(!changed.is_empty(), "the edit should have refitted a chunk");

        // Refitted buffers have to be self-consistent: every index in
        // range, whole triangles, and the surface back within tolerance.
        for (_, c) in &changed {
            assert_eq!(c.indices.len() % 3, 0);
            assert!(c.indices.iter().all(|&i| (i as usize) < c.vertices.len()));
        }
        let worst = worst_error(&tin, &level);
        assert!(worst <= tin.max_error, "residual error {}", worst);

        // And a further edit must still leave the surface fitted.
        let (x, y, w, h) = dig(&mut level, 85, 85, 6);
        tin.update(&level, x, y, w, h);
        let worst = worst_error(&tin, &level);
        assert!(
            worst <= tin.max_error,
            "residual error {} after refit",
            worst
        );
    }

    #[test]
    fn edits_outside_a_chunk_leave_it_alone() {
        let mut level = make_level(CHUNK_SIZE as i32 + 32);
        let config = Config { quality: 0.5 };
        let (mut tin, _) = Tin::build(&level, &config);
        let before = vertex_sets(&tin);

        // Well inside the first chunk, far from any border.
        let (x, y, w, h) = dig(&mut level, 40, 40, 10);
        let touched = tin.update(&level, x, y, w, h).len();
        assert!(
            touched < tin.chunks.len(),
            "a local edit touched every one of {} chunks",
            tin.chunks.len()
        );

        let after = vertex_sets(&tin);
        let changed = before
            .iter()
            .zip(&after)
            .filter(|(a, b)| a.len() != b.len())
            .count();
        assert!(changed >= 1, "the edit should have refined something");
        assert!(changed < tin.chunks.len(), "it should not have refined all");
    }

    #[test]
    fn predicates_are_exact_on_the_grid() {
        // The four corners of a unit square are cocircular - the degenerate
        // case that eats floating-point predicates.
        let (a, b, c, d) = ([0, 0], [1, 0], [1, 1], [0, 1]);
        assert!(orient2d(a, b, c) > 0);
        assert_eq!(in_circle(a, b, c, d), 0);
        assert!(in_circle(a, b, c, [1, 0]) <= 0);
        // Centre of the square is strictly inside.
        assert!(in_circle([0, 0], [2, 0], [2, 2], [1, 1]) > 0);
    }

    #[test]
    fn insertion_keeps_a_valid_delaunay_triangulation() {
        let grid = flat_grid(9, 9);
        let mut chunk = Chunk::new(&grid);
        check_invariants(&chunk, &grid);

        // A deterministic scatter of interior points, plus points landing
        // exactly on the hull edges to exercise the collinear path.
        let interior = [(3, 4), (5, 2), (7, 6), (1, 1), (4, 7), (6, 3), (2, 6)];
        for &(x, y) in &interior {
            let gi = grid.index(x, y);
            let seed = chunk.locate(&grid, grid.coord(gi), 0);
            chunk.insert(&grid, gi, seed);
            check_invariants(&chunk, &grid);
        }
        for &(x, y) in &[(4, 0), (0, 5), (8, 3), (6, 8)] {
            let gi = grid.index(x, y);
            let seed = chunk.locate(&grid, grid.coord(gi), 0);
            chunk.insert(&grid, gi, seed);
            check_invariants(&chunk, &grid);
        }
        check_delaunay(&chunk, &grid);
        assert_eq!(chunk.verts.len(), 4 + interior.len() + 4);
    }

    #[test]
    fn triangulation_covers_the_whole_chunk() {
        let grid = flat_grid(7, 7);
        let mut chunk = Chunk::new(&grid);
        for &(x, y) in &[(2, 3), (5, 1), (3, 5), (1, 4), (4, 2)] {
            let gi = grid.index(x, y);
            let seed = chunk.locate(&grid, grid.coord(gi), 0);
            chunk.insert(&grid, gi, seed);
        }
        // Total area must still be the full rectangle: no gaps, no overlaps.
        let total: i64 = chunk
            .tris
            .iter()
            .filter(|t| t.alive)
            .map(|t| {
                orient2d(
                    chunk.pos(&grid, t.v[0]),
                    chunk.pos(&grid, t.v[1]),
                    chunk.pos(&grid, t.v[2]),
                )
            })
            .sum();
        assert_eq!(total, 2 * 6 * 6);
    }

    #[test]
    fn line_simplification_is_deterministic_and_within_tolerance() {
        let mut samples = vec![Sample::default(); 33];
        for (i, s) in samples.iter_mut().enumerate() {
            let h = ((i as f32) * 0.7).sin() * 20.0;
            s.low = h;
            s.mid = h;
            s.high = h;
        }
        let mut a = Vec::new();
        simplify_line(&samples, 1.0, &mut a);
        let mut b = Vec::new();
        simplify_line(&samples, 1.0, &mut b);
        assert_eq!(a, b, "must be a pure function of the samples");
        assert!(!a.is_empty());

        // Everything the simplification kept must now be within tolerance.
        let mut kept = a.clone();
        kept.push(0);
        kept.push(samples.len() as u32 - 1);
        kept.sort_unstable();
        for w in kept.windows(2) {
            let (lo, hi) = (w[0], w[1]);
            let span = (hi - lo) as f32;
            for i in lo + 1..hi {
                let t = (i - lo) as f32 / span;
                let interp = samples[lo as usize].low
                    + (samples[hi as usize].low - samples[lo as usize].low) * t;
                assert!((samples[i as usize].low - interp).abs() <= 1.0 + 1e-4);
            }
        }
    }

    #[test]
    fn greedy_insertion_reaches_the_error_target() {
        // A ridge that a two-triangle approximation cannot represent.
        let size = 32u32;
        let n = size + 1;
        let mut grid = flat_grid(n, n);
        for y in 0..n {
            for x in 0..n {
                let fx = x as f32 / size as f32;
                let h = if fx < 0.5 { fx } else { 1.0 - fx } * 60.0;
                let s = &mut grid.samples[(y * n + x) as usize];
                s.low = h;
                s.mid = h;
                s.high = h;
            }
        }
        let mut chunk = Chunk::new(&grid);
        for t in 0..chunk.tris.len() as u32 {
            chunk.compute_candidate(&grid, t);
        }
        let worst_before = chunk
            .tris
            .iter()
            .filter(|t| t.alive)
            .fold(0.0f32, |acc, t| acc.max(t.err));
        assert!(worst_before > 10.0);

        for _ in 0..400 {
            let mut t = NONE;
            let mut err = 0.0f32;
            let mut cand = NONE;
            for (i, tri) in chunk.tris.iter().enumerate() {
                if tri.alive && tri.cand != NONE && tri.err > err {
                    t = i as u32;
                    err = tri.err;
                    cand = tri.cand;
                }
            }
            if t == NONE || err <= 1.0 {
                break;
            }
            for slot in chunk.insert(&grid, cand, t) {
                chunk.compute_candidate(&grid, slot);
            }
        }

        check_invariants(&chunk, &grid);
        let worst = chunk
            .tris
            .iter()
            .filter(|t| t.alive)
            .fold(0.0f32, |acc, t| acc.max(t.err));
        assert!(worst <= 1.0, "worst residual error was {}", worst);
        // The ridge is a fold, so it needs far fewer than all 33x33 samples.
        assert!(
            chunk.verts.len() < 200,
            "used {} vertices",
            chunk.verts.len()
        );
    }
}
