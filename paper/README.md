# JCGT submission — working directory

Draft, figures and the exact commands that produce them.

`draft.md` is the paper. JCGT review is single-blind **PDF**, in any
reasonable article format ([write.html](https://jcgt.org/write.html)).
Their LaTeX/BibTeX template is required only after acceptance, as a
condition of publication. The submission artifact is the review PDF from
`tools/build-paper-package.py` (pandoc + weasyprint). Nothing here is a
substitute for the harness — every number in the draft is reproduced by a
command recorded next to it, and a number without one is marked `TODO`.

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

# After collecting a full six-method run from each machine into remote/:
tools/merge-bench.py remote/results-*.json > paper/results.md

# Rebuild the corrected accuracy baseline cheaply on one machine. Timings from
# this one-frame run are intentionally discarded.
tools/compare-terrain.py --accuracy-only

# Hangar stills for the six-method teaser live in paper/figures/teaser-N.png.
# The crater figure reads work/edit-figure/{RayTraced,RayVoxel,Mesh_q_0_5}-{before,after}.png.
# Generate every data-driven SVG, including teaser.svg and edit.svg:
tools/plot-paper.py --results 'remote/results-*.json' \
  --accuracy-results remote/results-amd-radeon-780m-graphics-radv-phoenix.json \
  --edit-dir work/edit-figure

# Generate the synchronized six-method supplemental flythrough. It starts
# at the -30° portal scene (1176, 11567), raised to eye height 180, and
# flies 520 units horizontally along yaw 308° at fixed Z. The H.264 mosaic
# is a JCGT supplement, not a journal-hosted data archive.
tools/render-paper-video.py

# From a clean checkout, build the review PDF and revision-pinned source
# archive under work/. That PDF is the JCGT submission; the journal's
# LaTeX template is a publication step after acceptance.
nix-shell -p pandoc python3Packages.weasyprint --run tools/build-paper-package.py
```

The comparison tools require Python with NumPy and Pillow. Video generation
also requires `ffmpeg`; the Rust renderer itself is built by Cargo.

The merge tool rejects different cameras, devices, renderer arguments,
shadow modes, or other protocol fields instead of silently combining
unlike batches. The publication comparison is six methods: RayTraced 128,
RayVoxel 100, Sliced 512, Scattered 4,4,4, Painted, and Mesh q=0.5.
Vulkan rows use GPU timestamps; Apple Metal uses the retained CPU
submit-and-wait average because encoder timestamps did not reliably bracket
the multipass frame. wgpu does not promise strict ordering for arbitrary
command-encoder timestamps, so this was an invalid harness assumption rather
than evidence of incorrect rendering. The merge output keeps those timing
classes explicit.
The benchmarking checkout is tagged `terrain-paper` (`5586e2a`). That is
the only measurement tag; editorial work can continue without moving it.

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
- [x] Dynamic terrain edits. The §4.4 protocol measures five repeated
      first-post-edit frames (CPU submit-and-wait and GPU timestamps),
      frames to consistency, and color/depth agreement against a fresh
      edited build. A before/after figure shows the same crater on
      RayTraced, RayVoxel, and Mesh. All five publication adapters ran
      the protocol; Mesh CPU Δ is 5–10 ms and remains history-dependent.
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
- [x] Selected-configuration timing. §5.1 and §5.2 share one six-method
      set (Ray 128, Voxel 100, Sliced, Scattered, Painted, Mesh q=0.5).
      Five devices recollected at those settings; Mesh q=0.5 is the
      twelve-scene mean winner on every adapter.
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
      RayTraced 128 and RayVoxel 100. The 1280×800 mesh sweep's error floor
      is 0.3% from q=0.5 up; the paper publishes that single mesh quality
      so the comparison, teaser, and video share one operating point.
- [x] Auditable method-memory accounting. The collector reports explicit
      persistent GPU buffers and CPU acceleration/fitting state for every
      method. §5.4 includes both mesh settings and the tuned RayVoxel grid and
      states the exclusions: shared resources, transient staging, opaque
      driver allocations, and whole-process overhead are not portable wgpu
      metrics.
- [x] Data license. Fostral is CC BY-SA 4.0 from Association K-D Lab,
      `KranX/Vangers` commit `f1ad7d7`. The harness fetches that tree.
      Do not re-host `fostral.zip` as a JCGT supplement (ShareAlike vs
      JCGT's non-restrictive-data rule). The other nine survey worlds
      remain user-supplied.
- [x] Core figures. §3 uses six consistent vector algorithm schematics;
      `tools/plot-paper.py` generates the encoding, pitch/quality, Vulkan
      performance, preparation, fit-survey, six-method teaser, and crater
      before/after SVGs.
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
      the draft, BibTeX, figures, and harness. That PDF is the submission
      artifact. JCGT's LaTeX template is required only after acceptance.
- [x] Bibliography pass. `references.bib` contains complete publication data,
      stable URLs, and DOIs for the references discussed in the draft.
      Original-game screenshot provenance remains part of the data-license gate.
- [x] Supplemental video. `tools/render-paper-video.py` produces the
      synchronized six-configuration, eight-second H.264 mosaic and poster from
      the -30° portal camera, raised to eye height 180, moving 520 units
      horizontally along yaw 308°. Derived Fostral imagery uses the CC BY-SA
      4.0 attribution. The flythrough does not include a crater; that
      evidence is the §5.4 figure and table.
