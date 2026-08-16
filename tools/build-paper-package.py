#!/usr/bin/env python3
"""Build the internal review PDF and a revision-pinned source archive."""

import argparse
import os
import pathlib
import shutil
import subprocess
import tarfile


ROOT = pathlib.Path(__file__).resolve().parents[1]


def run(command, cwd=ROOT):
    print("+", " ".join(str(part) for part in command), flush=True)
    subprocess.run(command, cwd=cwd, check=True)


def output(command):
    return subprocess.check_output(command, cwd=ROOT, text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", default="work/submission")
    parser.add_argument("--allow-dirty", action="store_true",
                        help="build an editorial preview not tied to HEAD")
    args = parser.parse_args()

    dirty = bool(output(["git", "status", "--porcelain"]))
    if dirty and not args.allow_dirty:
        raise SystemExit("refusing to package a dirty checkout; commit first or "
                         "use --allow-dirty for an internal preview")
    for command in ("pandoc", "weasyprint"):
        if shutil.which(command) is None:
            raise SystemExit(f"{command} is required")

    destination = (ROOT / args.out).resolve()
    destination.mkdir(parents=True, exist_ok=True)
    html = destination / "vange-rs-paper.html"
    pdf = destination / "vange-rs-paper.pdf"
    run([
        "pandoc", "draft.md", "--from=gfm", "--standalone",
        "--embed-resources", "--resource-path=.:..",
        "--css=review.css", "--metadata", "lang=en",
        "--metadata", "pagetitle=Six Ways to Draw Vangers with WebGPU",
        "--output", html,
    ], ROOT / "paper")
    run(["weasyprint", html, pdf])

    revision = output(["git", "rev-parse", "--short=12", "HEAD"])
    archive = destination / f"vange-rs-paper-source-{revision}.tar.gz"
    run([
        "git", "archive", "--format=tar.gz",
        f"--prefix=vange-rs-paper-{revision}/", "--output", archive, "HEAD",
    ])
    with tarfile.open(archive, "r:gz") as source:
        names = source.getnames()
    expected = [
        f"vange-rs-paper-{revision}/paper/draft.md",
        f"vange-rs-paper-{revision}/paper/references.bib",
        f"vange-rs-paper-{revision}/tools/compare-terrain.py",
    ]
    missing = [name for name in expected if name not in names]
    if missing:
        raise SystemExit("source archive is missing: " + ", ".join(missing))
    print(f"wrote {pdf} ({pdf.stat().st_size} bytes)")
    print(f"wrote {archive} ({len(names)} paths)")
    if dirty:
        print("warning: PDF reflects the working tree; source archive reflects HEAD")


if __name__ == "__main__":
    main()
