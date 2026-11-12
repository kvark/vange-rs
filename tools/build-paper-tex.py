#!/usr/bin/env python3
"""Compile the JCGT manuscript and reject unresolved references or overflow."""
import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", default="work/submission/vange-rs-paper.pdf")
    parser.add_argument("--figures", action="store_true",
                        help="regenerate the print-sized manuscript figures")
    args = parser.parse_args()
    paper = ROOT / "paper"
    build = ROOT / "work/paper-build"
    build.mkdir(parents=True, exist_ok=True)
    # Start from source even if an interrupted earlier run left partial auxiliaries.
    for suffix in ("aux", "bbl", "blg", "brf", "log", "out", "pdf", "toc"):
        (build / f"paper.{suffix}").unlink(missing_ok=True)
    if args.figures:
        subprocess.run(["python3", str(ROOT / "tools/draw-paper-figures.py")], check=True)
    for command in ("pdflatex", "bibtex"):
        if shutil.which(command) is None:
            raise SystemExit(f"{command} is required")
    env = os.environ.copy()
    env["BIBINPUTS"] = str(paper) + os.pathsep + env.get("BIBINPUTS", "")
    env["BSTINPUTS"] = str(paper) + os.pathsep + env.get("BSTINPUTS", "")
    latex = ["pdflatex", "-interaction=nonstopmode", "-halt-on-error",
             "-file-line-error", "-no-shell-escape",
             f"-output-directory={build}", "paper.tex"]
    for index, command in enumerate((latex, ["bibtex", "paper"], latex, latex, latex)):
        cwd = build if command[0] == "bibtex" else paper
        result = subprocess.run(command, cwd=cwd, env=env, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        transcript = build / f"pass-{index+1}.txt"
        transcript.write_text(result.stdout)
        if result.returncode:
            print(result.stdout[-8000:])
            raise SystemExit(f"{command[0]} failed; see {transcript}")
    log = (build / "paper.log").read_text(errors="replace")
    problems = re.findall(
        r"^.*(?:Overfull \\[hv]box|undefined references|"
        r"Citation .* undefined|Reference .* undefined|"
        r"Label .* multiply defined|Rerun to get cross-references right).*$",
        log, re.MULTILINE)
    if problems:
        raise SystemExit("Manuscript checks failed:\n" + "\n".join(problems))
    destination = (ROOT / args.out).resolve()
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_suffix(".tmp.pdf")
    shutil.copy2(build / "paper.pdf", temporary)
    temporary.replace(destination)
    # Keep the supplied bibliography synchronized with the source for packaging.
    shutil.copy2(build / "paper.bbl", paper / "paper.bbl")
    print(f"Wrote {destination} ({destination.stat().st_size} bytes)")
    print("No unresolved references, citations, or overfull boxes.")

if __name__ == "__main__":
    main()
