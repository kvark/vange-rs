# Six Ways to Draw Vangers: Terrain Rendering on Hand-Authored Multi-Layer Height Fields

**Status: draft.** Numbers marked `[lavapipe]` are from a software
rasterizer and are placeholders for hardware runs. Numbers marked `TODO`
have no measurement behind them yet and must not survive into a
submission.

---

## Abstract

Terrain level-of-detail is measured almost exclusively on digital
elevation models: single-valued, smooth at the sampling scale, sampled
from real topography. Game terrain is often none of these. We compare six
rendering methods — height-field ray marching, voxel-octree ray marching,
sliced proxy geometry, per-sample bar rasterization, compute scattering,
and a fitted triangle mesh — implemented in a single engine over a single
data path, on the hand-authored multi-layer terrain of *Vangers* (1998),
scored against a CPU ray cast of the same source data.

We report three results. First, the cost of fitting a triangulated
irregular network to this terrain varies by more than an order of
magnitude across the ten shipped worlds, and what predicts the variation
is not the terrain's relief but the fraction of it carrying a second
layer. Worlds with a single layer compress in line with published
elevation-model results; heavily double-level worlds are several times
worse, and a large share of the fit's vertex budget is spent resolving
one discontinuity — at a cost that does not fall as the tolerance
tightens, which is the signature of a geometric feature rather than a fit
converging. Second, the six methods are nearly indistinguishable when the
camera looks down and separate sharply as it comes to the horizon: the
viewpoint the original engine never used, and a modern one cannot avoid.
Third, a correctness metric based on coverage alone cannot detect
over-drawing; ours concealed a real geometric defect through several
rounds of apparently passing measurement, and we give the decomposition
that exposes it, along with the conditions under which a ground-truth
comparison stops being able to resolve a method's own quality setting.

We release the engine, the evaluation harness, and a per-device
measurement protocol that reduces a full run to a single command.

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
4. A ten-world survey isolating what actually drives fit cost, and the
   mechanism behind it.

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

Measured with GPU timestamp queries bracketing the frame's command
encoder, so the figure is the device's own view of its work with no
submission or round trip in it. On lavapipe the CPU-side submit-and-poll
bracket runs about 9% higher than the timestamp pair; on a fast GPU with
a sub-millisecond frame it can be most of the number, which is why an
earlier consumer-GPU run reported every mesh configuration at 0.8–1.0 ms
regardless of a 3× difference in triangle count.

**TODO: hardware runs. The harness reports both figures and records which
one each row used.**

### 5.3 Preparation cost

Per-frame numbers exclude one-time work, which differs by two orders of
magnitude between these methods. khox, CPU wall time in milliseconds:
`[lavapipe]`

| method | setup | first frame | warmup |
|---|---|---|---|
| RayTraced | 68 | 35 | 36 |
| RayVoxel | 71 | 310 | **7535** |
| Sliced | 44 | 31 | 39 |
| Scattered | 44 | 33 | 56 |
| Painter | 47 | 144 | 345 |
| Mesh q=0.25 | 38 | **1319** | 1324 |
| Mesh q=0.75 | 38 | **2173** | 2182 |

`setup` builds pipelines and uploads the terrain texture; `first frame`
adds whatever the method builds lazily; `warmup` covers every pre-timing
frame. The two methods that pay anything substantial pay it differently.
The mesh fits its triangulation once, on the CPU, in a single blocking
1.3 s — a load-time cost that a level cannot be entered without. The
voxel grid bakes incrementally under a per-frame texel budget, spreading
7.5 s across frames that are individually playable but render through
terrain the bake has not reached yet.

Neither is visible in a steady-state frame time, and for a level-loading
budget the difference between them matters more than the per-frame gap.

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
tolerance, and 45–182× for the single-layer stock worlds (§6.2). The
stock worlds are the better control than an external elevation model
would be: they vary only content, holding encoding, quantisation, texel
scale and authoring pipeline fixed, so the comparison isolates one
variable rather than four.

### 5.4 Tuning

Each method was swept over its own quality knob and given the cheapest
setting within one percentage point of its own best error, so no method
is charged for a setting that buys nothing or credited with speed it
reaches only by being wrong. Fostral, four viewpoints at the horizon,
400x260, view distance 600. `[lavapipe]`

| method | knob | swept | chosen | error at choice |
|---|---|---|---|---|
| RayTraced | — | none exists | — | 40.4% |
| Painted | — | none beyond view distance | — | 2.5% |
| Sliced | slices | 32–512 | **256** | 14.0% |
| Scattered | density | 1–4 | **4,4,4** | 26.8% |
| RayVoxel | grid, steps | 2 grids × 40–400 | **4,8,2, 100 steps** | 2.9% |
| Mesh | fit tolerance | q 0.0–1.0 | **q=0.0** | 3.1% |

Three of the six results are worth stating.

**The slicer does not degrade, it collapses.** One slice per altitude
unit is not a quality setting but a correctness threshold: 256 slices
leave 4.7% see-through, 128 leave 61.2% and move surfaces by 259 u. The
shipped default was already at the threshold, but nothing recorded that
it was a threshold rather than a preference.

**The voxel step budget was mistuned in both directions.** 40 steps —
the value this work inherited — leaves 6.4% see-through where 100 gets
to 2.5%. We first corrected it to 200, which is equally accurate and 30%
more expensive. The knee is 100.

**The reference cannot resolve mesh quality at the horizon.** Every
setting from q=0.0 to q=1.0 lands within 0.5 points of coverage error and
1 u of depth error, against a reference whose own floor there is ~50 u
(§6.1). The selection rule therefore picks the cheapest, which is correct
given the measurement and wrong as a shipping default: measured against
its own finest fit instead of against the reference, the same knob moves
surfaces by up to 289 u. Where a parameter changes geometry more finely
than the ground truth can see, it has to be tuned self-referentially.
This is the same blind spot as §4.2, in a different place.

A fourth result is about the platform rather than the method: the voxel
tracer's production grid (2,4,1) needs 153 MB of storage buffer against
llvmpipe's 134 MB limit, so every voxel figure here is for a grid eight
times coarser than the one that ships. That is a caveat on the software
rasterizer, not on the method, and it resolves on hardware.

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

### 6.2 The multi-layer encoding, not the terrain, sets the fit cost

The obvious reading of a 14.9× reduction where a smooth surface gives
~80× is that hand-authored terrain is simply harder to fit. The ten
shipped worlds let us test that directly: same engine, same encoding,
same 8-bit quantisation, same texel scale, same authoring tools, varying
only content. Fitted at identical tolerance:

| level | texels | triangles | reduction | slab tris | dual texels | rough(floor) | rough(surface) |
|---|---|---|---|---|---|---|---|
| weexow | 4.2 M | 0.05 M | 182.0× | 0.0% | 0.0% | 1.84 | 1.84 |
| threall | 4.2 M | 0.19 M | 45.3× | 0.0% | 0.0% | 2.85 | 2.85 |
| xplo | 8.4 M | 0.87 M | 19.4× | 17.0% | 1.4% | 7.54 | 8.80 |
| khox | 4.2 M | 0.52 M | 16.1× | 9.3% | 4.8% | 6.94 | 7.71 |
| boozeena | 4.2 M | 1.46 M | 5.7× | 41.5% | 13.3% | 3.44 | 25.15 |
| hmok | 4.2 M | 1.61 M | 5.2× | 47.2% | 38.0% | 2.71 | 18.77 |

`rough(floor)` is the mean absolute discrete Laplacian of the `low` layer
alone — the terrain's own curvature, blind to the second layer.
`rough(surface)` is the same measure on the composite surface the fitter
sees.

The hypothesis fails. Across these worlds, correlation of log reduction
against `rough(floor)` is **−0.25**; against `rough(surface)` it is
**−0.88**, and against the double-level fraction **−0.88**. Relief does
not predict the fit cost; the second layer does.

The clearest case is `hmok`. Its floor is *smoother* than `threall`'s
(2.71 against 2.85), and `threall` compresses 45.3×. `hmok` manages
5.2× — nine times worse on flatter ground — because 47% of its triangles
are slab. Its composite roughness is seven times its floor roughness, and
all of that excess is the encoding.

So the honest claim is not that authored terrain defeats greedy TIN.
Single-layer authored terrain compresses 45–182×, comfortably in the
range the literature reports for elevation models, which also confirms
the fitter itself is not the weak link. What defeats it is a second layer
whose altitudes are structural rather than continuous. §6.3 is the
mechanism.

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

- One engine and one terrain format. The ten stock worlds span a factor
  of 35 in fit cost and isolate the multi-layer variable cleanly, but
  every one of them is Vangers data. Whether the mechanism in §6.3
  generalises to other discontinuous auxiliary fields is argued, not
  measured.
- Frame timing is per-frame latency, not pipelined throughput: each frame
  is submitted and awaited in isolation, so nothing overlaps. The
  timestamps make the number GPU work rather than round trip, but they do
  not make it a frame rate.
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
