---
layout: post
title: Terrain as a Mesh
---

Every terrain renderer in vange-rs so far has marched rays through the
height map. This one does not: it fits an actual triangle mesh to the
level and hands it to the rasterizer. This post covers how the mesh is
built, how the multi-layer terrain survives the trip, and how it measures
up against the ray marchers on a real level.

## Why

The existing methods struggle at eye level. Ray marching a height field
misses occluders at grazing angles, and voxel tracing needs an
acceleration structure that must be baked before it is correct. A mesh
has neither problem: the geometry *is* the answer, and the hardware
rasterizer is the part of the GPU most likely to be fast on the widest
range of devices — which matters if the goal is to run everywhere,
WebGL included.

## Fitting the mesh

The level is a height map, so the obvious mesh is two triangles per
texel. For Fostral that is 67 million triangles, which is absurd for
terrain that is mostly smooth. Instead we build a *triangulated irregular
network*: a Delaunay triangulation over a chosen subset of the samples,
fitted by greedy insertion, following Garland & Heckbert's
[Fast Polygonal Approximation of Terrains and Height Fields][gh] (1995).

Start with two triangles spanning a chunk. Every triangle remembers the
grid sample inside it with the largest vertical error. Repeatedly take
the globally worst sample, insert it as a vertex, and re-scan the
triangles that changed. Stop when every sample is within tolerance.

Two things make this cheap. Because each triangle already knows its own
worst sample, popping the global worst needs no point location at all —
we know which triangle contains it. And because insertion is local
(Bowyer-Watson: collect the triangles whose circumcircle contains the new
point, retriangulate that cavity), each step touches only a handful of
triangles.

The predicates run on chunk-local *integer* coordinates, so `orient2d`
and `in_circle` are exact in `i64`. This is not fussiness: a height map
grid is massively cocircular — every axis-aligned square of four samples
lies on a circle — and that is precisely the case floating-point
predicates get inconsistent, which produces flipped triangles or
non-terminating flip loops.

[gh]: https://mgarland.org/files/papers/scape.pdf

## The multi-layer part

A Vangers texel is not a single height. It may be *double level*: a floor,
a cave ceiling, and a slab top, stacked over the same spot. Fostral is
10.9% double-level — tunnels, bridges, the structures around citadels.

Rather than triangulating each surface separately, all three share one
planar triangulation in XY. The error metric is the largest deviation
across *any* layer, so a point gets inserted when any surface needs it,
and every vertex carries all three altitudes. Where a region ends the
layers collapse onto each other.

From that, each triangle emits:

* the `low` floor, always;
* the `high` slab top and the `mid` cave ceiling, if the triangle lies
  inside a double-level region;
* vertical walls from `mid` to `high` along the edges where the slab ends,
  so it does not stop in mid-air.

A triangle counts as inside a double-level region only if *every* grid
sample within it is dual, not merely its three corners. Corner testing
looks equivalent and is not: real regions have ragged outlines, so a
coarse triangle can have three dual corners while spanning terrain that
mostly is not. Interpolating `high` across that gap builds a roof sloping
up to some distant vertex's slab, walled off at its edges — which on a
river view covered the entire sky.

## Chunking, LOD and updates

The level is cut into 128x128 chunks, each with its own vertex and index
buffer. Per-chunk buffers are not just tidiness: one buffer for a whole
level runs past `max_buffer_size` on WebGL-class limits long before the
geometry itself is unreasonable — Fostral at quality 0.75 is 119 MB of
vertices and 177 MB of indices, but only ~57 KB + ~84 KB per chunk per
level of detail.

Chunk borders are simplified by a Douglas-Peucker pass over the same
"worst layer" metric. It is a pure function of the samples, so the two
chunks sharing a border independently derive the same vertices and the
seam is crack-free — including after an edit, because chunks overlap by
their shared row or column, so an edit on a border dirties both sides.

Greedy insertion gives level of detail almost for free: level *k* is the
same fit stopped at a doubled tolerance, costing a fraction to build and
roughly half the triangles per step. Each chunk keeps several, and the
renderer picks one per chunk from its distance to the camera, after
frustum-culling every (chunk, wrap tile) pair. Survivors are drawn near to
far so the depth test can reject occluded chunks before shading them.

Terrain edits refit in place. The triangulation is planar in XY and
altitudes are plain vertex attributes read at emit time, so an edit that
merely raises or lowers the surface re-emits with identical topology; new
vertices appear only where an edit introduced detail the existing
triangles cannot represent. Vertices are only ever added, never removed,
which means an edited chunk can end up denser than a fresh fit of the same
terrain — but never coarser, so the tolerance always holds.

## Shading

Lighting uses the polygon's own normal, taken from screen-space
derivatives of the world position. The ray marchers have to rebuild a
normal from height map taps — four smooth surface lookups, sixteen texture
fetches per fragment — and this mode was doing the same until it was
pointed out that it already had the real thing. Besides being cheaper
(39% off the frame where fragments dominate) it is the only way to get a
sensible normal on vertical walls and cave ceilings, where the height
field gradient is undefined.

Terrain *types* are still read per fragment from the terrain texture
rather than baked into vertices, so type boundaries stay at full texel
resolution however coarse the triangles get.

## How it measures up

Scored against a CPU ray cast of the level's own height data, on Fostral,
first person, eye 8 units above the surface. The number that matters is
the fraction of solid terrain a renderer draws as sky. Reproduce with
`tools/compare-terrain.py`.

Depth agreement between renderers, as the fraction of the frame more than
60 units apart, at a view distance of 600:

| view | mesh vs voxel | mesh vs painter | painter vs voxel |
|---|---|---|---|
| tunnel interior | 1.0% | 1.2% | 0.2% |
| river below a span | 0.6% | 0.7% | 0.3% |
| deep canyon | 6.5% | 8.3% | 1.9% |
| open ridge | 0.3% | 0.4% | 0.1% |

The voxel tracer, the painter and the mesh agree to about a percent
everywhere except the deep canyon, which is a high-relief view where the
fit tolerance shows: raising quality to 1.0 takes it to 2.2%. Height-field
ray tracing is the outlier, drawing 38-60% of solid terrain as sky at eye
level regardless of settings - it is the method this work exists to
replace.

Three settings have to be right or the comparison is meaningless, and
each of them cost a wrong conclusion during development:

* **View distance** must be bounded. The painter emits one instance per
  visible ground sample and clamps at a million, so an unbounded distance
  leaves 95% of its frame unpainted.
* **The voxel grid must be fully baked.** It bakes incrementally at a
  million texels a frame, so a 2048x16384 level needs ~150 frames. A
  partial grid is both wrong *and* slow, because rays march on without
  early termination.
* **The voxel step budget must be large enough.** 40 was fine for the
  top-down views it was tuned against; at eye level long sightlines
  exhaust it and terrain reads as sky.

Frame times, 400x260 on lavapipe — a *software* rasterizer, so these
understate the one advantage the mesh is built around:

| view | RayTraced | RayVoxel | Mesh q=0.25 | Mesh q=0.75 |
|---|---|---|---|---|
| tunnel | 3.4 | 22.0 | 11.6 | 28.9 |
| river | 4.2 | 51.6 | 48.6 | 84.5 |
| canyon | 2.9 | 17.7 | 65.8 | 95.8 |
| ridge | 5.6 | 25.7 | 19.8 | 33.8 |

Height-field ray tracing fails everywhere at eye level — around half of
every frame — which is what motivated all of this. Voxel tracing is
correct once two things are true: the grid is fully baked (~150 frames on
Fostral, budgeted at a million texels a frame) and the ray-march step
budget is large enough. The old default of 40 steps was fine for the
top-down views it was tuned against and badly wrong in first person; at
200 the errors above drop to near zero.

The mesh is the only method that never sees through terrain, at either
quality. Its cost varies three-fold with how much geometry lands in the
frustum, which points at an untuned LOD distance rather than anything
fundamental.

## What this is not, yet

* **The reduction on real terrain is modest.** A smooth synthetic surface
  compresses 80x; Fostral manages 4.6x at quality 0.75 and 16x at 0.0.
  Vangers levels are hand-authored, cliff-heavy and detailed per texel —
  not the natural DEMs the greedy-insertion literature is measured on.
* **Memory is the open problem.** ~300 MB resident at quality 0.75 on a
  full level. Per-chunk buffers removed the hard limit; keeping only the
  chunks near the camera resident is the remaining work, and it is what
  stands between this and the "runs on any device" goal.
* **The LOD distance is a constant.** It has not been tuned, and the
  three-fold cost swing between views is the evidence.
* **The double-level boundary is conservative.** Triangles straddling a
  region's edge drop the slab rather than approximating it, which reads as
  a gap in a tunnel roof. Constraining the triangulation to the
  `DOUBLE_LEVEL` outline would make it exact.
* **No GPU numbers.** Every measurement here is from a software
  rasterizer, which is the one platform that cannot show what hardware
  rasterization is for.
