#!/usr/bin/env python3
"""Compare terrain rendering methods on a level, scored against ground truth.

Renders the same camera with each method, dumps the depth buffer, and
scores every renderer against a CPU ray cast of the level's own height
data. Scoring on depth rather than on pixel colours matters: "nothing was
drawn here" is a cleared depth value exactly, whereas deciding whether a
bluish pixel is sky or water is guesswork on a level like Fostral.

Produces a grid image (rows = viewpoints, columns = methods) annotated
with frame time and see-through error, plus the numbers on stdout.

Reading the two error columns together is what makes them useful.
`see-through` is solid terrain the renderer left as background;
`covers-sky` is background it filled in. A renderer that is genuinely
missing geometry moves only the first - height-field ray tracing scores
tens of percent see-through against 0.0% covers-sky. When both move by
the same amount it is the reference disagreeing about where the
silhouette falls, not the renderer, and every method in the row shows it.

`depth p50/p95` is how far off the surfaces are, in world units, where
the renderer and the reference both think something is there. Its noise
floor depends almost entirely on pitch, which is worth knowing before
reading any row. The same mesh against the same reference:

    pitch    0   see-through 6.70%   covers-sky 7.36%   depth p50 25.3u
    pitch  -15   see-through 0.00%   covers-sky 0.00%   depth p50  5.5u
    pitch  -30   see-through 0.00%   covers-sky 0.00%   depth p50  3.7u
    pitch  -60   see-through 0.00%   covers-sky 0.00%   depth p50  1.7u

Tilt the camera off the horizon and the disagreement vanishes. At eye
level most of the ground is nearly edge-on, and a sub-pixel difference in
ray direction moves the hit by tens of units - so at pitch 0 the
reference, not the renderer, sets the number. Treat ~25u as its floor
there and compare columns rather than reading a row as absolute error.

`speckle` is the part depth agreement cannot see, and is why it exists:
the fraction of pixels whose distance disagrees with their own 3x3
neighbourhood, in excess of the reference doing the same. Sliced and
Scattered score 7-8% against 0.0-0.4% for the coherent renderers, on a
reference that is itself 0.3% rough. A renderer can put every surface at
the right distance incoherently and still look wrong.

Timing is `submit` then poll to completion, per frame, so it measures GPU
work rather than submission - but serially, which makes it per-frame
latency rather than pipelined throughput. `WGPU_BACKEND` selects the
backend (`vulkan`, `dx12`, `metal`, `gl`).

Prerequisites
-------------
    cargo build --release --bin level --bin convert
    cargo run --release --bin convert -- <level>/world.ini <workdir>/level.ron

The `convert` step writes the height layers this script ray casts against.
Needs numpy and pillow.

Example
-------
    tools/compare-terrain.py \\
        --level-zip fostral.zip --common-zip common.zip \\
        --layers work/level.ron --out work/compare \\
        --view "tunnel:1984,624:100:under" \\
        --view "river:2006,1730:120"
"""

import argparse
import json
import math
import os
import re
import subprocess
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFont

Image.MAX_IMAGE_PIXELS = None

# (label, --terrain value, extra args, warmup frames)
#
# The voxel grid bakes incrementally under a per-frame texel budget, so it
# needs a long warmup on a full level or it renders through terrain it has
# not reached yet - roughly 150 frames for a 2048x16384 map.
METHODS = [
    ("RayTraced", "RayTraced", [], 3),
    ("RayVoxel", "RayVoxelTraced", ["--voxel-size", "4,8,2"], 170),
    ("Sliced", "Sliced", [], 3),
    ("Scattered", "Scattered", [], 8),
    ("Painter", "Painted", [], 3),
    ("Mesh q=0.25", "Mesh", ["--mesh-quality", "0.25"], 3),
    ("Mesh q=0.75", "Mesh", ["--mesh-quality", "0.75"], 3),
]

# Fostral first-person viewpoints: a tunnel interior, a river under a
# span, a deep canyon and an open ridge. Chosen for obstructions at eye
# level, which is where the renderers disagree; a view of open ground
# tells you nothing.
#
# Each was checked against the reference before being listed here - the
# residual disagreement of a *correct* renderer at these, in the order
# below, is 0.0%, 5.0%, 1.8% and 8.5%. It tracks how much horizon is in
# frame, because that is where the CPU march and the rasterizer round the
# silhouette differently. Treat those as the noise floor: a renderer is
# only interesting where it is worse than its neighbours in the same row.
DEFAULT_VIEWS = [
    "tunnel:1984,624:100:under",
    "river:2006,1730:120",
    "canyon:1700,4200:270",
    "ridge:1500,900:45",
]


def parse_view(spec):
    """`name:x,y:yaw[:under]` -> dict."""
    parts = spec.split(":")
    if len(parts) < 3:
        raise ValueError(f"view needs name:x,y:yaw, got {spec!r}")
    x, y = (int(v) for v in parts[1].split(","))
    return {
        "name": parts[0],
        "x": x,
        "y": y,
        "yaw": float(parts[2]),
        "under": len(parts) > 3 and parts[3] == "under",
    }


def load_layers(ron_path):
    """Height layers written by `convert`: low, dual mask, mid, high."""
    base = os.path.dirname(os.path.abspath(ron_path))
    a = np.asarray(Image.open(os.path.join(base, "height.png"))).astype(np.int16)
    low, high, delta = a[:, :, 0], a[:, :, 1], a[:, :, 2]
    dual = (high != low) | (delta != 0)
    #  clamps to , matching ; some
    # source texels encode low + delta above high.
    return low, dual, np.minimum(low + delta, high), high


def ground_truth(layers, view, width, height, eye_height, far, pitch=0.0):
    """Ray cast the height data with the same camera.

    Returns `(sky, dist, dirs, origin)`: a sky mask, the distance along
    each ray to the first solid sample, and the ray directions so the
    caller can turn a renderer's depth buffer into the same quantity.

    Marches coarser with distance, then bisects inside the bracketing
    interval. The refinement matters: a march-quantised distance field is
    fine for a sky mask but useless for measuring how far off a surface
    is, and the step would swamp the differences being looked for.
    """
    low, dual, mid, high = layers
    H, W = low.shape
    base = low if view["under"] else high
    eye = float(base[view["y"], view["x"]]) + eye_height
    yaw, pit = math.radians(view["yaw"]), math.radians(pitch)
    # Yaw first, then tilt within the vertical plane, matching
    # `make_camera` in bin/level/headless.rs. `right` stays horizontal, so
    # the horizon stays level.
    flat = np.array([math.sin(yaw), math.cos(yaw), 0.0])
    right = np.cross(flat, np.array([0.0, 0.0, 1.0]))
    right /= np.linalg.norm(right)
    fwd = flat * math.cos(pit) + np.array([0.0, 0.0, 1.0]) * math.sin(pit)
    up = np.cross(right, fwd)
    # `DEFAULT_FOCAL_PX`. The renderer takes its vertical FOV from
    # `fov_from_focal_px(512, height)`, i.e. a focal length fixed in
    # *pixels*, so the reference must use the same constant and let the
    # frame size decide the angle. Scaling it by the height instead only
    # agreed at 300 px tall and diverged toward the frame edges elsewhere.
    focal = 512.0
    j, i = np.meshgrid(np.arange(height), np.arange(width), indexing="ij")
    d = (
        fwd[None, None, :] * focal
        + right[None, None, :] * (i - width / 2 + 0.5)[..., None]
        + up[None, None, :] * (height / 2 - j - 0.5)[..., None]
    )
    d /= np.linalg.norm(d, axis=2, keepdims=True)
    loc = np.array([float(view["x"]), float(view["y"]), eye])

    def solid(t):
        """Is the sample at distance `t` inside terrain?

        `z <= 0` counts as solid. Terrain cannot go below zero, so a ray
        that gets there has run into the bottom of the world - and the
        alternative is worse: guarding the floor test with `z >= 0` makes
        a texel of height 0 unhittable, since no `z` satisfies both
        `z >= 0` and `z < 0`. Sea level is a real height on these maps, so
        that turned every downward ray over water into sky. It cost ~23%
        of a straight-down frame and a few percent of a level one, which
        had been written off as silhouette noise.
        """
        p = loc[None, None, :] + d * t[..., None]
        z = p[..., 2]
        xi = (p[..., 0].astype(np.int64)) % W
        yi = (p[..., 1].astype(np.int64)) % H
        floor = (z <= 0) | (z < low[yi, xi])
        cave = dual[yi, xi] & (z >= mid[yi, xi]) & (z < high[yi, xi])
        return (z <= 255) & (floor | cave)

    alive = np.ones((height, width), bool)
    hit = np.zeros((height, width), bool)
    dist = np.full((height, width), np.inf)
    prev = np.zeros((height, width))
    t = 0.5
    while t < far:
        step = 0.5 if t < 250 else 2.0
        ts = np.full((height, width), t)
        p = loc[None, None, :] + d * t
        z = p[..., 2]
        alive &= ~(alive & (z > 255) & (d[..., 2] > 0))
        s = alive & solid(ts)
        fresh = s & ~hit
        dist[fresh] = t
        prev[fresh] = t - step
        hit |= s
        alive &= ~s
        if not alive.any():
            break
        t += step

    # Bisect within the bracketing interval. 12 halvings take the 2-unit
    # far step under a thousandth of a unit.
    lo, hi = prev.copy(), np.where(hit, dist, 0.0)
    for _ in range(12):
        m = 0.5 * (lo + hi)
        inside = solid(m)
        hi = np.where(inside, m, hi)
        lo = np.where(inside, lo, m)
    dist = np.where(hit, hi, np.inf)
    return ~hit, dist, d, loc


def ray_distance(depth, dirs, near, far):
    """A depth buffer as distance along each ray, to match the reference.

    The buffer holds distance along the *view axis*; the reference marches
    along the ray. They differ by the cosine off-centre, which reaches 20%
    at the corners of a wide frame - enough to fake a systematic error.
    """
    z = np.clip(depth.astype(np.float64), 0.0, 0.9999995)
    view_z = near * far / (far - z * (far - near))
    fwd = dirs[dirs.shape[0] // 2, dirs.shape[1] // 2]
    fwd = fwd / np.linalg.norm(fwd)
    cos = np.clip(dirs @ fwd, 1e-6, 1.0)
    return view_z / cos


def median3(a):
    """3x3 median, by stacking the nine shifts."""
    p = np.pad(a, 1, mode="edge")
    return np.median(
        np.stack([p[y:y + a.shape[0], x:x + a.shape[1]]
                  for y in range(3) for x in range(3)]),
        axis=0,
    )


def speckle(dist, solid, rel=0.04):
    """Pixels whose distance disagrees with their own neighbourhood.

    This is the part depth agreement alone cannot see. A renderer can put
    every surface at very nearly the right distance and still look wrong
    if it does so incoherently - the sliced terrain draws a stack of
    horizontal quads, and viewed edge-on those alternate between surface
    and gap on neighbouring rows. Averaged over the frame that is a small
    depth error; on screen it is stripes.

    So: flag a pixel whose distance differs from its 3x3 median by more
    than `rel` of its own distance. Comparing the count against the same
    measure on the reference is what keeps genuine detail - foliage,
    rubble, a cliff edge - from being scored as noise.
    """
    med = median3(np.where(solid, dist, np.nan))
    with np.errstate(invalid="ignore"):
        out = np.abs(dist - med) > rel * np.maximum(dist, 1.0)
    return np.nan_to_num(out, nan=False) & solid


def render(args, view, method, out_dir, pitch=0.0):
    label, terrain, extra, warmup = method
    tag = re.sub(r"[^A-Za-z0-9]+", "_", label)
    stem = f"{view['name']}-p{int(pitch)}-{tag}"
    png = os.path.join(out_dir, f"{stem}.png")
    depth = os.path.join(out_dir, f"{stem}.f32")
    bench = os.path.join(out_dir, f"{stem}.json")
    cmd = [
        args.binary, "--snapshot", png, "--depth-out", depth,
        "--terrain", terrain, *extra,
        "--fp", f"{view['x']},{view['y']}",
        "--fp-height", str(args.eye_height),
        f"--fp-yaw={view['yaw']}", f"--fp-pitch={pitch}",
        # The default near plane clips the ground out from under a
        # first-person camera; rasterized terrain then shows through.
        "--near", "1", "--far", str(args.far),
        "--width", str(args.width), "--height", str(args.height),
        "--frames", str(args.frames), "--warmup", str(warmup),
        "--bench-out", bench,
    ]
    if view["under"]:
        cmd.append("--fp-under")
    if args.level_zip:
        cmd += ["--level-zip", args.level_zip]
    if args.common_zip:
        cmd += ["--common-zip", args.common_zip]
    env = dict(os.environ, RUST_LOG="info")
    res = subprocess.run(cmd, capture_output=True, text=True, env=env)
    if res.returncode != 0:
        print(res.stderr[-2000:], file=sys.stderr)
        raise SystemExit(f"render failed: {label} / {view['name']}")
    with open(bench) as f:
        meta = json.load(f)
    return png, depth, meta


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--binary", default="./target/release/level")
    ap.add_argument("--level-zip")
    ap.add_argument("--common-zip")
    ap.add_argument("--layers", required=True,
                    help="level.ron written by the `convert` binary")
    ap.add_argument("--out", required=True)
    ap.add_argument("--view", action="append",
                    help="name:x,y:yaw[:under], repeatable. Defaults to the "
                         "four Fostral viewpoints in DEFAULT_VIEWS")
    ap.add_argument("--pitch", action="append", type=float,
                    help="camera pitch in degrees, repeatable. 0 is "
                         "horizontal, -90 straight down. Defaults to 0 alone; "
                         "the interesting axis is that most methods degrade as "
                         "the view flattens toward the horizon")
    ap.add_argument("--label", default="",
                    help="name for this machine, recorded in the results file")
    ap.add_argument("--json-out",
                    help="write every measurement here, for merging runs from "
                         "several devices with tools/merge-bench.py")
    ap.add_argument("--frames", type=int, default=20,
                    help="timed frames per render. Each is submitted and "
                         "polled to completion, so this measures GPU work "
                         "rather than submission - but serially, so it is "
                         "per-frame latency, not pipelined throughput")
    ap.add_argument("--width", type=int, default=400)
    ap.add_argument("--height", type=int, default=260)
    ap.add_argument("--eye-height", type=float, default=8.0)
    ap.add_argument("--far", type=float, default=600.0,
                    help="view distance, shared by the renderers and the "
                         "reference. Keep it bounded: the painter emits one "
                         "instance per visible ground sample and clamps at a "
                         "million, so an unbounded distance leaves most of "
                         "its frame unpainted")
    args = ap.parse_args()

    os.makedirs(args.out, exist_ok=True)
    views = [parse_view(v) for v in args.view or DEFAULT_VIEWS]
    layers = load_layers(args.layers)

    pitches = args.pitch or [0.0]
    cells, stats, rows = {}, {}, []
    device = None
    for pitch in pitches:
        for view in views:
            sky, ref_dist, dirs, _ = ground_truth(layers, view, args.width, args.height,
                                                  args.eye_height, args.far, pitch)
            ref_solid = ~sky
            ref_speckle = speckle(ref_dist, ref_solid)
            print(f"{view['name']} @ pitch {pitch:g}: ground truth sky = "
                  f"{100 * sky.mean():.1f}% of frame, "
                  f"{100 * ref_speckle.mean():.1f}% of it genuinely rough")
            for method in METHODS:
                png, depth, meta = render(args, view, method, args.out, pitch)
                if device is None:
                    device = {k: meta[k] for k in
                              ("adapter", "backend", "device_type", "driver", "driver_info")}
                ms = meta["avg_ms"]
                d = np.fromfile(depth, dtype="<f4").reshape(args.height, args.width)
                empty = d >= 0.999999          # cleared depth: nothing drawn
                see_through = 100 * (empty & ~sky).mean()
                covers_sky = 100 * ((~empty) & sky).mean()

                # Geometry, where both agree something is there. Median
                # rather than mean: a handful of silhouette pixels straddling
                # a cliff edge otherwise set the number for the whole frame.
                got = ray_distance(d, dirs, 1.0, args.far)
                both = (~empty) & ref_solid
                if both.any():
                    err = np.abs(got[both] - ref_dist[both])
                    p50, p95 = (float(v) for v in np.percentile(err, [50, 95]))
                else:
                    p50 = p95 = float("nan")

                # Coherence, in excess of the reference's own. Counts only
                # pixels the reference thinks are on a smooth surface, so
                # terrain that is genuinely rough is not charged to anyone.
                spk = speckle(got, ~empty)
                excess = 100 * (spk & ~ref_speckle & ref_solid).mean()

                key = (view["name"], pitch, method[0])
                cells[key] = png
                stats[key] = (ms, see_through, covers_sky, p50, p95, excess)
                rows.append({
                    "view": view["name"], "pitch": pitch, "method": method[0],
                    "avg_ms": ms, "min_ms": meta["min_ms"], "max_ms": meta["max_ms"],
                    "frame_ms": meta["frame_ms"],
                    "see_through": see_through, "covers_sky": covers_sky,
                    "depth_p50": p50, "depth_p95": p95, "speckle": excess,
                })
                print(f"    {method[0]:12s} {ms:7.1f} ms   "
                      f"see-through {see_through:5.1f}%   covers-sky {covers_sky:5.1f}%   "
                      f"depth p50 {p50:6.1f}u p95 {p95:7.1f}u   speckle {excess:5.1f}%")

    if args.json_out:
        with open(args.json_out, "w") as f:
            json.dump({
                "label": args.label or (device or {}).get("adapter", "unknown"),
                "device": device,
                "width": args.width, "height": args.height,
                "far": args.far, "frames": args.frames,
                "rows": rows,
            }, f, indent=1)
        print(f"\nwrote {args.json_out}")

    # Grid image: rows are viewpoints, columns are methods.
    w, h, pad, lab, hdr = args.width, args.height, 5, 18, 30
    W = len(METHODS) * (w + pad) + pad
    try:
        font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", 15)
        small = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 11)
    except OSError:
        font = small = ImageFont.load_default()
    grid_views = [(v, p) for p in pitches for v in views]
    H = hdr + len(grid_views) * (h + lab + pad) + pad
    out = Image.new("RGB", (W, H), (20, 22, 26))
    dr = ImageDraw.Draw(out)
    for j, method in enumerate(METHODS):
        dr.text((pad + j * (w + pad) + 2, 8), method[0], font=font, fill=(235, 238, 242))
    for i, (view, pitch) in enumerate(grid_views):
        y = hdr + i * (h + lab + pad)
        dr.text((pad + 2, y + 2),
                f"{view['name']} at ({view['x']},{view['y']}) yaw {view['yaw']:g} "
                f"pitch {pitch:g}" + (" (under)" if view["under"] else ""),
                font=small, fill=(150, 196, 255))
        for j, method in enumerate(METHODS):
            x = pad + j * (w + pad)
            key = (view["name"], pitch, method[0])
            out.paste(Image.open(cells[key]).convert("RGB"), (x, y + lab))
            ms, err, _, _, _, spk = stats[key]
            colour = (255, 120, 120) if err > 5 else ((255, 210, 130) if err > 0.5 else (150, 230, 150))
            dr.text((x + 5, y + lab + h - 16), f"{ms:.1f} ms", font=small, fill=(255, 220, 140))
            dr.text((x + 80, y + lab + h - 16), f"{err:.1f}% see-through", font=small, fill=colour)
            sc = (255, 120, 120) if spk > 5 else ((255, 210, 130) if spk > 1 else (150, 230, 150))
            dr.text((x + 210, y + lab + h - 16), f"{spk:.1f}% speckle", font=small, fill=sc)
    grid = os.path.join(args.out, "comparison.png")
    out.save(grid)
    print(f"\nwrote {grid}")


if __name__ == "__main__":
    main()
