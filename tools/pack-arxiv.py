#!/usr/bin/env python3
"""Assemble the arXiv source zip: pdfpages wrapper + paper PDF + video."""

import argparse
import pathlib
import shutil
import subprocess
import zipfile


ROOT = pathlib.Path(__file__).resolve().parents[1]


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
    metadata = (ROOT / "paper" / "arxiv-metadata.txt").read_text()
    (destination / "metadata.txt").write_text(metadata)

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
