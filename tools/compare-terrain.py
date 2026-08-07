#!/usr/bin/env python3
"""Compare terrain rendering methods on a level, scored against ground truth.

Renders the same camera with each method, dumps the depth buffer, and
scores every renderer against a CPU ray cast of the level's own height
data. Scoring on depth rather than on pixel colours matters: "nothing was
drawn here" is a cleared depth value exactly, whereas deciding whether a
bluish pixel is sky or water is guesswork on a level like Fostral.

Produces a grid image (rows = viewpoints, columns = methods) annotated
with frame time and see-through error, plus the numbers on stdout.

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
    ("Painter", "Painted", [], 3),
    ("Mesh q=0.75", "Mesh", ["--mesh-quality", "0.75"], 3),
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


def ground_truth(layers, view, width, height, eye_height, far):
    """Sky mask from ray casting the height data with the same camera.

    Marches coarser with distance: near geometry sets the silhouette, and
    a fine step all the way to the far plane would be needlessly slow.
    """
    low, dual, mid, high = layers
    H, W = low.shape
    base = low if view["under"] else high
    eye = float(base[view["y"], view["x"]]) + eye_height
    yaw = math.radians(view["yaw"])
    fwd = np.array([math.sin(yaw), math.cos(yaw), 0.0])
    right = np.cross(fwd, np.array([0.0, 0.0, 1.0]))
    right /= np.linalg.norm(right)
    up = np.cross(right, fwd)
    # Matches `DEFAULT_FOCAL_PX` scaled to the render height.
    focal = 512.0 * (height / 300.0)
    j, i = np.meshgrid(np.arange(height), np.arange(width), indexing="ij")
    d = (
        fwd[None, None, :] * focal
        + right[None, None, :] * (i - width / 2 + 0.5)[..., None]
        + up[None, None, :] * (height / 2 - j - 0.5)[..., None]
    )
    d /= np.linalg.norm(d, axis=2, keepdims=True)
    loc = np.array([float(view["x"]), float(view["y"]), eye])

    alive = np.ones((height, width), bool)
    hit = np.zeros((height, width), bool)
    t = 0.5
    while t < far:
        step = 0.5 if t < 250 else 2.0
        p = loc[None, None, :] + d * t
        z = p[..., 2]
        alive &= ~(alive & (z > 255) & (d[..., 2] > 0))
        inb = alive & (z >= 0) & (z <= 255)
        xi = (p[..., 0].astype(np.int64)) % W
        yi = (p[..., 1].astype(np.int64)) % H
        s = inb & ((z < low[yi, xi]) | (dual[yi, xi] & (z >= mid[yi, xi]) & (z < high[yi, xi])))
        hit |= s
        alive &= ~s
        if not alive.any():
            break
        t += step
    return ~hit


def render(args, view, method, out_dir):
    label, terrain, extra, warmup = method
    tag = re.sub(r"[^A-Za-z0-9]+", "_", label)
    png = os.path.join(out_dir, f"{view['name']}-{tag}.png")
    depth = os.path.join(out_dir, f"{view['name']}-{tag}.f32")
    cmd = [
        args.binary, "--snapshot", png, "--depth-out", depth,
        "--terrain", terrain, *extra,
        "--fp", f"{view['x']},{view['y']}",
        "--fp-height", str(args.eye_height),
        "--fp-yaw", str(view["yaw"]),
        # The default near plane clips the ground out from under a
        # first-person camera; rasterized terrain then shows through.
        "--near", "1", "--far", str(args.far),
        "--width", str(args.width), "--height", str(args.height),
        "--frames", "3", "--warmup", str(warmup),
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
    m = re.search(r"avg=([0-9.]+)ms", res.stderr)
    return png, depth, float(m.group(1)) if m else float("nan")


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--binary", default="./target/release/level")
    ap.add_argument("--level-zip")
    ap.add_argument("--common-zip")
    ap.add_argument("--layers", required=True,
                    help="level.ron written by the `convert` binary")
    ap.add_argument("--out", required=True)
    ap.add_argument("--view", action="append", required=True,
                    help="name:x,y:yaw[:under], repeatable")
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
    views = [parse_view(v) for v in args.view]
    layers = load_layers(args.layers)

    cells, stats = {}, {}
    for view in views:
        sky = ground_truth(layers, view, args.width, args.height,
                           args.eye_height, args.far)
        print(f"{view['name']}: ground truth sky = {100 * sky.mean():.1f}% of frame")
        for method in METHODS:
            png, depth, ms = render(args, view, method, args.out)
            d = np.fromfile(depth, dtype="<f4").reshape(args.height, args.width)
            empty = d >= 0.999999          # cleared depth: nothing drawn
            see_through = 100 * (empty & ~sky).mean()
            covers_sky = 100 * ((~empty) & sky).mean()
            cells[(view["name"], method[0])] = png
            stats[(view["name"], method[0])] = (ms, see_through, covers_sky)
            print(f"    {method[0]:12s} {ms:7.1f} ms   "
                  f"see-through {see_through:5.1f}%   covers-sky {covers_sky:5.1f}%")

    # Grid image: rows are viewpoints, columns are methods.
    w, h, pad, lab, hdr = args.width, args.height, 5, 18, 30
    W = len(METHODS) * (w + pad) + pad
    H = hdr + len(views) * (h + lab + pad) + pad
    out = Image.new("RGB", (W, H), (20, 22, 26))
    dr = ImageDraw.Draw(out)
    try:
        font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", 15)
        small = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 11)
    except OSError:
        font = small = ImageFont.load_default()
    for j, method in enumerate(METHODS):
        dr.text((pad + j * (w + pad) + 2, 8), method[0], font=font, fill=(235, 238, 242))
    for i, view in enumerate(views):
        y = hdr + i * (h + lab + pad)
        dr.text((pad + 2, y + 2),
                f"{view['name']} at ({view['x']},{view['y']}) yaw {view['yaw']:g}"
                + (" (under)" if view["under"] else ""),
                font=small, fill=(150, 196, 255))
        for j, method in enumerate(METHODS):
            x = pad + j * (w + pad)
            out.paste(Image.open(cells[(view["name"], method[0])]).convert("RGB"), (x, y + lab))
            ms, err, _ = stats[(view["name"], method[0])]
            colour = (255, 120, 120) if err > 5 else ((255, 210, 130) if err > 0.5 else (150, 230, 150))
            dr.text((x + 5, y + lab + h - 16), f"{ms:.1f} ms", font=small, fill=(255, 220, 140))
            dr.text((x + 80, y + lab + h - 16), f"{err:.1f}% see-through", font=small, fill=colour)
    grid = os.path.join(args.out, "comparison.png")
    out.save(grid)
    print(f"\nwrote {grid}")


if __name__ == "__main__":
    main()
