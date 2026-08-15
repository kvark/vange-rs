# JCGT submission — working directory

Draft, figures and the exact commands that produce them.

`draft.md` is the paper. It is written in Markdown to keep revision cheap;
JCGT wants LaTeX, and the conversion is mechanical once the content and
the numbers settle. Nothing here is a substitute for the harness — every
number in the draft is reproduced by a command recorded next to it, and a
number without one is marked `TODO`.

## Reproducing every figure

```bash
# On each machine. That is the whole invocation - the defaults are the
# publication configuration: every method, twelve viewpoints (three per
# pitch: 0, -30, -60, -90 degrees), 1280x800, 40 frames. It builds what is missing, fetches and converts the
# level on first run, reuses all of it afterwards, and names the results
# file after whichever adapter wgpu chose. Expect roughly an hour, with a
# running estimate printed as it goes.
tools/compare-terrain.py

# Verify the checkout and exact camera plan without starting a render.
tools/compare-terrain.py --list-scenes

# Collect the work/results-*.json files from every machine, then
tools/merge-bench.py work/results-*.json > paper/results.md

# The current final-scene hardware batch is retained under remote/;
# regenerate its complete tables with
tools/merge-bench.py remote/results-*.json > paper/results.md

# Extract the hangar row from one final 9000x9911 grid without loading it
# repeatedly, then generate every data-driven SVG.
cargo run --release --example paper-teaser -- \
  remote/cmp-k6.png paper/figures 1
tools/plot-paper.py
```

The merge tool rejects different cameras, renderer arguments, shadow modes,
or other protocol fields instead of silently combining unlike batches. The
retained final batch uses the selected 64-step RayTraced setting throughout.
Vulkan rows use GPU timestamps; Apple Metal uses the retained CPU
submit-and-wait average because encoder timestamps did not reliably bracket
the multipass frame. The merge output keeps those timing classes explicit.
The measurement snapshot is tagged `terrain-paper-v1`. Raw runs identify
`21875dc`, the clean renderer revision they measured; the tag additionally
contains the conservative Metal timing fallback. Editorial and figure work can
continue on `shift` without moving the frozen measurement tag.

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

## What is still missing

Tracked here rather than in the draft so the gaps stay visible:

- [x] Related work and novelty scope. The draft now compares the shipped
      interval encoding with Benes/Forsbach layered terrain, Grounded
      Heightmap Trees, QuadStack and Layered Depth Images, then relates each
      renderer to ray-height-field, voxel, slicing, splatting and terrain-LOD
      literature. The contribution is explicitly the controlled comparison,
      not priority for the underlying method families.
- [~] Dynamic terrain edits. All six paths consume dirty rectangles and the
      headless `--dig` probe exercises them. The final batch still needs the §4.4
      measurement: first post-edit frame, frames to consistency, and a
      snapshot/depth comparison against a fresh build of the edited level.
- [x] A control isolating what drives fit cost. Done: all ten stock
      worlds, `tools/level-survey.py`. The result reversed the original
      framing - floor relief does not predict the reduction (r = -0.17
      over all ten), the multi-layer encoding does (dual fraction
      r = -0.77, composite-surface roughness r = -0.82).
- [ ] An external elevation model, for comparability with published
      numbers rather than for the causal claim. Lower priority now that
      the single-layer worlds land at 45-182x.
- [x] More than one device. The final batch covers a Radeon 780M, Radeon RX
      7900 XT, Intel RPL-U, GeForce RTX 5070, and Apple M3 across Vulkan and
      Metal. The five grids agree geometrically outside a small spread within
      the already failing Scattered hangar row.
- [x] Timing-source validation. Vulkan uses GPU timestamps. Implausibly short,
      method-invariant Metal intervals exposed that encoder-level timestamps
      did not bracket the multipass frame, so Metal now falls back to CPU
      submit-and-wait and is reported separately.
- [~] Equal tuning across methods. `tools/level-survey.py` sibling
      `tools/tune-methods.py` sweeps every knob under one selection rule.
      Caveat recorded in the draft: the small tuning image cannot resolve
      mesh quality that the full-size hangar scene does, so that knob needs
      a full-resolution or self-referential selection.
      The first slicer sweep also measured a knob artifact (bottom
      truncation rather than coarser spacing) — fixed and re-swept, and
      recorded in §5.5 as a finding of its own. The corrected RayTraced sweep
      selects 64 steps and the final batch uses it on all five devices. Mesh
      quality still needs a publication-resolution selection.
- [ ] Memory. ~300 MB resident for the mesh at q=0.75 on a full level is
      the honest limit on the portability claim.
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
- [ ] Author block. JCGT review is single-blind — names and affiliation
      go on the submission.
- [ ] Supplemental video. JCGT explicitly prefers "shorter articles with
      supplemental code, data, and video"; a single flythrough rendered
      once per method (same path, same seed) would carry §5.1 better
      than any table.
