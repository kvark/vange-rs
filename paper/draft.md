# Six Ways to Draw a Voxel World: A Controlled Comparison on Hand-Authored Multi-Layer Terrain

**Status: draft.** Numbers marked `[lavapipe]` are from a software
rasterizer and are placeholders for hardware runs. Numbers marked `TODO`
have no measurement behind them yet and must not survive into a
submission.

---

## Abstract

*(Draft — to be tightened once the hardware runs land.)*

Terrain level-of-detail is usually measured on digital elevation models:
single-valued, smooth at the sampling scale, derived from real
topography. We report what happens to six rendering methods when the
terrain is none of those things. Our dataset is the terrain of *Vangers*
(1998), a hand-authored 2048×16384 height map with a two-layer encoding
that admits caves and overhangs, and per-texel detail that no natural
surface exhibits. We implement all six methods in one engine over one
data path, and score them against a CPU ray cast of the same source data
using metrics that separate missing geometry from disagreement about
silhouettes, and both from spatial incoherence.

Three results are worth reporting. First, greedy triangulated irregular
network fitting — the standard approach, and the strongest performer here
— reduces triangle count by 14.9× on this data against roughly 80× on a
smooth surface, and 23% of its vertex budget is consumed by a single
structural feature: the boundary between single- and double-layer
regions, where the auxiliary layers step discontinuously and an
error-driven fit chases a discontinuity no tolerance satisfies. Second,
the methods are nearly indistinguishable when the camera looks down and
diverge sharply as it approaches the horizon, which is the viewpoint the
original engine never used and every modern one does. Third, we show that
a correctness metric based on coverage alone is structurally unable to
detect over-drawing, and that this hid a geometric defect in our own
implementation through several rounds of apparently passing measurement.

## 1. Introduction

*(TODO — frame around the gap: terrain LOD literature is measured on
DEMs, games ship authored terrain, nobody has published the comparison.)*

Contributions:

1. A controlled comparison of six terrain rendering methods sharing one
   engine, one data path and one camera, so differences are attributable
   to the method rather than the surrounding system.
2. An evaluation methodology scoring against a CPU ray cast of the source
   data, decomposed into coverage, geometric and coherence error, with
   the failure mode of the naive version documented.
3. A public dataset, harness and reference implementations.
4. Measurements of where greedy TIN fitting degrades on authored terrain,
   and why.

## 2. The Data

Vangers terrain is a height map with a per-texel-*pair* dual encoding.
A texel is either single-valued, or carries three altitudes: a floor
(`low`), a cave ceiling (`mid`) and a slab top (`high`), with the even
texel of a pair storing `low`, the odd one `high`, and `mid` derived from
delta bits in both. Fostral, the level used throughout, is 2048×16384 and
10.9% double-level.

Two properties matter for everything that follows. The surface is
**authored, not sampled**: cliffs are vertical by construction rather than
by steep gradient, and adjacent texels routinely differ by tens of units.
And `mid`/`high` are **structural, not fields**: they describe where a
slab is, and interpolating them across the region boundary produces
geometry that exists nowhere. Section 6.3 is about what that costs.

*(TODO: figure — a texel-pair diagram, and a slice through a double-level
region.)*

## 3. Methods Compared

All six are implemented in the same engine, share the terrain texture and
palette, and are driven by the same camera. They differ only in how they
turn the height data into pixels.

| method | order | primitive | needs compute |
|---|---|---|---|
| Height-field ray march | image | per-pixel ray | no |
| Voxel-octree ray march | image | per-pixel ray | yes (bake) |
| Sliced | object | horizontal quads | no |
| Painted | object | one bar per ground sample | no |
| Scattered | object | compute-scattered samples | yes |
| **Mesh (TIN)** | object | fitted triangles | no |

*(TODO: one paragraph per method. The taxonomy — backward per-pixel
marching vs forward per-sample splatting vs proxy geometry vs a fitted
surface — is the organising idea and should be stated as such.)*

### 3.6 Mesh

Greedy insertion following Garland and Heckbert [1995]: start from a
coarse triangulation, repeatedly insert the sample with the largest
vertical error, stop at a tolerance. Each triangle caches its own worst
sample, so extracting the global worst needs no point location.

Two details are specific to this data.

**One triangulation, three surfaces.** All three altitudes of a texel
share a single planar triangulation. A point is inserted when *any* layer
needs it, and every vertex carries all three altitudes. Vertical walls
appear where the slab ends.

**Exact predicates on integer coordinates.** A height-map grid is
massively cocircular — every axis-aligned square of four samples — which
is where floating-point orientation and incircle tests go inconsistent.
Chunk-local integer coordinates keep both exact in `i64`.

The level is cut into 128×128 chunks with three detail levels, frustum
culled, and drawn near-to-far. Chunk borders are simplified at the
*finest* tolerance regardless of the chunk's own level, so neighbouring
chunks at different levels derive identical boundary vertices and the
seam cannot crack.

## 4. Evaluation

### 4.1 Reference

A CPU ray cast of the same height data with the same camera, marching
coarsely and bisecting within the bracketing interval. It yields a sky
mask and a distance field.

Two mistakes in building this are worth recording because both produced
plausible, wrong numbers for some time.

The reference camera's focal length was scaled by render height while the
renderer's is fixed in pixels. The two agree only at one resolution; at
1080p the reference would have had a 3.6× wider field of view than the
image it was scoring.

The solidity test guarded the floor comparison with `z >= 0`, which makes
a texel of height zero unhittable — no `z` satisfies both `z >= 0` and
`z < 0`. Sea level is a real height on these maps, so every downward ray
over water was classified as sky. It cost 23% of a straight-down frame.

### 4.2 Metrics

**Coverage**, decomposed. `see-through` is solid terrain the renderer left
as background; `covers-sky` is background it filled in. The decomposition
is what makes them diagnostic: only the first moves when a renderer is
genuinely missing geometry, and both move together when the reference is
the one disagreeing about a silhouette.

**Geometry.** Median and 95th-percentile distance error where renderer and
reference agree something is present. This must be read comparatively:
its floor is set almost entirely by pitch (§6.1).

**Coherence.** The fraction of pixels whose distance disagrees with their
own 3×3 neighbourhood, in excess of the reference doing the same.

That last metric exists because of a failure. Our first correctness
measure scored coverage only. It is structurally unable to detect
over-drawing: a renderer that draws *too much* geometry scores perfectly,
because every pixel that should be covered is. The mesh scored 0.0%
see-through for weeks *because* it was interpolating the slab layers
across region boundaries and building a roof over everything. Comparing
depth rather than classifying pixels found it; comparing coherence finds
the class of artifact — striping, speckle, cracks — that correct-on-average
depth still misses.

## 5. Results

### 5.1 Pitch is the axis that separates them

Fostral, river viewpoint, view distance 600. `[lavapipe]`

| pitch | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.25 |
|---|---|---|---|---|---|---|
| 0° (horizon) | 25.6 / 0.0 / 3.9 | 8.4 / 9.3 / 0.3 | 14.3 / 8.3 / 7.3 | 22.8 / 3.3 / 5.1 | 8.1 / 9.3 / 0.2 | 8.3 / 9.1 / 0.1 |
| −45° | 0.0 / 0.0 / 0.0 | 0.0 / 0.0 / 0.1 | 0.0 / 0.0 / 0.9 | 29.9 / 0.0 / 29.9 | 0.0 / 0.0 / 0.0 | 0.0 / 0.0 / 0.0 |

*(see-through / covers-sky / speckle, %)*

Tilt the camera 45° down and five of the six become indistinguishable:
every coverage figure is 0.0%. At the horizon they diverge by more than
an order of magnitude. Frame time behaves the same way — the painter goes
from 209 ms to 1.4 ms on the same scene.

This is the central result. **These methods were developed and validated
at the viewpoint where they agree.** The original engine was top-down; so
was every screenshot used to check the reimplementations. The
differences that matter appear only at eye level, which is where a
first-person or chase camera lives.

*(TODO: full sweep at 0/−30/−60/−90 across all four viewpoints, on
hardware. The two-row table above is a sample, not the result.)*

### 5.2 Frame time

`[lavapipe]` and one consumer GPU run that was floor-limited — every mesh
figure landed in 0.8–1.0 ms including quality settings that differ 3× in
triangle count, which means submission cost dominated. **TODO: re-run at
4K with timestamp queries.**

### 5.3 Fit cost

Fostral, triangles against a full grid mesh:

| quality | max error | vertices | triangles | reduction |
|---|---|---|---|---|
| 0.0 | 16 | 1.74 M | 3.39 M | 19.8× |
| 0.25 | 8 | 2.31 M | 4.50 M | 14.9× |
| 0.5 | 4 | 4.28 M | 8.47 M | 7.9× |
| 0.75 | 2 | 7.14 M | 14.1 M | 4.7× |
| 1.0 | 1 | 11.3 M | 22.4 M | 3.0× |

Against roughly 80× for a smooth synthetic surface at comparable
tolerance. **TODO: the same sweep on a natural DEM, same code — this is
the control that makes the claim a measurement.**

## 6. Findings

### 6.1 The reference is only tight away from grazing incidence

Same renderer, same reference, varying pitch:

| pitch | see-through | covers-sky | depth p50 |
|---|---|---|---|
| 0° | 6.70% | 7.36% | 25.3u |
| −15° | 0.00% | 0.00% | 5.5u |
| −30° | 0.00% | 0.00% | 3.7u |
| −60° | 0.00% | 0.00% | 1.7u |

At eye level most of the ground is nearly edge-on, and a sub-pixel
difference in ray direction moves the hit by tens of units. At pitch 0
the converged renderers agree with *each other* to 1.3 u while all
sitting ~29 u from the reference, at a signed median of 0.01 u — scatter,
not bias. Any ground-truth comparison of first-person terrain has this
floor, and reporting absolute error without it overstates precision.

### 6.2 Greedy TIN degrades on authored terrain

14.9× where a smooth surface gives ~80×. Vangers terrain is cliff-heavy
and detailed per texel; the error-driven insertion that exploits
smoothness has little to exploit.

### 6.3 A quarter of the vertex budget goes to one discontinuity

Counting what drove each insertion, Fostral at quality 0.25:

| driver | share |
|---|---|
| `low` (the floor) | 53.5% |
| slab interior | 18.8% |
| **single/double-level boundary** | **23.3%** |
| chunk-border simplification | 4.4% |

A single-level texel reports `mid = high = low`, so both step by the full
slab thickness across a region edge, and the error metric chases a
discontinuity no tolerance can satisfy — it only shrinks triangles toward
texel size along every boundary. The absolute cost is near-constant in
quality (338 k insertions at q=0, 396 k at q=1) while the floor's grows
444 k → 6.1 M. That signature — flat in tolerance — distinguishes a
geometric feature from a fit converging, and generalises to any
error-driven fit over a field with embedded discontinuities.

Constraining the triangulation to the region outline would remove it, and
is the same change that would stop straddling triangles dropping the slab.

### 6.4 Coverage metrics cannot see over-drawing

§4.2. Worth stating as a methodological result: a correctness measure for
a renderer must be able to fail in both directions.

## 7. Limitations

- One dataset. Generalisation beyond Vangers terrain is asserted, not
  measured, until the DEM control and the other stock worlds are run.
- Timing is per-frame latency, not pipelined throughput.
- Unequal tuning. The voxel step budget was found badly mistuned during
  this work (40 → 200 steps, worth 21.8% of a frame); the slicer and the
  painter have had no equivalent pass, and their numbers should be read
  as a lower bound on their potential.
- The mesh needs ~300 MB resident at q=0.75 on a full level. Chunk
  streaming is not implemented, and until it is, "runs on low-end
  devices" is a claim about the pipeline, not the memory budget.

## 8. Conclusion

*(TODO)*

## References

- Garland, M. and Heckbert, P. 1995. *Fast Polygonal Approximation of
  Terrains and Height Fields.* CMU-CS-95-181.
- Fowler, R. and Little, J. 1979. *Automatic extraction of irregular
  network digital terrain models.* SIGGRAPH.
- Shewchuk, J. R. 1997. *Adaptive Precision Floating-Point Arithmetic and
  Fast Robust Geometric Predicates.*
- Duchaineau, M. et al. 1997. *ROAMing terrain: real-time optimally
  adapting meshes.* IEEE Visualization.
- Ulrich, T. 2002. *Rendering massive terrains using chunked level of
  detail control.* SIGGRAPH course.
- Losasso, F. and Hoppe, H. 2004. *Geometry clipmaps.* SIGGRAPH.
- *(TODO: voxel/ray-march terrain lineage; Vangers technical history.)*
