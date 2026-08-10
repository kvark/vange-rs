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
rendering methods — height-field ray marching, voxel-accelerated ray
marching, sliced proxy geometry, per-sample bar rasterization, compute
scattering, and a fitted triangle mesh — implemented in a single engine
over a single data path, on the hand-authored multi-layer terrain of
*Vangers* (1998), scored against a CPU ray cast of the same source data.

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
DEMs, games ship authored terrain, nobody has published the comparison.
JCGT prefers short introductions and only directly relevant citations,
so the related work belongs here as one tight paragraph per method
family — marching, voxel traversal, TIN/LOD — not as its own section.)*

Contributions:

1. A controlled comparison of six terrain rendering methods sharing one
   engine, one data path, one camera and one fragment-stage shading
   path, so differences are attributable to the method rather than the
   surrounding system.
2. An evaluation methodology scoring against a CPU ray cast of the source
   data, decomposed into coverage, geometric and coherence error, with
   the failure mode of the naive version documented.
3. The engine, the harness and the measurement protocol, released under
   a permissive license (see *Data availability* below).
4. A ten-world survey isolating what actually drives fit cost, and the
   mechanism behind it.

**Data availability.** *(TODO — resolve before submission; JCGT requires
provided code and data to carry a non-restrictive open-source license.)*
The engine and every tool in the harness are Apache-2.0. The terrain
itself is the original game's content: the harness fetches the stock
level archives from a public release and converts them on first run, and
the paper must state the redistribution terms of that data explicitly —
it is the one artifact here whose license is not ours to choose. The
fallback, if redistribution cannot be cleared, is to publish the
converter and fetch scripts and point at the game's own distribution.

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
palette, and are driven by the same camera. More than that: **all six
colour through one shared function** over the same surface data. Five
evaluate it per output pixel in the fragment stage, so terrain-type
boundaries stay at full texel resolution however coarse the geometry;
the scatterer evaluates the same function per sample in compute and
splats the resulting palette index, so its colour resolution is its
sample density. What varies between methods is how a pixel finds its
piece of surface — which is exactly the variable under test. (The one
other shading difference: the mesh takes its normal from the polygon's
own screen-space derivatives, which is what makes its vertical walls and
cave ceilings shade sensibly where a height gradient is undefined.)

The organising taxonomy is *who asks the question*. Two methods are
backward, image-order: each pixel casts a ray and asks the height data
what it hits. Three are forward, object-order: pieces of terrain are
enumerated and ask the screen where they land — as proxy volume slices,
as rasterized bars, or as compute-splatted points. The sixth fits an
explicit surface at load time and lets the rasterizer do what it is
built for.

| method | order | primitive | needs compute |
|---|---|---|---|
| Height-field ray march | image | per-pixel ray | no |
| Voxel-accelerated ray march | image | per-pixel ray | yes (bake) |
| Sliced | object | horizontal proxy quads | no |
| Painted | object | bars per ground sample | no |
| Scattered | object | compute-scattered points | yes |
| **Mesh (TIN)** | object | fitted triangles | no |

### 3.1 Height-field ray march

A full-screen pass; each fragment reconstructs its world-space ray from
the inverse view-projection and marches the height field: fixed forward
steps to bracket the first crossing, then binary refinement (16 and 4
above ground, 12 and 3 under a slab; compile-time constants). The dual layer makes the march stateful: a ray
that passes under a cave ceiling continues *underneath* the slab,
testing against the floor, with the right to re-emerge past the slab's
far edge. Depth is written from the hit point, so everything composes
with rasterized geometry. There is no preprocessing and no per-level
state; the failure mode is the fixed step budget, which under-samples
exactly when rays run flat along the ground (§5).

### 3.2 Voxel-accelerated ray march

The same per-pixel cast, accelerated by a conservative occupancy
structure baked on the GPU: one bit per voxel, Morton-tiled in a storage
buffer, with a pyramid of levels each halving the resolution — an
implicit octree over the height field. Traversal is hierarchical DDA:
descend a level on hitting an occupied cell, skip whole cells where
empty, climb back up when leaving an octant. Inside an occupied leaf the
hit is refined by linearly sampling the *actual* height data. The
voxels only skip empty space — the surface rendered is the exact
height-field geometry, which is why this method scores with the
converged group in §5 rather than at voxel resolution. The bake spreads
over frames under a per-frame texel budget, which is a real cost the
per-frame numbers hide (§5.3).

### 3.3 Sliced

The level's height range is divided into `N` horizontal quads spanning
the visible sample range, drawn instanced from the top down. Each
fragment reads the surface under it and keeps the pixel only if the
slice's altitude is inside the solid column — below the floor, or
between the cave ceiling and the slab top — discarding otherwise. The
union of cross-sections approximates the volume; at one slice per
altitude unit the quantised heights make it exact, and the publication
default (§5.5) runs two. Cave
interiors are dimmed by a constant factor rather than shadowed.

### 3.4 Painted

One instance per ground sample in view, rasterizing the sample's column
directly: a bar from zero to the floor height, and a second bar from
cave ceiling to slab top where the texel is double-level. Only the three
camera-facing faces of each bar are emitted, chosen by comparing the
camera position against the bar's centre, and instances are generated
in front-to-back order along the dominant camera axis. *(TODO: confirm
against the original source before claiming it — this is believed to be
the closest of the six to the original engine's software renderer.)*

### 3.5 Scattered

A compute pass distributes point samples over a camera-aligned footprint
of the visible ground, warped so that sample density falls off with
distance. Each sample reads the surface and splats single pixels — along
the column's vertical extent, both layers — with a 32-bit `atomicMin`
into a storage buffer, packing 24 bits of depth over 8 bits of palette
index so the depth test and the colour resolve are one atomic. A
full-screen pass then unpacks the buffer into colour and depth.
Under-sampling shows up not as missing spans but as isolated wrong
pixels, which is precisely the artifact class the coherence metric
(§4.2) exists to count.

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
culled, and drawn near-to-far. Chunk borders are simplified with
Douglas–Peucker [1973] at the *finest* tolerance regardless of the
chunk's own level, so neighbouring chunks at different levels derive
identical boundary vertices and the seam cannot crack.

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

### 4.3 Protocol

Every number in this paper is produced by one command with no arguments
(`tools/compare-terrain.py`), whose defaults *are* the publication
configuration: every method, three viewpoints, four pitches (0°, −30°,
−60°, −90°), 1280×800, 40 timed frames after per-method warmup. The
harness builds the binaries, fetches and converts the level on first
run, and names its output after the adapter wgpu selected, so runs
collected from different machines merge without hand-labelling
(`tools/merge-bench.py`). The design goal is that a measurement session
on a new device costs its owner one invocation and about an hour — the
protocol is only as reproducible as it is cheap to follow.

Frame times come from GPU timestamp queries bracketing the frame's
command encoder, falling back to a CPU submit-and-poll bracket where the
device lacks them; each row records which timing it used, because the
two disagree in exactly the regime where these methods are fast (§5.2).
One-time costs are recorded separately as setup / first frame / warmup
(§5.3), since per-frame figures structurally exclude them. Accuracy
metrics are device-independent; the merge tool reports them once and
cross-checks the other devices against the first, because an adapter
that disagrees about geometry is a finding, not noise.

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

*(TODO: the full sweep on the current viewpoints — a hardware sweep over
all four pitches on the previous fixture is recorded and has the same
shape, and the protocol makes the re-run one command. The two-row table
above is a sample, not the result.)*

### 5.2 Frame time

Measured with GPU timestamp queries bracketing the frame's command
encoder, so the figure is the device's own view of its work with no
submission or round trip in it. On lavapipe the CPU-side submit-and-poll
bracket runs about 9% higher than the timestamp pair; on a fast GPU with
a sub-millisecond frame it can be most of the number, which is why an
earlier consumer-GPU run reported every mesh configuration at 0.8–1.0 ms
regardless of a 3× difference in triangle count.

The first hardware pass (Radeon 890M, RADV, GPU timestamps; river view
of the earlier fixture) is the pitch axis of §5.1 as frame time:

| pitch | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.25 |
|---|---|---|---|---|---|---|
| 0° | 0.38 | 5.00 | 5.88 | 7.47 | 9.16 | 0.24 |
| −30° | 0.65 | 3.46 | 2.06 | 7.88 | 9.81 | 0.18 |
| −60° | 0.75 | 2.07 | 0.96 | 6.48 | 0.24 | 0.12 |
| −90° | 0.69 | 2.00 | 0.85 | 7.64 | 0.26 | 0.16 |

*(ms per frame)* The painter's 35× swing between the horizon and
straight down is the timing version of the coverage story: cost tracks
how much of the scene the method has to emit, and looking down emits
almost nothing. The ray marcher is the mirror image — cheaper at the
horizon than against a full frame of ground. *(TODO: re-run on the
current viewpoints; the protocol makes that one command.)*

### 5.3 Preparation cost

Per-frame numbers exclude one-time work, which differs by two orders of
magnitude between these methods. Fostral, CPU wall time in milliseconds:
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

### 5.4 Fit cost

Fostral, triangles against a full grid mesh:

| quality | max error | vertices | triangles | reduction |
|---|---|---|---|---|
| 0.0 | 16 | 1.74 M | 3.39 M | 19.8× |
| 0.25 | 8 | 2.31 M | 4.50 M | 14.9× |
| 0.5 | 4 | 4.28 M | 8.47 M | 7.9× |
| 0.75 | 2 | 7.14 M | 14.1 M | 4.7× |
| 1.0 | 1 | 11.3 M | 22.4 M | 3.0× |

Against roughly 80× for a smooth synthetic surface at comparable
tolerance (our own control, not a literature figure), and 45–182× for
the single-layer stock worlds (§6.2). The
stock worlds are the better control than an external elevation model
would be: they vary only content, holding encoding, quantisation, texel
scale and authoring pipeline fixed, so the comparison isolates one
variable rather than four.

### 5.5 Tuning

Each method was swept over its own quality knob and given the cheapest
setting within one percentage point of its own best error, so no method
is charged for a setting that buys nothing or credited with speed it
reaches only by being wrong. Fostral, three viewpoints at the horizon,
400x260, view distance 600. Accuracy is device-independent (§4.3), so
the sweep runs wherever is convenient; the numbers below are from the
GPU pass recorded in `paper/tuning.md`.

| method | knob | swept | chosen | error at choice |
|---|---|---|---|---|
| RayTraced | — | compile-time steps (16 fwd / 4 binary) | — | 66.1% |
| Painted | — | none beyond view distance | — | 4.5% |
| Sliced | slices | 32–512 | **512** | 9.8% |
| Scattered | density | 1–4 | **4,4,4** | 57.3% |
| RayVoxel | grid, steps | 2 grids × 40–400 | **4,8,2, 40 steps** | 5.5% |
| Mesh | fit tolerance | q 0.0–1.0 | **q=0.0** | 4.4% |

Four of the results are worth stating.

**The first slicer sweep measured the knob, not the method.** The slice
count was exposed for this comparison, and its first implementation kept
unit spacing and truncated slices off the *bottom* of the height range —
so "128 slices" silently deleted all terrain below half height, which
the sweep read as a collapse: 61.2% see-through and surfaces moved by
259 u. With the count honestly spreading slices over the whole range,
128 slices leave 6.2%. A tuning sweep validates the knob's
implementation as much as the method behind it, and a result this
discontinuous is a reason to suspect the former.

**The corrected slicer knob changes the error's kind, and the fixture
decides the setting.** From 32 to 512 slices, cost rises 7.6× (0.19 →
1.44 ms) while see-through falls 17.2% → 5.4% and speckle rises 2.4% →
4.4%, peaking at 10.9% at 256: slices convert missing spans into
isolated wrong pixels. On the first corrected fixture the sum stayed
within 2.4 points across the whole sweep, the rule had nothing to say,
and we kept one slice per altitude unit — the setting at which the
quantised heights make the cross-sections exact. On the current fixture,
whose views carry more wall and slab in frame, 512 pulls eight points
clear of the next-best setting and the rule takes it. The lesson does
not change: a knob whose total error is flat relative to the measurement
cannot be tuned by an error-based rule, and a sweep whose outcome moves
this much when the fixture changes is a reason to re-read the knob's
implementation before trusting either number.

**The voxel step budget is a property of the fixture, not the method.**
40 steps — the value this work inherited — left 6.4% see-through on the
first fixture's long sightlines, where 100 got to 2.5%; we first
"corrected" it to 200, which was equally accurate and 30% more
expensive. On the current fixture the sightlines are shorter: 40 steps
lands within a point of 100 (5.5% against 4.8%), and the rule keeps the
inherited value. A step budget tunes the longest sightline the
viewpoints put in frame; change the viewpoints and it needs re-tuning,
which is what the protocol's tuning pass is for.

**The reference cannot resolve mesh quality at the horizon.** Every
setting from q=0.0 to q=1.0 lands within 0.1 points of coverage error
(4.2–4.5%) and 5 u of depth error, against a reference whose own floor
there is ~25 u (§6.1). The selection rule therefore picks the cheapest,
which is correct given the measurement and wrong as a shipping default:
measured against its own finest fit instead of against the reference,
the same knob moves surfaces by up to 289 u. Where a parameter changes
geometry more finely than the ground truth can see, it has to be tuned
self-referentially. This is the same blind spot as §4.2, in a different
place.

A fifth result is about the platform rather than the method: the voxel
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
sitting ~25 u from the reference, at a signed median of 0.01 u — scatter,
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
| ark-a-znoy | 4.2 M | 0.05 M | 154.8× | 0.0% | 0.0% | 5.52 | 5.52 |
| threall | 4.2 M | 0.19 M | 45.3× | 0.0% | 0.0% | 2.85 | 2.85 |
| xplo | 8.4 M | 0.87 M | 19.4× | 17.0% | 1.4% | 7.54 | 8.80 |
| khox | 4.2 M | 0.52 M | 16.1× | 9.3% | 4.8% | 6.94 | 7.71 |
| fostral | 33.6 M | 4.50 M | 14.9× | 35.6% | 10.9% | 3.44 | 6.54 |
| necross | 33.6 M | 8.41 M | 8.0× | 42.5% | 17.0% | 4.41 | 14.43 |
| glorx | 33.6 M | 10.97 M | 6.1× | 37.2% | 30.0% | 6.53 | 14.66 |
| boozeena | 4.2 M | 1.46 M | 5.7× | 41.5% | 13.3% | 3.44 | 25.15 |
| hmok | 4.2 M | 1.61 M | 5.2× | 47.2% | 38.0% | 2.71 | 18.77 |

`rough(floor)` is the mean absolute discrete Laplacian of the `low` layer
alone — the terrain's own curvature, blind to the second layer.
`rough(surface)` is the same measure on the composite surface the fitter
sees.

The hypothesis fails. Across all ten worlds, correlation of log
reduction against `rough(floor)` is **−0.17**; against the double-level
texel fraction it is **−0.77**, and against `rough(surface)` — the data
the fitter actually sees, curvature the slab edges put there — it is
**−0.82**. (The share of the fitted triangles that end up on slab
surfaces correlates at −0.89, which is the fit's own account of where
its budget went rather than an independent predictor.) Relief does not
predict the fit cost; the second layer does. `ark-a-znoy` makes the
floor-relief hypothesis fail in the other direction too: it is *three*
times as rough as `weexow` on the floor (5.52 against 1.84) and
compresses essentially as well (154.8× against 182.0×), because neither
world has a slab.

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

The same shape recurred twice more during this work, each time as a
measurement that held something true while the thing that mattered was
wrong: a sweep parameter whose implementation did not do what its name
said (§5.5, the slicer's "count" that truncated), and a camera test
asserting a direction that survives a flipped basis. The common failure
is asserting a property the defect preserves. The practical defence we
ended with is to require every metric, knob and fixture to demonstrate
that it *can* fail: reproduce a known-bad configuration and check the
measurement moves.

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
- Residual unequal tuning, after the uniform pass of §5.5. The
  height-field marcher's step counts are compile-time constants and were
  set by eye against its WebGL2 silhouette quality, not swept under the
  rule; and the mesh quality is tuned self-referentially because the
  reference cannot resolve it (§5.5). Both are documented rather than
  eliminated.
- The mesh needs ~300 MB resident at q=0.75 on a full level. Chunk
  streaming is not implemented, and until it is, "runs on low-end
  devices" is a claim about the pipeline, not the memory budget.

## 8. Conclusion

*(TODO)*

## Planned figures

The draft currently has none, which for a graphics journal is the
largest single gap. Each figure below has (or needs) a generating
command, same rule as the numbers:

1. **Teaser** — the six methods side by side at the horizon viewpoint
   where they differ, plus the reference. The harness's `--out` PNGs
   are the source; needs a layout script.
2. **§2** — texel-pair encoding diagram, and a vertical slice through a
   double-level region (floor, cave, slab) rendered from the data.
3. **§4.2** — error decomposition triptych for one frame: see-through
   mask, covers-sky mask, speckle mask, over the rendered image. The
   harness already emits the masks.
4. **§5.1** — pitch sweep chart: error vs pitch per method, the
   "separate sharply at the horizon" curve.
5. **§6.2** — scatter plot of log reduction vs double-level fraction
   across the ten worlds, the r = −0.77 picture (`tools/level-survey.py`
   output).
6. **§6.3** — mesh wireframe at a single/double-level region boundary,
   showing triangles shrinking to texel size along the discontinuity.
7. **§3.6 / appendix** — top-down frustum + per-chunk LOD/culling plan
   (`tools/plot-cull.py`, already implemented).

## References

- Garland, M. and Heckbert, P. 1995. *Fast Polygonal Approximation of
  Terrains and Height Fields.* CMU-CS-95-181.
- Fowler, R. and Little, J. 1979. *Automatic extraction of irregular
  network digital terrain models.* SIGGRAPH.
- Shewchuk, J. R. 1997. *Adaptive Precision Floating-Point Arithmetic and
  Fast Robust Geometric Predicates.*
- Douglas, D. and Peucker, T. 1973. *Algorithms for the reduction of the
  number of points required to represent a digitized line or its
  caricature.* Cartographica. (Chunk-border simplification, §3.6.)
- Duchaineau, M. et al. 1997. *ROAMing terrain: real-time optimally
  adapting meshes.* IEEE Visualization.
- Ulrich, T. 2002. *Rendering massive terrains using chunked level of
  detail control.* SIGGRAPH course.
- Losasso, F. and Hoppe, H. 2004. *Geometry clipmaps.* SIGGRAPH.
- Amanatides, J. and Woo, A. 1987. *A fast voxel traversal algorithm for
  ray tracing.* Eurographics. (The DDA that §3.2 runs hierarchically.)
- Laine, S. and Karras, T. 2010. *Efficient sparse voxel octrees.* I3D.
  (The contrast: §3.2's voxels accelerate an exact height field rather
  than replace it.)
- Policarpo, F., Oliveira, M. and Comba, J. 2005. *Real-time relief
  mapping on arbitrary polygonal surfaces.* I3D. (Per-pixel height-field
  marching lineage for §3.1.)
- Tevs, A., Ihrke, I. and Seidel, H.-P. 2008. *Maximum mipmaps for fast,
  accurate, and scalable dynamic height field rendering.* I3D.
- *(TODO: Vangers technical history — K-D Lab 1998; the open-source
  release of the original engine is the citable artifact for the
  "Surmap" column renderer that §3.4 mirrors.)*
- *(TODO: a citable source for the DEM reduction ratios that §5.4/§6.2
  compare against — Garland–Heckbert's own reported ratios if they
  suffice, otherwise a survey; the "~80×" smooth-surface figure is our
  own synthetic control and must be labelled as such where used.)*
