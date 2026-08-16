#!/usr/bin/env python3
"""Render a synchronized mosaic flythrough of all publication methods.

The level binary renders one numbered PNG sequence per method while moving a
first-person camera horizontally in its viewing direction at constant world
altitude. ffmpeg labels and assembles the seven sequences into a single H.264
mosaic. Output stays under ``work/`` until the terrain-image license permits
publication.
"""

import argparse
import importlib.util
import os
import shutil
import subprocess
import sys
from pathlib import Path
from types import SimpleNamespace


ROOT = Path(__file__).resolve().parents[1]


def load_compare_module():
    path = ROOT / "tools" / "compare-terrain.py"
    spec = importlib.util.spec_from_file_location("compare_terrain", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run(command, description, env=None):
    print(f"{description}...", flush=True)
    result = subprocess.run(command, cwd=ROOT, env=env)
    if result.returncode:
        raise SystemExit(f"{description} failed with status {result.returncode}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", default="work/paper-video/terrain-methods.mp4")
    parser.add_argument("--start", default="837,3984",
                        help="starting first-person X,Y")
    parser.add_argument("--distance", type=float, default=800.0,
                        help="horizontal travel distance in terrain units")
    parser.add_argument("--yaw", type=float, default=299.0)
    parser.add_argument("--pitch", type=float, default=-30.0)
    parser.add_argument("--eye-height", type=float, default=78.0)
    parser.add_argument("--duration", type=float, default=8.0)
    parser.add_argument("--fps", type=int, default=30)
    parser.add_argument("--width", type=int, default=640)
    parser.add_argument("--height", type=int, default=400)
    parser.add_argument("--work", default="work")
    parser.add_argument("--keep-frames", action="store_true")
    args = parser.parse_args()

    frame_count = max(2, round(args.duration * args.fps))
    output = (ROOT / args.output).resolve()
    video_work = output.parent
    frames_root = video_work / "frames"
    if frames_root.exists():
        shutil.rmtree(frames_root)
    frames_root.mkdir(parents=True)

    compare = load_compare_module()
    assets = SimpleNamespace(
        work=str((ROOT / args.work).resolve()),
        level="fostral",
        level_zip=None,
        common_zip=None,
        layers=None,
        out=None,
    )
    compare.ensure_tools(assets)
    compare.ensure_assets(assets)

    env = dict(os.environ, RUST_LOG="warn")
    inputs = []
    labels = []
    for label, terrain, extra, warmup in compare.METHODS:
        slug = compare.slug(label)
        frame_dir = frames_root / slug
        frame_dir.mkdir()
        command = [
            str(ROOT / "target" / "release" / "level"),
            "--snapshot", str(video_work / f"{slug}-last.png"),
            "--frame-dir", str(frame_dir),
            "--terrain", terrain, *extra,
            "--fp", args.start,
            "--fp-travel", str(args.distance),
            "--fp-height", str(args.eye_height),
            f"--fp-yaw={args.yaw}", f"--fp-pitch={args.pitch}",
            "--near", "1", "--far", "600",
            "--width", str(args.width), "--height", str(args.height),
            "--frames", str(frame_count), "--warmup", str(warmup),
            "--shadow-ray",
            "--level-zip", assets.level_zip,
            "--common-zip", assets.common_zip,
        ]
        run(command, f"rendering {label}", env)
        inputs += ["-framerate", str(args.fps), "-i", str(frame_dir / "%06d.png")]
        labels.append(label)

    filters = []
    for index, label in enumerate(labels):
        escaped = label.replace("'", r"\'").replace(":", r"\:")
        filters.append(
            f"[{index}:v]scale=480:300:flags=lanczos,"
            "drawbox=x=0:y=0:w=iw:h=38:color=black@0.62:t=fill,"
            f"drawtext=text='{escaped}':fontcolor=white:fontsize=24:x=12:y=8[v{index}]"
        )
    layout = "0_0|480_0|960_0|0_300|480_300|960_300|480_600"
    filters.append(
        "".join(f"[v{index}]" for index in range(len(labels)))
        + f"xstack=inputs={len(labels)}:layout={layout}:fill=0x111318[v]"
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    ffmpeg = [
        "ffmpeg", "-hide_banner", "-loglevel", "warning", "-y", *inputs,
        "-filter_complex", ";".join(filters),
        "-map", "[v]", "-frames:v", str(frame_count),
        "-c:v", "libx264", "-preset", "medium", "-crf", "18",
        "-pix_fmt", "yuv420p", "-movflags", "+faststart",
        "-metadata", "title=Six Ways to Draw Vangers with WebGPU",
        str(output),
    ]
    run(ffmpeg, "encoding mosaic")
    poster = output.with_suffix(".png")
    run([
        "ffmpeg", "-hide_banner", "-loglevel", "warning", "-y",
        "-ss", f"{args.duration * 0.5:.3f}", "-i", str(output),
        "-frames:v", "1", "-update", "1", str(poster),
    ], "extracting poster frame")
    if not args.keep_frames:
        shutil.rmtree(frames_root)
    print(f"wrote {output}")
    print(f"wrote {poster}")


if __name__ == "__main__":
    main()
