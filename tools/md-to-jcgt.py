#!/usr/bin/env python3
"""One-shot Markdown -> JCGT LaTeX conversion. paper.tex is the source after this."""

import pathlib
import re
import subprocess


ROOT = pathlib.Path(__file__).resolve().parents[1]
PAPER = ROOT / "paper"

CITES = [
    (r"Benes and Forsbach \[2001\]", r"CITET{benes2001layered}"),
    (r"Grounded Heightmap\s+Trees \[Alonso and Joan-Arinyo 2008\]",
     r"Grounded Heightmap Trees CITEP{alonso2008grounded}"),
    (r"QuadStack \[Graciano et al\. 2021\]", r"QuadStack CITEP{graciano2021quadstack}"),
    (r"\[Shade et al\.\s*1998\]", r"CITEP{shade1998ldi}"),
    (r"\[Policarpo et al\. 2005\]", r"CITEP{policarpo2005relief}"),
    (r"\[Mantler and Jeschke 2006\]", r"CITEP{mantler2006landscape}"),
    (r"\[Tevs et al\. 2008\]", r"CITEP{tevs2008maximum}"),
    (r"\[Laine and Karras 2010\]", r"CITEP{laine2010svo}"),
    (r"\[Cabral et al\. 1994\]", r"CITEP{cabral1994volume}"),
    (r"\[Westover 1990; Zwicker et al\. 2001\]",
     r"CITEP{westover1990footprint,zwicker2001splatting}"),
    (r"\[Fowler and Little 1979; Garland and Heckbert 1995;\s*Duchaineau et al\. 1997; Losasso and Hoppe 2004\]",
     r"CITEP{fowler1979tin,garland1995terrain,duchaineau1997roam,losasso2004clipmaps}"),
    (r"Garland and Heckbert \[1995\]", r"CITET{garland1995terrain}"),
    (r"\[Malyshau 2020; gfx-rs; W3C WebGPU; W3C WGSL\]",
     r"CITEP{malyshau2020webgpu,wgpu,w3cwebgpu,w3cwgsl}"),
    (r"\[Malyshau 2021\]", r"CITEP{malyshau2021rust}"),
    (r"\[K-D Lab / KranX\]", r"CITEP{vangerssource}"),
]

HEADING_STRIP = [
    (r"^## 1\. Introduction\s*$", "# Introduction"),
    (r"^### 1\.1 Related work and scope\s*$", "## Related work and scope"),
    (r"^### 1\.2 A decade-long WebGPU testbed\s*$", "## A decade-long WebGPU testbed"),
    (r"^## 2\. The Data\s*$", "# The Data"),
    (r"^## 3\. Methods Compared\s*$", "# Methods Compared"),
    (r"^### 3\.1 Height-field ray march\s*$", "## Height-field ray march"),
    (r"^### 3\.2 Voxel-accelerated ray march\s*$", "## Voxel-accelerated ray march"),
    (r"^### 3\.3 Sliced\s*$", "## Sliced"),
    (r"^### 3\.4 Painted\s*$", "## Painted"),
    (r"^### 3\.5 Scattered\s*$", "## Scattered"),
    (r"^### 3\.6 Mesh\s*$", "## Mesh"),
    (r"^## 4\. Evaluation\s*$", "# Evaluation"),
    (r"^### 4\.1 Reference\s*$", "## Reference"),
    (r"^### 4\.2 Metrics\s*$", "## Metrics"),
    (r"^### 4\.3 Protocol\s*$", "## Protocol"),
    (r"^### 4\.4 Dynamic-edit protocol\s*$", "## Dynamic-edit protocol"),
    (r"^## 5\. Results\s*$", "# Results"),
    (r"^### 5\.1 Pitch is the axis that separates them\s*$",
     "## Pitch is the axis that separates them"),
    (r"^### 5\.2 Frame time\s*$", "## Frame time"),
    (r"^### 5\.3 Preparation cost\s*$", "## Preparation cost"),
    (r"^### 5\.4 Editing and retained method data\s*$",
     "## Editing and retained method data"),
    (r"^### 5\.5 Fit cost\s*$", "## Fit cost"),
    (r"^### 5\.6 Tuning\s*$", "## Tuning"),
    (r"^### 5\.7 Fastest is not free\s*$", "## Fastest is not free"),
    (r"^## 6\. Findings\s*$", "# Findings"),
    (r"^### 6\.1 The remaining depth offset\s*$", "## The remaining depth offset"),
    (r"^### 6\.2 The multi-layer encoding, not the terrain, sets the fit cost\s*$",
     "## The multi-layer encoding, not the terrain, sets the fit cost"),
    (r"^### 6\.3 A quarter of the vertex budget goes to one discontinuity\s*$",
     "## A quarter of the vertex budget goes to one discontinuity"),
    (r"^### 6\.4 Coverage metrics cannot see over-drawing\s*$",
     "## Coverage metrics cannot see over-drawing"),
    (r"^## 7\. Limitations\s*$", "# Limitations"),
    (r"^## 8\. Conclusion\s*$", "# Conclusion"),
]


PREAMBLE = r"""\documentclass{jcgt}

\setciteauthor{Malyshau}
\setcitetitle{Six Ways to Draw Vangers with WebGPU: Real-Time Rendering of Editable Multi-Layer Height Fields}
\setheadtitle{Six Ways to Draw Vangers with WebGPU}
\submitted{2026-08-17}

\usepackage{longtable}
\usepackage{booktabs}
\usepackage{array}
\providecommand{\tightlist}{\setlength{\itemsep}{0pt}\setlength{\parskip}{0pt}}
\providecommand{\pandocbounded}[1]{#1}
\providecommand{\passthrough}[1]{#1}

\begin{document}

\title{Six Ways to Draw Vangers with WebGPU:\\Real-Time Rendering of Editable Multi-Layer Height Fields}

\author{Dzmitry Malyshau~\href{https://orcid.org/0009-0005-6410-4276}{\includegraphics[width=8pt]{ORCIDlogo}}\\Independent Researcher}

\teaser{
  \includegraphics[width=\columnwidth]{figures/teaser.pdf}
  \caption{The six selected methods at the hangar horizon scene. Scattered loses coverage; slicing shows grazing bands; the selected mesh keeps the wall.}
  \label{fig:teaser}
}

\maketitle

\begin{abstract}
\small
BODY_ABSTRACT
\end{abstract}

"""

POSTAMBLE = r"""
\subsection*{Acknowledgements}

I thank Association K-D Lab for \textit{Vangers} and for publishing the Fostral
world data that this comparison uses; Yury Zhuravlev for maintaining the
open-source Vangers tree; and the players and other maintainers who have
kept the game alive for nearly three decades.

Large language models --- OpenAI Codex, Anthropic Claude, and xAI Grok ---
assisted with drafting, editing, literature search, and work on the
evaluation harness. I reviewed every claim, number, and citation; the
remaining errors are mine.

\small
\bibliographystyle{jcgt}
\bibliography{references}

\section*{Index of Supplemental Materials}
The synchronized six-method flythrough is \texttt{anc/terrain-methods.mp4},
rendered by \texttt{tools/render-paper-video.py} from the $-30^\circ$ portal
camera at $(1176, 11567)$, eye height 180, yaw $308^\circ$, 520 world units
at fixed altitude. The engine, harness, and measurement protocol are in the
accompanying source repository (\href{https://github.com/kvark/vange-rs}{https://github.com/kvark/vange-rs}),
tag \texttt{terrain-paper}. Fostral world data is not redistributed; the
harness fetches Association K-D Lab's CC~BY-SA~4.0 tree at commit
\texttt{f1ad7d7}. Figure-construction commands are recorded in
\texttt{paper/README.md}.

\section*{Author Contact Information}

Dzmitry Malyshau\\
Independent Researcher\\
\href{mailto:kvark@fastmail.com}{kvark@fastmail.com}\\
ORCID \href{https://orcid.org/0009-0005-6410-4276}{0009-0005-6410-4276}

\afterdoc

\end{document}
"""


def strip_front_and_back(text):
    text = re.sub(r"^# .*?\n## 1\. Introduction\n", "# Introduction\n", text, count=1, flags=re.S)
    text = re.sub(r"\n## Acknowledgements\n.*", "\n", text, flags=re.S)
    return text


def apply_headings(text):
    lines = []
    for line in text.splitlines():
        replaced = None
        for pat, repl in HEADING_STRIP:
            if re.match(pat, line):
                replaced = repl
                break
        if replaced is None:
            lines.append(line)
        elif replaced != "":
            lines.append(replaced)
    return "\n".join(lines) + "\n"


def apply_cites(text):
    for pat, repl in CITES:
        text = re.sub(pat, repl, text)
    text = re.sub(r"§(\d+(?:\.\d+)?)", r"SECREF{\1}", text)
    return text


def extract_abstract(text):
    m = re.search(r"## Abstract\n\n(.*?)\n\n!\[", text, re.S)
    if not m:
        raise SystemExit("abstract not found")
    abs_md = m.group(1).strip()
    # convert emphasis and dashes for latex later via pandoc snippet
    return abs_md


def pandoc_fragment(md, extra=None):
    cmd = [
        "pandoc", "--from=gfm", "--to=latex", "--wrap=none",
        "--listings",
    ]
    if extra:
        cmd.extend(extra)
    result = subprocess.run(cmd, input=md, text=True, capture_output=True, check=True)
    return result.stdout


def postprocess_latex(body):
    body = body.replace("figures/", "figures/")
    body = re.sub(
        r"\\(?:includegraphics|includesvg)(?:\[[^\]]*\])?\{figures/([^.}]+)\.svg\}",
        r"\\includegraphics[width=\\columnwidth]{figures/\1.pdf}",
        body,
    )
    body = body.replace("../docs/assets/original.jpg", "figures/original.jpg")
    body = re.sub(
        r"\\(?:includegraphics|includesvg)(?:\[[^\]]*\])?\{(?:\.\./docs/assets/)?(?:figures/)?original\.jpg\}",
        r"\\includegraphics[width=\\columnwidth]{figures/original.jpg}",
        body,
    )
    # drop the teaser image (already in \teaser)
    body = re.sub(
        r"\\begin\{figure\}\n\\centering\n\\includegraphics\[width=\\columnwidth\]\{figures/teaser\.pdf\}\n\\caption\{.*?\}\n\\end\{figure\}\n*",
        "",
        body,
        count=1,
        flags=re.S,
    )
    # section labels
    labels = {
        r"\\section\{Introduction\}": r"\\section{Introduction}\\label{sec:introduction}",
        r"\\subsection\{Related work and scope\}": r"\\subsection{Related work and scope}\\label{sec:related}",
        r"\\subsection\{A decade-long WebGPU testbed\}": r"\\subsection{A decade-long WebGPU testbed}\\label{sec:webgpu}",
        r"\\section\{The Data\}": r"\\section{The Data}\\label{sec:data}",
        r"\\section\{Methods Compared\}": r"\\section{Methods Compared}\\label{sec:methods}",
        r"\\subsection\{Height-field ray march\}": r"\\subsection{Height-field ray march}\\label{sec:ray}",
        r"\\subsection\{Voxel-accelerated ray march\}": r"\\subsection{Voxel-accelerated ray march}\\label{sec:voxel}",
        r"\\subsection\{Sliced\}": r"\\subsection{Sliced}\\label{sec:sliced}",
        r"\\subsection\{Painted\}": r"\\subsection{Painted}\\label{sec:painted}",
        r"\\subsection\{Scattered\}": r"\\subsection{Scattered}\\label{sec:scattered}",
        r"\\subsection\{Mesh\}": r"\\subsection{Mesh}\\label{sec:mesh}",
        r"\\section\{Evaluation\}": r"\\section{Evaluation}\\label{sec:eval}",
        r"\\subsection\{Reference\}": r"\\subsection{Reference}\\label{sec:reference}",
        r"\\subsection\{Metrics\}": r"\\subsection{Metrics}\\label{sec:metrics}",
        r"\\subsection\{Protocol\}": r"\\subsection{Protocol}\\label{sec:protocol}",
        r"\\subsection\{Dynamic-edit protocol\}": r"\\subsection{Dynamic-edit protocol}\\label{sec:edit-protocol}",
        r"\\section\{Results\}": r"\\section{Results}\\label{sec:results}",
        r"\\subsection\{Pitch is the axis that separates them\}":
            r"\\subsection{Pitch is the axis that separates them}\\label{sec:pitch}",
        r"\\subsection\{Frame time\}": r"\\subsection{Frame time}\\label{sec:timing}",
        r"\\subsection\{Preparation cost\}": r"\\subsection{Preparation cost}\\label{sec:prep}",
        r"\\subsection\{Editing and retained method data\}":
            r"\\subsection{Editing and retained method data}\\label{sec:edit}",
        r"\\subsection\{Fit cost\}": r"\\subsection{Fit cost}\\label{sec:fit}",
        r"\\subsection\{Tuning\}": r"\\subsection{Tuning}\\label{sec:tuning}",
        r"\\subsection\{Fastest is not free\}": r"\\subsection{Fastest is not free}\\label{sec:choice}",
        r"\\section\{Findings\}": r"\\section{Findings}\\label{sec:findings}",
        r"\\subsection\{The remaining depth offset\}":
            r"\\subsection{The remaining depth offset}\\label{sec:offset}",
        r"\\subsection\{The multi-layer encoding, not the terrain, sets the fit cost\}":
            r"\\subsection{The multi-layer encoding, not the terrain, sets the fit cost}\\label{sec:survey}",
        r"\\subsection\{A quarter of the vertex budget goes to one discontinuity\}":
            r"\\subsection{A quarter of the vertex budget goes to one discontinuity}\\label{sec:boundary}",
        r"\\subsection\{Coverage metrics cannot see over-drawing\}":
            r"\\subsection{Coverage metrics cannot see over-drawing}\\label{sec:overdraw}",
        r"\\section\{Limitations\}": r"\\section{Limitations}\\label{sec:limits}",
        r"\\section\{Conclusion\}": r"\\section{Conclusion}\\label{sec:conclusion}",
        r"\\section\{Acknowledgements\}": r"\\section*{Acknowledgements}",
        r"\\section\{Figure provenance\}": r"\\section*{Figure provenance}",
    }
    for pat, repl in labels.items():
        body = re.sub(pat, repl, body)
    # section refs from §
    refmap = {
        "1": "sec:introduction",
        "6": "sec:findings",
        "3.5": "sec:scattered",
        "3.1": "sec:ray",
        "3.2": "sec:voxel",
        "3.6": "sec:mesh",
        "4.1": "sec:reference",
        "4.2": "sec:metrics",
        "4.3": "sec:protocol",
        "4.4": "sec:edit-protocol",
        "5.1": "sec:pitch",
        "5.2": "sec:timing",
        "5.3": "sec:prep",
        "5.4": "sec:edit",
        "5.6": "sec:tuning",
        "6.1": "sec:offset",
        "6.2": "sec:survey",
        "6.3": "sec:boundary",
    }
    def fix_ref(m):
        key = m.group(1)
        return r"Section~\ref{%s}" % refmap.get(key, "sec:FIXME" + key)
    body = re.sub(r"CITEP\\\{([^}]+)\\\}", r"\\citep{\1}", body)
    body = re.sub(r"CITET\\\{([^}]+)\\\}", r"\\citet{\1}", body)
    body = re.sub(r"SECREF\\\{([0-9.]+)\\\}", fix_ref, body)
    # listings: disable jcgt float on every listing
    body = body.replace(r"\begin{lstlisting}", r"\begin{lstlisting}[float=false,language=]")
    # unicode leftovers
    repls = {
        "←": r"$\leftarrow$",
        "−": r"--",
        "≤": r"$\leq$",
        "∨": r"$\lor$",
        "∞": r"$\infty$",
        "∈": r"$\in$",
        "τ": r"$\tau$",
        "·": r"$\cdot$",
        "×": r"$\times$",
        "†": r"$^\dagger$",
        "Δ": r"$\Delta$",
        "—": r"---",
        "–": r"--",
        "“": "``",
        "”": "''",
        "‘": "`",
        "’": "'",
    }
    for a, b in repls.items():
        body = body.replace(a, b)
    return body


def main():
    raw = (PAPER / "draft.md").read_text()
    abstract_md = extract_abstract(raw)
    abstract_tex = pandoc_fragment(abstract_md).strip()
    # drop trailing par
    body_md = strip_front_and_back(raw)
    # remove the teaser image markdown (handled by \teaser)
    body_md = re.sub(
        r"!\[The six selected methods at the hangar horizon scene\..*?\]\(figures/teaser\.svg\)\n+",
        "",
        body_md,
        count=1,
    )
    # remove acknowledgements + figure provenance; postamble owns them
    body_md = re.sub(r"\n## Acknowledgements\n.*", "\n", body_md, flags=re.S)
    body_md = apply_headings(body_md)
    body_md = apply_cites(body_md)
    body = pandoc_fragment(body_md)
    body = postprocess_latex(body)
    tex = PREAMBLE.replace("BODY_ABSTRACT", abstract_tex) + body + POSTAMBLE
    (PAPER / "paper.tex").write_text(tex)
    print(f"wrote {PAPER / 'paper.tex'} ({len(tex)} chars)")


if __name__ == "__main__":
    main()
