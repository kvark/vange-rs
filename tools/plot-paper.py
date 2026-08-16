#!/usr/bin/env python3
"""Generate the paper's data-driven SVG figures without plotting packages.

The hardware charts read the final `compare-terrain.py` JSON files. The fit
chart reads `paper/survey.json`, which `level-survey.py --json-out` can replace.
SVG keeps text and lines sharp in the paper and makes every visual reproducible
with the Python standard library.

    tools/plot-paper.py
"""

import argparse
import base64
import glob
import html
import json
import math
import os
import statistics


BG = "#fbfaf7"
INK = "#20252b"
MUTED = "#68717b"
GRID = "#d9dde1"
PANEL = "#ffffff"
METHODS = ["RayTraced", "RayVoxel", "Sliced", "Scattered", "Painted",
           "Mesh q=0.5"]
SHORT = ["Ray 128", "Voxel 100", "Sliced", "Scatter", "Painted", "Mesh .5"]
COLOURS = {
    "RayTraced": "#168c9e",
    "RayVoxel": "#4776c5",
    "Sliced": "#d99125",
    "Scattered": "#d65757",
    "Painted": "#8b62c6",
    "Mesh q=0.5": "#24764a",
    "Mesh q=0.0": "#69a879",
    "Mesh q=0.25": "#69a879",
    "Mesh q=0.75": "#155d38",
}


class Svg:
    def __init__(self, width, height, title):
        self.width = width
        self.height = height
        self.items = [
            f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
            f'viewBox="0 0 {width} {height}" role="img" aria-label="{html.escape(title)}">',
            "<style>text{font-family:Inter,DejaVu Sans,sans-serif;fill:#20252b}"
            ".title{font-size:23px;font-weight:700}.subtitle{font-size:13px;fill:#68717b}"
            ".panel-title{font-size:15px;font-weight:700}.axis{font-size:11px;fill:#68717b}"
            ".label{font-size:12px}.value{font-size:10px;font-weight:700}"
            ".light{fill:#f8fafc}</style>",
            f'<rect width="{width}" height="{height}" fill="{BG}" rx="14"/>',
        ]

    def add(self, value):
        self.items.append(value)

    def text(self, x, y, value, cls="label", anchor="start", **attrs):
        extra = " ".join(f'{key.replace("_", "-")}="{val}"' for key, val in attrs.items())
        self.add(f'<text x="{x:.1f}" y="{y:.1f}" class="{cls}" '
                 f'text-anchor="{anchor}" {extra}>{html.escape(str(value))}</text>')

    def save(self, path):
        self.items.append("</svg>\n")
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as output:
            output.write("\n".join(self.items))


def load_runs(pattern):
    paths = sorted(glob.glob(pattern))
    if not paths:
        raise SystemExit(f"no result files match {pattern}")
    return [json.load(open(path)) for path in paths]


def log_position(value, low, high, start, length):
    value = max(low, min(high, value))
    return start + length * (math.log(value / low) / math.log(high / low))


def figure_performance(runs, out):
    vulkan = [run for run in runs if run["device"]["backend"] == "Vulkan"]
    width, height = 1240, 430
    svg = Svg(width, height, "Steady-state terrain frame time on four Vulkan adapters")
    svg.text(34, 36, "Performance is portable only in the broad sense", "title")
    svg.text(34, 58, "Mean GPU time over 12 scenes · logarithmic scale · common 1024² shadow pass", "subtitle")
    left, top, panel_width, plot_height = 38, 92, 288, 250
    y_low, y_high = 0.25, 128.0
    ticks = [0.5, 1, 2, 4, 8, 16, 32, 64, 128]
    for panel, run in enumerate(vulkan):
        x0 = left + panel * (panel_width + 8)
        svg.add(f'<rect x="{x0}" y="{top}" width="{panel_width}" height="{plot_height}" '
                f'fill="{PANEL}" stroke="{GRID}" rx="8"/>')
        name = run["device"]["adapter"]
        name = ("Radeon 780M" if "780M" in name else "Radeon 7900 XT" if "7900" in name
                else "Intel RPL-U" if "Intel" in name else "GeForce RTX 5070")
        svg.text(x0 + 12, top + 22, name, "panel-title")
        for tick in ticks:
            y = top + plot_height - 30 - log_position(tick, y_low, y_high, 0, plot_height - 58)
            svg.add(f'<line x1="{x0 + 38}" y1="{y:.1f}" x2="{x0 + panel_width - 8}" '
                    f'y2="{y:.1f}" stroke="{GRID}" stroke-width="1"/>')
            if panel == 0 and tick in (0.5, 1, 2, 4, 8, 16, 32, 64, 128):
                svg.text(x0 + 32, y + 4, f"{tick:g}", "axis", "end")
        means = {method: statistics.mean(row["avg_ms"] for row in run["rows"]
                                         if row["method"] == method)
                 for method in METHODS}
        base_y = top + plot_height - 30
        for index, method in enumerate(METHODS):
            value = means[method]
            x = x0 + 45 + index * 33
            bar_height = log_position(value, y_low, y_high, 0, plot_height - 58)
            svg.add(f'<rect x="{x}" y="{base_y - bar_height:.1f}" width="23" height="{bar_height:.1f}" '
                    f'fill="{COLOURS[method]}" rx="3"/>')
            label = f"{value:.2f}" if value < 10 else f"{value:.1f}"
            svg.text(x + 11.5, base_y - bar_height - 5, label, "value", "middle")
            svg.text(x + 11.5, top + plot_height - 12, SHORT[index], "axis", "end",
                     transform=f"rotate(-48 {x + 11.5:.1f} {top + plot_height - 12:.1f})")
        budget_y = base_y - log_position(16.7, y_low, y_high, 0, plot_height - 58)
        svg.add(f'<line x1="{x0 + 38}" y1="{budget_y:.1f}" x2="{x0 + panel_width - 8}" '
                f'y2="{budget_y:.1f}" stroke="#a84f4f" stroke-width="1.5" stroke-dasharray="5 4"/>')
    svg.text(22, 230, "GPU time (ms, log)", "axis", "middle", transform="rotate(-90 22 230)")
    svg.text(620, 411, "Frame time alone does not score close-view quality, edit cost, memory, physics reuse, or uncertainty.",
             "subtitle", "middle")
    svg.save(out)


def figure_quality(baseline, out):
    width, height = 1240, 490
    svg = Svg(width, height, "Terrain artifacts as camera pitch changes")
    svg.text(34, 36, "The horizon separates methods that look alike from above", "title")
    svg.text(34, 58, f"Mean over three scenes · {baseline['label']} corrected CPU reference · logarithmic scale", "subtitle")
    pitches = [0.0, -30.0, -60.0, -90.0]
    panels = [("see_through", "Missing reference terrain"), ("speckle", "Excess local incoherence")]
    y_low, y_high = 0.05, 100.0
    ticks = [0.1, 0.3, 1, 3, 10, 30, 100]
    for panel, (field, title) in enumerate(panels):
        x0, y0, plot_width, plot_height = 70 + panel * 590, 96, 510, 285
        svg.add(f'<rect x="{x0}" y="{y0}" width="{plot_width}" height="{plot_height}" '
                f'fill="{PANEL}" stroke="{GRID}" rx="8"/>')
        svg.text(x0 + 14, y0 + 24, title, "panel-title")
        for tick in ticks:
            y = y0 + plot_height - 38 - log_position(tick, y_low, y_high, 0, plot_height - 72)
            svg.add(f'<line x1="{x0 + 48}" y1="{y:.1f}" x2="{x0 + plot_width - 18}" '
                    f'y2="{y:.1f}" stroke="{GRID}"/>')
            svg.text(x0 + 41, y + 4, f"{tick:g}%", "axis", "end")
        for pitch_index, pitch in enumerate(pitches):
            x = x0 + 68 + pitch_index * 135
            svg.text(x, y0 + plot_height - 15, f"{abs(pitch):g}° down", "axis", "middle")
        for method in METHODS:
            points = []
            for pitch_index, pitch in enumerate(pitches):
                values = [row[field] for row in baseline["rows"]
                          if row["method"] == method and row["pitch"] == pitch]
                value = statistics.mean(values)
                x = x0 + 68 + pitch_index * 135
                y = y0 + plot_height - 38 - log_position(max(value, y_low), y_low, y_high,
                                                          0, plot_height - 72)
                points.append((x, y, value))
            path = " ".join(("M" if index == 0 else "L") + f" {x:.1f} {y:.1f}"
                            for index, (x, y, _) in enumerate(points))
            svg.add(f'<path d="{path}" fill="none" stroke="{COLOURS[method]}" '
                    f'stroke-width="2.4" stroke-linejoin="round"/>')
            for x, y, value in points:
                svg.add(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="3.5" fill="{COLOURS[method]}" '
                        f'stroke="white" stroke-width="1"/>')
    legend_y = 425
    for index, method in enumerate(METHODS):
        x = 85 + index * 160
        svg.add(f'<line x1="{x}" y1="{legend_y}" x2="{x + 22}" y2="{legend_y}" '
                f'stroke="{COLOURS[method]}" stroke-width="4"/>')
        svg.text(x + 28, legend_y + 4, SHORT[index], "axis")
    svg.text(620, 467, "Zero values are placed at 0.05% so coherent top-down results remain visible.",
             "subtitle", "middle")
    svg.save(out)


def figure_fit(survey, out):
    rows = survey["rows"]
    width, height = 900, 525
    svg = Svg(width, height, "TIN reduction against double-level terrain fraction")
    svg.text(34, 38, "A second terrain layer predicts the collapse in mesh reduction", "title")
    svg.text(34, 61, "Ten shipped worlds · TIN quality 0.25 · reduction axis is logarithmic", "subtitle")
    x0, y0, plot_width, plot_height = 92, 96, 730, 340
    svg.add(f'<rect x="{x0}" y="{y0}" width="{plot_width}" height="{plot_height}" '
            f'fill="{PANEL}" stroke="{GRID}" rx="8"/>')
    svg.add(f'<rect x="{x0}" y="{y0}" width="{plot_width * 2 / 40:.1f}" height="{plot_height}" '
            f'fill="#e9f4ec" opacity="0.9"/>')
    for tick in [0, 5, 10, 20, 30, 40]:
        x = x0 + tick / 40 * plot_width
        svg.add(f'<line x1="{x:.1f}" y1="{y0}" x2="{x:.1f}" y2="{y0 + plot_height}" stroke="{GRID}"/>')
        svg.text(x, y0 + plot_height + 22, tick, "axis", "middle")
    for tick in [5, 10, 20, 50, 100, 200]:
        y = y0 + plot_height - log_position(tick, 4, 220, 0, plot_height)
        svg.add(f'<line x1="{x0}" y1="{y:.1f}" x2="{x0 + plot_width}" y2="{y:.1f}" stroke="{GRID}"/>')
        svg.text(x0 - 10, y + 4, f"{tick}×", "axis", "end")
    xs = [row.get("dual", row.get("dual_pct")) for row in rows]
    ys = [math.log(row["reduction"]) for row in rows]
    mean_x, mean_y = statistics.mean(xs), statistics.mean(ys)
    slope = sum((x - mean_x) * (y - mean_y) for x, y in zip(xs, ys)) / sum((x - mean_x) ** 2 for x in xs)
    intercept = mean_y - slope * mean_x
    line = []
    for value in (0, 40):
        reduction = math.exp(intercept + slope * value)
        x = x0 + value / 40 * plot_width
        y = y0 + plot_height - log_position(reduction, 4, 220, 0, plot_height)
        line.append((x, y))
    svg.add(f'<line x1="{line[0][0]:.1f}" y1="{line[0][1]:.1f}" '
            f'x2="{line[1][0]:.1f}" y2="{line[1][1]:.1f}" stroke="#8b949e" '
            f'stroke-width="2" stroke-dasharray="7 5"/>')
    offsets = {"weexow": (8, -8), "ark-a-znoy": (8, 15), "threall": (8, 4),
               "xplo": (8, -8), "khox": (8, 15), "fostral": (10, -10),
               "necross": (8, -8), "glorx": (-8, -10), "boozeena": (8, 16),
               "hmok": (-8, 16)}
    for row in rows:
        dual = row.get("dual", row.get("dual_pct"))
        x = x0 + dual / 40 * plot_width
        y = y0 + plot_height - log_position(row["reduction"], 4, 220, 0, plot_height)
        highlight = row["level"] == "fostral"
        colour = "#d65757" if highlight else "#24764a" if dual else "#168c9e"
        radius = 8 if highlight else 6
        svg.add(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="{radius}" fill="{colour}" '
                f'stroke="white" stroke-width="2"/>')
        dx, dy = offsets[row["level"]]
        anchor = "end" if dx < 0 else "start"
        svg.text(x + dx, y + dy, row["level"], "label", anchor)
    svg.text(x0 + plot_width / 2, 486, "double-level texels (%)", "axis", "middle")
    svg.text(25, y0 + plot_height / 2, "grid-to-TIN reduction", "axis", "middle",
             transform=f"rotate(-90 25 {y0 + plot_height / 2:.1f})")
    svg.text(810, 82, "r = −0.77", "panel-title", "end")
    svg.text(810, 100, "corr(log reduction, dual fraction)", "axis", "end")
    svg.save(out)


def figure_preparation(runs, out):
    baseline = next(run for run in runs if "780M" in run["device"]["adapter"])
    width, height = 900, 475
    svg = Svg(width, height, "One-time preparation cost by terrain method")
    svg.text(34, 38, "Fast frames can hide seconds of preparation", "title")
    svg.text(34, 61, "Maximum over 12 scenes on Radeon 780M · CPU wall time · logarithmic scale", "subtitle")
    x0, y0, plot_width = 180, 105, 650
    low, high = 8, 4096
    for tick in [10, 30, 100, 300, 1000, 3000]:
        x = log_position(tick, low, high, x0, plot_width)
        svg.add(f'<line x1="{x:.1f}" y1="{y0 - 18}" x2="{x:.1f}" y2="{y0 + 7 * 43 - 4}" '
                f'stroke="{GRID}"/>')
        svg.text(x, y0 - 27, f"{tick:g} ms", "axis", "middle")
    for index, method in enumerate(METHODS):
        y = y0 + index * 43
        rows = [row for row in baseline["rows"] if row["method"] == method]
        first = max(row["prep_first_frame_ms"] for row in rows)
        warm = max(row["prep_warmup_ms"] for row in rows)
        x_first = log_position(first, low, high, x0, plot_width)
        x_warm = log_position(warm, low, high, x0, plot_width)
        svg.text(x0 - 16, y + 4, SHORT[index], "label", "end")
        svg.add(f'<line x1="{x_first:.1f}" y1="{y}" x2="{x_warm:.1f}" y2="{y}" '
                f'stroke="{COLOURS[method]}" stroke-width="7" stroke-linecap="round" opacity="0.65"/>')
        svg.add(f'<circle cx="{x_first:.1f}" cy="{y}" r="5" fill="white" '
                f'stroke="{COLOURS[method]}" stroke-width="3"/>')
        svg.add(f'<circle cx="{x_warm:.1f}" cy="{y}" r="5" fill="{COLOURS[method]}"/>')
        svg.text(x_warm + 9, y + 4, f"{warm:.0f}", "value")
    svg.add('<circle cx="610" cy="431" r="5" fill="white" stroke="#68717b" stroke-width="2"/>')
    svg.text(622, 435, "first frame", "axis")
    svg.add('<circle cx="713" cy="431" r="5" fill="#68717b"/>')
    svg.text(725, 435, "all warmup", "axis")
    svg.save(out)


def figure_encoding(out):
    svg = Svg(1080, 500, "Vangers dual-layer terrain encoding")
    svg.text(34, 38, "Two texels encode two vertical solids", "title")
    svg.text(34, 61, "The compact simulation state stores a floor, a cave ceiling, and a slab top", "subtitle")
    svg.text(160, 104, "paired source samples", "panel-title", "middle")
    for index, (name, colour) in enumerate((("even texel", "#168c9e"), ("odd texel", "#8b62c6"))):
        x = 55 + index * 210
        svg.add(f'<rect x="{x}" y="130" width="175" height="230" rx="12" fill="white" '
                f'stroke="{colour}" stroke-width="3"/>')
        svg.text(x + 87.5, 160, name, "panel-title", "middle")
        labels = (("height byte", "low" if index == 0 else "high"),
                  ("delta bits", "part of mid"), ("material", "surface type"))
        for row, (field, meaning) in enumerate(labels):
            y = 185 + row * 52
            svg.add(f'<rect x="{x + 18}" y="{y}" width="139" height="38" rx="5" '
                    f'fill="{colour}" opacity="{0.16 + row * 0.04}"/>')
            svg.text(x + 28, y + 16, field, "axis")
            svg.text(x + 147, y + 27, meaning, "value", "end")
    svg.add('<path d="M 440 244 C 485 244, 500 244, 545 244" fill="none" stroke="#68717b" '
            'stroke-width="3" marker-end="url(#arrow)"/>')
    svg.add('<defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" '
            'markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="#68717b"/></marker></defs>')
    svg.text(492, 224, "decode", "axis", "middle")
    svg.text(790, 104, "one vertical terrain column", "panel-title", "middle")
    x, w = 660, 265
    svg.add(f'<rect x="{x}" y="130" width="{w}" height="275" rx="10" fill="#e7f0f7" stroke="{GRID}"/>')
    svg.add(f'<rect x="{x}" y="315" width="{w}" height="90" fill="#776858"/>')
    svg.add(f'<rect x="{x}" y="175" width="{w}" height="58" fill="#8d8171"/>')
    svg.add(f'<rect x="{x}" y="233" width="{w}" height="82" fill="#283643"/>')
    for y, label, colour in ((175, "high · slab top", "#8b62c6"),
                             (233, "mid · cave ceiling", "#d99125"),
                             (315, "low · floor", "#168c9e")):
        svg.add(f'<line x1="{x - 18}" y1="{y}" x2="{x + w + 18}" y2="{y}" '
                f'stroke="{colour}" stroke-width="3"/>')
        svg.text(x + w + 30, y + 4, label, "label")
    svg.text(x + w / 2, 208, "upper solid", "label light", "middle")
    svg.text(x + w / 2, 279, "cave / empty", "label light", "middle")
    svg.text(x + w / 2, 367, "lower solid", "label light", "middle")
    svg.text(540, 462, "A renderer must preserve both intervals and their vertical boundary walls.",
             "subtitle", "middle")
    svg.save(out)


def figure_teaser(out):
    names = METHODS
    files = [f"teaser-{index}.png" for index in range(len(names))]
    if not all(os.path.exists(os.path.join(os.path.dirname(out), file)) for file in files):
        return False
    svg = Svg(1240, 560, "The six measured terrain methods at the hangar horizon scene")
    svg.text(34, 38, "One horizon scene exposes six different tradeoffs", "title")
    svg.text(34, 61, "Hangar · pitch 0° · identical camera, lighting, fog, and shadow pass", "subtitle")
    positions = [(25 + col * 403, 105) for col in range(3)] + [(25 + col * 403, 345) for col in range(3)]
    kinds = ["image-order", "image-order", "forward samples",
             "forward samples", "forward samples", "fitted triangles"]
    for (x, y), name, file, kind in zip(positions, names, files, kinds):
        colour = COLOURS[name]
        svg.add(f'<rect x="{x - 5}" y="{y - 30}" width="390" height="225" rx="8" '
                f'fill="white" stroke="{colour}" stroke-width="2"/>')
        svg.text(x + 190, y - 10, name, "panel-title", "middle")
        with open(os.path.join(os.path.dirname(out), file), "rb") as source:
            encoded = base64.b64encode(source.read()).decode("ascii")
        svg.add(f'<image href="data:image/png;base64,{encoded}" x="{x}" y="{y}" width="380" height="168" '
                f'preserveAspectRatio="xMidYMid slice"/>')
        svg.text(x + 190, y + 185, kind, "axis", "middle")
    svg.text(620, 540, "Scattered loses coverage; slicing bands; the selected mesh keeps the hangar wall.",
             "subtitle", "middle")
    svg.save(out)
    return True


def figure_edit(out, edit_dir):
    """Before/after crater for the three update classes."""
    pairs = [
        ("RayTraced", "RayTraced-before.png", "RayTraced-after.png"),
        ("RayVoxel", "RayVoxel-before.png", "RayVoxel-after.png"),
        ("Mesh q=0.5", "Mesh_q_0_5-before.png", "Mesh_q_0_5-after.png"),
    ]
    paths = []
    for name, before, after in pairs:
        before_path = os.path.join(edit_dir, before)
        after_path = os.path.join(edit_dir, after)
        if not (os.path.exists(before_path) and os.path.exists(after_path)):
            return False
        paths.append((name, before_path, after_path))
    svg = Svg(1240, 620, "A local crater becomes visible without reloading the level")
    svg.text(34, 38, "The same dirty rectangle reaches every method", "title")
    svg.text(34, 61, "Same radius-48 crater as the timing fixture · closer/higher view · before (top) and first updated frame (bottom)",
             "subtitle")
    kinds = ["direct texture read", "incremental occupancy rebuild", "local chunk refit"]
    for index, ((name, before_path, after_path), kind) in enumerate(zip(paths, kinds)):
        x = 30 + index * 403
        colour = COLOURS[name]
        svg.add(f'<rect x="{x}" y="84" width="390" height="500" rx="8" fill="white" '
                f'stroke="{colour}" stroke-width="2"/>')
        svg.text(x + 195, 108, name, "panel-title", "middle")
        for row, (label, path) in enumerate((("before", before_path), ("after", after_path))):
            y = 122 + row * 215
            with open(path, "rb") as source:
                encoded = base64.b64encode(source.read()).decode("ascii")
            svg.add(f'<image href="data:image/png;base64,{encoded}" x="{x + 10}" y="{y}" '
                    f'width="370" height="190" preserveAspectRatio="xMidYMid slice"/>')
            svg.text(x + 22, y + 18, label, "value", fill="white")
        svg.text(x + 195, 560, kind, "axis", "middle")
    svg.save(out)
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results", default="remote/results-*.json")
    parser.add_argument("--accuracy-results", required=True,
                        help="protocol-v3 accuracy-only or publication JSON")
    parser.add_argument("--survey", default="paper/survey.json")
    parser.add_argument("--out", default="paper/figures")
    parser.add_argument("--edit-dir", default="work/edit-figure",
                        help="directory of before/after crater PNGs")
    args = parser.parse_args()
    runs = load_runs(args.results)
    with open(args.accuracy_results) as source:
        accuracy = json.load(source)
    if (accuracy.get("protocol_version", 0) < 3 or
            accuracy.get("purpose") not in ("accuracy-only", "publication")):
        raise SystemExit("--accuracy-results must be a protocol-v3 "
                         "accuracy-only or publication run")
    survey = json.load(open(args.survey))
    figures = {
        "performance.svg": lambda path: figure_performance(runs, path),
        "quality-pitch.svg": lambda path: figure_quality(accuracy, path),
        "fit-survey.svg": lambda path: figure_fit(survey, path),
        "preparation.svg": lambda path: figure_preparation(runs, path),
        "encoding.svg": figure_encoding,
    }
    for name, generate in figures.items():
        path = os.path.join(args.out, name)
        generate(path)
        print(f"wrote {path}")
    teaser = os.path.join(args.out, "teaser.svg")
    if figure_teaser(teaser):
        print(f"wrote {teaser}")
    edit = os.path.join(args.out, "edit.svg")
    if figure_edit(edit, args.edit_dir):
        print(f"wrote {edit}")


if __name__ == "__main__":
    main()
