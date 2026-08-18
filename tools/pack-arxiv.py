#!/usr/bin/env python3
"""Assemble the arXiv source zip from the JCGT LaTeX tree plus the video."""

import argparse
import pathlib
import shutil
import zipfile


ROOT = pathlib.Path(__file__).resolve().parents[1]
PAPER = ROOT / "paper"

INCLUDE = [
    "paper.tex",
    "references.bib",
    "paper.bbl",
    "jcgt.cls",
    "jcgt.bst",
    "ORCIDlogo.pdf",
    "CC-BY-ND.png",
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", default="work/arxiv")
    parser.add_argument("--video", default="work/paper-video/terrain-methods.mp4")
    args = parser.parse_args()

    destination = (ROOT / args.out).resolve()
    source = destination / "source"
    if source.exists():
        shutil.rmtree(source)
    (source / "anc").mkdir(parents=True)
    (source / "figures").mkdir()

    for name in INCLUDE:
        path = PAPER / name
        if not path.is_file():
            raise SystemExit(f"missing {path}; compile paper/paper.tex first")
        shutil.copy2(path, source / name)

    for pdf in sorted((PAPER / "figures").glob("*.pdf")):
        shutil.copy2(pdf, source / "figures" / pdf.name)
    original = PAPER / "figures" / "original.jpg"
    if not original.is_file():
        raise SystemExit("missing paper/figures/original.jpg")
    shutil.copy2(original, source / "figures" / "original.jpg")

    video = ROOT / args.video
    if not video.is_file():
        raise SystemExit(f"missing supplemental video: {video}")
    shutil.copy2(video, source / "anc" / "terrain-methods.mp4")

    metadata = PAPER / "arxiv-metadata.txt"
    if metadata.is_file():
        shutil.copy2(metadata, destination / "metadata.txt")

    archive = destination / "arxiv-source.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zf:
        for path in sorted(source.rglob("*")):
            if path.is_file():
                zf.write(path, path.relative_to(source).as_posix())

    print(f"wrote {archive} ({archive.stat().st_size} bytes)")
    print("contents:")
    with zipfile.ZipFile(archive) as zf:
        for info in zf.infolist():
            print(f"  {info.file_size:10d}  {info.filename}")


if __name__ == "__main__":
    main()
