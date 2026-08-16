# JCGT submission — working directory

Draft, figures and the exact commands that produce them.

`draft.md` is the paper. It is written in Markdown to keep revision cheap;
JCGT wants LaTeX, and the conversion is mechanical once the content and
the numbers settle. Nothing here is a substitute for the harness — every
number in the draft is reproduced by a command recorded next to it, and a
number without one is marked `TODO`.

## Reproducing every figure

```bash
# On a new machine. That is the whole invocation - the defaults are the six
# selected configurations, twelve viewpoints (three per pitch: 0, -30, -60,
# -90 degrees), 1280x800, 40 frames. It builds what is missing, fetches and converts the
# level on first run, reuses all of it afterwards, and names the results
# file after whichever adapter wgpu chose. Expect roughly an hour, with a
# running estimate printed as it goes.
tools/compare-terrain.py

# Verify the checkout and exact camera plan without starting a render.
tools/compare-terrain.py --list-scenes

# Upgrade one of the frozen seven-configuration runs. This renders only the
# retuned RayTraced 128, RayVoxel 100, and Mesh q=0.25 cells (36 rather than
# 72 steady rows), then runs edit/memory for all six selected methods.
tools/compare-terrain.py --supplement-only

# After collecting the matching base and supplement JSONs:
tools/upgrade-paper-results.py remote/results-GPU.json \
  remote/supplement-results-GPU.json --out work/final/results-GPU.json

# Collect the work/results-*.json files from every machine, then
tools/merge-bench.py --accuracy-results work/accuracy-results-selected.json \
  work/final/results-*.json > paper/results.md

# Rebuild the corrected accuracy baseline cheaply on one machine. Timings from
# this one-frame run are intentionally discarded.
tools/compare-terrain.py --accuracy-only

# Extract the hangar row from one final 9000x9911 grid without loading it
# repeatedly, then generate every data-driven SVG.
cargo run --release --example paper-teaser -- \
  remote/cmp-k6.png paper/figures 1
tools/plot-paper.py --results 'work/final/results-*.json' \
  --accuracy-results work/accuracy-results-selected.json

# Generate the synchronized six-method supplemental flythrough. It starts
# at the river spot, pitches down 30 degrees, and flies forward at fixed Z.
# The derived video remains under work/ until the data grant is executed.
tools/render-paper-video.py

# From a clean checkout, build the internal review PDF and revision-pinned
# source archive. Both stay under work/ until the data checklist is complete.
nix-shell -p pandoc weasyprint --run tools/build-paper-package.py
```

The comparison tools require Python with NumPy and Pillow. Video generation
also requires `ffmpeg`; the Rust renderer itself is built by Cargo.

The merge and upgrade tools reject different cameras, devices, renderer
arguments, shadow modes, or other protocol fields instead of silently
combining unlike batches. The frozen batch remains an auditable measurement
of RayTraced 64, RayVoxel 40, and the q=0/q=0.75 mesh bracket; the short
supplement supplies the three selected replacements without recollecting
unchanged Sliced, Scattered, or Painter rows.
Vulkan rows use GPU timestamps; Apple Metal uses the retained CPU
submit-and-wait average because encoder timestamps did not reliably bracket
the multipass frame. wgpu does not promise strict ordering for arbitrary
command-encoder timestamps, so this was an invalid harness assumption rather
than evidence of incorrect rendering. The merge output keeps those timing
classes explicit.
The initial measurement snapshot is tagged `terrain-paper-v1`. Raw runs identify
`21875dc`, the clean renderer revision they measured; the tag additionally
contains the conservative Metal timing fallback. Editorial and figure work can
continue on `shift` without moving the frozen measurement tag. Protocol v3
receives its own tag; old tags are never moved.

At startup the harness removes any previous `work/compare/comparison.png` and
writes `work/compare/run-manifest.json` with the checkout revision and exact
camera coordinates. The comparison PNG is published atomically only after all
renders finish. If the manifest still says `"status": "running"`, there is no
current comparison to collect; an older PNG must not be mistaken for this run.

`--quick` shrinks it to a few seconds for checking the harness runs at
all. Those numbers are not results: too few frames to be stable, and too
few pixels for the reference to agree with anything.

The ten-world fit survey behind §6.2 and its graph input:

```bash
tools/level-survey.py --json-out paper/survey.json
```

`tools/plot-cull.py` produces the frustum/LOD plan figures from
`level --cull-dump`.

The lightweight edit-path smoke test uses the built-in level and records the
edit frame in each benchmark JSON:

```bash
for terrain in RayTraced RayVoxelTraced Sliced Scattered Painted Mesh; do
  cargo run --release --bin level -- --terrain "$terrain" --dig --dig-frame 1 \
    --width 160 --height 100 --frames 3 --near 1 --far 300 \
    --snapshot "work/dig-$terrain.png" --bench-out "work/dig-$terrain.json"
done
```

This checks that every update path executes; it is not the §4.4 latency or
fresh-build equivalence experiment.

That smoke test was the limit of the original five-machine collector: the edit
normally occurred during warmup, only the last frame survived, no fresh edited
renderer was constructed, and memory appeared only in informal allocation
logs. The frozen 84-row batches therefore cannot answer either question. The
default collector now appends the full protocol, while `--supplement-only`
adds edit/memory and only the three retuned steady configurations.

## What is still missing

Tracked here rather than in the draft so the gaps stay visible:

- [x] Related work and novelty scope. The draft now compares the shipped
      interval encoding with Benes/Forsbach layered terrain, Grounded
      Heightmap Trees, QuadStack and Layered Depth Images, then relates each
      renderer to ray-height-field, voxel, slicing, splatting and terrain-LOD
      literature. The contribution is explicitly the controlled comparison,
      not priority for the underlying method families.
- [x] Dynamic terrain edits. The §4.4 protocol now measures five repeated
      first-post-edit frames, frames to consistency, and color/depth agreement
      against a fresh edited build. One Radeon 890M result is integrated; run
      `tools/compare-terrain.py --supplement-only` on the remaining final-batch
      machines for cross-device update latency and the three retuned rows.
- [x] A control isolating what drives fit cost. Done: all ten stock
      worlds, `tools/level-survey.py`. The result reversed the original
      framing - floor relief does not predict the reduction (r = -0.17
      over all ten), the multi-layer encoding does (dual fraction
      r = -0.77, composite-surface roughness r = -0.82).
- [—] An external elevation model is not a submission gate. The single-layer
      stock worlds are the tighter control because they hold encoding,
      quantisation, texel scale, and authoring pipeline fixed; they already
      establish the 45–182× comparison range.
- [x] More than one device. The final batch covers a Radeon 780M, Radeon RX
      7900 XT, Intel RPL-U, GeForce RTX 5070, and Apple M3 across Vulkan and
      Metal. The five grids agree geometrically outside a small spread within
      the already failing Scattered hangar row.
- [~] Selected-configuration timing. The Radeon 890M protocol-v3 supplement
      completed cleanly. Run `tools/compare-terrain.py --supplement-only` on
      the five retained machines, collect the 36-row JSONs, and compose them
      with `tools/upgrade-paper-results.py`; the 84-row batch is not repeated.
- [x] Timing-source validation. Vulkan uses GPU timestamps. Implausibly short,
      method-invariant Metal intervals exposed that encoder-level timestamps
      did not bracket the multipass frame. The API does not guarantee that
      ordering, so Metal now falls back to CPU submit-and-wait and is reported
      with `*` in the same table; pass-level instrumentation is the future
      GPU-only fix.
- [x] Equal tuning across methods. `tools/level-survey.py` sibling
      `tools/tune-methods.py` sweeps every knob under one selection rule.
      Caveat recorded in the draft: the small tuning image cannot resolve
      mesh quality that the full-size hangar scene does, so that knob needs
      a full-resolution or self-referential selection.
      The first slicer sweep also measured a knob artifact (bottom
      truncation rather than coarser spacing) — fixed and re-swept, and
      recorded in §5.6 as a finding of its own. The corrected sweep selects
      RayTraced 128 and RayVoxel 100. A separate 1280×800 mesh sweep selects
      q=0.25 under the same one-point rule.
- [x] Auditable method-memory accounting. The collector reports explicit
      persistent GPU buffers and CPU acceleration/fitting state for every
      method. §5.4 includes both mesh settings and the tuned RayVoxel grid and
      states the exclusions: shared resources, transient staging, opaque
      driver allocations, and whole-process overhead are not portable wgpu
      metrics.
- [~] Data license. A Fostral license is expected. Before release, record
      the executed grant, rights holder, permitted uses, redistribution
      terms, and whether derived images/measurements are covered. The
      ten-world survey needs the other nine levels named in the same grant
      or must be reduced to cleared data. Publish no archive or derived data
      bundle before execution.
      Use `DATA-LICENSE.md` as the release gate rather than interpreting
      informal permission during artifact packaging.
- [x] Core figures. §3 uses six consistent vector algorithm schematics;
      `tools/plot-paper.py` generates the encoding, pitch/quality, Vulkan
      performance, preparation, and fit-survey SVGs; the Rust crop example
      extracts a readable teaser from a final full-resolution grid.
- [x] Author block. Dzmitry Malyshau, Independent Researcher,
      `kvark@fastmail.com`.
- [x] Reference-floor diagnosis. The CPU screen-X basis was mirrored and its
      spherical far cutoff disagreed with the GPU's view-axis far plane.
      Regression tests cover both. Remaining off-axis point-sampling offsets
      are disclosed and absolute depth claims are narrowed.
- [x] Timing uncertainty. `tools/merge-bench.py` propagates the 40 within-row
      frame samples into a fixed-session 95% interval over twelve scenes. It
      explicitly does not claim session-to-session inference, and the text no
      longer promotes overlapping sub-millisecond means to a general winner.
- [x] Browser WebGPU smoke test. `browser-smoke.md` records a clean Wasm build
      and Firefox 152 run of the WebGPU-only voxel route on the Radeon 890M.
      The canvas rendered Fostral and reported `Backend: WebGPU`; the paper
      scopes browser evidence to execution/visual parity, not timing.
- [x] Submission PDF and archive tooling. `tools/build-paper-package.py` builds
      a styled review PDF and verifies a revision-pinned `git archive` contains
      the draft, BibTeX, figures, and harness. Keep the artifacts internal until
      `DATA-LICENSE.md` is complete; JCGT template conversion follows acceptance.
- [x] Bibliography pass. `references.bib` contains complete publication data,
      stable URLs, and DOIs for the references discussed in the draft.
      Original-game screenshot provenance remains part of the data-license gate.
- [~] Supplemental video. `tools/render-paper-video.py` produces the local
      synchronized six-configuration, eight-second H.264 mosaic and poster at
      fixed world Z, moving horizontally along the camera yaw.
      Keep both under `work/` until the terrain grant covers derived imagery;
      then add the licensed files and publication URL to the submission.
