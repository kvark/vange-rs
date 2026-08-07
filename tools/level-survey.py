#!/usr/bin/env python3
"""Fit every stock level at one quality and report what the fit cost.

Answers a question the single-level numbers cannot: how much of the mesh
reduction ratio is a property of the *method* and how much of the *data*.
Across the ten shipped worlds it spans a factor of 35, and the variable
it tracks is not terrain relief but the double-level encoding.

Also reports two roughness figures per level, which is what separates
those explanations. `rough(floor)` is the mean absolute discrete
Laplacian of the `low` layer alone - the terrain's own relief, blind to
the second layer. `rough(surface)` is the same on the composite surface
the fitter actually sees. Where those diverge, the slab is doing it.

Downloads and converts on demand into --work, skipping anything already
there, so a re-run is free.

Example
-------
    tools/level-survey.py                    # all ten, quality 0.25
    tools/level-survey.py --quality 0.75 --levels khox hmok
"""

import argparse
import os
import re
import subprocess
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
Image.MAX_IMAGE_PIXELS = None

LEVELS = ["weexow", "ark-a-znoy", "threall", "xplo", "khox",
          "fostral", "necross", "glorx", "boozeena", "hmok"]

STATS = re.compile(
    r"quality ([0-9.]+): (\d+) vertices, (\d+) triangles from (\d+) texels "
    r"\(([0-9.]+)x fewer[^)]*\), (\d+) slab triangles \(([0-9.]+)%\)"
)


def laplacian(a):
    """Mean |discrete Laplacian|, i.e. how far each sample sits from the
    average of its neighbours. A plane scores zero however steep it is,
    so this measures curvature rather than slope - which is what a
    planar-fit error metric responds to."""
    f = a.astype(np.float32)
    lap = np.abs(4 * f[1:-1, 1:-1] - f[:-2, 1:-1] - f[2:, 1:-1]
                 - f[1:-1, :-2] - f[1:-1, 2:])
    return float(lap.mean())


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("--levels", nargs="*", default=LEVELS)
    ap.add_argument("--quality", type=float, default=0.25)
    ap.add_argument("--work", default="work")
    ap.add_argument("--binary", default="./target/release/level")
    args = ap.parse_args()

    import importlib.util
    spec = importlib.util.spec_from_file_location(
        "ct", os.path.join(os.path.dirname(os.path.abspath(__file__)),
                           "compare-terrain.py"))
    ct = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(ct)

    rows = []
    for lvl in args.levels:
        sub = argparse.Namespace(
            level=lvl, work=args.work, binary=args.binary,
            level_zip=None, common_zip=None, layers=None, out=None,
        )
        ct.ensure_tools(sub)
        ct.ensure_assets(sub)

        cmd = [args.binary, "--level-zip", sub.level_zip,
               "--common-zip", sub.common_zip,
               "--terrain", "Mesh", "--mesh-quality", str(args.quality),
               "--snapshot", os.path.join(args.work, "survey.png"),
               "--width", "120", "--height", "80", "--frames", "1", "--warmup", "1"]
        res = subprocess.run(cmd, capture_output=True, text=True,
                             env=dict(os.environ, RUST_LOG="info"))
        m = STATS.search(res.stderr)
        if not m:
            print(f"{lvl}: no TIN stats (exit {res.returncode})", file=sys.stderr)
            continue
        _, verts, tris, texels, reduction, _, slab = m.groups()

        a = np.asarray(Image.open(os.path.join(args.work, lvl, "height.png")))
        a = a.astype(np.int16)
        low, high, delta = a[:, :, 0], a[:, :, 1], a[:, :, 2]
        dual = (high != low) | (delta != 0)
        surface = np.where(dual, high, low)

        rows.append({
            "level": lvl,
            "texels": int(texels),
            "verts": int(verts),
            "tris": int(tris),
            "reduction": float(reduction),
            "slab": float(slab),
            "dual": 100.0 * float(dual.mean()),
            "rough_floor": laplacian(low),
            "rough_surface": laplacian(surface),
        })
        r = rows[-1]
        print(f"{r['level']:11} {r['reduction']:7.1f}x  slab {r['slab']:5.1f}%  "
              f"dual {r['dual']:5.1f}%  rough floor {r['rough_floor']:6.3f}  "
              f"surface {r['rough_surface']:6.3f}", flush=True)

    if not rows:
        return
    print(f"\n| level | texels | triangles | reduction | slab tris | dual texels "
          f"| rough(floor) | rough(surface) |")
    print("|---|---|---|---|---|---|---|---|")
    for r in sorted(rows, key=lambda r: -r["reduction"]):
        print(f"| {r['level']} | {r['texels'] / 1e6:.1f} M | {r['tris'] / 1e6:.2f} M "
              f"| {r['reduction']:.1f}x | {r['slab']:.1f}% | {r['dual']:.1f}% "
              f"| {r['rough_floor']:.2f} | {r['rough_surface']:.2f} |")

    # The whole point of the survey: which of the two explains the spread.
    red = np.array([r["reduction"] for r in rows])
    for name in ("rough_floor", "rough_surface", "dual", "slab"):
        v = np.array([r[name] for r in rows])
        if len(rows) > 2 and v.std() > 0:
            rho = np.corrcoef(np.log(red), v)[0, 1]
            print(f"\ncorr(log reduction, {name}) = {rho:+.2f}", end="")
    print()


if __name__ == "__main__":
    main()
