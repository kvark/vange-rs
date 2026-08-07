#!/usr/bin/env python3
"""Plot, from above, what the mesh terrain's frustum test and LOD picker chose.

Reads the JSON that `level --cull-dump` writes. That file is read out of
the renderer after the frame, not recomputed here, so the picture cannot
drift from what was actually drawn.

Chunks are drawn once per wrap copy the plot window touches: outlined
where they were culled, filled by detail level where they survived. The
frustum is the footprint of its eight world-space corners.

Example
-------
    cargo run --release --bin level -- \\
        --level-zip fostral.zip --common-zip common.zip \\
        --terrain Mesh --fp 2006,1730 --fp-yaw 120 --near 1 --far 600 \\
        --snapshot view.png --cull-dump cull.json

    tools/plot-cull.py cull.json plan.png --view view.png
"""

import argparse
import json
import math

from PIL import Image, ImageDraw, ImageFont

BG = (18, 20, 24)
GRID = (60, 65, 76)
SEAM = (150, 110, 200)
RING = (96, 88, 120)
CULLED = (72, 78, 88)
FRUSTUM = (88, 132, 210)
CAMERA = (245, 245, 250)
TEXT = (200, 208, 218)
DIM = (130, 138, 150)
# Finest first. Deliberately not a red/green pair: the point is to read
# level *order* off the picture, so this ramps in one direction.
LOD_COLOURS = [(126, 217, 138), (232, 197, 92), (226, 122, 92), (198, 92, 176)]

SS = 3  # supersampling; PIL has no antialiasing of its own


def hull(points):
    """Convex hull, monotone chain. The frustum's eight corners project to
    a shape with interior points, and we want its silhouette."""
    pts = sorted(set(points))
    if len(pts) < 3:
        return pts

    def half(seq):
        out = []
        for p in seq:
            while len(out) >= 2:
                (ax, ay), (bx, by) = out[-2], out[-1]
                if (bx - ax) * (p[1] - ay) - (by - ay) * (p[0] - ax) > 0:
                    break
                out.pop()
            out.append(p)
        return out[:-1]

    return half(pts) + half(reversed(pts))


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("dump", help="JSON from `level --cull-dump`")
    ap.add_argument("out")
    ap.add_argument("--view", help="the first-person render, placed alongside")
    ap.add_argument("--size", type=int, default=760, help="plan size in pixels")
    ap.add_argument("--margin", type=float, default=180.0,
                    help="texels of padding around the drawn content")
    args = ap.parse_args()

    d = json.load(open(args.dump))
    cam = d["camera"]
    lw, lh = d["level_size"]
    chunks = d["chunks"]
    # chunk index -> (copy, lod) for everything that survived
    drawn = {(int(c), int(p)): int(l) for c, p, l, _ in d["draws"]}

    # The wrap copy grid the renderer used: a 3x3 around the camera's tile.
    cam_tile = (math.floor(cam[0] / lw), math.floor(cam[1] / lh))
    def offset(copy):
        return ((cam_tile[0] + copy % 3 - 1) * lw, (cam_tile[1] + copy // 3 - 1) * lh)

    # Window: everything drawn, plus the frustum footprint, plus margin.
    xs, ys = [cam[0]], [cam[1]]
    for c in d["frustum"]:
        xs.append(c[0])
        ys.append(c[1])
    for (ci, copy) in drawn:
        ox, oy = offset(copy)
        x0, y0, x1, y1 = chunks[ci]
        xs += [x0 + ox, x1 + ox]
        ys += [y0 + oy, y1 + oy]
    span = max(max(xs) - min(xs), max(ys) - min(ys)) + 2 * args.margin
    cx, cy = 0.5 * (max(xs) + min(xs)), 0.5 * (max(ys) + min(ys))
    lo = (cx - span / 2, cy - span / 2)

    S = args.size * SS
    scale = S / span
    # World Y grows the same way as image Y here: the level is stored that
    # way and the snapshots are taken that way, so north stays down.
    def to_px(x, y):
        return ((x - lo[0]) * scale, (y - lo[1]) * scale)

    img = Image.new("RGB", (S, S), BG)
    dr = ImageDraw.Draw(img, "RGBA")

    px0, py0 = to_px(cam[0], cam[1])

    # Chunks: every copy that reaches the window, culled ones first so the
    # drawn ones sit on top.
    def rect(ci, copy):
        ox, oy = offset(copy)
        x0, y0, x1, y1 = chunks[ci]
        return to_px(x0 + ox, y0 + oy) + to_px(x1 + ox, y1 + oy)

    for copy in range(9):
        for ci in range(len(chunks)):
            if (ci, copy) in drawn:
                continue
            a, b, c2, d2 = rect(ci, copy)
            if c2 < 0 or d2 < 0 or a > S or b > S:
                continue
            dr.rectangle([a, b, c2, d2], outline=CULLED, width=1 * SS)

    for (ci, copy), lod in sorted(drawn.items(), key=lambda kv: -kv[1]):
        col = LOD_COLOURS[min(lod, len(LOD_COLOURS) - 1)]
        a, b, c2, d2 = rect(ci, copy)
        dr.rectangle([a, b, c2, d2], fill=col + (78,), outline=col, width=2 * SS)

    # Distance rings at the LOD thresholds. The picker takes
    # `log2(dist / lod_distance)`, so each level starts at a doubling, and
    # the ring is where the fill colour is expected to change.
    for k in range(len(LOD_COLOURS)):
        rad = d["lod_distance"] * (2 ** k) * scale
        if rad > 2.2 * S:
            break
        dr.ellipse([px0 - rad, py0 - rad, px0 + rad, py0 + rad],
                   outline=RING + (170,), width=1 * SS)

    # Frustum footprint.
    poly = hull([to_px(c[0], c[1]) for c in d["frustum"]])
    if len(poly) >= 3:
        dr.polygon(poly, fill=FRUSTUM + (46,), outline=FRUSTUM + (235,), width=2 * SS)

    # The level's own edges, drawn over the chunks: where the mesh stops
    # covering the world once and starts repeating. Chunks on both sides of
    # one of these are the same chunks, drawn at different wrap offsets.
    for i in range(-2, 3):
        gx, _ = to_px((cam_tile[0] + i) * lw, 0)
        _, gy = to_px(0, (cam_tile[1] + i) * lh)
        dr.line([(gx, 0), (gx, S)], fill=SEAM + (210,), width=2 * SS)
        dr.line([(0, gy), (S, gy)], fill=SEAM + (210,), width=2 * SS)

    # Camera, with a stub for the view direction.
    px, py = px0, py0
    r = 5 * SS
    dr.ellipse([px - r, py - r, px + r, py + r], fill=CAMERA)
    dx, dy = d["dir"][0], d["dir"][1]
    n = math.hypot(dx, dy) or 1.0
    dr.line([(px, py), (px + dx / n * 26 * SS, py + dy / n * 26 * SS)],
            fill=CAMERA, width=3 * SS)

    plan = img.resize((args.size, args.size), Image.LANCZOS)

    # Labels go on after the downscale so the text stays crisp.
    dr = ImageDraw.Draw(plan, "RGBA")
    try:
        f = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", 14)
        fs = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 12)
    except OSError:
        f = fs = ImageFont.load_default()

    per_lod = {}
    for lod in drawn.values():
        per_lod[lod] = per_lod.get(lod, 0) + 1

    # Keep the legend out of the camera's corner - that is where the
    # frustum apex is, and it is the part you most want to see.
    rows = 5 + len(set(drawn.values()))
    lw_box, lh_box = 232, 24 + 17 * rows
    lx = 0 if px0 / SS > args.size / 2 else args.size - lw_box
    lx = max(0, min(args.size - lw_box, lx))
    dr.rectangle([lx, 0, lx + lw_box, lh_box], fill=BG + (215,))
    y = 10
    dr.text((lx + 10, y), "chunks drawn, from above", font=f, fill=TEXT)
    y += 20
    for lod in sorted(per_lod):
        col = LOD_COLOURS[min(lod, len(LOD_COLOURS) - 1)]
        dr.rectangle([lx + 10, y + 2, lx + 22, y + 12], fill=col + (110,), outline=col)
        dr.text((lx + 28, y), f"LOD {lod}  x{per_lod[lod]}", font=fs, fill=TEXT)
        y += 17
    dr.rectangle([lx + 10, y + 2, lx + 22, y + 12], outline=CULLED)
    culled = len(chunks) * 9 - len(drawn)
    dr.text((lx + 28, y), f"culled  x{culled}", font=fs, fill=DIM)
    y += 17
    dr.rectangle([lx + 10, y + 2, lx + 22, y + 12], fill=FRUSTUM + (60,), outline=FRUSTUM)
    dr.text((lx + 28, y), "frustum", font=fs, fill=DIM)
    y += 17
    dr.ellipse([lx + 12, y + 3, lx + 20, y + 11], outline=RING)
    dr.text((lx + 28, y), "LOD thresholds", font=fs, fill=DIM)
    y += 17
    dr.line([lx + 10, y + 7, lx + 22, y + 7], fill=SEAM, width=2)
    dr.text((lx + 28, y), "level edge (wrap seam)", font=fs, fill=DIM)

    copies = len({p for _, p in drawn})
    foot = (f"{len(drawn)} of {len(chunks) * 9} drawn, over {copies} wrap "
            f"{'copy' if copies == 1 else 'copies'}   "
            f"lod_distance {d['lod_distance']:g}   "
            f"culling {'on' if d['culling'] else 'off'}   "
            f"{span:.0f} texels across")
    dr.text((10, args.size - 18), foot, font=fs, fill=DIM)

    if not args.view:
        plan.save(args.out)
        print(f"wrote {args.out}")
        return

    view = Image.open(args.view).convert("RGB")
    vh = args.size
    vw = int(view.width * vh / view.height)
    view = view.resize((vw, vh), Image.LANCZOS)
    pad, hdr = 8, 24
    out = Image.new("RGB", (vw + args.size + 3 * pad, vh + hdr + pad), BG)
    d2 = ImageDraw.Draw(out)
    d2.text((pad, 5), "what the camera sees", font=f, fill=TEXT)
    d2.text((vw + 2 * pad, 5), "what was submitted", font=f, fill=TEXT)
    out.paste(view, (pad, hdr))
    out.paste(plan, (vw + 2 * pad, hdr))
    out.save(args.out)
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
