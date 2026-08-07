# JCGT submission — working directory

Draft, figures and the exact commands that produce them.

`draft.md` is the paper. It is written in Markdown to keep revision cheap;
JCGT wants LaTeX, and the conversion is mechanical once the content and
the numbers settle. Nothing here is a substitute for the harness — every
number in the draft is reproduced by a command recorded next to it, and a
number without one is marked `TODO`.

## Reproducing every figure

```bash
# On each machine. Builds what is missing, fetches and converts the level
# on first run, reuses all of it afterwards, and names the results file
# after whichever adapter wgpu chose.
tools/compare-terrain.py --pitch 0 --pitch -30 --pitch -60 --pitch -90 \
    --width 1280 --height 800 --frames 40

# Collect the work/results-*.json files from every machine, then
tools/merge-bench.py work/results-*.json > paper/results.md
```

The ten-world fit survey behind §6.2:

```bash
tools/level-survey.py            # add --quality to sweep tolerance
```

`tools/plot-cull.py` produces the frustum/LOD plan figures from
`level --cull-dump`.

## What is still missing

Tracked here rather than in the draft so the gaps stay visible:

- [x] A control isolating what drives fit cost. Done: all ten stock
      worlds, `tools/level-survey.py`. The result reversed the original
      framing - relief does not predict the reduction (r = -0.25), the
      multi-layer fraction does (r = -0.88).
- [ ] An external elevation model, for comparability with published
      numbers rather than for the causal claim. Lower priority now that
      the single-layer worlds land at 45-182x.
- [ ] More than one device. The harness is ready; the runs are not.
- [ ] GPU timestamp queries. Submit-and-poll measures GPU work but
      serially, so it reports per-frame latency and cannot see pipelining.
- [ ] Equal tuning across methods. The voxel step budget was found badly
      mistuned; the painter and the slicer have had no equivalent pass.
- [ ] Memory. ~300 MB resident for the mesh at q=0.75 on a full level is
      the honest limit on the portability claim.
