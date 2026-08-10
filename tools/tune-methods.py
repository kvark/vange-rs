#!/usr/bin/env python3
"""Sweep each method's own quality knob and pick a fair setting for it.

A comparison between methods is only meaningful if each was given a fair
chance, and these have very different numbers of dials: the voxel tracer
has a grid size and two step budgets, the mesh has a fit tolerance, the
slicer has a slice count, the scatterer has a sample density, and the
height-field marcher and the painter have none at all beyond the shared
view distance.

The selection rule is uniform, and applied to every method including the
ones we wrote:

    take the cheapest setting whose error is within `--slack` of that
    method's own best error across the sweep

so a method is never charged for a setting that buys nothing, and never
credited with speed it only reaches by being wrong. Error is
`see-through + speckle`: geometry it failed to draw, plus geometry it
drew incoherently. `covers-sky` is deliberately excluded, since it moves
with the reference rather than the renderer.

Depth error is reported but not used to select, because at the horizon it
cannot: the reference's own floor there is ~25u, and on Fostral every
mesh quality from 0.0 to 1.0 lands within a few u of it. Where a knob changes
geometry too finely for the reference to see, tune it against the
method's own finest setting instead - `--mesh-lod`/`--mesh-lod-distance`
with `tools/compare-terrain.py` does exactly that, and resolves
differences of hundreds of units that this sweep reads as zero.

Emits a Markdown table per method and a suggested METHODS block.

Example
-------
    tools/tune-methods.py --level fostral --view "river:2006,1730:120"
"""

import argparse
import importlib.util
import json
import os
import re
import subprocess
import sys

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location(
    "ct", os.path.join(HERE, "compare-terrain.py"))
ct = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ct)

# (label, --terrain, [(setting label, extra args, warmup frames)])
#
# `RayTraced` and `Painted` appear with a single entry because they have
# no knob; listing them keeps the table honest about which methods could
# be tuned at all, rather than quietly omitting them.
SWEEPS = [
    ("RayTraced", "RayTraced", [("(none)", [], 3)]),
    ("Painter", "Painted", [("(none)", [], 3)]),
    ("Sliced", "Sliced", [
        (f"{n} layers", ["--slice-layers", str(n)], 3)
        for n in (32, 64, 128, 256, 512)
    ]),
    ("Scattered", "Scattered", [
        (f"density {d}", ["--scatter-density", d], 8)
        for d in ("1,1,1", "2,2,2", "3,3,3", "4,4,4")
    ]),
    ("RayVoxel", "RayVoxelTraced", [
        (f"grid {g}, {s} steps",
         ["--voxel-size", g, "--voxel-steps", str(s)], 170)
        for g in ("4,8,2", "2,4,1")
        for s in (40, 100, 200, 400)
    ]),
    ("Mesh", "Mesh", [
        (f"q={q}", ["--mesh-quality", str(q)], 3)
        for q in (0.0, 0.25, 0.5, 0.75, 1.0)
    ]),
]


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("--level", default="fostral")
    ap.add_argument("--work", default="work")
    ap.add_argument("--binary", default="./target/release/level")
    ap.add_argument("--view", action="append",
                    help="name:x,y:yaw[:under]; defaults to the standard set")
    ap.add_argument("--pitch", type=float, default=0.0,
                    help="the horizon is where these methods differ, so tune "
                         "there rather than at a top-down angle where they "
                         "all score the same")
    ap.add_argument("--width", type=int, default=400)
    ap.add_argument("--height", type=int, default=260)
    ap.add_argument("--far", type=float, default=600.0)
    ap.add_argument("--frames", type=int, default=8)
    ap.add_argument("--slack", type=float, default=1.0,
                    help="percentage points of error a setting may give up "
                         "against the method's own best to count as good "
                         "enough")
    ap.add_argument("--only", action="append",
                    help="sweep only these methods (by label, e.g. Sliced); "
                         "repeatable. Default: all of them")
    ap.add_argument("--out", default="work/tuning.md")
    args = ap.parse_args()

    sub = argparse.Namespace(level=args.level, work=args.work, binary=args.binary,
                             level_zip=None, common_zip=None, layers=None, out=None)
    ct.ensure_tools(sub)
    ct.ensure_assets(sub)
    views = [ct.parse_view(v) for v in (args.view or ct.DEFAULT_VIEWS)]

    refs = {}
    for v in views:
        sky, dist, dirs, _ = ct.ground_truth(
            ct.load_layers(sub.layers), v, args.width, args.height, 8.0,
            args.far, args.pitch)
        refs[v["name"]] = (sky, dist, dirs, ct.speckle(dist, ~sky))

    tmp = os.path.join(args.work, "tune")
    os.makedirs(tmp, exist_ok=True)
    report, chosen = [], {}

    sweeps = [s for s in SWEEPS if not args.only or s[0] in args.only]
    if args.only and not sweeps:
        sys.exit(f"no method matches {args.only}; "
                 f"labels are {[s[0] for s in SWEEPS]}")

    for label, terrain, settings in sweeps:
        print(f"\n### {label}", flush=True)
        rows, failures = [], []
        for name, extra, warmup in settings:
            errs, spks, deps, times, ok = [], [], [], [], True
            for v in views:
                stem = os.path.join(tmp, re.sub(r"[^A-Za-z0-9]+", "_",
                                                f"{label}-{name}-{v['name']}"))
                cmd = [args.binary, "--snapshot", stem + ".png",
                       "--depth-out", stem + ".f32", "--bench-out", stem + ".json",
                       "--terrain", terrain, *extra,
                       "--fp", f"{v['x']},{v['y']}", "--fp-height", "8",
                       f"--fp-yaw={v['yaw']}", f"--fp-pitch={args.pitch}",
                       "--near", "1", "--far", str(args.far),
                       "--width", str(args.width), "--height", str(args.height),
                       "--frames", str(args.frames), "--warmup", str(warmup),
                       "--level-zip", sub.level_zip, "--common-zip", sub.common_zip]
                if v["under"]:
                    cmd.append("--fp-under")
                r = subprocess.run(cmd, capture_output=True, text=True)
                if r.returncode != 0:
                    # Keep the reason. A setting that cannot run on this
                    # device is a result about the device, and silently
                    # dropping it would leave the method looking tuned when
                    # its best configuration was never reachable.
                    m = re.search(r"(LimitsExceeded[^\n]*|panicked at [^\n]*)", r.stderr)
                    ok = m.group(1) if m else "failed"
                    break
                with open(stem + ".json") as f:
                    meta = json.load(f)
                gpu = meta.get("gpu_avg_ms")
                times.append(gpu if meta.get("gpu_timing") and gpu == gpu
                             else meta["avg_ms"])
                sky, ref, dirs, ref_spk = refs[v["name"]]
                d = np.fromfile(stem + ".f32", dtype="<f4").reshape(args.height, args.width)
                empty = d >= 0.999999
                errs.append(100 * (empty & ~sky).mean())
                got = ct.ray_distance(d, dirs, 1.0, args.far)
                spks.append(100 * (ct.speckle(got, ~empty) & ~ref_spk & ~sky).mean())
                both = (~empty) & ~sky
                deps.append(float(np.percentile(np.abs(got - ref)[both], 50))
                            if both.any() else float("nan"))
            if ok is not True:
                failures.append((name, ok))
                print(f"  {name:24} UNAVAILABLE - {ok[:96]}", flush=True)
                continue
            st, sp, ms = np.mean(errs), np.mean(spks), np.mean(times)
            dp = float(np.nanmean(deps)) if deps else float("nan")
            rows.append((name, extra, st, sp, st + sp, ms, dp))
            print(f"  {name:24} {ms:7.2f} ms   see-through {st:5.1f}%   "
                  f"speckle {sp:5.1f}%   error {st + sp:5.1f}%   "
                  f"depth {dp:5.1f}u", flush=True)

        if not rows:
            continue
        best = min(r[4] for r in rows)
        good = [r for r in rows if r[4] <= best + args.slack]
        pick = min(good, key=lambda r: r[5])
        chosen[label] = pick
        report.append((label, rows, pick, failures))
        print(f"  -> {pick[0]}  ({pick[5]:.2f} ms, error {pick[4]:.1f}%)", flush=True)

    with open(args.out, "w") as f:
        f.write("# Per-method tuning\n\n")
        f.write(f"Level {args.level}, pitch {args.pitch:g}°, "
                f"{args.width}x{args.height}, view distance {args.far:g}, "
                f"averaged over {len(views)} viewpoints.\n\n")
        f.write("Selection rule: the cheapest setting within "
                f"{args.slack:g} percentage points of that method's own best "
                "error, where error is see-through + speckle.\n")
        for label, rows, pick, failures in report:
            f.write(f"\n## {label}\n\n")
            f.write("| setting | GPU ms | see-through | speckle | error | depth p50 |\n")
            f.write("|---|---|---|---|---|---|\n")
            for name, _, st, sp, tot, ms, dp in rows:
                mark = " **<-**" if name == pick[0] else ""
                f.write(f"| {name}{mark} | {ms:.2f} | {st:.1f}% | {sp:.1f}% "
                        f"| {tot:.1f}% | {dp:.1f}u |\n")
            for name, why in failures:
                f.write(f"| {name} | — | — | — | — | not available: {why[:110]} |\n")
        f.write("\n## Chosen\n\n```python\nMETHODS = [\n")
        for label, _, pick, _ in report:
            terrain = next(t for l, t, _ in SWEEPS if l == label)
            extra = ", ".join(f'"{a}"' for a in pick[1])
            f.write(f'    ("{label}", "{terrain}", [{extra}], ...),  '
                    f'# {pick[0]}\n')
        f.write("]\n```\n")
    print(f"\nwrote {args.out}")


if __name__ == "__main__":
    main()
