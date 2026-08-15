# Six Ways to Draw Vangers with WebGPU: Real-Time Rendering of Editable Multi-Layer Height Fields

**Status: measurement draft after the final hardware batch.** Five-device
results from the final scenes, 64-step full-screen dual-solid ray marcher,
and common shadow protocol are integrated below. The dynamic-edit experiment
and execution of the terrain-data license remain before submission.

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
Every method must preserve the two solid intervals available at a ground
sample, render at interactive rates, and reflect local terrain destruction
without reloading the level. These constraints rule out treating caves as
decoration or amortising a static preprocessing step over an immutable map.

We report three results. First, the cost of fitting a triangulated
irregular network to this terrain varies by more than an order of
magnitude across the ten shipped worlds, and what predicts the variation
is not the terrain's relief but the fraction of it carrying a second
layer. Worlds with a single layer compress by 45–182×; heavily double-level
worlds are several times worse, and a large share of the fit's vertex
budget is spent resolving one discontinuity — at a cost that does not fall
as the tolerance tightens, which is the signature of a geometric feature
rather than a fit converging. Second, the six methods are nearly
indistinguishable when the camera looks down and separate sharply as they
approach the horizon — the viewpoint the original engine never used, and a
modern first- or third-person camera cannot avoid.
Third, a correctness metric based on coverage alone cannot detect
over-drawing; ours concealed a real geometric defect through several
rounds of apparently passing measurement, and we give the decomposition
that exposes it, along with the conditions under which a ground-truth
comparison stops being able to resolve a method's own quality setting.

All six implementations use the same WebGPU-compatible wgpu and canonical
WGSL path, including its validation, limits, and robust-access requirements.
We release the engine, the evaluation harness, and a per-device measurement
protocol that reduces a full run to a single command.

![The seven measured configurations at the hangar horizon scene. Scattered loses coverage, while the coarse mesh misses a wall recovered by the finer fit.](figures/teaser.svg)

## 1. Introduction

Terrain rendering research usually starts with a digital elevation model:
a single-valued surface whose resolution can be traded against screen-space
error. Authored game terrain can violate each part of that model. It may be
quantised, intentionally discontinuous, and multi-layered, and it is judged
from cameras that the original authoring tool never showed. Those
differences make familiar reduction ratios and quality settings poor
predictors until they are measured on the actual data.

The historical baseline deserves emphasis. Almost three decades ago,
*Vangers* rendered this destructible, multi-layer world in software on consumer
CPUs while fitting the game and its streamed terrain into a 16 MB-era memory
budget [K-D Lab / KranX; Malyshau 2019]. It succeeded by co-designing the
encoding, renderer, and a narrow oblique top-down camera. The distance since
then is simultaneously large and small: modern GPUs make arbitrary cameras,
common shadowing, and portable programmable shading practical, yet the horizon
and cave cases below still punish a representation that is not matched to the
data.

![The original Vangers software renderer presents volumetric, destructible terrain through a deliberately constrained oblique top-down view.](../docs/assets/original.jpg)

This study has three non-negotiable system constraints. First, the renderer
must preserve multiple vertical solid intervals, including the underside of
an upper slab. Second, it must run inside an interactive frame budget rather
than produce an offline conversion. Third, a bounded edit to height and layer
metadata must become visible without a level reload or a full rebuild. The
last constraint is inherited from the 1998 engine: deformation and terrain
destruction are gameplay operations, not authoring-time exceptions; the source
release is the primary implementation record [K-D Lab / KranX]. We report
frame latency rather than assigning a hardware-independent meaning to
"real-time", and separate steady-state rendering from post-edit maintenance.

### 1.1 Related work and scope

The closest published representation is not a conventional height field.
Benes and Forsbach [2001] store a 2D grid of vertical material intervals and
edit it with erosion; this is the same broad height-column/voxel compromise as
the shipped Vangers data, though with variable geological strata rather than
a fixed pair of solids. Their visualization converts the data to height fields
or triangles and does not preserve caves interactively. Grounded Heightmap
Trees [Alonso and Joan-Arinyo 2008] go further by attaching locally oriented
height maps to represent overhangs, tunnels, and large editing events. They are
more general than the fixed vertical Vangers encoding but require a changing
tree of local parameterizations. QuadStack [Graciano et al. 2021] is the
closest direct-rendering system: it run-length encodes vertical stacks,
compresses coherent neighbours in a quadtree, and ray casts the compressed
layers on the GPU. Its published construction and evaluation target compressed
volumes and provide no local edit algorithm; modifying a compressed region can
require recalculating the representation. Our source is already compact and
gameplay edits are required, so we retain direct random access and update only
method-specific dirty regions.

Layered Depth Images also store several samples along a ray [Shade et al.
1998], but those samples belong to one input camera and are splatted into
nearby views. Vangers layers instead live in world-space vertical columns;
they survive arbitrary camera motion and are the editable simulation state.
The similarity is therefore depth complexity, not representation semantics.

The rendering families themselves are established. Relief mapping uses a
linear search followed by refinement through a single height texture
[Policarpo et al. 2005], and GPU landscape ray casting makes the cost
output-sensitive [Mantler and Jeschke 2006]. Maximum mipmaps add conservative
hierarchical skipping while remaining cheap enough to update for dynamic,
single-valued height fields [Tevs et al. 2008]. Our first marcher keeps the
linear/refinement structure but evaluates two solid intervals; our voxel path
generalises the hierarchy to 3D occupancy so it can skip empty caves as well
as air. This is deliberately simpler and more updateable than a compressed
sparse voxel scene [Laine and Karras 2010].

The three forward methods likewise have clear precedents. Texture-based
volume rendering accumulates view-aligned slices [Cabral et al. 1994], while
volume and surface splatting spread each sample over a reconstruction
footprint [Westover 1990; Zwicker et al. 2001]. Our slicer uses horizontal
planes because membership in a vertical interval is then a direct texture
test; the resulting grazing-angle bands are the price. Our scatterer writes
one nearest-depth pixel per sample rather than a filtered footprint, which
explains its holes and temporal speckle. A multi-pixel splat is promising,
but it also multiplies contended atomic writes and is left as a measured
follow-up rather than folded into this comparison without tuning.

Finally, TIN fitting and terrain LOD provide the mesh lineage [Fowler and
Little 1979; Garland and Heckbert 1995; Duchaineau et al. 1997; Losasso and
Hoppe 2004]. Those systems approximate a single-valued surface. Our mesh
shares one topology across three discontinuous altitude fields and locally
refits dirty chunks after edits, which is the source of both its unusual fit
cost and its update burden.

The implementations in this repository were developed from first principles
before this literature audit. That history is not an algorithmic priority
claim. The contribution is the controlled comparison under the three
constraints above, plus the resulting failure analysis, rather than a claim
that ray marching, interval terrain, splatting, slicing, voxels, or greedy
TINs are new.

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
5. An edit-path audit showing which methods consume the live interval field
   directly and which must maintain derived acceleration or mesh data.
6. A five-device portability check across Vulkan and Metal using the same
   validated WGSL shaders and renderer configuration.

### 1.2 A decade-long WebGPU testbed

This comparison is also the record of a long-running implementation, not six
algorithms written for one benchmark. vange-rs began in June 2016 and passed
its tenth anniversary while this study was being prepared. The basic ray path
dates to that first month; the move to the pre-release native wgpu stack and
the sliced and scattered paths followed in 2019, Painted in 2020, the complete
WGSL migration in 2021, RayVoxel in 2022, and the current fitted mesh in 2026.
The techniques therefore accumulated gradually as the engine, API, shader
language, and available hardware matured.

The project served as an early non-trivial integration testbed for the native
wgpu stack developed alongside Firefox's WebGPU implementation. Firefox uses
wgpu-core to validate WebGPU operations and route them through native graphics
APIs, while Naga validates and translates WGSL [Malyshau 2020; gfx-rs; W3C
WebGPU; W3C WGSL]. vange-rs moved to wgpu 0.2 in March 2019, migrated all
terrain, object, and debug shaders to WGSL in 2021 [Malyshau 2021], and
subsequently deployed the same renderer to the web. This history is evidence
of integration pressure, not a priority claim.

No method in this paper bypasses that stack with backend-specific shaders or
unchecked native commands. The comparison therefore asks what quality and
performance these unusual terrain techniques achieve *inside* WebGPU's
portable, validated programming model, rather than how far one backend can be
special-cased. Agreement of the five result grids across four Vulkan adapters
and Apple Metal is the direct portability result; timing remains backend- and
device-specific, and §5.2 keeps unlike timing mechanisms separate.

**Data availability.** The engine and evaluation tools are Apache-2.0. The
terrain is original-game content and is not covered by that license. The
rights holder has indicated that a license for Fostral is forthcoming; the
paper and artifact will identify the grant, rights holder, permitted uses,
and redistribution terms once it is executed. Until then, Fostral archives
and derived image/data bundles remain internal. The ten-world fit survey
also derives statistics from nine other shipped levels, so those levels
must be included explicitly in the grant or replaced by cleared data. The
fallback artifact contains the converter and harness only and requires users
to supply lawfully obtained archives.

## 2. The Data

Vangers terrain is a height map with a per-texel-*pair* dual encoding.
A texel is either single-valued, or carries three altitudes: a floor
(`low`), a cave ceiling (`mid`) and a slab top (`high`), with the even
texel of a pair storing `low`, the odd one `high`, and `mid` derived from
delta bits in both. Fostral, the level used throughout, is 2048×16384 and
10.9% double-level.

![Two adjacent source texels decode into a lower solid and, where present, an upper slab separated by traversable cave space.](figures/encoding.svg)

Two properties matter for everything that follows. The surface is
**authored, not sampled**: cliffs are vertical by construction rather than
by steep gradient, and adjacent texels routinely differ by tens of units.
And `mid`/`high` are **structural, not fields**: they describe where a
slab is, and interpolating them across the region boundary produces
geometry that exists nowhere. Section 6.3 is about what that costs.

The height and metadata arrays are also mutable simulation state. A local
gameplay event may change both altitude and whether the upper interval exists.
The renderer receives a dirty rectangle, uploads the modified rows, and must
make that edit visible. Ray, Sliced, Painted, and Scattered read the updated
texture directly. RayVoxel incrementally rebuilds affected occupancy cells and
their ancestors; Mesh locally refines affected chunks and replaces their GPU
buffers. Thus all six support edits, but their time-to-consistency and update
cost are different quantities that steady-state frame time does not capture.

## 3. Methods Compared

All methods use the same decoded height and material data, camera, palette,
fog, diffuse-lighting function, and shadow map. The method under test is the
way visible surface samples reach the framebuffer. The mesh alone supplies
a geometric normal; the other five estimate a height-field gradient in the
shared shading function.

The common shading path is enforced rather than assumed. An early scatter
implementation stored a shaded palette index in its intermediate buffer,
while the other paths stored a terrain type and shaded later. That made the
entire scatter column darker. It now stores 24 bits of depth and 8 bits of
terrain type, then reconstructs world position and applies the same diffuse
and shadow evaluation in its resolve pass.

The methods fall into three groups. The two image-order methods cast one ray
per pixel. Sliced, Painted, and Scattered enumerate samples of the encoded
volume and project them forward. Mesh performs an offline fit and rasterizes
the resulting explicit surface.

| method | order | primitive | edit response |
|---|---|---|---|
| Height-field ray march | image | per-pixel ray | direct texture read |
| Voxel-accelerated ray march | image | per-pixel ray | incremental occupancy rebuild |
| Sliced | object | horizontal proxy quads | direct texture read |
| Painted | object | bars per ground sample | direct texture read |
| Scattered | object | compute-scattered points | direct texture read |
| **Mesh (TIN)** | object | fitted triangles | local chunk refit |

Every row evaluates both encoded solid intervals. "Direct" means the next
draw after the dirty texture upload sees the edit; it does not mean the CPU
upload itself is free. Derived rows can remain interactive only if the dirty
work is bounded or spread across frames.

### 3.1 Height-field ray march

![One screen pixel casts one ray; fixed samples bracket the first encoded solid and bisection refines the hit.](figures/ray.svg)

A full-screen pass reconstructs the near and far world-space point for each
pixel. The segment is sampled uniformly until it enters the encoded solid,
then four binary-search iterations refine the first crossing. For a
single-level texel, points at or below `high` are solid. For a dual-level
texel, `z <= low` and `mid <= z <= high` are solid. Testing this predicate
directly is important: it detects floors, slab tops, and cave ceilings for
rays travelling in either vertical direction.

The publication setting uses 64 forward samples over the clipped ray
segment. The budget is exposed as `--ray-steps` and is included in the
uniform tuning sweep. A sample interval can still skip a thinner feature;
that is the method's characteristic quality/performance tradeoff. Misses
return the cleared far depth instead of manufacturing a hit at the ground
plane. Hits write the reconstructed depth and therefore compose normally
with rasterized objects. The method needs no preprocessing or auxiliary
storage and remains the WebGL2 fallback.

### 3.2 Voxel-accelerated ray march

![The occupancy hierarchy skips known-empty cells and descends only where a leaf may contain terrain.](figures/voxel.svg)

This path accelerates the same per-pixel query with a conservative occupancy
pyramid. The finest level stores one occupancy bit per voxel in Morton-coded
8×8×8 tiles; each higher level is the union of eight children. Traversal
uses a hierarchical DDA: an empty cell advances directly to its exit plane,
while an occupied cell descends until the leaf level, where fixed sampling
tests the original height data. The structure skips only known-empty space;
it does not replace the source surface with cubes.

The GPU builds the hierarchy incrementally because a whole-level dispatch
can exceed driver watchdog limits. Preparation therefore appears separately
in §5.3. Runtime traversal has outer- and inner-step budgets. Dense vertical
variation can exhaust them, and the storage-buffer requirement excludes the
WebGL2 path.

### 3.3 Sliced

![Horizontal proxy planes retain fragments inside either encoded solid interval, exposing discrete bands at grazing angles.](figures/slice.svg)

The renderer draws `N` horizontal quads across the visible terrain bounds.
For each fragment, the surface decoder keeps the sample when its altitude is
inside the floor column or upper slab and discards it in empty space. The
slices are spread over the full altitude range; reducing `N` therefore
coarsens the representation rather than deleting the lowest part of it.

At the 256 quantized altitude levels, one slice per unit samples every
representable height. The tuned setting uses 512 slices. At grazing angles,
however, discrete planes remain visible as coherent horizontal bands and
can repeatedly cover a silhouette. This pattern can resemble a cast shadow
in a comparison image, but it is slice quantization; the shadow lookup is
the same one used by every other method.

### 3.4 Painted

![Each terrain texel emits a floor bar and, when present, an upper-slab bar for ordinary depth-tested rasterization.](figures/paint.svg)

Painted turns each ground sample into explicit column geometry. One bar
extends from zero to the floor; a dual-level sample adds a bar from cave
ceiling to slab top. The vertex shader emits only the three faces oriented
toward the camera. It derives every position from the instance index, so no
per-column vertex stream is uploaded.

The instance range is an axis-aligned bound of the camera footprint on the
terrain. Ordering the generated samples front-to-back lets early depth tests
remove most covered fragments; the original implementation measured 96%
early-Z rejection for its test view. Cost still scales with ground samples
inside the footprint rather than with visible pixels, which explains its
poor downward-looking timings. This method is related to, but not claimed
to reproduce, the original game's software renderer.

### 3.5 Scattered

![A warped footprint spends point samples near the camera; projected samples contend through an atomic depth write.](figures/scatter.svg)

A compute grid samples a camera-aligned ground footprint whose longitudinal
coordinate is warped to spend more samples nearby. Each invocation walks
both encoded vertical intervals and projects point samples into a screen
buffer. A 32-bit `atomicMin` selects the nearest result while retaining 24
bits of depth and 8 bits of terrain type. A full-screen resolve reconstructs
world position and performs material, diffuse, fog, cave-ambient, and shadow
evaluation.

The density vector controls the compute grid. Unlike rasterization, point
projection does not guarantee pixel coverage; undersampling appears as
isolated gaps and unstable pixels, especially near the horizon. The
coherence metric in §4.2 is intended to expose exactly this failure.

### 3.6 Mesh

![Greedy fitting inserts the largest-error source sample, leaving broad triangles on flats and dense triangles at discontinuities.](figures/mesh.svg)

The mesh path fits an explicit surface with greedy point insertion following
Garland and Heckbert [1995]. Each triangle tracks the source sample with the
largest vertical error; the globally worst sample is inserted until the
tolerance is met. Integer grid coordinates allow orientation and incircle
predicates to remain exact on the many cocircular cells.

One planar triangulation is shared by `low`, `mid`, and `high`. A sample is
inserted when any of the three fields exceeds tolerance, and boundary walls
close the upper slab where dual-level encoding begins or ends. This coupling
is the source of the fit-cost result in §6.

The level is partitioned into 128×128 chunks with three fitted detail levels.
Chunks are frustum culled and selected by camera distance. Boundaries use a
stable finest-level vertex sequence so neighbouring LODs do not crack. The
tradeoff moves out of the frame loop: fitting is a blocking CPU cost, the
full-level resident mesh can reach roughly 300 MB, and edits require local
refinement. Once present, the geometry uses the conventional raster pipeline
and supplies derivatives for smooth lighting.

The explicit representation also has two system-level advantages that a frame
time table does not price. First, the finest render LOD can be handed directly
to a conventional triangle-mesh physics collider. The original engine could
collide against its native voxel-like height-column state by rasterizing
collision shapes over the terrain, but mainstream rigid-body libraries have
no corresponding two-interval height-map primitive; indexed triangles are
standard input. The follow-up *Vandals and Heroes* prototype uses the same
per-chunk TIN for both drawing and Rapier collision, so the visible and
physical surfaces agree [Malyshau 2026]. Second, triangulation separates the
2D parameter grid from its 3D embedding. That prototype maps the fitted
vertices onto cylinders, spheres,
and tori, adding a curvature-error lattice where a planar chord would depart
from the intended world surface. These are downstream engineering advantages,
not additional measurements in the present comparison. Physics reuse also
widens the edit obligation: a refitted visual chunk needs its collider replaced
on the same consistency boundary.

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
its floor is set by grazing incidence and by the scene's sampling
discontinuities (§6.1).

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
configuration: every method, twelve viewpoints (three per pitch, at 0°,
−30°, −60°, −90°), 1280×800, 40 timed frames after per-method warmup.
The harness builds the binaries, fetches and converts the level on first
run, and names its output after the adapter wgpu selected, so runs
collected from different machines merge without hand-labelling
(`tools/merge-bench.py`). The design goal is that a measurement session
on a new device costs its owner one invocation and about an hour — the
protocol is only as reproducible as it is cheap to follow.

On Vulkan, frame times come from GPU timestamp queries around the frame's
command encoder. Encoder-level timestamps did not reliably bracket this
multipass workload on Metal: the returned intervals were quantised,
nearly method-independent, and much shorter than submit-to-completion. This is
a measurement failure, not evidence that Metal rendered incorrectly. wgpu
explicitly does not guarantee strict ordering for arbitrary command-encoder
timestamps, and its Metal backend may defer such a write to the next native
pass. We treated the pair as a guaranteed enclosing interval, which the API
does not promise. A future GPU-only measurement must timestamp each render and
compute pass and sum those intervals; the present Apple run conservatively
uses the retained CPU submit-and-wait average instead. The collector now
disables the invalid pair on Metal. Each row records its timing source because
the two mechanisms are not directly comparable (§5.2).
The publication configuration renders the same 1024² height-field shadow
map for every method. This deliberately measures a complete, visually
comparable terrain frame rather than an isolated discovery pass. The JSON
records the shadow mode, and `--no-shadows` is available for a separate
method-only diagnostic run. The final hardware batch uses this protocol;
visual review of all five grids confirms that the common pass reaches every
column.
One-time costs are recorded separately as setup / first frame / warmup
(§5.3), since per-frame figures structurally exclude them. Accuracy is
expected to be device-independent; the merge tool reports a baseline once
and cross-checks every field from the other devices, because an adapter
that disagrees about geometry is a finding, not noise.

### 4.4 Dynamic-edit protocol

The headless runner's `--dig` path removes altitude and upper-layer metadata
inside a bounded crater and submits the same dirty rectangle used by the
interactive game. For each method the final artifact will report (1) the
first post-edit frame latency, (2) subsequent latency until derived data are
consistent, and (3) image/depth agreement with a fresh renderer constructed
from the already-edited level. Direct-texture methods should converge in one
frame. RayVoxel may take several frames because its work is deliberately
budgeted; Mesh currently performs its local refit synchronously. This test is
separate from the steady-state protocol because averaging the edit frame into
40 ordinary frames would hide the cost that the constraint exists to expose.

## 5. Results

The final batch contains 84 rows on each of five devices: AMD Radeon 780M,
AMD Radeon RX 7900 XT, Intel RPL-U integrated graphics, NVIDIA GeForce RTX
5070, and Apple M3. The first four use Vulkan; the M3 uses Metal. Every run is
1280×800 with far distance 600, 40 timed frames, a common 1024² ray-traced
shadow pass, the selected 64 RayTraced steps, the same clean source revision
(`21875dc`), and the same twelve complete camera records. Complete per-view
tables and driver versions are in `paper/results.md`, regenerated by the
command in `paper/README.md`.

### 5.1 Pitch is the axis that separates them

Values below use the 780M as the accuracy baseline and are arithmetic means
over the three views at each pitch, reported as see-through / coherence error
(%). `covers-sky` is omitted because it is largely common-mode (§6.1).

![Missing terrain and local incoherence separate sharply near the horizon. The logarithmic scale keeps both catastrophic and sub-percent errors visible.](figures/quality-pitch.svg)

| pitch | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| 0° | 4.1 / 0.3 | 4.4 / 0.3 | 3.9 / 2.7 | **41.0 / 1.8** | 3.1 / 0.1 | 6.6 / 0.0 | 3.3 / 0.1 |
| −30° | 5.2 / 0.8 | 4.2 / 0.3 | 5.5 / 1.1 | **16.2 / 3.4** | 4.5 / 0.1 | 4.7 / 0.1 | 4.3 / 0.1 |
| −60° | 0.0 / 0.6 | 0.0 / 0.3 | 0.0 / 0.2 | 0.1 / **5.4** | 1.3 / 0.2 | 0.0 / 0.1 | 0.0 / 0.1 |
| −90° | 0.1 / 0.6 | 0.0 / 0.2 | 0.0 / 0.2 | 0.1 / **3.4** | 0.4 / 0.2 | 0.0 / 0.1 | 0.0 / 0.1 |

The corrected full-screen ray marcher now joins the coherent group instead of
losing the upper half of horizon frames. Scattering is the actual horizon
outlier: its three scenes leave 19.1–74.5% of reference terrain uncovered,
while the other methods' pitch means lie between 3.1% and 6.6%. Looking down
removes its coverage deficit but not its point-scale incoherence. Slicing
shows the complementary signature: good coverage but visible horizontal
bands, measured as 2.7% coherence error at 0°.

The two mesh settings expose a real quality trade rather than a shading bug.
At the hangar view q=0.0 leaves 16.3% uncovered, while q=0.75 leaves 6.5%; the
other scenes are much closer. The shared shadow pass and material resolve are
visually consistent across columns in all five grids.

Accuracy is strongly reproducible across devices. Outside the already broken
Scattered hangar row, the largest cross-device spreads are 0.064 percentage
points for coverage, 0.38 points for coherence (a sliced horizon row), 0.11
world units for median depth, and 0.80 units for p95 depth. On the Scattered
hangar row Apple differs from the 780M by 0.70 coverage points, 1.34 median
units, and 2.89 p95 units while both leave about three quarters of the terrain
uncovered. This is a small device sensitivity inside a catastrophic method
failure, not evidence that the other renderers produce different geometry.

The central observation is narrower and stronger than the preliminary one:
**methods developed from top-down screenshots can conceal failure modes that
become dominant at eye level.** Point scattering loses coverage, slicing
bands, and an aggressively simplified mesh can miss a scene-specific wall;
the image-order methods and the painter remain mutually coherent.

### 5.2 Frame time

The Vulkan table uses GPU timestamps and reports arithmetic means over the
three views at each pitch, in ms:

![Mean frame time across the twelve scenes. RayTraced records the lowest mean on three adapters, but the mesh configurations remain close and the ordering is not portable across devices.](figures/performance.svg)

| device / pitch | RayTraced | RayVoxel | Sliced | Scattered† | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| 780M / 0° | **3.552** | 7.310 | 10.442 | 14.172 | 8.472 | 4.309 | 3.950 |
| 780M / −30° | **3.793** | 7.943 | 7.819 | 8.656 | 12.811 | 4.324 | 4.029 |
| 780M / −60° | **3.717** | 8.000 | 8.958 | 6.983 | 20.048 | 4.163 | 3.929 |
| 780M / −90° | **3.804** | 7.636 | 10.153 | 7.079 | 21.998 | 4.146 | 3.813 |
| 7900 XT / 0° | **0.474** | 1.193 | 2.021 | 7.977 | 3.255 | 0.491 | 0.478 |
| 7900 XT / −30° | 0.605 | 1.313 | 1.599 | 1.505 | 4.828 | 0.517 | **0.478** |
| 7900 XT / −60° | 0.593 | 1.354 | 1.833 | 1.180 | 6.954 | 0.501 | **0.473** |
| 7900 XT / −90° | 0.617 | 1.276 | 2.027 | 1.177 | 6.466 | **0.473** | 0.498 |
| Intel RPL-U / 0° | **8.846** | 30.105 | 48.097 | 104.304 | 45.108 | 11.019 | 11.535 |
| Intel RPL-U / −30° | **10.131** | 28.183 | 28.909 | 89.467 | 57.296 | 11.032 | 11.560 |
| Intel RPL-U / −60° | **9.874** | 27.705 | 30.874 | 87.719 | 79.882 | 10.852 | 11.499 |
| Intel RPL-U / −90° | **10.692** | 26.385 | 35.341 | 66.507 | 101.991 | 10.817 | 11.595 |
| RTX 5070 / 0° | **0.417** | 1.089 | 2.265 | 1.721 | 3.257 | 0.484 | 0.544 |
| RTX 5070 / −30° | **0.465** | 1.189 | 1.546 | 1.534 | 4.159 | 0.487 | 0.614 |
| RTX 5070 / −60° | **0.449** | 1.189 | 1.775 | 1.329 | 5.682 | 0.474 | 0.559 |
| RTX 5070 / −90° | 0.481 | 1.133 | 2.019 | 1.305 | 5.686 | **0.470** | 0.602 |

At the selected 64 steps RayTraced has the lowest mean GPU time on the 780M,
Intel, and NVIDIA devices. Mesh q=0.75 narrowly wins on the 7900 XT (0.482 ms
overall against 0.572 ms for RayTraced), and the difference between the two is
under 0.15 ms at every 7900 pitch. This reverses the earlier 128-step result:
the quality sweep did not merely remove invisible work, it changed which
method wins. On Intel, only RayTraced and the meshes stay near 8.8–11.6 ms;
the derived-volume and forward methods cost 26–104 ms. Portability of an API
and shader does not imply portability of a performance ranking.

The Apple M3 values below are CPU submit-and-wait means. They include command
submission and the completion round trip, so they validate practical latency
and method ordering on Metal but are not comparable to the Vulkan table:

| Apple M3 / pitch | RayTraced | RayVoxel | Sliced | Scattered† | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| 0° | 5.606 | 10.740 | 12.292 | 40.515 | 17.338 | **5.584** | 6.132 |
| −30° | **5.334** | 11.825 | 11.442 | 12.155 | 32.564 | 5.534 | 6.136 |
| −60° | **5.337** | 12.261 | 11.869 | 10.975 | 49.864 | 5.884 | 5.811 |
| −90° | **5.327** | 11.291 | 12.710 | 11.046 | 31.304 | 5.405 | 6.078 |

Painter retains the clearest orientation dependence on every Vulkan adapter,
becoming progressively slower as more ground samples enter its emitted
footprint. Scattered is less orderly: the hangar takes 29.3 ms on the 780M,
21.6 ms on the 7900 XT, 148.8 ms on Intel, and 99.8 ms CPU time on Apple, but
only 2.5 ms on NVIDIA. †The pitch-0 arithmetic mean is therefore dominated by
a scene-and-adapter interaction and is not a general horizon cost.

### 5.3 Preparation cost

Per-frame numbers exclude one-time work. The final 780M run gives the
following CPU wall times in milliseconds (maximum over its twelve scenes):

![First-frame and total warmup costs reveal work excluded from the steady-state timing table.](figures/preparation.svg)

| method | setup | first frame | warmup |
|---|---|---|---|
| RayTraced | 9 | 56 | 87 |
| RayVoxel | 12 | 99 | **2644** |
| Sliced | 10 | 88 | 148 |
| Scattered | 11 | 128 | 367 |
| Painter | 10 | 121 | 202 |
| Mesh q=0.0 | 11 | **1427** | 1452 |
| Mesh q=0.75 | 10 | **3532** | 3566 |

`setup` builds pipelines and uploads the terrain texture; `first frame`
adds whatever the method builds lazily; `warmup` covers every pre-timing
frame. The two methods that pay anything substantial pay it differently.
The mesh fits its triangulation once, on the CPU, in a single blocking
1.4 s at q=0.0 — a load-time cost that a level cannot be entered without.
The voxel grid bakes incrementally under a per-frame texel budget,
spreading 2.6 s across frames that are individually playable but render
through terrain the bake has not reached yet.

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
400x260, view distance 600. Geometry scores are device-independent to the
small spreads measured in §5.1. The numbers below are from the GPU pass
recorded in `paper/tuning.md`; RayTraced was re-swept after the full-screen
fix, while the other methods already used the final river/hangar/ramp scenes.

| method | knob | swept | chosen | error at choice |
|---|---|---|---|---|
| RayTraced | forward steps | 16–256 | **64** | 9.4% |
| Painted | — | none beyond view distance | — | 4.5% |
| Sliced | slices | 32–512 | **512** | 9.8% |
| Scattered | density | 1–4 | **4,4,4** | 57.3% |
| RayVoxel | grid, steps | 2 grids × 40–400 | **4,8,2, 40 steps** | 5.5% |
| Mesh | fit tolerance | q 0.0–1.0 | **q=0.0** | 4.4% |

Five of the results are worth stating.

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

**Resolution changes whether the reference can resolve mesh quality.** In the
400×260 tuning pass every setting from q=0.0 to q=1.0 lands within 0.1 points
of coverage error (4.2–4.5%) and 5 u of depth error, so the rule picks the
cheapest. At the full 1280×800 hangar scene in the final batch, however,
q=0.0 leaves
16.3% uncovered and q=0.75 leaves 6.5%, at effectively equal frame cost. The
small tuning image hid a scene-specific wall that the publication resolution
can resolve. Mesh quality must therefore be selected at full resolution or
against its own finest fit; the latter moves surfaces by up to 289 u and
remains the stronger self-reference.

**The corrected ray marcher selects 64 steps.** Across the final horizon
scenes, 16 → 256 steps costs 0.25 → 1.26 ms while total error falls 12.5% →
8.4%. The 64-step setting is the cheapest within one point of the best (9.4%).
The final five-device batch uses 64 throughout. Relative to the earlier
128-step batch this saves about 1.2 ms on the 780M and 0.3–0.4 ms on the
discrete GPUs, enough to reverse the lowest-time method in §5.2.

A sixth result is about configuration rather than the method: the voxel
tracer's production grid (2,4,1) needs 153 MB of storage buffer and did not
fit the software rasterizer used for tuning. The final batch ran the selected
(4,8,2) grid even though all five hardware devices can accommodate the
production grid. The reported comparison therefore characterises the
tuned coarse configuration, not the renderer's shipping configuration;
the memory requirement is part of the configuration disclosure rather than
an unreported advantage.

### 5.6 Fastest is not best

The 64-step RayTraced configuration records the lowest mean GPU time on more
devices, but that is not a general recommendation over Mesh. Its fixed
sampling budget is visible as blocky or missed close-up detail, and grazing
rays are the worst possible
workload: preserving thin geometry requires more samples, while holding the
budget fixed admits the horizon artifacts measured in §5.1. The selected
setting is deliberately the cheapest point within the tuning rule, not a
quality match for the q=0.75 mesh. The mesh spends memory and preparation time
to make geometry explicit, then rasterizes continuously interpolated triangles
without a view-dependent traversal budget.

The choice is consequently workload-level. RayTraced is compelling when load
time, memory, and immediate edits dominate. Mesh is the stronger production
candidate when close-range quality, grazing views, physics reuse, or a curved
world representation matters enough to amortise fitting and local refits.
Neither statement follows from frame time alone.

## 6. Findings

### 6.1 Reference error is common-mode and scene-dependent

An early calibration fixture suggested that pitch alone controlled the
reference floor:

| pitch | see-through | covers-sky | depth p50 |
|---|---|---|---|
| 0° | 6.70% | 7.36% | 25.3u |
| −15° | 0.00% | 0.00% | 5.5u |
| −30° | 0.00% | 0.00% | 3.7u |
| −60° | 0.00% | 0.00% | 1.7u |

At eye level most of the ground is nearly edge-on, and a sub-pixel
difference in ray direction moves the hit by tens of units. At pitch 0
the converged renderers agreed with *each other* to 1.3 u while all sat
~25 u from the reference, at a signed median of 0.01 u — scatter, not
bias.

The final batch shows that the stronger claim — that tilting down makes the
reference absolutely tight — is false. At the three −90° views, five
independent methods cluster within a few units of one another while their
median distance errors against the reference are approximately 45, 21 and
24 u. At the same views, every method also shares 13.9%, 0.0% and 7.0%
`covers-sky`, respectively. This is common-mode disagreement with the
reference, not five renderers developing the same defect. Pitch remains a
major conditioning variable, but local quantisation, layer boundaries and
the CPU/GPU sampling convention also set the floor. Absolute depth error
must therefore be accompanied by inter-method agreement, and the remaining
top-down offset must be diagnosed before the depth metric supports a final
quality claim.

### 6.2 The multi-layer encoding, not the terrain, sets the fit cost

The obvious reading of a 14.9× reduction where a smooth surface gives
~80× is that hand-authored terrain is simply harder to fit. The ten
shipped worlds let us test that directly: same engine, same encoding,
same 8-bit quantisation, same texel scale, same authoring tools, varying
only content. Fitted at identical tolerance:

![Across the ten shipped worlds, increasing double-level coverage tracks a steep loss of grid-to-TIN reduction.](figures/fit-survey.svg)

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
Single-layer authored terrain compresses 45–182× in our own controls,
which confirms the fitter itself is not the weak link without importing a
ratio from a differently sampled elevation model. What defeats it is a
second layer whose altitudes are structural rather than continuous. §6.3
is the mechanism.

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
- All rendering comparisons use Fostral. A license is expected but is not
  executed at the time of writing; the nine additional survey worlds must
  be included explicitly or removed. No archive or derived data bundle can
  be released before the applicable grant is recorded.
- The final hardware batch covers four Vulkan devices and one Metal device,
  but not D3D12, WebGL2, mobile-class WebGPU, or multiple driver versions per
  adapter. It establishes cross-backend execution and image agreement, not a
  complete survey of WebGPU implementations.
- Frame timing is per-frame latency, not pipelined throughput: each frame
  is submitted and awaited in isolation, so nothing overlaps. The
  Vulkan timestamps make those numbers GPU work rather than round trip, but
  they do not make them a frame rate. Metal uses CPU submit-and-wait because
  its encoder timestamps failed the bracketing sanity check; those values are
  useful within the M3 table but cannot be compared directly with Vulkan.
- Residual tuning uncertainty remains after the uniform pass of §5.5. The
  selected 64 height-field steps are now measured everywhere, but mesh quality
  still needs selection at publication resolution because the small tuning
  image misses the hangar difference (§5.5).
- The dynamic edit path is implemented and exercised by `--dig`, but its
  cross-method latency and fresh-build image comparison are not yet part of
  final hardware batch. Until §4.4 is measured, edit support is a verified
  capability rather than a comparative performance result.
- The mesh needs ~300 MB resident at q=0.75 on a full level. Chunk
  streaming is not implemented, and until it is, "runs on low-end
  devices" is a claim about the pipeline, not the memory budget.

## 8. Conclusion

Six terrain renderers that look interchangeable from the original game's
top-down camera behave differently once the same authored data is viewed
at eye level. Horizontal slices expose bands and point scattering exposes
incoherent pixels. The audit also showed that a projected proxy envelope can
be mistaken for a ray-marching limitation; the corrected marcher now casts
the full screen and tests the dual-layer solid in both directions. Its selected
64-step configuration has the lowest mean GPU time on three of four Vulkan
devices, but that result is a speed/quality operating point, not an overall
victory. Close detail remains blocky, and grazing views either consume more
samples or expose
missed thin geometry. The finer mesh is more consistent at those views and
feeds ordinary rasterization, triangle-mesh physics, and curved-world
embeddings, at the cost of substantial memory, fitting, and edit maintenance.

The larger result is about the data and the measurement. Single-layer
worlds fit by 45–182×, while the structural second layer, not floor relief,
predicts the collapse in reduction; nearly a quarter of Fostral's vertex
insertions serve one layer-boundary discontinuity. Coverage alone hid an
over-drawing defect, and absolute depth against one CPU reference hid
common-mode disagreement between that reference and every renderer. A
credible comparison needs bidirectional coverage, coherence, inter-method
agreement, equal tuning, explicit preparation costs, and visual parity
checks in addition to a timing table. It also needs to treat post-edit
maintenance as a first-class result: a static hierarchy can win a frame and
still fail the workload if terrain destruction forces a reload. Conversely, a
method can win the timing table and still lose the application if its quality
budget is view-dependent or its representation cannot be reused by the rest
of the engine.

## Figure provenance

The paper now uses figures rather than treating them as a future pass. The six
method schematics are hand-authored SVGs with one vocabulary. The encoding,
pitch/quality, Vulkan performance, preparation, and ten-world fit figures are
generated by `tools/plot-paper.py`; hardware plots read the retained JSON
directly and the survey plot reads `paper/survey.json`, replaceable by
`tools/level-survey.py --json-out paper/survey.json`. The teaser crops one row
from a full final comparison grid with `examples/paper-teaser.rs` before the
same plotting command lays it out and labels it. Full five-device grids and
per-view tables remain supplemental evidence rather than being shrunk into
unreadable paper pages.

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
  adapting meshes.* IEEE Visualization. [Project and paper](https://www.cognigraph.com/ROAM_homepage/).
- Ulrich, T. 2002. *Rendering massive terrains using chunked level of
  detail control.* SIGGRAPH course.
- Losasso, F. and Hoppe, H. 2004. *Geometry clipmaps.* SIGGRAPH.
  [Author manuscript](https://hhoppe.com/geomclipmap.pdf).
- Amanatides, J. and Woo, A. 1987. *A fast voxel traversal algorithm for
  ray tracing.* Eurographics. (The DDA that §3.2 runs hierarchically.)
- Laine, S. and Karras, T. 2010. *Efficient sparse voxel octrees.* I3D.
  (The contrast: §3.2's voxels accelerate an exact height field rather
  than replace it.) [NVIDIA Research](https://research.nvidia.com/publication/2010-02_efficient-sparse-voxel-octrees).
- Policarpo, F., Oliveira, M. and Comba, J. 2005. *Real-time relief
  mapping on arbitrary polygonal surfaces.* I3D. (Per-pixel height-field
  marching lineage for §3.1.) [Author manuscript](https://www.inf.ufrgs.br/~comba/papers/2005/tog-2005.pdf).
- Tevs, A., Ihrke, I. and Seidel, H.-P. 2008. *Maximum mipmaps for fast,
  accurate, and scalable dynamic height field rendering.* I3D.
  [Max Planck publication record](https://pure.mpg.de/pubman/item/item_1325622).
- Benes, B. and Forsbach, R. 2001. *Layered data representation for visual
  simulation of terrain erosion.* SCCG, 80–86.
  [doi:10.1109/SCCG.2001.945341](https://doi.org/10.1109/SCCG.2001.945341).
- Alonso, J. and Joan-Arinyo, R. 2008. *The Grounded Heightmap Tree: A New
  Data Structure for Terrain Representation.* GRAPP, 80–85.
  [doi:10.5220/0001094300800085](https://doi.org/10.5220/0001094300800085).
- Graciano, A., Rueda, A. J., Pospíšil, A., Bittner, J. and Beneš, B. 2021.
  *QuadStack: An Efficient Representation and Direct Rendering of Layered
  Datasets.* IEEE TVCG 27(9), 3733–3744.
  [doi:10.1109/TVCG.2020.2981565](https://doi.org/10.1109/TVCG.2020.2981565).
- Shade, J., Gortler, S., He, L.-W. and Szeliski, R. 1998. *Layered Depth
  Images.* SIGGRAPH.
  [Microsoft Research](https://www.microsoft.com/en-us/research/publication/layered-depth-images/).
- Mantler, S. and Jeschke, S. 2006. *Interactive Landscape Visualization
  Using GPU Ray Casting.* GRAPHITE.
  [TU Wien publication record](https://www.cg.tuwien.ac.at/research/publications/2006/Mantler-06-landscape/).
- Cabral, B., Cam, N. and Foran, J. 1994. *Accelerated volume rendering and
  tomographic reconstruction using texture mapping hardware.* Symposium on
  Volume Visualization. [doi:10.1145/197938.197972](https://doi.org/10.1145/197938.197972).
- Westover, L. 1990. *Footprint evaluation for volume rendering.* SIGGRAPH.
  [Paper](https://cgl.ethz.ch/teaching/scivis_common/Literature/Westover90.pdf).
- Zwicker, M., Pfister, H., van Baar, J. and Gross, M. 2001. *Surface
  Splatting.* SIGGRAPH. [MERL publication record](https://www.merl.com/publications/TR2001-20).
- K-D Lab / KranX. *Vangers* source release, `KranX/Vangers`, GPL-3.0,
  [GitHub](https://github.com/KranX/Vangers). (Primary source for the original
  renderer; game resources are explicitly obtained separately.)
- Malyshau, D. 2020. *A Taste of WebGPU in Firefox.* Mozilla Hacks.
  [Mozilla](https://hacks.mozilla.org/2020/04/experimental-webgpu-in-firefox/).
- gfx-rs contributors. *wgpu: Safe and portable graphics for Rust.*
  [Project repository](https://github.com/gfx-rs/wgpu).
- W3C GPU for the Web Working Group. *WebGPU* and *WebGPU Shading Language*.
  [WebGPU specification](https://www.w3.org/TR/webgpu/);
  [WGSL specification](https://www.w3.org/TR/WGSL/).
- Malyshau, D. 2021. *Pure Rust.* vange-rs development log.
  [Project article](https://vange.rs/2021/08/25/pure-rust.html).
- Malyshau, D. 2019. *Data Formats.* vange-rs development log.
  [Project article](https://vange.rs/2019/12/12/data-formats.html).
- Malyshau, D. 2026. *Vandals and Heroes.* Follow-up prototype using a shared
  render/physics TIN on cylindrical, spherical, and toroidal worlds.
  [Project repository](https://github.com/kvark/vandals-and-heroes).
