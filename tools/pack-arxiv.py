#!/usr/bin/env python3
"""Assemble the arXiv source zip: pdfpages wrapper + paper PDF + video."""

import argparse
import pathlib
import shutil
import subprocess
import zipfile


ROOT = pathlib.Path(__file__).resolve().parents[1]
METADATA = """Title:
  Six Ways to Draw Vangers with WebGPU: Real-Time Rendering of Editable Multi-Layer Height Fields

Authors:
  Dzmitry Malyshau

Comments:
  22 pages. Submitted to the Journal of Computer Graphics Techniques. Supplemental video as ancillary.

Report no:
  (leave blank)

MSC class:
  (leave blank)

ACM class:
  I.3.7

Journal-ref:
  (leave blank until JCGT publishes)

DOI:
  (leave blank until JCGT publishes)

License:
  arXiv.org perpetual, non-exclusive license to distribute (nonexclusive-distrib 1.0)

Primary category:
  cs.GR

Cross-lists:
  (none)

Abstract:
Terrain level-of-detail is measured almost exclusively on digital elevation models: single-valued, smooth at the sampling scale, sampled from real topography. Game terrain is often none of these. We compare six rendering methods -- height-field ray marching, voxel-accelerated ray marching, sliced proxy geometry, per-sample bar rasterization, compute scattering, and a fitted triangle mesh -- implemented in a single engine over a single data path, on the hand-authored multi-layer terrain of Vangers (1998), scored against a CPU ray cast of the same source data. Every method must preserve the two solid intervals available at a ground sample, render at interactive rates, and reflect local terrain destruction without reloading the level. These constraints rule out treating caves as decoration or amortising a static preprocessing step over an immutable map.

From the original game's top-down camera the six methods look interchangeable. At eye-level horizons they do not: point scattering loses coverage, slicing bands, and an over-simplified mesh can miss a wall. At the selected quality settings a greedy triangulated irregular network (TIN) has the lowest mean frame time on every device we measured, but the fit cost is set by the second layer rather than by floor relief, and making that mesh editable retains 319 MiB of GPU geometry and 535 MiB of CPU triangulation. All six implementations use the same native wgpu / WebGPU API and canonical WGSL. We release the engine, the harness, and a one-command measurement protocol.
"""


def pdf_pages(path):
    info = subprocess.check_output(["pdfinfo", str(path)], text=True)
    for line in info.splitlines():
        if line.startswith("Pages:"):
            return int(line.split(":", 1)[1])
    raise SystemExit(f"could not read page count from {path}")


def wrapper_tex(pages):
    ships = "\n".join(
        "\\shipout\\hbox{%\n"
        f"  \\includegraphics[width=8.5in,height=11in,page={n}]{{paper-body.pdf}}"
        "}%"
        for n in range(1, pages + 1)
    )
    return (
        "\\pdfoutput=1\n"
        "\\documentclass{minimal}\n"
        "\\usepackage{graphicx}\n"
        "\\pdfpagewidth=8.5in\n"
        "\\pdfpageheight=11in\n"
        "\\begin{document}\n"
        f"{ships}\n"
        "\\end{document}\n"
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", default="work/arxiv")
    parser.add_argument("--pdf", default="work/submission/vange-rs-paper.pdf")
    parser.add_argument("--video", default="work/paper-video/terrain-methods.mp4")
    args = parser.parse_args()

    destination = (ROOT / args.out).resolve()
    source = destination / "source"
    if source.exists():
        shutil.rmtree(source)
    (source / "anc").mkdir(parents=True)

    pdf = ROOT / args.pdf
    video = ROOT / args.video
    if not pdf.is_file():
        raise SystemExit(f"missing paper PDF: {pdf}")
    if not video.is_file():
        raise SystemExit(f"missing supplemental video: {video}")

    shutil.copy2(pdf, source / "paper-body.pdf")
    shutil.copy2(video, source / "anc" / "terrain-methods.mp4")
    (source / "ms.tex").write_text(wrapper_tex(pdf_pages(pdf)))
    (destination / "metadata.txt").write_text(METADATA)

    archive = destination / "arxiv-source.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zf:
        for path in sorted(source.rglob("*")):
            if path.is_file():
                zf.write(path, path.relative_to(source).as_posix())

    check = destination / "compile-check"
    if check.exists():
        shutil.rmtree(check)
    shutil.copytree(source, check)
    subprocess.run(
        ["pdflatex", "-interaction=nonstopmode", "ms.tex"],
        cwd=check, check=True, stdout=subprocess.DEVNULL,
    )
    pages = subprocess.check_output(
        ["pdfinfo", str(check / "ms.pdf")], text=True)
    print(f"wrote {archive} ({archive.stat().st_size} bytes)")
    print(f"wrote {destination / 'metadata.txt'}")
    print(pages.strip())


if __name__ == "__main__":
    main()
