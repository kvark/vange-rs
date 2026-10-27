#!/usr/bin/env python3
"""Build a review PDF and a matching LaTeX source archive from the working tree."""
import argparse
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", default="work/submission")
    parser.add_argument("--allow-dirty", action="store_true",
                        help="package the current edits for review")
    args = parser.parse_args()
    dirty = bool(subprocess.check_output(
        ["git", "status", "--porcelain"], cwd=ROOT, text=True).strip())
    if dirty and not args.allow_dirty:
        raise SystemExit("uncommitted changes; use --allow-dirty for a review package")
    destination = (ROOT / args.out).resolve()
    destination.mkdir(parents=True, exist_ok=True)
    subprocess.run(["python3", str(ROOT / "tools/build-paper-tex.py"),
                    "--out", str(destination / "vange-rs-paper.pdf")], check=True)
    package = destination / "source-package"
    subprocess.run(["python3", str(ROOT / "tools/pack-arxiv.py"),
                    "--out", str(package), "--without-video"], check=True)
    archive = destination / "vange-rs-paper-source.zip"
    shutil.copy2(package / "arxiv-source.zip", archive)
    revision = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    (destination / "source-revision.txt").write_text(
        f"Base revision: {revision}\n"
        f"Uncommitted working-tree changes included: {dirty}\n"
        "The PDF and LaTeX archive use the same working-tree source.\n"
        "The supplemental video is not included in this review package.\n")
    print(f"Wrote {archive}")

if __name__ == "__main__":
    main()
