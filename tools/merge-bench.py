#!/usr/bin/env python3
"""Merge `compare-terrain.py --json-out` runs from several machines.

Each run records the adapter, backend and driver it was measured on, so
the files can be collected from different devices and turned into one set
of tables without hand-editing anything.

Emits Markdown: a device roster, then frame time per device, then the
accuracy columns (which are device-independent, so they are reported once
and cross-checked across devices instead of repeated).

Example
-------
    # on each machine (defaults: three views per pitch, four pitches)
    tools/compare-terrain.py --layers work/level.ron --out work/cmp \\
        --label "RTX 4070" --json-out results-4070.json

    # then, anywhere
    tools/merge-bench.py results-*.json > results.md
"""

import argparse
import json
import statistics
import sys


def fmt(v, digits=1):
    if v is None or v != v:  # NaN
        return "—"
    return f"{v:.{digits}f}"


def table(rows, headers):
    out = ["| " + " | ".join(headers) + " |",
           "|" + "|".join("---" for _ in headers) + "|"]
    out += ["| " + " | ".join(r) + " |" for r in rows]
    return "\n".join(out)


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("results", nargs="+", help="JSON files from --json-out")
    ap.add_argument("--metric", default="avg_ms",
                    choices=["avg_ms", "min_ms", "median_ms"],
                    help="which frame time to tabulate. `min_ms` is the least "
                         "noisy on a busy machine; `avg_ms` includes hitches")
    args = ap.parse_args()

    runs = []
    for path in args.results:
        with open(path) as f:
            runs.append(json.load(f))

    metal_runs = [run for run in runs
                  if (run.get("device") or {}).get("backend") == "Metal"]
    if metal_runs and args.metric != "avg_ms":
        sys.exit("Metal results retain only the CPU average alongside the "
                 "invalid encoder timestamps; use --metric avg_ms")

    protocol_fields = ("level", "width", "height", "far", "frames",
                       "shadows", "lighting", "scenes", "methods")
    baseline = {field: runs[0].get(field) for field in protocol_fields}
    for path, run in zip(args.results[1:], runs[1:]):
        changed = [field for field in protocol_fields
                   if run.get(field) != baseline[field]]
        if changed:
            sys.exit(f"{path}: benchmark protocol differs in "
                     f"{', '.join(changed)}; do not merge unlike runs")

    methods, keys = [], []
    for run in runs:
        for r in run["rows"]:
            if r["method"] not in methods:
                methods.append(r["method"])
            k = (r["view"], r["pitch"])
            if k not in keys:
                keys.append(k)

    def value(run, view, pitch, method):
        for r in run["rows"]:
            if r["view"] == view and r["pitch"] == pitch and r["method"] == method:
                if ((run.get("device") or {}).get("backend") == "Metal" and
                        args.metric == "avg_ms"):
                    return r.get("cpu_avg_ms")
                if args.metric == "median_ms":
                    return statistics.median(r["frame_ms"]) if r["frame_ms"] else None
                return r[args.metric]
        return None

    def acc(run, view, pitch, method, field):
        for r in run["rows"]:
            if r["view"] == view and r["pitch"] == pitch and r["method"] == method:
                return r[field]
        return None

    print("## Devices\n")
    rows = []
    for run in runs:
        d = run.get("device") or {}
        rows.append([
            run.get("label", "?"),
            d.get("adapter", "?"),
            f"{d.get('backend', '?')} / {d.get('device_type', '?')}",
            d.get("driver_info", d.get("driver", "?")) or "?",
            f"{run['width']}x{run['height']}, far {run['far']:g}, {run['frames']} frames; "
            f"shadows {run.get('shadows', 'disabled (legacy default)')}",
        ])
    print(table(rows, ["label", "adapter", "backend / type", "driver", "config"]))

    timings = {
        "cpu" if (run.get("device") or {}).get("backend") == "Metal"
        else r.get("timing", "cpu")
        for run in runs for r in run["rows"]
    }
    print(f"\n## Frame time, {args.metric} (ms)\n")
    if timings == {"gpu"}:
        print("From GPU timestamp queries: the device's own view of how long "
              "its work took, with no submission or round trip in it.\n")
    elif "cpu" in timings:
        print("**Mixed timing.** Vulkan rows use GPU timestamp queries. Metal "
              "uses its retained CPU submit-and-wait average because encoder "
              "timestamps did not reliably bracket this multipass workload. "
              "The Metal values include the round trip and must not be compared "
              "directly with Vulkan GPU times.\n")
    for run in runs:
        if len(runs) > 1:
            print(f"\n### {run.get('label', '?')}\n")
        rows = []
        for view, pitch in keys:
            cells = [f"{view} @ {pitch:g}°"]
            for m in methods:
                cells.append(fmt(value(run, view, pitch, m)))
            rows.append(cells)
        print(table(rows, ["view"] + methods))
        pitch_rows = []
        for pitch in dict.fromkeys(p for _, p in keys):
            cells = [f"{pitch:g}°"]
            pitch_keys = [(view, p) for view, p in keys if p == pitch]
            for m in methods:
                values = [value(run, view, p, m) for view, p in pitch_keys]
                values = [v for v in values if v is not None]
                cells.append(fmt(statistics.mean(values), 3) if values else "—")
            pitch_rows.append(cells)
        print("\nArithmetic mean over the views at each pitch:\n")
        print(table(pitch_rows, ["pitch"] + methods))

    # Accuracy should not depend on the device, so report it from the first
    # run as a baseline - but check the others agree, because a mismatch
    # means a driver is producing something different and that is a finding.
    base = runs[0]
    print("\n## Accuracy: see-through / covers-sky / speckle (%)\n")
    print(f"Expected to be device-independent; baseline taken from "
          f"**{base.get('label', '?')}** and cross-checked below. "
          "`see-through` is solid terrain left as background and "
          "`covers-sky` is background filled in — only the first moves when "
          "a renderer is really missing geometry, and both move together "
          "when the reference is the one disagreeing. `speckle` is what "
          "depth agreement cannot see: pixels whose distance disagrees with "
          "their own neighbourhood, in excess of the reference doing the "
          "same.\n")
    rows = []
    for view, pitch in keys:
        cells = [f"{view} @ {pitch:g}°"]
        for m in methods:
            st = acc(base, view, pitch, m, "see_through")
            cs = acc(base, view, pitch, m, "covers_sky")
            sp = acc(base, view, pitch, m, "speckle")
            cells.append(f"{fmt(st)} / {fmt(cs)} / {fmt(sp)}")
        rows.append(cells)
    print(table(rows, ["view"] + methods))
    pitch_rows = []
    for pitch in dict.fromkeys(p for _, p in keys):
        cells = [f"{pitch:g}°"]
        pitch_keys = [(view, p) for view, p in keys if p == pitch]
        for m in methods:
            means = []
            for field in ("see_through", "speckle"):
                values = [acc(base, view, p, m, field) for view, p in pitch_keys]
                values = [v for v in values if v is not None]
                means.append(statistics.mean(values) if values else None)
            cells.append(f"{fmt(means[0])} / {fmt(means[1])}")
        pitch_rows.append(cells)
    print("\nArithmetic mean over the views at each pitch, see-through / speckle (%):\n")
    print(table(pitch_rows, ["pitch"] + methods))

    mismatch_fields = {
        "see_through": (0.5, "%"),
        "covers_sky": (0.5, "%"),
        "speckle": (0.5, "%"),
        "depth_p50": (0.5, "u"),
        "depth_p95": (2.0, "u"),
    }
    mismatched = []
    for run in runs[1:]:
        for view, pitch in keys:
            for m in methods:
                for field, (threshold, unit) in mismatch_fields.items():
                    a = acc(base, view, pitch, m, field)
                    b = acc(run, view, pitch, m, field)
                    if a is not None and b is not None and abs(a - b) > threshold:
                        mismatched.append(
                            (run.get("label", "?"), view, pitch, m, field, unit, a, b)
                        )
    if mismatched:
        print("\n> **Cross-device threshold crossings.** Accuracy should not "
              "normally vary with the adapter; inspect these rows before "
              "deciding whether the spread is material.\n")
        for label, view, pitch, m, field, unit, a, b in mismatched:
            print(f"> - {m} `{field}` at {view} @ {pitch:g}°: "
                  f"{base.get('label', '?')} {a:.1f}{unit} vs {label} {b:.1f}{unit}")

    # One-time cost, which the per-frame numbers deliberately exclude.
    print("\n## Preparation cost (ms, CPU wall time)\n")
    print("`setup` builds pipelines and uploads the terrain texture. "
          "`first frame` additionally carries whatever the method builds "
          "lazily — for the mesh that is the whole triangulation. `warmup` "
          "is every pre-timing frame, which is where an incrementally baked "
          "voxel grid actually gets paid for.\n")
    rows = []
    for m in methods:
        cells = [m]
        for field in ("prep_setup_ms", "prep_first_frame_ms", "prep_warmup_ms"):
            vals = [acc(base, v, p, m, field) for v, p in keys]
            vals = [x for x in vals if x is not None]
            cells.append(fmt(max(vals), 0) if vals else "—")
        rows.append(cells)
    print(table(rows, ["method", "setup", "first frame", "warmup"]))

    print("\n## Depth error, p50 / p95 (world units)\n")
    print("Read comparatively. Grazing rays can move their hit point by tens "
          "of units for a sub-pixel direction change, while a diagnostic "
          "batch also shows scene-dependent common-mode offsets away from "
          "the horizon. "
          "Inter-method agreement is therefore as important as the absolute "
          "error against this reference.\n")
    rows = []
    for view, pitch in keys:
        cells = [f"{view} @ {pitch:g}°"]
        for m in methods:
            p50 = acc(base, view, pitch, m, "depth_p50")
            p95 = acc(base, view, pitch, m, "depth_p95")
            cells.append(f"{fmt(p50)} / {fmt(p95)}")
        rows.append(cells)
    print(table(rows, ["view"] + methods))


if __name__ == "__main__":
    main()
