# JCGT manuscript

The manuscript is `paper.tex`: **Rendering Editable Layered Terrain with
WebGPU: A Comparison of Six Methods**. The October 2026 revision reorganizes
the existing study around image quality, frame cost, local edits, and mesh
size. It does not introduce new benchmark runs.

## Build and review

```bash
python3 tools/build-paper-tex.py
# work/submission/vange-rs-paper.pdf

# Regenerate the figures, then build:
python3 tools/build-paper-tex.py --figures
```

The build needs pdfLaTeX and BibTeX with the packages used by the JCGT class.
Figure generation also needs Python with matplotlib and NumPy, plus Inkscape.
The build stops on unresolved references, missing citations, or overfull
boxes. These checks complement a visual read of the final PDF; they do not
prove that a diagram or an argument is correct.

`tools/draw-paper-figures.py` generates the figures used by this revision:

- The encoding diagram defines the decoded columns.
- The ray/slice diagram uses explicit sample positions and solid intervals.
- The quality plot reads the published, rounded pitch means in
  `paper/results.md`; it is not a new measurement.
- The teaser and edit mosaics use unchanged captures extracted from the
  earlier SVGs into `paper/figures/source/`. No capture pixels were repainted.
  The generated SVGs embed these images so they can be opened on their own.
- Standalone `ray.svg` and `slice.svg` are also regenerated so the older
  links show the corrected diagrams.

Every figure used in the manuscript has a caption and a cross-reference.
The older `tools/plot-paper.py` can still produce the original analysis charts
from raw result JSON, but it is not the typesetting path for this revision.

## Reproduce the measurements

The measurement revision is tag `terrain-paper`, commit `5586e2a`. Keep that
tag fixed when reproducing the published numbers. Editorial changes do not
move it.

```bash
# Twelve views, six selected methods, 1280x800, 40 timed frames.
# Builds the runner and fetches the licensed Fostral data if needed.
python3 tools/compare-terrain.py

# Inspect the camera plan without rendering.
python3 tools/compare-terrain.py --list-scenes

# Merge collected device runs.
python3 tools/merge-bench.py remote/results-*.json > paper/results.md

# Separate studies: configuration sweeps and ten-world mesh survey.
python3 tools/tune-methods.py
python3 tools/level-survey.py --json-out paper/survey.json
```

The selected configurations are Ray 128, Voxel cells (4,8,2) with 100 outer
and 100 inner steps, Sliced 512, Scattered density (4,4,4), Painted, and Mesh q=0.5.
The manuscript explains the mesh exception to the one-percentage-point
selection rule. Vulkan rows use GPU timestamps; Metal uses CPU
submit-and-wait. These timing classes are not interchangeable.

The evidence remains in `results.md`, `tuning.md`, `survey.json`, and the
measurement code. The current repository's engine has evolved after the
tag; running it at a newer revision is a new experiment.

## Data and video

Fostral is published by Association K-D Lab under CC BY-SA 4.0 at
KranX/Vangers commit `f1ad7d7`. The harness fetches that source. The other nine
survey worlds require a lawfully obtained game copy. See `DATA-LICENSE.md`.
The figure captures are derived from Fostral and retain that attribution.

```bash
python3 tools/render-paper-video.py
```

The supplemental flythrough uses the six selected configurations. It starts
from the portal view at (1176,11567), eye height 180, yaw 308 degrees, and
travels 520 units at fixed height. The video does not include the crater test.
It is not rebuilt by the manuscript build.

## Package the source

```bash
# Review package: PDF and the source files actually used by the paper.
python3 tools/build-paper-package.py --allow-dirty

# arXiv package with the existing supplemental video:
python3 tools/pack-arxiv.py

# Source-only review package when the video is not available locally:
python3 tools/pack-arxiv.py --without-video
```

These commands create local review artifacts. They do not submit to JCGT or
arXiv. `arxiv-metadata.txt` is draft metadata and must be checked against the
eventual submission. The earlier JCGT submission was rejected; this revision
has not been resubmitted by these tools.
