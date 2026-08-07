# JCGT submission — working directory

Draft, figures and the exact commands that produce them.

`draft.md` is the paper. It is written in Markdown to keep revision cheap;
JCGT wants LaTeX, and the conversion is mechanical once the content and
the numbers settle. Nothing here is a substitute for the harness — every
number in the draft is reproduced by a command recorded next to it, and a
number without one is marked `TODO`.

## Reproducing every figure

```bash
cargo build --release --bin level --bin convert

# assets, once per machine
mkdir -p work/src && (cd work/src && unzip ../../fostral.zip && unzip ../../common.zip)
./target/release/convert work/src/world.ini work/level.ron

# the whole matrix: methods x viewpoints x pitch
tools/compare-terrain.py \
    --level-zip fostral.zip --common-zip common.zip \
    --layers work/level.ron --out work/cmp \
    --pitch 0 --pitch -30 --pitch -60 --pitch -90 \
    --width 1280 --height 800 --frames 40 \
    --label "<this machine>" --json-out work/results-<machine>.json

# collect the json files from every machine, then
tools/merge-bench.py work/results-*.json > paper/results.md
```

`tools/plot-cull.py` produces the frustum/LOD plan figures from
`level --cull-dump`.

## What is still missing

Tracked here rather than in the draft so the gaps stay visible:

- [ ] More than one dataset. Other stock worlds are cheap; a natural DEM
      is the control that turns "authored terrain is harder" from an
      assertion into a measurement.
- [ ] More than one device. The harness is ready; the runs are not.
- [ ] GPU timestamp queries. Submit-and-poll measures GPU work but
      serially, so it reports per-frame latency and cannot see pipelining.
- [ ] Equal tuning across methods. The voxel step budget was found badly
      mistuned; the painter and the slicer have had no equivalent pass.
- [ ] Memory. ~300 MB resident for the mesh at q=0.75 on a full level is
      the honest limit on the portability claim.
