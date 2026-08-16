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
missing geometry moves only the first. When both move by the same amount it
is usually the reference disagreeing about where the silhouette falls, not
the renderer, especially when every method in the row shows it.

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
        --view "span:1108,15875:227" \\
        --view "cave:1492,15833:180:under"
"""

import argparse
import datetime
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

# (label, --terrain value, extra args, warmup frames)
#
# The voxel grid bakes incrementally under a per-frame texel budget, so it
# needs a long warmup on a full level or it renders through terrain it has
# not reached yet - roughly 150 frames for a 2048x16384 map.
# Settings come from tools/tune-methods.py, which sweeps each method's own
# knob and takes the cheapest setting within a point of that method's best
# error. One of these is not tunable at all, which is itself worth
# knowing when reading their columns.
METHODS = [
    ("RayTraced", "RayTraced", ["--ray-steps", "64"], 3),
    # The step budget the fixture's sightlines set: 40 steps lands within a
    # point of 100 on these views (5.5% vs 4.8% error), so the selection
    # rule keeps it. The old fixture's long sightlines exhausted it - 6.4%
    # see-through at 40 where 100 got to 2.5% - which is a fact about that
    # fixture, not a different method.
    ("RayVoxel", "RayVoxelTraced",
     ["--voxel-size", "4,8,2", "--voxel-steps", "40"], 170),
    # Two slices per altitude unit, the setting the selection rule picks
    # once slices spread honestly over the height range: see-through keeps
    # falling (17.2% at 32 to 5.4% at 512) while speckle peaks mid-sweep
    # (10.9% at 256), slices converting missing spans into isolated wrong
    # pixels.
    ("Sliced", "Sliced", ["--slice-layers", "512"], 3),
    # Density 4 is the best this method reaches and it is still last by a
    # wide margin; lower densities are cheaper and worse in every column.
    ("Scattered", "Scattered", ["--scatter-density", "4,4,4"], 8),
    ("Painter", "Painted", [], 3),
    # The tuning-resolution reference picks the cheapest fit. Keep q=0.75
    # beside it because the full-size hangar scene resolves a coverage
    # difference that the smaller tuning pass does not.
    ("Mesh q=0.0", "Mesh", ["--mesh-quality", "0.0"], 3),
    ("Mesh q=0.75", "Mesh", ["--mesh-quality", "0.75"], 3),
]

# Fostral viewpoints, three per pitch. Each view runs at its own pitch -
# a camera looking down renders a different subject than one at the
# horizon, so a view is defined by where it looks from *and* how far it
# tilts. The eye height is part of the spec too: these were picked by
# walking the map in the viewer, so each one has the height that framed
# its subject.
#
# The pitch-0 views are deliberately elevated above the local surface so
# obstructions and depth layering remain visible at the horizon. Keep the
# per-view eye height in the spec: omitting it silently falls back to the
# global eight-unit default and no longer reproduces the framed shot.
DEFAULT_PITCHES = [0.0, -30.0, -60.0, -90.0]

# A bounded, visible edit fixture. The camera stands south of the crater and
# looks toward it; keeping this separate from the twelve steady-state scenes
# avoids hiding an update spike inside an ordinary frame average.
EDIT_VIEW = {
    "name": "edit-crater", "x": 1024, "y": 8350, "yaw": 180.0,
    "eye_height": 40.0, "under": False,
}
EDIT_PITCH = -30.0
EDIT_CENTER = (1024, 8192)
EDIT_RADIUS = 48
EDIT_FRAME_COUNTS = (1, 2, 4, 8, 16)
EDIT_TIMING_REPEATS = 5
PROTOCOL_VERSION = 3

DEFAULT_VIEWS = {
    0.0: [
        "river:837,3984:299:78",
        "hangar:1588,4101:286:12",
        "ramp:1513,15659:172:28",
    ],
    -30.0: [
        "portal:1176,11567:293:83",
        "entrance:1764,15254:293:108",
        "river-down:457,12337:158:180",
    ],
    -60.0: [
        "stash:1929,13864:293:161",
        "copterig charger:1727,4805:199:297",
        "wires:519,14910:293:192",
    ],
    -90.0: [
        "spiral charger:1554,7798:10:312",
        "gorb charger:471,892:13:332",
        "secret:237,12065:10:376",
    ],
}


RELEASE = "https://github.com/kvark/vange-rs/releases/download/data-0"


def load_dependencies():
    global np, Image, ImageDraw, ImageFont
    if "np" in globals():
        return
    import numpy as np
    from PIL import Image, ImageDraw, ImageFont
    Image.MAX_IMAGE_PIXELS = None


def run(cmd, what):
    print(f"  {what}...", flush=True)
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        sys.stderr.write(r.stdout[-3000:] + r.stderr[-3000:])
        raise SystemExit(f"{what} failed")


def source_state():
    """Identify the checkout that produced a long-running benchmark."""
    try:
        revision = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], text=True,
            stderr=subprocess.DEVNULL).strip()
        dirty = bool(subprocess.check_output(
            ["git", "status", "--porcelain", "--untracked-files=no"],
            text=True, stderr=subprocess.DEVNULL).strip())
    except (OSError, subprocess.CalledProcessError):
        revision, dirty = "unknown", None
    return {"revision": revision, "dirty": dirty}


def write_json_atomic(path, value):
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(value, f, indent=1)
        f.write("\n")
    os.replace(tmp, path)


def start_run(out_dir, manifest):
    """Invalidate prior output before a long run can be mistaken for it."""
    os.makedirs(out_dir, exist_ok=True)
    grid = os.path.join(out_dir, "comparison.png")
    partial = os.path.join(out_dir, "comparison.partial.png")
    for path in (grid, partial):
        if os.path.exists(path):
            os.remove(path)
    manifest_path = os.path.join(out_dir, "run-manifest.json")
    write_json_atomic(manifest_path, manifest)
    return manifest_path, grid, partial


def print_run_plan(source, scenes, methods=None):
    revision = source["revision"][:12]
    dirty = " dirty" if source["dirty"] else ""
    print(f"source {revision}{dirty}")
    print("scenes: " + ", ".join(
        f"{scene['name']}=({scene['x']},{scene['y']}) yaw {scene['yaw']:g} "
        f"pitch {scene['pitch']:g} height {scene['eye_height']:g}" +
        (" under" if scene["under"] else "")
        for scene in scenes))
    if methods is not None:
        print("methods: " + ", ".join(
            method["label"] +
            (" (" + " ".join(method["args"]) + ")" if method["args"] else "")
            for method in methods))


def ensure_tools(args):
    """Build the binaries. Always - cargo is the authority on whether they
    are current, and asking it is nearly free when they are.

    Skipping this when the file merely exists is wrong: a binary left over
    from an older checkout runs happily and writes an older result format,
    which surfaces much later as a missing field rather than as "your build
    is stale"."""
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
    """`name:x,y:yaw[:height][:under]` -> dict."""
    parts = spec.split(":")
    if len(parts) < 3 or len(parts) > 5:
        raise ValueError(f"view needs name:x,y:yaw[:height][:under], got {spec!r}")
    x, y = (int(v) for v in parts[1].split(","))
    tail = parts[3:]
    under = bool(tail and tail[-1] == "under")
    if under:
        tail = tail[:-1]
    if len(tail) > 1 or any(value == "under" for value in tail):
        raise ValueError(f"view needs name:x,y:yaw[:height][:under], got {spec!r}")
    return {
        "name": parts[0],
        "x": x,
        "y": y,
        "yaw": float(parts[2]),
        "eye_height": float(tail[0]) if tail else None,
        "under": under,
    }


def load_layers(ron_path):
    """Height layers written by `convert`: low, dual mask, mid, high."""
    base = os.path.dirname(os.path.abspath(ron_path))
    a = np.asarray(Image.open(os.path.join(base, "height.png"))).astype(np.int16)
    low, high, delta = a[:, :, 0], a[:, :, 1], a[:, :, 2]
    dual = (high != low) | (delta != 0)
    # Clamp `mid` to `high`, matching `Level::get_mid_altitude`; some source
    # texels encode low + delta above high.
    return low, dual, np.minimum(low + delta, high), high


def camera_rays(width, height, yaw_degrees, pitch_degrees):
    """World-space pixel-center rays matching the headless camera basis."""
    yaw, pit = math.radians(yaw_degrees), math.radians(pitch_degrees)
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
    # `make_camera` folds a Y reflection into the view matrix and stores
    # `-right` in the rotation's X column. Consequently increasing screen X
    # points along world `-right`, not `right`. The old sign horizontally
    # mirrored the CPU reference: the centre ray still agreed, while wide
    # top-down frames compared each rendered pixel with unrelated terrain.
    d = (
        fwd[None, None, :] * focal
        - right[None, None, :] * (i - width / 2 + 0.5)[..., None]
        + up[None, None, :] * (height / 2 - j - 0.5)[..., None]
    )
    d /= np.linalg.norm(d, axis=2, keepdims=True)
    return d, fwd


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
    d, fwd = camera_rays(width, height, view["yaw"], pitch)
    loc = np.array([float(view["x"]), float(view["y"]), eye])
    # Perspective near/far planes are distances along the view axis, not
    # spherical distances from the eye. An off-axis ray therefore reaches
    # `far / cos(theta)` before the GPU clips it. Stopping every CPU ray at
    # `far` made the wide top-down frames report common-mode "covers sky"
    # and large depth errors near their edges even when all renderers agreed.
    cos_view = np.clip(d @ fwd, 1e-6, 1.0)
    near_t = 1.0 / cos_view
    far_t = far / cos_view

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
    while t <= float(far_t.max()):
        step = 0.5 if t < 250 else 2.0
        ts = np.full((height, width), t)
        p = loc[None, None, :] + d * t
        z = p[..., 2]
        alive &= t <= far_t
        alive &= ~(alive & (z > 255) & (d[..., 2] > 0))
        s = alive & (t >= near_t) & solid(ts)
        fresh = s & ~hit
        dist[fresh] = t
        prev[fresh] = np.maximum(t - step, near_t[fresh])
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
    # Symmetric pixel-center rays average to the true camera forward vector;
    # choosing one of the four middle pixels in an even-sized image does not.
    fwd = dirs.mean(axis=(0, 1))
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
        "--fp-height", str(view["eye_height"] if view["eye_height"] is not None else args.eye_height),
        f"--fp-yaw={view['yaw']}", f"--fp-pitch={pitch}",
        # The default near plane clips the ground out from under a
        # first-person camera; rasterized terrain then shows through.
        "--near", "1", "--far", str(args.far),
        "--width", str(args.width), "--height", str(args.height),
        "--frames", str(args.frames), "--warmup", str(warmup),
        "--bench-out", bench,
    ]
    # Visual parity is part of the publication protocol. Every method
    # receives the same 1024² height-field shadow pass unless an explicitly
    # method-only diagnostic run asks to omit it.
    if not args.no_shadows:
        cmd.append("--shadow-ray")
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


def render_edit_case(args, method, out_dir, frames, fresh):
    label, terrain, extra, warmup = method
    tag = re.sub(r"[^A-Za-z0-9]+", "_", label)
    kind = "fresh" if fresh else f"incremental-{frames}"
    stem = os.path.join(out_dir, f"{tag}-{kind}")
    cmd = [
        args.binary, "--snapshot", stem + ".png",
        "--depth-out", stem + ".f32", "--bench-out", stem + ".json",
        "--terrain", terrain, *extra,
        "--fp", f"{EDIT_VIEW['x']},{EDIT_VIEW['y']}",
        "--fp-height", str(EDIT_VIEW["eye_height"]),
        f"--fp-yaw={EDIT_VIEW['yaw']}", f"--fp-pitch={EDIT_PITCH}",
        "--near", "1", "--far", str(args.far),
        "--width", str(args.width), "--height", str(args.height),
        "--frames", str(frames), "--warmup", str(warmup),
        "--dig", "--dig-frame", str(0 if fresh else warmup),
        "--dig-center", f"{EDIT_CENTER[0]},{EDIT_CENTER[1]}",
        "--dig-radius", str(EDIT_RADIUS),
    ]
    if not args.no_shadows:
        cmd.append("--shadow-ray")
    if args.level_zip:
        cmd += ["--level-zip", args.level_zip]
    if args.common_zip:
        cmd += ["--common-zip", args.common_zip]
    result = subprocess.run(cmd, capture_output=True, text=True,
                            env=dict(os.environ, RUST_LOG="info"))
    if result.returncode:
        print(result.stderr[-3000:], file=sys.stderr)
        raise SystemExit(f"edit render failed: {label} / {kind}")
    with open(stem + ".json") as source:
        meta = json.load(source)
    return stem + ".png", stem + ".f32", meta


def edit_agreement(reference_png, reference_depth, candidate_png,
                   candidate_depth, dirs, width, height, far):
    ref = np.fromfile(reference_depth, dtype="<f4").reshape(height, width)
    got = np.fromfile(candidate_depth, dtype="<f4").reshape(height, width)
    ref_empty = ref >= 0.999999
    got_empty = got >= 0.999999
    class_mismatch = 100.0 * np.not_equal(ref_empty, got_empty).mean()
    both = ~ref_empty & ~got_empty
    if both.any():
        delta = np.abs(ray_distance(ref, dirs, 1.0, far)[both] -
                       ray_distance(got, dirs, 1.0, far)[both])
        depth_p50, depth_p95 = (float(value) for value in
                                np.percentile(delta, [50, 95]))
    else:
        depth_p50 = depth_p95 = float("nan")
    ref_color = np.asarray(Image.open(reference_png).convert("RGB"), dtype=np.int16)
    got_color = np.asarray(Image.open(candidate_png).convert("RGB"), dtype=np.int16)
    color_mae = float(np.abs(ref_color - got_color).mean())
    consistent = class_mismatch <= 0.01 and depth_p95 == depth_p95 and depth_p95 <= 0.1
    return {
        "class_mismatch_pct": class_mismatch,
        "depth_p50": depth_p50,
        "depth_p95": depth_p95,
        "color_mae_8bit": color_mae,
        "consistent": consistent,
    }


def run_edit_protocol(args, layers):
    out_dir = os.path.join(args.work, "edit-protocol")
    os.makedirs(out_dir, exist_ok=True)
    _, _, dirs, _ = ground_truth(
        layers, EDIT_VIEW, args.width, args.height,
        EDIT_VIEW["eye_height"], args.far, EDIT_PITCH)
    rows = []
    device = None
    print("\nDynamic-edit protocol:", flush=True)
    for method in METHODS:
        label = method[0]
        fresh_png, fresh_depth, fresh_meta = render_edit_case(
            args, method, out_dir, 10, True)
        device = device or {key: fresh_meta[key] for key in
                            ("adapter", "backend", "device_type", "driver", "driver_info")}
        first_cpu_samples = []
        final = None
        consistent_after = None
        for frames in EDIT_FRAME_COUNTS:
            png, depth, meta = render_edit_case(args, method, out_dir, frames, False)
            if not first_cpu_samples:
                first_cpu_samples.append(meta["frame_ms"][0])
            final = edit_agreement(fresh_png, fresh_depth, png, depth, dirs,
                                   args.width, args.height, args.far)
            if final["consistent"]:
                consistent_after = frames
                break
        while len(first_cpu_samples) < EDIT_TIMING_REPEATS:
            _, _, meta = render_edit_case(args, method, out_dir, 1, False)
            first_cpu_samples.append(meta["frame_ms"][0])
        first_cpu_ms = float(np.median(first_cpu_samples))
        first_cpu_p25, first_cpu_p75 = (
            float(value) for value in np.percentile(first_cpu_samples, [25, 75]))
        steady_cpu_ms = float(np.median(fresh_meta["frame_ms"]))
        row = {
            "method": label,
            "steady_edited_cpu_ms": steady_cpu_ms,
            "first_post_edit_cpu_ms": first_cpu_ms,
            "first_post_edit_cpu_samples_ms": first_cpu_samples,
            "first_post_edit_cpu_p25_ms": first_cpu_p25,
            "first_post_edit_cpu_p75_ms": first_cpu_p75,
            "edit_overhead_cpu_ms": first_cpu_ms - steady_cpu_ms,
            "consistent_after_frames": consistent_after,
            **final,
            "method_gpu_bytes": fresh_meta.get("method_gpu_bytes"),
            "method_cpu_bytes": fresh_meta.get("method_cpu_bytes"),
        }
        rows.append(row)
        state = str(consistent_after) if consistent_after is not None else ">16"
        print(f"  {label:12s} first {first_cpu_ms:8.2f} ms CPU, "
              f"overhead {row['edit_overhead_cpu_ms']:+7.2f} ms, "
              f"consistent after {state:>3s} frame(s), "
              f"class {final['class_mismatch_pct']:.4f}%, "
              f"depth p95 {final['depth_p95']:.3f}u", flush=True)
    return device, rows


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
                    help="name:x,y:yaw[:height][:under], repeatable. Defaults "
                         "to DEFAULT_VIEWS, which pairs three views with each "
                         "pitch")
    ap.add_argument("--pitch", action="append", type=float,
                    help="camera pitch in degrees, repeatable. 0 is "
                         "horizontal, -90 straight down. Only meaningful with "
                         "--view, where it pairs with every view given; the "
                         "defaults already carry their own pitches")
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
    ap.add_argument("--accuracy-only", action="store_true",
                    help="rebuild the corrected geometry baseline with one "
                         "timed frame per cell; timings from this run are not "
                         "publication measurements")
    ap.add_argument("--allow-dirty", action="store_true",
                    help="permit a non-reproducible run from a dirty checkout")
    ap.add_argument("--frames", type=int, default=40,
                    help="timed frames per render. Each is submitted and "
                         "polled to completion, so this measures GPU work "
                         "rather than submission - but serially, so it is "
                         "per-frame latency, not pipelined throughput")
    ap.add_argument("--width", type=int, default=1280)
    ap.add_argument("--height", type=int, default=800)
    ap.add_argument("--eye-height", type=float, default=8.0,
                    help="eye height above terrain. Per-view override via the "
                         "fourth field of --view")
    ap.add_argument("--far", type=float, default=600.0,
                    help="view distance, shared by the renderers and the "
                         "reference. Keep it bounded: the painter emits one "
                         "instance per visible ground sample and clamps at a "
                         "million, so an unbounded distance leaves most of "
                         "its frame unpainted")
    ap.add_argument("--no-shadows", action="store_true",
                    help="omit the common shadow pass for a method-only "
                         "diagnostic; publication defaults keep it enabled")
    ap.add_argument("--list-scenes", action="store_true",
                    help="print source revision and exact camera plan, then "
                         "exit without building, fetching, or rendering")
    ap.add_argument("--skip-edits", action="store_true",
                    help="skip the publication dynamic-edit protocol")
    ap.add_argument("--edits-only", action="store_true",
                    help="run only the short dynamic-edit and memory protocol; "
                         "use this to supplement an older complete batch")
    args = ap.parse_args()
    if args.accuracy_only:
        args.frames = 1
        args.skip_edits = True
    if args.quick:
        args.width, args.height, args.frames = 320, 200, 4
        args.view = args.view or ["quick:200,200:0"]
        args.pitch = args.pitch or [0.0]
        if not args.edits_only:
            args.skip_edits = True

    # A "scene" is a view at the pitch it was framed at. DEFAULT_VIEWS
    # already pairs each view with its pitch; --view/--pitch overrides fall
    # back to the old cross product for ad-hoc sweeps.
    if args.view:
        scenes = [(parse_view(v), p)
                  for p in (args.pitch or DEFAULT_PITCHES)
                  for v in args.view]
    else:
        scenes = [(parse_view(v), p)
                  for p, vlist in DEFAULT_VIEWS.items()
                  for v in vlist]
    views = [v for v, _ in scenes]
    # Rows, cells and the grid are keyed by view name, so two views sharing
    # one would silently overwrite each other. `level` dumps every camera
    # as "spot:", which is exactly how a collision sneaks in.
    names = [v["name"] for v in views]
    dupes = sorted({n for n in names if names.count(n) > 1})
    if dupes:
        raise SystemExit(
            f"duplicate view name(s) {', '.join(dupes)}: give each view its "
            "own name (the level viewer dumps them all as 'spot:')"
        )

    source = source_state()
    if source["dirty"] and not args.allow_dirty:
        raise SystemExit(
            "refusing to collect from a dirty checkout; commit/stash changes "
            "or pass --allow-dirty for a non-publication diagnostic")
    scene_records = [
        {
            "name": view["name"], "x": view["x"], "y": view["y"],
            "yaw": view["yaw"], "pitch": pitch,
            "eye_height": (view["eye_height"] if view["eye_height"] is not None
                           else args.eye_height),
            "under": view["under"],
        }
        for view, pitch in scenes
    ]
    method_records = [
        {"label": label, "terrain": terrain, "args": extra,
         "warmup_frames": warmup}
        for label, terrain, extra, warmup in METHODS
    ]
    if args.list_scenes:
        print_run_plan(source, scene_records, method_records)
        return

    if not args.out:
        args.out = os.path.join(args.work, "compare")
    manifest = {
        "status": "running",
        "protocol_version": PROTOCOL_VERSION,
        "purpose": "accuracy-only" if args.accuracy_only else "publication",
        "started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "source": source,
        "level": args.level,
        "width": args.width, "height": args.height,
        "far": args.far, "frames": args.frames,
        "shadows": ("disabled" if args.no_shadows else
                    "ray-traced 1024x1024"),
        "scenes": scene_records,
        "methods": method_records,
    }
    # A full run takes about an hour. Invalidate the previous aggregate before
    # builds, downloads, or rendering begin, then publish its replacement only
    # after every cell succeeds. An edit-only supplement must not invalidate
    # an already complete comparison grid.
    if args.edits_only:
        manifest_path = grid = grid_tmp = None
    else:
        manifest_path, grid, grid_tmp = start_run(args.out, manifest)

    load_dependencies()

    ensure_tools(args)
    ensure_assets(args)

    layers = load_layers(args.layers)

    if args.edits_only:
        device, edit_rows = run_edit_protocol(args, layers)
        json_out = args.json_out or os.path.join(
            args.work, f"edit-results-{slug(device['adapter'])}.json")
        write_json_atomic(json_out, {
            "protocol_version": PROTOCOL_VERSION,
            "purpose": "edit-and-memory",
            "label": args.label or device["adapter"],
            "source": source,
            "level": args.level,
            "device": device,
            "width": args.width, "height": args.height,
            "far": args.far,
            "shadows": ("disabled" if args.no_shadows else
                        "ray-traced 1024x1024"),
            "edit_protocol": {
                "view": EDIT_VIEW, "pitch": EDIT_PITCH,
                "center": EDIT_CENTER, "radius": EDIT_RADIUS,
                "tested_frame_counts": EDIT_FRAME_COUNTS,
                "timing_repeats": EDIT_TIMING_REPEATS,
                "timing": "CPU submit-and-wait",
            },
            "edit_rows": edit_rows,
        })
        print(f"\nwrote {json_out}")
        return

    cells, stats, rows = {}, {}, []
    device = None

    # A default run is a couple of hundred renders and takes a while, so
    # say so up front and keep a running estimate rather than going quiet.
    total = len(scenes) * len(METHODS)
    done, started = 0, time.time()
    print_run_plan(source, scene_records, method_records)
    print(f"{total} renders: {len(METHODS)} methods x {len(scenes)} scenes, "
          f"at {args.width}x{args.height}\n")

    for view, pitch in scenes:
        eye = view["eye_height"] if view["eye_height"] is not None else args.eye_height
        sky, ref_dist, dirs, _ = ground_truth(layers, view, args.width, args.height,
                                              eye, args.far, pitch)
        ref_solid = ~sky
        ref_speckle = speckle(ref_dist, ref_solid)
        print(f"{view['name']} @ pitch {pitch:g}: ground truth sky = "
              f"{100 * sky.mean():.1f}% of frame, "
              f"{100 * ref_speckle.mean():.1f}% of it genuinely rough")
        for method in METHODS:
            png, depth, meta = render(args, view, method, args.out, pitch)
            fields = ("adapter", "backend", "device_type", "driver",
                      "driver_info")
            missing = [key for key in fields if key not in meta]
            if missing:
                raise SystemExit(
                    f"{args.binary} wrote a result without {missing}. "
                    "That binary predates the fields this script needs; "
                    "`cargo build --release --bin level` and re-run.")
            cell_device = {key: meta[key] for key in fields}
            if device is None:
                device = cell_device
            elif cell_device != device:
                raise SystemExit(
                    f"adapter changed during collection: {device['adapter']} "
                    f"to {cell_device['adapter']}")
            expected_meta = {
                "width": args.width,
                "height": args.height,
                "frames": args.frames,
                "fp_yaw_deg": view["yaw"],
                "fp_pitch_deg": pitch,
                "near": 1.0,
                "far": args.far,
            }
            wrong = [key for key, value in expected_meta.items()
                     if key not in meta or abs(meta[key] - value) > 1e-5]
            frame_count = len(meta.get("frame_ms", []))
            if wrong or frame_count != args.frames:
                raise SystemExit(
                    f"{method[0]} / {view['name']} wrote incompatible "
                    f"benchmark metadata (fields {wrong}, frames "
                    f"{frame_count}); rebuild and re-run")
            # Prefer the GPU's own view when the adapter can give it.
            # The CPU figure brackets submit-and-poll, so it carries the
            # round trip; on lavapipe that is ~9%, and on a real GPU with
            # a fast frame it can be most of the number.
            gpu = meta.get("gpu_avg_ms")
            # Encoder-level timestamp writes are not a reliable enclosing
            # pair on Metal for this multipass frame. Older collectors may
            # still report them, so reject them here as well as in Rust.
            have_gpu = (meta.get("backend") != "Metal" and
                        meta.get("gpu_timing") and gpu is not None and gpu == gpu)
            ms = gpu if have_gpu else meta["avg_ms"]
            selected_samples = meta["gpu_ms"] if have_gpu else meta["frame_ms"]
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
                "x": view["x"], "y": view["y"], "yaw": view["yaw"],
                "eye_height": eye, "under": view["under"],
                "avg_ms": ms,
                "timing": "gpu" if have_gpu else "cpu",
                "cpu_avg_ms": meta["avg_ms"],
                "min_ms": min(selected_samples),
                "max_ms": max(selected_samples),
                "frame_ms": selected_samples,
                "cpu_frame_ms": meta["frame_ms"],
                "gpu_frame_ms": meta["gpu_ms"],
                "prep_setup_ms": meta.get("prep_setup_ms"),
                "prep_first_frame_ms": meta.get("prep_first_frame_ms"),
                "prep_warmup_ms": meta.get("prep_warmup_ms"),
                "method_gpu_bytes": meta.get("method_gpu_bytes"),
                "method_cpu_bytes": meta.get("method_cpu_bytes"),
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

    edit_rows = []
    if not args.skip_edits:
        edit_device, edit_rows = run_edit_protocol(args, layers)
        if edit_device != device:
            raise SystemExit("edit protocol selected a different adapter")

    # Named from the adapter wgpu actually chose, not from anything the
    # caller had to know in advance.
    json_out = args.json_out
    if json_out is None and device:
        prefix = "accuracy-results" if args.accuracy_only else "results"
        json_out = os.path.join(
            args.work, f"{prefix}-{slug(device['adapter'])}.json")
    if json_out:
        write_json_atomic(json_out, {
            "protocol_version": PROTOCOL_VERSION,
            "purpose": "accuracy-only" if args.accuracy_only else "publication",
            "label": args.label or (device or {}).get("adapter", "unknown"),
            "source": source,
            "level": args.level,
            "device": device,
            "width": args.width, "height": args.height,
            "far": args.far, "frames": args.frames,
            "shadows": ("disabled" if args.no_shadows else
                        "ray-traced 1024x1024"),
            "lighting": "unbaked diffuse",
            "scenes": scene_records,
            "methods": method_records,
            "rows": rows,
            "edit_protocol": ({
                "view": EDIT_VIEW, "pitch": EDIT_PITCH,
                "center": EDIT_CENTER, "radius": EDIT_RADIUS,
                "tested_frame_counts": EDIT_FRAME_COUNTS,
                "timing_repeats": EDIT_TIMING_REPEATS,
                "timing": "CPU submit-and-wait",
            } if edit_rows else None),
            "edit_rows": edit_rows,
        })
        print(f"\nwrote {json_out}")

    # Grid image: rows are viewpoints, columns are methods.
    w, h, pad, lab, hdr = args.width, args.height, 5, 18, 30
    W = len(METHODS) * (w + pad) + pad
    try:
        font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", 15)
        small = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 11)
    except OSError:
        font = small = ImageFont.load_default()
    grid_views = scenes
    H = hdr + len(grid_views) * (h + lab + pad) + pad
    out = Image.new("RGB", (W, H), (20, 22, 26))
    dr = ImageDraw.Draw(out)
    for j, method in enumerate(METHODS):
        dr.text((pad + j * (w + pad) + 2, 8), method[0], font=font, fill=(235, 238, 242))
    for i, (view, pitch) in enumerate(grid_views):
        y = hdr + i * (h + lab + pad)
        eye_height = (view["eye_height"] if view["eye_height"] is not None
                      else args.eye_height)
        label = (
            f"{view['name']} at ({view['x']},{view['y']}) yaw {view['yaw']:g} "
            f"pitch {pitch:g} height {eye_height:g}" +
            (" (under)" if view["under"] else "")
        )
        dr.text((pad + 2, y + 2), label, font=small, fill=(150, 196, 255))
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
    out.save(grid_tmp)
    os.replace(grid_tmp, grid)
    print(f"\nwrote {grid}")

    manifest.update({
        "status": "complete",
        "completed_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "device": device,
        "results": json_out,
        "comparison": grid,
    })
    write_json_atomic(manifest_path, manifest)


if __name__ == "__main__":
    main()
