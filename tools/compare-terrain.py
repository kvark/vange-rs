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
import shutil
import subprocess
import sys
import time
import urllib.request
import zipfile

import numpy as np
from PIL import Image, ImageDraw, ImageFont

Image.MAX_IMAGE_PIXELS = None

# (label, --terrain value, extra args, warmup frames)
#
# The voxel grid bakes incrementally under a per-frame texel budget, so it
# needs a long warmup on a full level or it renders through terrain it has
# not reached yet - roughly 150 frames for a 2048x16384 map.
# Settings come from tools/tune-methods.py, which sweeps each method's own
# knob and takes the cheapest setting within a point of that method's best
# error. Two of these are not tunable at all, which is itself worth
# knowing when reading their columns.
METHODS = [
    ("RayTraced", "RayTraced", [], 3),
    # 100 steps is the knee: 40 leaves 6.4% of the frame see-through, 100
    # gets to 2.5%, and 200 buys nothing for 30% more time.
    ("RayVoxel", "RayVoxelTraced",
     ["--voxel-size", "4,8,2", "--voxel-steps", "100"], 170),
    # One slice per altitude unit. Below that the method does not degrade
    # gracefully, it falls off a cliff - 128 layers leaves 61% of the frame
    # see-through and moves surfaces by 259u.
    ("Sliced", "Sliced", ["--slice-layers", "256"], 3),
    # Density 4 is the best this method reaches and it is still last by a
    # wide margin; lower densities are cheaper and worse in every column.
    ("Scattered", "Scattered", ["--scatter-density", "4,4,4"], 8),
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
# The whole point of the sweep. Every method is close to correct looking
# down; they separate as the camera comes to the horizon, which is where a
# first-person or chase camera actually sits.
DEFAULT_PITCHES = [0.0, -30.0, -60.0, -90.0]

DEFAULT_VIEWS = [
    "tunnel:1984,624:100:under",
    "river:2006,1730:120",
    "canyon:1700,4200:270",
    "ridge:1500,900:45",
]


RELEASE = "https://github.com/kvark/vange-rs/releases/download/data-0"


def run(cmd, what):
    print(f"  {what}...", flush=True)
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        sys.stderr.write(r.stdout[-3000:] + r.stderr[-3000:])
        raise SystemExit(f"{what} failed")


def ensure_tools(args):
    """Build the binaries if they are missing. Cargo is the authority on
    whether they are up to date, so this is cheap when they already are."""
    need = not os.path.exists(args.binary) or not os.path.exists("./target/release/convert")
    if need:
        run(["cargo", "build", "--release", "--bin", "level", "--bin", "convert"],
            "building level + convert")


def fetch(url, dest):
    if os.path.exists(dest) and os.path.getsize(dest) > 0:
        return
    print(f"  fetching {os.path.basename(dest)}...", flush=True)
    os.makedirs(os.path.dirname(dest) or ".", exist_ok=True)
    tmp = dest + ".part"
    with urllib.request.urlopen(url) as r, open(tmp, "wb") as f:
        shutil.copyfileobj(r, f)
    os.replace(tmp, dest)


def ensure_assets(args):
    """Fetch, unpack and convert whatever is not already there.

    Every step checks for its own output first, so a re-run does nothing
    but print. The conversion is the expensive one - it writes the height
    layers the reference ray casts against - and it is keyed on the level,
    so switching levels does not invalidate the others.
    """
    work = args.work
    lvl = args.level
    os.makedirs(work, exist_ok=True)

    if not args.level_zip:
        args.level_zip = os.path.join(work, f"{lvl}.zip")
        fetch(f"{RELEASE}/{lvl}.zip", args.level_zip)
    if not args.common_zip:
        args.common_zip = os.path.join(work, "common.zip")
        fetch(f"{RELEASE}/common.zip", args.common_zip)

    if not args.layers:
        src = os.path.join(work, lvl, "src")
        args.layers = os.path.join(work, lvl, "level.ron")
        height = os.path.join(work, lvl, "height.png")
        if not (os.path.exists(args.layers) and os.path.exists(height)):
            os.makedirs(src, exist_ok=True)
            ini = os.path.join(src, "world.ini")
            if not os.path.exists(ini):
                print(f"  unpacking {lvl}.zip...", flush=True)
                for z in (args.level_zip, args.common_zip):
                    with zipfile.ZipFile(z) as zf:
                        zf.extractall(src)
            run(["./target/release/convert", ini, args.layers],
                f"converting {lvl} height layers")

    if not args.out:
        args.out = os.path.join(work, "compare")


def slug(text):
    return re.sub(r"[^A-Za-z0-9]+", "-", text).strip("-").lower() or "unknown"


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
    ap.add_argument("--level", default="fostral",
                    help="stock level id. Its archive is fetched from the "
                         "data-0 release if not already in --work")
    ap.add_argument("--work", default="work",
                    help="cache for archives, unpacked data and the converted "
                         "height layers. Everything here is reused on a "
                         "re-run, so only the first invocation pays for it")
    ap.add_argument("--level-zip", help="override the fetched archive")
    ap.add_argument("--common-zip", help="override the fetched common.zip")
    ap.add_argument("--layers", help="override the converted level.ron")
    ap.add_argument("--out", help="where snapshots go; defaults under --work")
    ap.add_argument("--view", action="append",
                    help="name:x,y:yaw[:under], repeatable. Defaults to the "
                         "four Fostral viewpoints in DEFAULT_VIEWS")
    ap.add_argument("--pitch", action="append", type=float,
                    help="camera pitch in degrees, repeatable. 0 is "
                         "horizontal, -90 straight down. Defaults to the full "
                         "sweep, which is the axis that separates these "
                         "methods: they agree looking down and diverge at the "
                         "horizon")
    ap.add_argument("--label", default="",
                    help="name for this machine. Defaults to the adapter the "
                         "run actually used, which is what you want unless "
                         "one box has several")
    ap.add_argument("--json-out",
                    help="results file. Defaults to "
                         "<work>/results-<adapter>.json, named from the "
                         "adapter wgpu selected")
    ap.add_argument("--quick", action="store_true",
                    help="small and fast, for checking the harness works. Not "
                         "a result: too few frames to be stable and too few "
                         "pixels for the reference to agree with anything")
    ap.add_argument("--frames", type=int, default=40,
                    help="timed frames per render. Each is submitted and "
                         "polled to completion, so this measures GPU work "
                         "rather than submission - but serially, so it is "
                         "per-frame latency, not pipelined throughput")
    ap.add_argument("--width", type=int, default=1280)
    ap.add_argument("--height", type=int, default=800)
    ap.add_argument("--eye-height", type=float, default=8.0)
    ap.add_argument("--far", type=float, default=600.0,
                    help="view distance, shared by the renderers and the "
                         "reference. Keep it bounded: the painter emits one "
                         "instance per visible ground sample and clamps at a "
                         "million, so an unbounded distance leaves most of "
                         "its frame unpainted")
    args = ap.parse_args()
    if args.quick:
        args.width, args.height, args.frames = 320, 200, 4
        args.pitch = args.pitch or [0.0]

    ensure_tools(args)
    ensure_assets(args)
    os.makedirs(args.out, exist_ok=True)
    views = [parse_view(v) for v in args.view or DEFAULT_VIEWS]
    layers = load_layers(args.layers)

    pitches = args.pitch or DEFAULT_PITCHES
    cells, stats, rows = {}, {}, []
    device = None

    # A default run is a couple of hundred renders and takes a while, so
    # say so up front and keep a running estimate rather than going quiet.
    total = len(pitches) * len(views) * len(METHODS)
    done, started = 0, time.time()
    print(f"{total} renders: {len(METHODS)} methods x {len(views)} views x "
          f"{len(pitches)} pitches, at {args.width}x{args.height}\n")

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
                # Prefer the GPU's own view when the adapter can give it.
                # The CPU figure brackets submit-and-poll, so it carries the
                # round trip; on lavapipe that is ~9%, and on a real GPU with
                # a fast frame it can be most of the number.
                gpu = meta.get("gpu_avg_ms")
                have_gpu = meta.get("gpu_timing") and gpu is not None and gpu == gpu
                ms = gpu if have_gpu else meta["avg_ms"]
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
                    "avg_ms": ms,
                    "timing": "gpu" if have_gpu else "cpu",
                    "cpu_avg_ms": meta["avg_ms"],
                    "min_ms": meta.get("gpu_min_ms") if have_gpu else meta["min_ms"],
                    "max_ms": meta["max_ms"],
                    "frame_ms": meta["gpu_ms"] if have_gpu else meta["frame_ms"],
                    "prep_setup_ms": meta.get("prep_setup_ms"),
                    "prep_first_frame_ms": meta.get("prep_first_frame_ms"),
                    "prep_warmup_ms": meta.get("prep_warmup_ms"),
                    "see_through": see_through, "covers_sky": covers_sky,
                    "depth_p50": p50, "depth_p95": p95, "speckle": excess,
                })
                # `nan` when the renderer and the reference never agree
                # anything is there - which is itself the finding, so say
                # so rather than printing a number-shaped blank.
                dep = ("      n/a" if p50 != p50
                       else f"p50 {p50:6.1f}u p95 {p95:7.1f}u")
                tag = "gpu" if have_gpu else "cpu"
                done += 1
                elapsed = time.time() - started
                eta = elapsed / done * (total - done)
                print(f"    {method[0]:12s} {ms:7.1f} ms {tag}  "
                      f"see-through {see_through:5.1f}%   covers-sky {covers_sky:5.1f}%   "
                      f"depth {dep}   speckle {excess:5.1f}%"
                      f"   [{done}/{total}, {eta / 60:.0f} min left]", flush=True)

    # Named from the adapter wgpu actually chose, not from anything the
    # caller had to know in advance.
    json_out = args.json_out
    if json_out is None and device:
        json_out = os.path.join(args.work, f"results-{slug(device['adapter'])}.json")
    if json_out:
        with open(json_out, "w") as f:
            json.dump({
                "label": args.label or (device or {}).get("adapter", "unknown"),
                "level": args.level,
                "device": device,
                "width": args.width, "height": args.height,
                "far": args.far, "frames": args.frames,
                "rows": rows,
            }, f, indent=1)
        print(f"\nwrote {json_out}")

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
