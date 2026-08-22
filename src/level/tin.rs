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
/// Terrain type is sampled from the height texture in the fragment shader,
/// so material boundaries stay at full texel resolution on coarse
/// triangles. Lighting uses the triangle's geometric normal
/// (`evaluate_color_normal` in `terrain/color.inc.wgsl`) rather than a
/// height-field gradient, which is undefined on vertical walls and cave
/// ceilings. All the vertex has to say is *which* of the stacked surfaces
/// it belongs to.
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
pub const CHUNK_SIZE: u32 = 128;

/// How far `mid` or `high` may vary across one slab triangle, in altitude
/// units, before the slab is dropped rather than ramped across the step.
const SLAB_STEP: f32 = 8.0;

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
    /// Triangles emitted for the double-level slab (top, ceiling, walls).
    pub slab_triangles: usize,
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

impl From<Texel> for Sample {
    fn from(texel: Texel) -> Self {
        match texel {
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
}

impl Sample {
    /// Largest deviation of any layer from the given interpolated values.
    /// This is the "insert if *any* layer needs it" criterion.
    ///
    /// Measured cost of the criterion on Fostral at quality 0.25, by
    /// counting what drove each insertion: 53.5% `low`, 18.8% the slab
    /// interior, 4.4% chunk-border simplification, and 23.3% the boundary
    /// between single- and double-level regions.
    ///
    /// That last share is waste. A single-level texel reports
    /// `mid = high = low`, so both step by the full slab thickness across a
    /// region's edge, and this metric then chases a discontinuity that no
    /// tolerance can satisfy - it only shrinks triangles toward texel size
    /// along every boundary. Its absolute cost is near-constant in quality
    /// (338k insertions at 0, 396k at 1) while `low`'s grows 444k -> 6.1M,
    /// which is a fixed geometric feature rather than a fit converging.
    /// Constraining the triangulation to the `DOUBLE_LEVEL` outline would
    /// remove it, and is the same change that would stop `is_slab` from
    /// dropping the slab on straddling triangles.
    #[inline(always)]
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
#[inline(always)]
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

/// Incremental edge functions for the triangle `abc`.
///
/// `at` is the three `orient2d` values at some starting sample, and
/// `step_x`/`step_y` are what they change by when the sample moves one
/// texel. A sample lies inside the triangle exactly when all three are
/// non-negative, so scanning a bounding box costs three adds a sample
/// rather than three cross products - and because `orient2d` is exact
/// integer arithmetic, the stepped values are bit for bit the ones the
/// direct form produces.
///
/// Both bounding-box scans in this module - the fit's error search and the
/// emitter's slab test - are hot enough to want this.
struct Edges {
    at: [i64; 3],
    step_x: [i64; 3],
    step_y: [i64; 3],
}

impl Edges {
    fn new(a: [i32; 2], b: [i32; 2], c: [i32; 2], start: [i32; 2]) -> Self {
        Edges {
            at: [
                orient2d(b, c, start),
                orient2d(c, a, start),
                orient2d(a, b, start),
            ],
            step_x: [
                -((c[1] - b[1]) as i64),
                -((a[1] - c[1]) as i64),
                -((b[1] - a[1]) as i64),
            ],
            step_y: [
                (c[0] - b[0]) as i64,
                (a[0] - c[0]) as i64,
                (b[0] - a[0]) as i64,
            ],
        }
    }

    /// A row meets a triangle in a single interval, so a scan that has
    /// been inside and is now out can stop rather than walk the rest of
    /// the bounding box.
    #[inline(always)]
    fn inside(w: [i64; 3]) -> bool {
        w[0] >= 0 && w[1] >= 0 && w[2] >= 0
    }

    #[inline(always)]
    fn advance(w: &mut [i64; 3], step: [i64; 3]) {
        for i in 0..3 {
            w[i] += step[i];
        }
    }
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
    /// Lowest floor and highest slab top over the whole chunk, folded in
    /// while the samples were read.
    alt: (f32, f32),
    nx: u32,
    ny: u32,
    x0: i32,
    y0: i32,
}

impl Grid {
    fn new(level: &Level, x0: i32, y0: i32, w: u32, h: u32) -> Self {
        let nx = w + 1;
        let ny = h + 1;
        // Reading the rectangle a texel at a time re-derives the level's
        // constants and wraps both coordinates every time. The columns wrap
        // the same way on every row, so resolve them once.
        let bits = level.terrain_bits();
        let altitude_scale = level.altitude_scale();
        let columns = (0..nx)
            .map(|lx| (x0 + lx as i32).rem_euclid(level.size.0) as usize)
            .collect::<Vec<_>>();
        let mut samples = Vec::with_capacity((nx * ny) as usize);
        let mut alt = (f32::MAX, f32::MIN);
        for ly in 0..ny {
            let row = (y0 + ly as i32).rem_euclid(level.size.1) as usize * level.size.0 as usize;
            for &column in columns.iter() {
                let sample = Sample::from(level.get_wrapped(row + column, bits, altitude_scale));
                alt = (alt.0.min(sample.low), alt.1.max(sample.high));
                samples.push(sample);
            }
        }
        Grid {
            samples,
            alt,
            nx,
            ny,
            x0,
            y0,
        }
    }

    #[inline(always)]
    fn index(&self, lx: u32, ly: u32) -> u32 {
        ly * self.nx + lx
    }

    #[inline(always)]
    fn coord(&self, index: u32) -> [i32; 2] {
        [(index % self.nx) as i32, (index / self.nx) as i32]
    }

    #[inline(always)]
    fn sample(&self, index: u32) -> &Sample {
        &self.samples[index as usize]
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
    #[inline(always)]
    fn pos(&self, grid: &Grid, v: u32) -> [i32; 2] {
        grid.coord(self.verts[v as usize])
    }

    #[inline(always)]
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
        // A grid is at most a chunk plus its borders, so `orient2d` stays
        // far inside the 24 bits that `f32` represents exactly: every edge
        // value below, and `area2` itself, converts without loss.
        let inv = 1.0 / area2 as f32;

        let min_x = a[0].min(b[0]).min(c[0]).max(0);
        let max_x = a[0].max(b[0]).max(c[0]).min(grid.nx as i32 - 1);
        let min_y = a[1].min(b[1]).min(c[1]).max(0);
        let max_y = a[1].max(b[1]).max(c[1]).min(grid.ny as i32 - 1);

        // This is the hot loop of the whole TIN: a refit measures it once
        // per triangle over the triangle's bounding box.
        let edges = Edges::new(a, b, c, [min_x, min_y]);
        let mut row = edges.at;

        // Barycentric blend of the three vertex samples, for weights that
        // need not be normalized. Fed the exact edge values it interpolates
        // the plane at that sample; fed an edge step it gives what the plane
        // itself steps by, which is constant because the plane is affine.
        // That turns the interpolation into three adds a sample rather than
        // twelve multiplies.
        //
        // A row only accumulates across the run of samples inside the
        // triangle, seeded from the exact weights at its start: starting from
        // the bounding box corner instead would carry the rounding of a
        // sliver triangle's plane there, which can be enormous, into the
        // small values along the sliver itself.
        let blend = |w: [i64; 3]| {
            [
                (w[0] as f32 * sa.low + w[1] as f32 * sb.low + w[2] as f32 * sc.low) * inv,
                (w[0] as f32 * sa.mid + w[1] as f32 * sb.mid + w[2] as f32 * sc.mid) * inv,
                (w[0] as f32 * sa.high + w[1] as f32 * sb.high + w[2] as f32 * sc.high) * inv,
            ]
        };
        let plane_x = blend(edges.step_x);

        let mut best = NONE;
        let mut best_err = 0.0f32;
        for y in min_y..=max_y {
            let mut w = row;
            let mut plane = [0.0f32; 3];
            let mut inside = false;
            let start = grid.index(min_x as u32, y as u32);
            for gi in start..=start + (max_x - min_x) as u32 {
                if Edges::inside(w) {
                    if inside {
                        for i in 0..3 {
                            plane[i] += plane_x[i];
                        }
                    } else {
                        plane = blend(w);
                        inside = true;
                    }
                    let err = grid.sample(gi).deviation(plane[0], plane[1], plane[2]);
                    if err > best_err {
                        best_err = err;
                        best = gi;
                    }
                } else if inside {
                    break;
                }
                Edges::advance(&mut w, edges.step_x);
            }
            Edges::advance(&mut row, edges.step_y);
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
    /// How many of `indices` belong to slab geometry, for diagnostics.
    slab_indices: usize,
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
    // A triangle carries a slab only if it lies *entirely* inside the
    // double-level region.
    //
    // Testing just the three corners is not enough. Real levels have
    // fragmented dual regions - Fostral is 10.9% double-level with ragged
    // outlines - so a coarse triangle can have all three corners dual while
    // spanning terrain that mostly is not. Interpolating `high` across that
    // gap produced a roof sloping up to a distant vertex's slab, with tall
    // vertical curtains where its edges were walled off: on a river view it
    // covered the entire sky.
    //
    // Erring the other way merely drops the slab near a region's edge,
    // which reads as a gap in a tunnel roof rather than a wall through the
    // horizon. Properly fixing it means constraining the triangulation to
    // the `DOUBLE_LEVEL` outline so no triangle straddles it at all.
    let is_slab = |tri: &Tri| -> bool {
        // Cheapest rejections first. Rasterising the triangle to find a
        // non-dual sample inside it is far more work than noticing that a
        // corner is already non-dual, and on a level that is a tenth
        // double-level almost every triangle ends here.
        let (va, vb, vc) = (
            chunk.sample(grid, tri.v[0]),
            chunk.sample(grid, tri.v[1]),
            chunk.sample(grid, tri.v[2]),
        );
        if !va.is_dual || !vb.is_dual || !vc.is_dual {
            return false;
        }
        // `mid` and `high` are structural, not smooth: a cave ceiling can
        // jump tens of units between neighbouring texels. Interpolating
        // across such a step builds a sloped roof that exists nowhere in
        // the data, and a near-vertical one where the step is sharp. The
        // low surface has no such problem, which is why only the slab is
        // gated here.
        let spread = |x: f32, y: f32, z: f32| x.max(y).max(z) - x.min(y).min(z);
        if spread(va.mid, vb.mid, vc.mid) > SLAB_STEP
            || spread(va.high, vb.high, vc.high) > SLAB_STEP
        {
            return false;
        }
        let (a, b, c) = (
            chunk.pos(grid, tri.v[0]),
            chunk.pos(grid, tri.v[1]),
            chunk.pos(grid, tri.v[2]),
        );
        if orient2d(a, b, c) <= 0 {
            return false;
        }
        let min_x = a[0].min(b[0]).min(c[0]).max(0);
        let max_x = a[0].max(b[0]).max(c[0]).min(grid.nx as i32 - 1);
        let min_y = a[1].min(b[1]).min(c[1]).max(0);
        let max_y = a[1].max(b[1]).max(c[1]).min(grid.ny as i32 - 1);
        let edges = Edges::new(a, b, c, [min_x, min_y]);
        let mut row = edges.at;
        let mut has_thickness = false;
        for y in min_y..=max_y {
            let mut w = row;
            let start = grid.index(min_x as u32, y as u32);
            let mut inside = false;
            for gi in start..=start + (max_x - min_x) as u32 {
                if Edges::inside(w) {
                    inside = true;
                    let s = grid.sample(gi);
                    if !s.is_dual {
                        return false;
                    }
                    has_thickness |= s.high > s.low;
                } else if inside {
                    break;
                }
                Edges::advance(&mut w, edges.step_x);
            }
            Edges::advance(&mut row, edges.step_y);
        }
        // A zero-thickness slab has nothing to show.
        has_thickness
    };

    // Every slab triangle also asks about its three neighbours, to decide
    // where the slab needs closing off with a wall. Answering that on
    // demand rasterises most triangles four times over, so answer it once.
    let slab = (0..chunk.tris.len())
        .map(|t| {
            let tri = &chunk.tris[t];
            tri.alive && is_slab(tri)
        })
        .collect::<Vec<_>>();

    let mut emitter = Emitter {
        chunk,
        grid,
        out: ChunkMesh::default(),
        cache: vec![[NONE; 3]; chunk.verts.len()],
    };
    // Every live triangle emits at least its floor, and every vertex at
    // least the low layer, so growing from empty means re-copying the
    // whole buffer several times over on the way there.
    emitter.out.vertices.reserve(chunk.verts.len());
    emitter.out.indices.reserve(chunk.tris.len() * 3);

    for t in 0..chunk.tris.len() {
        let tri = &chunk.tris[t];
        if !tri.alive {
            continue;
        }
        let [a, b, c] = tri.v;
        emitter.tri((a, Layer::Low), (b, Layer::Low), (c, Layer::Low));

        if !slab[t] {
            continue;
        }
        let before = emitter.out.indices.len();
        // Slab top, and the ceiling underneath it wound the other way.
        emitter.tri((a, Layer::High), (b, Layer::High), (c, Layer::High));
        emitter.tri((c, Layer::Mid), (b, Layer::Mid), (a, Layer::Mid));

        for i in 0..3 {
            let nb = tri.n[i];
            if nb != NONE && slab[nb as usize] {
                continue;
            }
            let (e0, e1) = (tri.v[i], tri.v[(i + 1) % 3]);
            emitter.tri((e0, Layer::Mid), (e1, Layer::Mid), (e1, Layer::High));
            emitter.tri((e1, Layer::High), (e0, Layer::High), (e0, Layer::Mid));
        }
        emitter.out.slab_indices += emitter.out.indices.len() - before;
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
/// `border_error` is deliberately separate from `max_error`: it is the
/// *finest* tolerance across all detail levels, not this level's own.
///
/// The seam between two chunks is crack-free because both derive the same
/// vertices from the same samples - but only if they use the same
/// tolerance to do it. Two neighbours at different detail levels do not,
/// so simplifying the border at the chunk's own tolerance opens a gap
/// wherever the levels differ. Fitting every border at the finest
/// tolerance makes the shared edge identical no matter which levels meet
/// there. It costs little: a chunk's border is `4 * CHUNK_SIZE` samples
/// against `CHUNK_SIZE^2` in the interior.
///
/// `measure` drives the interior pass. The border is always re-derived -
/// it is cheap, and both sides of a seam have to agree on it every time -
/// but the interior fit can be skipped for a chunk that has stopped
/// gaining vertices. Returns whether any vertex was added.
fn refine(
    chunk: &mut Chunk,
    grid: &Grid,
    max_error: f32,
    border_error: f32,
    dirty: Option<&[GridRect]>,
    measure: bool,
) -> bool {
    use std::collections::BinaryHeap;

    let mut added = false;

    // Border vertices first. Both chunks sharing a border derive the same
    // set from the same samples, so the seam matches exactly - and because
    // we only add, re-deriving after an edit keeps them in step: both sides
    // end up with the union of their old picks and the identical new ones.
    let (mx, my) = (grid.nx - 1, grid.ny - 1);
    // Only the borders an edit actually reached can have changed. The
    // samples decide the picks, so a side no edit touched re-derives the
    // set it already holds - and both chunks sharing that seam skip it on
    // the same grounds, so they stay in step.
    let mut border = Vec::new();
    let mut line = Vec::with_capacity(grid.nx.max(grid.ny) as usize);
    for (fixed, horizontal) in [(0, true), (my, true), (0, false), (mx, false)] {
        if let Some(rects) = dirty {
            let f = fixed as i32;
            let touched = rects.iter().any(|r| {
                if horizontal {
                    r.y0 <= f && f <= r.y1
                } else {
                    r.x0 <= f && f <= r.x1
                }
            });
            if !touched {
                continue;
            }
        }
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
        simplify_line(&line, border_error, &mut picks);
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
    // Which of those the triangulation already holds. Asking the other way
    // round - hashing every vertex of the chunk into a set - costs a pass
    // over thousands of vertices on every refit to answer a question about
    // a few hundred border samples.
    let mut present = vec![false; border.len()];
    for v in chunk.verts.iter() {
        if let Ok(i) = border.binary_search(v) {
            present[i] = true;
        }
    }
    for (i, gi) in border.iter().copied().enumerate() {
        if present[i] {
            continue;
        }
        let seed = chunk.locate(grid, grid.coord(gi), 0);
        // A border insertion rewrites triangles, so their candidates have to
        // be recomputed whether or not an edit reached them.
        added = true;
        for slot in chunk.insert(grid, gi, seed) {
            chunk.compute_candidate(grid, slot);
        }
    }

    if !measure {
        return added;
    }

    // Every triangle remembers its own worst sample, so popping the global
    // worst needs no point location - we already know which triangle
    // contains it.
    //
    // Those candidates stay valid wherever the samples did not move, so a
    // refit only recomputes the triangles standing over an edit. `dirty` is
    // `None` on the initial build, where nothing is cached yet and every
    // triangle has to be measured. Recomputing all of them on every refit
    // was by far the most expensive thing an edit did: it is a pass over
    // the chunk's whole sample grid, and a frame of moving land refits
    // dozens of chunks.
    for t in 0..chunk.tris.len() as u32 {
        if !chunk.tris[t as usize].alive {
            continue;
        }
        let stale = match dirty {
            None => true,
            Some(rects) => {
                let tri = &chunk.tris[t as usize];
                let (mut lo, mut hi) = ([i32::MAX; 2], [i32::MIN; 2]);
                for &v in tri.v.iter() {
                    let c = chunk.pos(grid, v);
                    lo[0] = lo[0].min(c[0]);
                    hi[0] = hi[0].max(c[0]);
                    lo[1] = lo[1].min(c[1]);
                    hi[1] = hi[1].max(c[1]);
                }
                rects
                    .iter()
                    .any(|r| lo[0] <= r.x1 && r.x0 <= hi[0] && lo[1] <= r.y1 && r.y0 <= hi[1])
            }
        };
        if stale {
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
        added = true;
        for slot in chunk.insert(grid, cand, t) {
            chunk.compute_candidate(grid, slot);
            let tri = &chunk.tris[slot as usize];
            if tri.cand != NONE {
                heap.push((tri.err.to_bits(), slot));
            }
        }
    }
    added
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
        let (verts, indices) = per_lod.iter().fold((0, 0), |(v, i), m| {
            (v + m.vertices.len(), i + m.indices.len())
        });
        let mut buffers = ChunkBuffers {
            vertices: Vec::with_capacity(verts),
            indices: Vec::with_capacity(indices),
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

#[cfg_attr(test, derive(Clone))]
struct ChunkState {
    x0: i32,
    y0: i32,
    w: u32,
    h: u32,
    /// Altitude span over the chunk, for the culling bounding box.
    alt: (f32, f32),
    lods: Vec<LodState>,
    /// Edits banked since this chunk was last refitted. Empty for a chunk
    /// that is up to date with the level.
    pending: Option<Rect>,
    /// Update on which the first still-pending edit arrived. The web
    /// scheduler uses this to keep a continuously animated chunk from
    /// starving its neighbours.
    pending_since: u32,
    /// LOD triangulations which have not yet measured all accumulated edits.
    /// Their vertices are still re-emitted at current heights.
    stale_lods: u8,
    /// Whether this update is the one that refits the chunk.
    due: bool,
    /// Set while the renderer is not drawing this chunk anywhere. Coming
    /// back into view makes it due at once, so the first frame that shows
    /// it again shows it current.
    hidden: bool,
}

/// How many detail steps a chunk that far from the viewer has dropped:
/// zero within `lod_distance`, and one more for every doubling past it.
/// Zero or less for `lod_distance` means full detail everywhere.
///
/// The renderer clamps this to the levels that exist and draws the chunk
/// at that LOD. `Tin::update` then follows the decision rather than making
/// its own - see `Drawn`.
pub fn detail_steps(distance: f32, lod_distance: f32) -> u32 {
    if lod_distance <= 0.0 {
        return 0;
    }
    (distance / lod_distance).max(1.0).log2().floor() as u32
}

/// What the renderer last decided to do with each chunk, by chunk index:
/// the finest LOD it was drawn at, or `None` if it was not drawn at all.
///
/// A refit follows that decision exactly. A chunk drawn at half detail
/// refits every second update, one at a quarter every fourth, and one
/// that is not on screen does not refit until it is - it banks its edits
/// instead. Tying the two together is what makes the lag invisible: a
/// chunk only falls behind once the renderer has already stopped drawing
/// the detail that would show it, and one that is not drawn shows
/// nothing at all.
///
/// The decision is a frame old by the time a refit reads it, so the
/// renderer records it with a margin around the frustum - see
/// `terrain::Context::prepare`.
pub type Drawn<'a> = &'a [Option<DrawInfo>];

/// The renderer's most important visible copy of a chunk.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawInfo {
    pub lod: u8,
    pub distance: f32,
    /// Every LOD used by any visible wrapped copy of this chunk.
    pub lod_mask: u8,
}

impl ChunkState {
    /// How many updates this chunk may bank before it has to refit.
    ///
    /// Only the *geometry* waits. Collision and the moving land itself read
    /// the level, not the mesh, so nothing that can be driven into or stood
    /// on behaves differently - a deferred chunk is a picture a few ticks
    /// old, at a distance where the renderer is already dropping detail for
    /// the same reason.
    ///
    /// The level wraps, so the distance does too: the far edge of the map
    /// is next door.
    /// Whether the chunk at `index` refits on update `tick`, given the
    /// period its detail level earns it.
    ///
    /// The index offsets the phase, so chunks at the same level do not all
    /// come due on the same tick. Without that, the work the deferral
    /// spread out arrives all at once every `period` updates - the
    /// periodic hitch it exists to remove. The phase runs free rather than
    /// counting from each chunk's first banked edit, which would put every
    /// chunk that refits and is edited again straight back on slot zero.
    fn due_on(tick: u32, index: usize, period: u32) -> bool {
        tick.wrapping_add(index as u32).is_multiple_of(period)
    }
}

#[cfg_attr(test, derive(Clone))]
struct LodState {
    tri: Chunk,
    /// Refits left to skip before measuring the interior again.
    settle: u32,
    /// How many to skip next time the interior comes back converged.
    backoff: u32,
}

/// Most refits a converged chunk may skip measuring for.
///
/// A moving land cycles: after one full cycle the triangulation already
/// holds every vertex any phase of it needs, because refining only ever
/// adds. From then on measuring the interior every frame only re-derives
/// the answer "nothing to add" - which is the single most expensive thing
/// a refit does. So a chunk that gains nothing backs off, and one that
/// gains something starts measuring every frame again.
///
/// The cap bounds how stale a *new* deformation can leave the tessellation.
/// The surface itself never lags: vertices are re-emitted from the current
/// heights every refit either way. Only the density of them waits, and at
/// the game's 20 refits a second this is under half a second.
const MAX_SETTLE: u32 = 8;

impl ChunkState {
    /// Chunks own the texels `[x0 ..= x0 + w]`, i.e. they share their border
    /// row/column with the neighbour. That overlap is what makes an edit on
    /// a border line dirty *both* chunks, which is exactly what keeps the
    /// seam consistent.
    fn overlaps(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        w > 0
            && h > 0
            && self.x0 < x + w
            && x <= self.x0 + self.w as i32
            && self.y0 < y + h
            && y <= self.y0 + self.h as i32
    }

    /// Only the perf probe asks this now: `update` needs to know *which*
    /// rectangles reach a chunk, not merely whether any does.
    #[cfg(test)]
    fn overlaps_any(&self, rects: &[Rect]) -> bool {
        rects
            .iter()
            .any(|r| self.overlaps(r.x, r.y, r.w as i32, r.h as i32))
    }
}

/// An edited rectangle in one chunk's grid-local coordinates, inclusive at
/// both ends.
#[derive(Clone, Copy)]
struct GridRect {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

/// An edited region of the level, in texels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    fn union(self, other: Self) -> Self {
        let x0 = self.x.min(other.x);
        let y0 = self.y.min(other.y);
        let x1 = (self.x + self.w as i32).max(other.x + other.w as i32);
        let y1 = (self.y + self.h as i32).max(other.y + other.h as i32);
        Rect {
            x: x0,
            y: y0,
            w: (x1 - x0) as u32,
            h: (y1 - y0) as u32,
        }
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
#[cfg_attr(test, derive(Clone))]
pub struct Tin {
    max_error: f32,
    quality: f32,
    chunks: Vec<ChunkState>,
    chunk_columns: usize,
    /// Free-running update counter, for spreading refits across the ticks
    /// of a period - see `ChunkState::due_on`.
    tick: u32,
    pub stats: Stats,
}

impl Tin {
    /// Whether a chunk has level edits which its render geometry has not
    /// incorporated yet. The renderer uses a conservative vertical bound
    /// for such chunks so an animation can move terrain into the frustum.
    pub fn has_pending(&self, index: usize) -> bool {
        self.chunks
            .get(index)
            .is_some_and(|state| state.pending.is_some())
    }

    /// Heap memory retained by the editable triangulation, excluding the
    /// render-ready vertex/index buffers owned by the renderer.
    pub fn allocated_bytes(&self) -> usize {
        let mut bytes = self.chunks.capacity() * std::mem::size_of::<ChunkState>();
        for state in &self.chunks {
            bytes += state.lods.capacity() * std::mem::size_of::<LodState>();
            for lod in &state.lods {
                bytes += lod.tri.verts.capacity() * std::mem::size_of::<u32>();
                bytes += lod.tri.tris.capacity() * std::mem::size_of::<Tri>();
                bytes += lod.tri.free.capacity() * std::mem::size_of::<u32>();
            }
        }
        bytes
    }

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
            let alt = grid.alt;
            let mut lods: Vec<LodState> = Vec::with_capacity(LOD_COUNT);
            let mut meshes = Vec::with_capacity(LOD_COUNT);
            for k in 0..LOD_COUNT {
                let mut chunk = Chunk::new(&grid);
                refine(
                    &mut chunk,
                    &grid,
                    max_error * (1 << k) as f32,
                    max_error,
                    None,
                    true,
                );
                meshes.push(emit_chunk(&chunk, &grid));
                lods.push(LodState {
                    tri: chunk,
                    settle: 0,
                    backoff: 0,
                });
            }
            (
                ChunkState {
                    x0: x,
                    y0: y,
                    w,
                    h,
                    alt,
                    lods,
                    pending: None,
                    pending_since: 0,
                    stale_lods: 0,
                    due: false,
                    hidden: false,
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
            tick: 0,
            max_error,
            quality: config.quality,
            chunks,
            chunk_columns: spans_of(level.size.0).len(),
            stats: mesh.stats,
        };
        tin.log("built");
        (tin, mesh)
    }

    fn log(&self, what: &str) {
        info!(
            "Terrain TIN {} at quality {}: {} vertices, {} triangles from {} texels \
             ({:.1}x fewer triangles, max error {:.2}), {} slab triangles ({:.1}%)",
            what,
            self.quality,
            self.stats.vertices,
            self.stats.triangles,
            self.stats.source_texels,
            2.0 * self.stats.source_texels as f32 / self.stats.triangles.max(1) as f32,
            self.stats.max_error,
            self.stats.slab_triangles,
            100.0 * self.stats.slab_triangles as f32 / self.stats.triangles.max(1) as f32,
        );
    }

    /// Re-fit the mesh after the level changed inside the given texel rect.
    ///
    /// Only vertices are added: the triangulation is planar in XY and
    /// altitudes are plain attributes, so an edit that merely raises or
    /// lowers the surface re-emits with no topology change at all. New
    /// vertices appear only where the edit introduced detail the existing
    /// triangles cannot represent.
    /// Refits every chunk that any of `rects` touches.
    ///
    /// Takes the whole batch rather than one rectangle at a time. A frame of
    /// moving land dirties dozens of rectangles, many of them landing in the
    /// same chunk; refitting per rectangle rebuilt those chunks - and their
    /// GPU buffers - once per rectangle, and rescanned every chunk in the
    /// level for overlap each time. Fostral has 2048 chunks.
    ///
    /// The refit itself runs in parallel. Chunks are independent, and so are
    /// the LODs within a chunk, so the whole thing is one flat set of tasks:
    /// a wide edit parallelises across chunks, and a single-chunk edit still
    /// gets its LODs done at once.
    /// Refit the chunks the given edits reach.
    ///
    /// Chunks follow what the renderer last drew them at, banking their
    /// edits until they come due - see `Drawn`. `None` refits every reached
    /// chunk on every call, which is what an offline render wants.
    pub fn update(&mut self, level: &Level, rects: &[Rect], drawn: Option<Drawn<'_>>) -> Update {
        profiling::scope!("Update Terrain TIN");
        #[cfg(not(target_arch = "wasm32"))]
        use rayon::prelude::*;

        // Banked work still has to come due, so an idle call is only idle
        // when nothing is waiting - otherwise an animation that stops would
        // leave the last of its edits banked forever.
        #[cfg(target_arch = "wasm32")]
        let visible_lod_is_stale = drawn.is_some_and(|list| {
            self.chunks.iter().enumerate().any(|(index, state)| {
                list.get(index)
                    .copied()
                    .flatten()
                    .is_some_and(|info| state.stale_lods & info.lod_mask != 0)
            })
        });
        #[cfg(not(target_arch = "wasm32"))]
        let visible_lod_is_stale = false;
        if rects.is_empty()
            && !visible_lod_is_stale
            && self.chunks.iter().all(|st| st.pending.is_none())
        {
            return Vec::new();
        }

        // Bank this call's edits and decide who is due, sequentially: it is
        // a few hundred chunks of arithmetic against a refit apiece, and the
        // decision has to be made before the parallel pass can filter on it.
        self.tick = self.tick.wrapping_add(1);
        let tick = self.tick;
        // Chunks form a regular row-major grid. Route each edit straight to
        // the handful of chunks it reaches instead of testing every edit
        // against every chunk in the level. A chunk owns its far border, so
        // an edit exactly on a chunk boundary reaches both neighbours.
        for &rect in rects {
            if rect.w == 0 || rect.h == 0 {
                continue;
            }
            let axis = |start: i32, len: u32, count: usize| {
                let first = if start > 0 && start % CHUNK_SIZE as i32 == 0 {
                    start / CHUNK_SIZE as i32 - 1
                } else {
                    start / CHUNK_SIZE as i32
                };
                let last = (start + len as i32 - 1) / CHUNK_SIZE as i32;
                first.max(0) as usize..=last.max(0).min(count as i32 - 1) as usize
            };
            let rows = self.chunks.len().div_ceil(self.chunk_columns);
            for cy in axis(rect.y, rect.h, rows) {
                for cx in axis(rect.x, rect.w, self.chunk_columns) {
                    let index = cy * self.chunk_columns + cx;
                    let Some(state) = self.chunks.get_mut(index) else {
                        continue;
                    };
                    if !state.overlaps(rect.x, rect.y, rect.w as i32, rect.h as i32) {
                        continue;
                    }
                    state.pending = Some(match state.pending {
                        None => {
                            state.pending_since = tick;
                            rect
                        }
                        Some(old) => old.union(rect),
                    });
                    state.stale_lods |= (1u8 << LOD_COUNT) - 1;
                }
            }
        }

        for (index, state) in self.chunks.iter_mut().enumerate() {
            state.due = false;
            #[cfg(target_arch = "wasm32")]
            if state.pending.is_none()
                && let Some(info) = drawn.and_then(|list| list.get(index).copied().flatten())
                && state.stale_lods & info.lod_mask != 0
            {
                state.pending_since = tick;
                state.pending = Some(Rect {
                    x: state.x0,
                    y: state.y0,
                    w: state.w + 1,
                    h: state.h + 1,
                });
            }
            if state.pending.is_none() {
                continue;
            }
            // Not built yet - the web build scaffolds every chunk and fills
            // them in over the first ticks (`build_chunk`). Until one is
            // built its LODs are empty, and a refit has no triangulation to
            // refine. `build_chunk` reads the current level, so the edit
            // banked here is picked up then.
            if state.lods.is_empty() {
                continue;
            }
            let info = match drawn {
                None => DrawInfo {
                    lod: 0,
                    distance: 0.0,
                    lod_mask: (1u8 << LOD_COUNT) - 1,
                },
                Some(list) => match list.get(index).copied().flatten() {
                    Some(info) => info,
                    // Off screen: bank and wait. Nothing is showing this
                    // chunk, so nothing can show that it is behind.
                    None => {
                        state.hidden = true;
                        continue;
                    }
                },
            };
            state.due = if state.hidden {
                state.hidden = false;
                true
            } else {
                ChunkState::due_on(tick, index, 1u32 << info.lod)
            };
        }

        let max_error = self.max_error;
        let refit = |index: usize, state: &mut ChunkState| {
            let grid = Grid::new(level, state.x0, state.y0, state.w, state.h);
            state.alt = grid.alt;
            #[cfg(target_arch = "wasm32")]
            let visible_lods = drawn
                .and_then(|list| list.get(index).copied().flatten())
                .map(|info| info.lod_mask);
            // The edits that reach this chunk, in its own grid
            // coordinates and clipped to it.
            let local = state
                .pending
                .iter()
                .filter_map(|r| {
                    let g = GridRect {
                        x0: (r.x - state.x0).max(0),
                        y0: (r.y - state.y0).max(0),
                        x1: (r.x + r.w as i32 - state.x0).min(state.w as i32),
                        y1: (r.y + r.h as i32 - state.y0).min(state.h as i32),
                    };
                    (g.x0 <= g.x1 && g.y0 <= g.y1).then_some(g)
                })
                .collect::<Vec<_>>();
            state.pending = None;
            #[cfg(target_arch = "wasm32")]
            match visible_lods {
                Some(mask) => state.stale_lods &= !mask,
                None => state.stale_lods = 0,
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                state.stale_lods = 0;
            }
            let per_lod = {
                let emit_lod = |k: usize, lod: &mut LodState| {
                    #[cfg(target_arch = "wasm32")]
                    if visible_lods.is_some_and(|mask| mask & (1u8 << k) == 0) {
                        // Keep every LOD's vertex heights current so a later
                        // distance transition cannot reveal stale terrain,
                        // but spend the expensive error search only on the
                        // LODs the renderer is using for this chunk now. When
                        // another LOD becomes visible its next edit/refit
                        // measures it normally.
                        return emit_chunk(&lod.tri, &grid);
                    }
                    // Skip measuring a chunk that has stopped gaining
                    // vertices, but always re-emit: the geometry has to
                    // follow the new heights either way.
                    let measure = lod.settle == 0;
                    let added = refine(
                        &mut lod.tri,
                        &grid,
                        max_error * (1 << k) as f32,
                        max_error,
                        Some(&local),
                        measure,
                    );
                    if added {
                        lod.backoff = 0;
                        lod.settle = 0;
                    } else if measure {
                        lod.backoff = (lod.backoff + 1).min(MAX_SETTLE);
                        lod.settle = lod.backoff;
                    } else {
                        lod.settle -= 1;
                    }
                    emit_chunk(&lod.tri, &grid)
                };
                #[cfg(not(target_arch = "wasm32"))]
                {
                    state
                        .lods
                        .par_iter_mut()
                        .enumerate()
                        .map(|(k, lod)| emit_lod(k, lod))
                        .collect::<Vec<_>>()
                }
                #[cfg(target_arch = "wasm32")]
                {
                    state
                        .lods
                        .iter_mut()
                        .enumerate()
                        .map(|(k, lod)| emit_lod(k, lod))
                        .collect::<Vec<_>>()
                }
            };
            (index, ChunkBuffers::new(state, per_lod))
        };

        #[cfg(not(target_arch = "wasm32"))]
        {
            self.chunks
                .par_iter_mut()
                .enumerate()
                .filter(|entry| entry.1.due)
                .map(|(index, state)| refit(index, state))
                .collect()
        }
        #[cfg(target_arch = "wasm32")]
        {
            // One 128² refine plus a WebGL buffer upload is several
            // milliseconds in the browser. A cyclic set on Fostral can
            // put tens of chunks due at once; doing them all in one
            // frame is the one-second hitch. Keep the one-refit ceiling, but
            // choose the oldest visible edit first; LOD and distance break
            // equal-age ties. That bounds frame cost without letting map
            // index order hide a nearby animation for thousands of frames.
            const MAX_REFITS: usize = 1;
            let n = self.chunks.len();
            if n == 0 {
                return Vec::new();
            }
            let drawn = drawn.unwrap_or(&[]);
            let mut due = (0..n)
                .filter(|&index| self.chunks[index].due && !self.chunks[index].lods.is_empty())
                .collect::<Vec<_>>();
            due.sort_by(|&a, &b| {
                let sa = &self.chunks[a];
                let sb = &self.chunks[b];
                let ia = drawn.get(a).copied().flatten().unwrap_or(DrawInfo {
                    lod: u8::MAX,
                    distance: f32::INFINITY,
                    lod_mask: 0,
                });
                let ib = drawn.get(b).copied().flatten().unwrap_or(DrawInfo {
                    lod: u8::MAX,
                    distance: f32::INFINITY,
                    lod_mask: 0,
                });
                // Old work wins first, preventing a location edited every
                // tick from monopolising the single-threaded budget. LOD and
                // distance break ties in favour of what is easiest to see.
                sa.pending_since
                    .cmp(&sb.pending_since)
                    .then_with(|| ia.lod.cmp(&ib.lod))
                    .then_with(|| ia.distance.total_cmp(&ib.distance))
            });
            let mut out = Vec::new();
            for index in due.into_iter().take(MAX_REFITS) {
                let state = &mut self.chunks[index];
                out.push(refit(index, state));
            }
            out
        }
    }

    /// Chunk scaffolding with nothing triangulated, for the web build.
    ///
    /// `build` triangulates the whole level up front; on a single-threaded
    /// wasm that is the seconds-long hitch the loading screen sits on.
    /// `scaffold` creates every `ChunkState` so chunk indices, the render
    /// wrapper's per-chunk arrays and the draw walk are all stable, but
    /// leaves every LOD empty. [`Tin::build_chunk`] then fills them in a
    /// few at a time, nearest the camera first, over the first frames.
    #[cfg(target_arch = "wasm32")]
    pub fn scaffold(level: &Level, config: &Config) -> (Self, Mesh) {
        profiling::scope!("Scaffold Terrain TIN");

        let max_error = config.max_error(level);
        let step = CHUNK_SIZE as i32;
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

        let chunks = origins
            .iter()
            .map(|&(x, y, w, h)| ChunkState {
                x0: x,
                y0: y,
                w,
                h,
                // Filled by `build_chunk`, which also refreshes the bbox.
                alt: (0.0, 0.0),
                lods: Vec::new(),
                pending: None,
                pending_since: 0,
                stale_lods: 0,
                due: false,
                hidden: false,
            })
            .collect::<Vec<_>>();
        let meshes = origins
            .iter()
            .map(|_| Vec::<ChunkMesh>::new())
            .collect::<Vec<_>>();
        let mesh = assemble(&chunks, meshes, level, max_error);

        let tin = Tin {
            tick: 0,
            max_error,
            quality: config.quality,
            chunks,
            chunk_columns: spans_of(level.size.0).len(),
            stats: mesh.stats,
        };
        tin.log("scaffolded");
        (tin, mesh)
    }

    /// Builds (first time) the chunk at `index` from its grid, leaving the
    /// chunk triangulated so later edits refine it in place - the same
    /// work `build` does for one chunk, run on demand.
    #[cfg(target_arch = "wasm32")]
    pub fn build_chunk(&mut self, level: &Level, index: usize) -> ChunkBuffers {
        let grid = {
            let st = &self.chunks[index];
            Grid::new(level, st.x0, st.y0, st.w, st.h)
        };
        let max_error = self.max_error;
        let per_lod = {
            let state = &mut self.chunks[index];
            state.alt = grid.alt;
            // The grid was read after any banked edit was applied, so the
            // edits are baked in and the bank can be dropped.
            state.pending = None;
            state.stale_lods = 0;
            state.lods.clear();
            state.lods = (0..LOD_COUNT)
                .map(|k| {
                    let mut tri = Chunk::new(&grid);
                    refine(
                        &mut tri,
                        &grid,
                        max_error * (1 << k) as f32,
                        max_error,
                        None,
                        true,
                    );
                    LodState {
                        tri,
                        settle: 0,
                        backoff: 0,
                    }
                })
                .collect();
            state
                .lods
                .iter()
                .map(|lod| emit_chunk(&lod.tri, &grid))
                .collect()
        };
        ChunkBuffers::new(&self.chunks[index], per_lod)
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
                stats.slab_triangles += fine.slab_indices / 3;
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

    /// A refit that only remeasures the triangles standing over the edit
    /// has to land on exactly the mesh a full remeasure would. Fitting the
    /// same TIN both ways and comparing is the check on that: passing a
    /// rectangle covering the whole level makes every triangle count as
    /// dirty, which is what the code did before it took the edit into
    /// account.
    #[test]
    fn a_narrow_refit_matches_a_full_one() {
        let mut level = make_level(192);
        let (tin, _) = Tin::build(&level, &Config::default());
        let (cx, cy, r) = (96, 96, 24);
        dig(&mut level, cx, cy, r);

        let mut narrow = tin.clone();
        let mut full = tin;
        let edit = rect(cx - r, cy - r, 2 * r, 2 * r);
        let a = narrow.update(&level, &[edit], None);
        let b = full.update(&level, &[rect(0, 0, level.size.0, level.size.1)], None);

        // The full-level rectangle naturally refits every chunk, so compare
        // the ones the narrow edit did touch.
        assert!(!a.is_empty(), "the edit refitted nothing");
        for entry in a.iter() {
            let (ia, ba) = (entry.0, &entry.1);
            let bb = &b
                .iter()
                .find(|other| other.0 == ia)
                .unwrap_or_else(|| panic!("chunk {ia} missing from the full refit"))
                .1;
            assert_eq!(ba.indices, bb.indices, "chunk {ia} indices differ");
            assert_eq!(ba.lods, bb.lods, "chunk {ia} lod ranges differ");
            assert_eq!(
                ba.vertices.len(),
                bb.vertices.len(),
                "chunk {ia} vertex count differs"
            );
            for (va, vb) in ba.vertices.iter().zip(&bb.vertices) {
                assert_eq!(va.pos, vb.pos, "chunk {ia} vertex moved");
            }
        }
    }

    /// Shorthand for the one-rectangle edits the tests make.
    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect {
            x,
            y,
            w: w as u32,
            h: h as u32,
        }
    }

    fn flat_grid(nx: u32, ny: u32) -> Grid {
        Grid {
            samples: vec![Sample::default(); (nx * ny) as usize],
            alt: (0.0, 0.0),
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

    /// The seam also has to survive the two chunks being at *different*
    /// detail levels, which is the normal case as soon as one is further
    /// from the camera than the other. Simplifying each border at its own
    /// level's tolerance is what used to open a visible crack there.
    #[test]
    fn adjacent_chunks_agree_across_detail_levels() {
        let level = make_level(64);
        let size = 16u32;
        let left = Grid::new(&level, 0, 0, size, size);
        let right = Grid::new(&level, size as i32, 0, size, size);
        let base_error = 1.0f32;

        let border_of = |grid: &Grid, lx: u32| -> Vec<u32> {
            let line: Vec<Sample> = (0..grid.ny)
                .map(|y| *grid.sample(grid.index(lx, y)))
                .collect();
            let mut picks = Vec::new();
            // Every level fits its border at the base tolerance, whatever
            // its own interior tolerance is.
            simplify_line(&line, base_error, &mut picks);
            picks
        };

        // The finest chunk on the left, the coarsest on the right.
        for k in 0..LOD_COUNT {
            let mut fine = Chunk::new(&left);
            refine(&mut fine, &left, base_error, base_error, None, true);
            let mut coarse = Chunk::new(&right);
            let coarse_error = base_error * (1 << k) as f32;
            refine(&mut coarse, &right, coarse_error, base_error, None, true);

            assert_eq!(
                border_of(&left, size),
                border_of(&right, 0),
                "level {} border disagrees with the finest one",
                k
            );

            // And the vertices actually present on the shared column match.
            let on_column = |chunk: &Chunk, grid: &Grid, lx: u32| -> Vec<i32> {
                let mut ys: Vec<i32> = chunk
                    .verts
                    .iter()
                    .map(|&gi| grid.coord(gi))
                    .filter(|c| c[0] == lx as i32)
                    .map(|c| c[1])
                    .collect();
                ys.sort_unstable();
                ys
            };
            assert_eq!(
                on_column(&fine, &left, size),
                on_column(&coarse, &right, 0),
                "level {} does not place the same vertices on the seam",
                k
            );
        }
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
        tin.update(&level, &[rect(x, y, w, h)], None);
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

    /// Edits under terrain the renderer is drawing coarsely wait their
    /// turn, they do not all wait for the same tick, and when a chunk's
    /// turn comes it lands on exactly the geometry an immediate refit
    /// would have produced - nothing banked is lost, and the surface is
    /// re-read from the level as it stands then.
    #[test]
    fn coarse_chunks_wait_their_turn_and_lose_nothing() {
        let mut level = make_level_wh(8 * CHUNK_SIZE as i32, 2 * CHUNK_SIZE as i32);
        let (tin, _) = Tin::build(&level, &Config::default());
        let edits = (0..8)
            .map(|i| {
                let (x, y, w, h) = dig(&mut level, 128 + i * 128, 64, 24);
                rect(x, y, w, h)
            })
            .collect::<Vec<_>>();

        // All on screen, but at the coarsest level the mesh has.
        let coarse = vec![
            Some(DrawInfo {
                lod: LOD_COUNT as u8 - 1,
                distance: 1024.0,
                lod_mask: 1u8 << (LOD_COUNT - 1),
            });
            tin.chunks.len()
        ];
        let period = 1u32 << (LOD_COUNT - 1);
        let count = tin
            .chunks
            .iter()
            .filter(|st| st.overlaps_any(&edits))
            .count();

        let mut deferred = tin.clone();
        let mut immediate = tin;
        let now = immediate.update(&level, &edits, None);
        assert_eq!(now.len(), count);

        // Spread out: the first tick refits only some of them, and by the
        // end of one period every one has gone exactly once.
        let first = deferred.update(&level, &edits, Some(&coarse));
        assert!(
            first.len() < count,
            "all {count} reached chunks refitted at once, so nothing was deferred"
        );
        let mut refitted = first;
        for _ in 1..period {
            refitted.extend(deferred.update(&level, &[], Some(&coarse)));
        }
        assert_eq!(
            refitted.len(),
            count,
            "chunks refitted more or less than once each over a period"
        );
        assert!(
            deferred.update(&level, &[], Some(&coarse)).is_empty(),
            "a chunk kept refitting after its banked edits were spent"
        );

        for entry in refitted.iter() {
            let other = &now
                .iter()
                .find(|e| e.0 == entry.0)
                .unwrap_or_else(|| panic!("chunk {} missing from the immediate refit", entry.0))
                .1;
            assert_eq!(entry.1.indices, other.indices, "chunk {} differs", entry.0);
            assert_eq!(entry.1.lods, other.lods, "chunk {} lods differ", entry.0);
        }
    }

    /// A chunk nobody is drawing does not refit at all - and the frame that
    /// starts drawing it again gets it current, not a period later.
    #[test]
    fn an_undrawn_chunk_waits_until_it_is_drawn_again() {
        let mut level = make_level_wh(4 * CHUNK_SIZE as i32, 2 * CHUNK_SIZE as i32);
        let (tin, _) = Tin::build(&level, &Config::default());
        let (x, y, w, h) = dig(&mut level, 2 * CHUNK_SIZE as i32, 64, 24);
        let edit = rect(x, y, w, h);

        let hidden = vec![None; tin.chunks.len()];
        // Back at the coarsest level, so returning to view is the only
        // thing that could make it refit this promptly.
        let shown = vec![
            Some(DrawInfo {
                lod: LOD_COUNT as u8 - 1,
                distance: 1024.0,
                lod_mask: 1u8 << (LOD_COUNT - 1),
            });
            tin.chunks.len()
        ];

        let mut deferred = tin.clone();
        let mut immediate = tin;
        for _ in 0..4 {
            assert!(
                deferred.update(&level, &[edit], Some(&hidden)).is_empty(),
                "an off-screen chunk refitted"
            );
        }
        let back = deferred.update(&level, &[], Some(&shown));
        let now = immediate.update(&level, &[edit], None);
        assert!(!back.is_empty(), "coming back into view refitted nothing");
        assert_eq!(back.len(), now.len());
        for entry in back.iter() {
            let other = &now
                .iter()
                .find(|e| e.0 == entry.0)
                .unwrap_or_else(|| panic!("chunk {} missing", entry.0))
                .1;
            assert_eq!(entry.1.indices, other.indices, "chunk {} differs", entry.0);
        }
    }

    /// A chunk refits once for every update its detail level earns it:
    /// LOD 0 every tick, LOD 1 every second, LOD 2 every fourth.
    #[test]
    fn the_refit_period_follows_the_detail_level() {
        let mut level = make_level_wh(4 * CHUNK_SIZE as i32, 2 * CHUNK_SIZE as i32);
        let (tin, _) = Tin::build(&level, &Config::default());
        let (x, y, w, h) = dig(&mut level, 2 * CHUNK_SIZE as i32, 64, 24);
        let edit = rect(x, y, w, h);
        let reached = tin
            .chunks
            .iter()
            .filter(|st| st.overlaps_any(&[edit]))
            .count();

        for lod in 0..LOD_COUNT as u8 {
            let drawn = vec![
                Some(DrawInfo {
                    lod,
                    distance: 256.0 * (1 << lod) as f32,
                    lod_mask: 1u8 << lod,
                });
                tin.chunks.len()
            ];
            let mut tin = tin.clone();
            // Several periods, with the edit re-offered every tick the way
            // a running animation offers it.
            let ticks = 8u32 << lod;
            let mut refits = 0;
            for _ in 0..ticks {
                refits += tin.update(&level, &[edit], Some(&drawn)).len();
            }
            assert_eq!(
                refits,
                reached * (ticks >> lod) as usize,
                "a chunk drawn at LOD {lod} should refit every {} ticks",
                1u32 << lod
            );
        }
    }

    /// `detail_steps` is the ladder the renderer picks a LOD from, and the
    /// refit period is that LOD - so this is what ties the two together.
    #[test]
    fn detail_steps_drop_a_level_per_doubling() {
        let lod_distance = 256.0;
        for steps in 0..5u32 {
            let inside = lod_distance * (1 << steps) as f32 * 1.5;
            assert_eq!(detail_steps(inside, lod_distance), steps);
        }
        assert_eq!(detail_steps(0.0, lod_distance), 0);
        // Zero means full detail everywhere, which means refitting all of it.
        assert_eq!(detail_steps(1.0e6, 0.0), 0);
    }

    /// Every chunk comes due exactly once per period, and chunks at the
    /// same detail level are spread across the ticks of it rather than all
    /// arriving on one.
    #[test]
    fn refits_spread_across_the_ticks_of_a_period() {
        for period in [1u32, 2, 4] {
            let chunks = 4 * period as usize;
            for index in 0..chunks {
                let hits = (0..period)
                    .filter(|tick| ChunkState::due_on(*tick, index, period))
                    .count();
                assert_eq!(hits, 1, "chunk {index} came due {hits} times in {period}");
            }
            for tick in 0..period {
                let due = (0..chunks)
                    .filter(|index| ChunkState::due_on(tick, *index, period))
                    .count();
                assert_eq!(
                    due,
                    chunks / period as usize,
                    "period {period} bunches {due} chunks onto tick {tick}"
                );
            }
        }
    }

    /// A chunk that has stopped gaining vertices stops measuring, so a new
    /// deformation is allowed to lag - but only up to `MAX_SETTLE` refits,
    /// after which the fit has to be back inside tolerance.
    #[test]
    fn a_settled_chunk_still_catches_a_new_edit() {
        let mut level = make_level(96);
        let config = Config { quality: 0.5 };
        let (mut tin, _) = Tin::build(&level, &config);
        let tolerance = tin.max_error;
        let all = rect(0, 0, level.size.0, level.size.1);

        // Refit with nothing changing until every chunk has backed off as
        // far as it can.
        for _ in 0..2 * MAX_SETTLE {
            tin.update(&level, &[all], None);
        }

        let (x, y, w, h) = dig(&mut level, 70, 70, 12);
        assert!(worst_error(&tin, &level) > tolerance * 4.0);

        // The first refit does not measure - that is the whole point of the
        // back-off - so the fit is still wrong here. If this ever stops
        // holding the test below has stopped proving anything.
        let edit = rect(x, y, w, h);
        tin.update(&level, &[edit], None);
        assert!(
            worst_error(&tin, &level) > tolerance,
            "the chunk never settled, so the catch-up below is vacuous"
        );

        for _ in 0..MAX_SETTLE {
            tin.update(&level, &[edit], None);
        }
        let worst = worst_error(&tin, &level);
        assert!(
            worst <= tolerance,
            "a settled chunk never caught up: residual {worst} against {tolerance}"
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
        tin.update(&level, &[rect(x, y, w, h)], None);
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
        let changed = tin.update(&level, &[rect(0, 0, level.size.0, level.size.1)], None);
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
        tin.update(&level, &[rect(x, y, w, h)], None);

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
        let changed = tin.update(&level, &[rect(20, 20, 60, 60)], None);
        assert!(!changed.is_empty(), "the edit should have refitted a chunk");

        // Refitted buffers have to be self-consistent: every index in
        // range, whole triangles, and the surface back within tolerance.
        for entry in &changed {
            let c = &entry.1;
            assert_eq!(c.indices.len() % 3, 0);
            assert!(c.indices.iter().all(|&i| (i as usize) < c.vertices.len()));
        }
        let worst = worst_error(&tin, &level);
        assert!(worst <= tin.max_error, "residual error {}", worst);

        // And a further edit must still leave the surface fitted.
        let (x, y, w, h) = dig(&mut level, 85, 85, 6);
        tin.update(&level, &[rect(x, y, w, h)], None);
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
        let touched = tin.update(&level, &[rect(x, y, w, h)], None).len();
        assert!(
            touched < tin.chunks.len(),
            "a local edit touched every one of {} chunks",
            tin.chunks.len()
        );

        let after = vertex_sets(&tin);
        let changed = before
            .iter()
            .zip(&after)
            .filter(|pair| pair.0.len() != pair.1.len())
            .count();
        assert!(changed >= 1, "the edit should have refined something");
        assert!(changed < tin.chunks.len(), "it should not have refined all");
    }

    #[test]
    fn a_half_open_edit_does_not_dirty_the_next_chunk() {
        let mut level = make_level_wh(3 * CHUNK_SIZE as i32, CHUNK_SIZE as i32);
        let (mut tin, _) = Tin::build(&level, &Config::default());
        // Ends at x=127. Chunk 1 begins at x=128 and shares that texel with
        // chunk 0, but the edit itself never reaches it.
        for y in 20..30 {
            for x in 10..CHUNK_SIZE as i32 {
                let i = level.wrap((x, y));
                level.height[i] = level.height[i].saturating_sub(1);
            }
        }
        let changed = tin.update(
            &level,
            &[Rect {
                x: 10,
                y: 20,
                w: CHUNK_SIZE - 10,
                h: 10,
            }],
            None,
        );
        assert_eq!(
            changed.iter().map(|entry| entry.0).collect::<Vec<_>>(),
            vec![0]
        );
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

#[cfg(test)]
/// Where a refit's time actually goes, on a real level.
///
/// Ignored by default because it needs game data. Run it as
///
/// ```text
/// PROBE_INI=path/to/world.ini cargo test --release --lib \
///     perf_probe -- --ignored --nocapture
/// ```
///
/// It reports the parallel cost of a batch of edits and then the same work
/// single threaded, split into sampling the grid, refitting the
/// triangulation and emitting the buffers - which is what says whether a
/// slow refit is worth attacking, and where.
mod perf_probe {
    use super::*;
    use std::time::Instant;

    #[test]
    #[ignore]
    fn breakdown() {
        let path = std::path::PathBuf::from(
            std::env::var("PROBE_INI").expect("set PROBE_INI to a world.ini"),
        );
        let config = crate::level::LevelConfig::load(&path);
        let level = crate::level::load(&config, &crate::config::settings::Geometry::default());
        let t = Instant::now();
        let (mut tin, _mesh) = Tin::build(&level, &Config::default());
        let tris = tin
            .chunks
            .iter()
            .map(|c| c.lods[0].tri.tris.len())
            .sum::<usize>();
        println!(
            "build: {:?}, {} chunks, {tris} lod0 triangles",
            t.elapsed(),
            tin.chunks.len()
        );

        // Eight scattered edits, like a frame of moving land.
        let rects = (0..8)
            .map(|i| Rect {
                x: 200 + i * 180,
                y: 300 + i * 150,
                w: 120,
                h: 120,
            })
            .collect::<Vec<_>>();

        let hit = tin.chunks.iter().filter(|c| c.overlaps_any(&rects)).count();
        println!("{hit} chunks overlap");

        // Per tick rather than in total: the complaint about a refit is
        // never the average, it is the tick that misses its frame.
        let ticks = |tin: &mut Tin, drawn: Option<Drawn<'_>>, label: &str| {
            let mut each = Vec::new();
            for _ in 0..40 {
                let t = Instant::now();
                let _ = tin.update(&level, &rects, drawn);
                each.push(t.elapsed().as_secs_f64() * 1e3);
            }
            let total = each.iter().sum::<f64>();
            let worst = each.iter().cloned().fold(0.0f64, f64::max);
            println!(
                "{label}: {:.2}ms a tick, worst {worst:.2}ms",
                total / each.len() as f64
            );
        };
        ticks(&mut tin, None, "every reached chunk, every tick");

        // What the renderer would decide with the viewer standing at the
        // first edit: one animation underfoot, the rest scattered across
        // the map, and whatever falls outside the frustum not drawn at
        // all. That is the shape of a frame of real play.
        let eye = glam::Vec2::new(rects[0].x as f32, rects[0].y as f32);
        let facing = glam::Vec2::new(1.0, 1.0).normalize();
        let size = glam::Vec2::new(level.size.0 as f32, level.size.1 as f32);
        let drawn = tin
            .chunks
            .iter()
            .map(|st| {
                let centre = glam::Vec2::new(
                    st.x0 as f32 + st.w as f32 * 0.5,
                    st.y0 as f32 + st.h as f32 * 0.5,
                );
                // Nearest wrap copy, then a 90-degree cone in front of the
                // viewer standing in for the frustum.
                let mut to = centre - eye;
                to.x -= (to.x / size.x).round() * size.x;
                to.y -= (to.y / size.y).round() * size.y;
                let distance = to.length();
                let ahead = distance < 1.0 || to.normalize().dot(facing) > 0.7;
                ahead.then(|| DrawInfo {
                    lod: detail_steps(distance, 256.0).min(LOD_COUNT as u32 - 1) as u8,
                    distance,
                    lod_mask: 1u8 << detail_steps(distance, 256.0).min(LOD_COUNT as u32 - 1),
                })
            })
            .collect::<Vec<_>>();
        let seen = drawn.iter().filter(|d| d.is_some()).count();
        println!("{seen} of {} chunks drawn", drawn.len());
        ticks(&mut tin, Some(&drawn), "following what the renderer drew");

        // Phase split, single threaded, over the same chunks.
        let max_error = tin.max_error;
        let (mut grid_t, mut refine_t, mut emit_t) = (0.0f64, 0.0f64, 0.0f64);
        for _ in 0..10 {
            for state in tin.chunks.iter_mut().filter(|c| c.overlaps_any(&rects)) {
                let t = Instant::now();
                let grid = Grid::new(&level, state.x0, state.y0, state.w, state.h);
                grid_t += t.elapsed().as_secs_f64();
                let local = rects
                    .iter()
                    .filter_map(|r| {
                        let g = GridRect {
                            x0: (r.x - state.x0).max(0),
                            y0: (r.y - state.y0).max(0),
                            x1: (r.x + r.w as i32 - state.x0).min(state.w as i32),
                            y1: (r.y + r.h as i32 - state.y0).min(state.h as i32),
                        };
                        (g.x0 <= g.x1 && g.y0 <= g.y1).then_some(g)
                    })
                    .collect::<Vec<_>>();
                for (k, lod) in state.lods.iter_mut().enumerate() {
                    let t = Instant::now();
                    refine(
                        &mut lod.tri,
                        &grid,
                        max_error * (1 << k) as f32,
                        max_error,
                        Some(&local),
                        true,
                    );
                    refine_t += t.elapsed().as_secs_f64();
                    let t = Instant::now();
                    let _ = emit_chunk(&lod.tri, &grid);
                    emit_t += t.elapsed().as_secs_f64();
                }
            }
        }
        println!(
            "per update (1 thread): grid {:.2}ms  refine {:.2}ms  emit {:.2}ms",
            grid_t * 100.0,
            refine_t * 100.0,
            emit_t * 100.0
        );
    }
}
