#!/usr/bin/env python3
"""Compile paper/paper.tex with pdflatex + bibtex."""

import argparse
import pathlib
import subprocess


ROOT = pathlib.Path(__file__).resolve().parents[1]


def run(command, cwd):
    print("+", " ".join(str(part) for part in command), flush=True)
    subprocess.run(command, cwd=cwd, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", default="work/submission/vange-rs-paper.pdf")
    args = parser.parse_args()
    paper = ROOT / "paper"
    if not (paper / "paper.tex").is_file():
        raise SystemExit("missing paper/paper.tex")
    for _ in range(2):
        run(["pdflatex", "-interaction=nonstopmode", "paper.tex"], paper)
    run(["bibtex", "paper"], paper)
    for _ in range(2):
        run(["pdflatex", "-interaction=nonstopmode", "paper.tex"], paper)
    destination = ROOT / args.out
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes((paper / "paper.pdf").read_bytes())
    print(f"wrote {destination} ({destination.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
