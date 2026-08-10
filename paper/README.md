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
# publication configuration: every method, three viewpoints, four pitches,
# 1280x800, 40 frames. It builds what is missing, fetches and converts the
# level on first run, reuses all of it afterwards, and names the results
# file after whichever adapter wgpu chose. Expect roughly an hour, with a
# running estimate printed as it goes.
tools/compare-terrain.py

# Collect the work/results-*.json files from every machine, then
tools/merge-bench.py work/results-*.json > paper/results.md
```

`--quick` shrinks it to a few seconds for checking the harness runs at
all. Those numbers are not results: too few frames to be stable, and too
few pixels for the reference to agree with anything.

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
      framing - floor relief does not predict the reduction (r = -0.17
      over all ten), the multi-layer encoding does (dual fraction
      r = -0.77, composite-surface roughness r = -0.82).
- [ ] An external elevation model, for comparability with published
      numbers rather than for the causal claim. Lower priority now that
      the single-layer worlds land at 45-182x.
- [ ] More than one device. The harness is ready; the runs are not.
- [x] GPU timestamp queries. Done; the harness prefers them and records
      which timing each row used. Still per-frame latency rather than
      pipelined throughput, which is a separate limitation.
- [x] Equal tuning across methods. Done: `tools/level-survey.py` sibling
      `tools/tune-methods.py` sweeps every knob under one selection rule.
      Caveat recorded in the draft: the reference cannot resolve mesh
      quality at the horizon, so that one knob is tuned self-referentially.
      The first slicer sweep also measured a knob artifact (bottom
      truncation rather than coarser spacing) — fixed and re-swept, and
      recorded in §5.5 as a finding of its own.
- [ ] Memory. ~300 MB resident for the mesh at q=0.75 on a full level is
      the honest limit on the portability claim.
- [ ] Data license. JCGT requires provided code and data under a
      non-restrictive OSS license. The engine and harness are Apache-2.0;
      the level archives fetched by the harness are the game's content
      and their redistribution terms must be stated in the paper (see
      the *Data availability* TODO in the draft). Only kvark can resolve
      this one.
- [ ] Figures. None exist yet; the draft now carries a "Planned figures"
      list with a generating command per figure. The teaser layout script
      and the §6.2 scatter plot are the two not yet implemented.
- [ ] Author block. JCGT review is single-blind — names and affiliation
      go on the submission.
- [ ] Supplemental video. JCGT explicitly prefers "shorter articles with
      supplemental code, data, and video"; a single flythrough rendered
      once per method (same path, same seed) would carry §5.1 better
      than any table.
