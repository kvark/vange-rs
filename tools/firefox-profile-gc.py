#!/usr/bin/env python3
"""Name the JS/DOM objects a Firefox profiler capture is collecting.

This profile format does not store a per-class GC census. What it *does*
store, if you recorded the content process, is:

- GC / cycle-collector markers (how many cells, which reason, pause)
- DOMEvent markers (abort storms show up here)
- native and wasm stacks on the content thread

Walk those stacks during CC / abort and you get constructor names
(AbortController, WebTask, WebGLBuffer, …) plus the Rust functions that
created them (winit Schedule, wgpu create_buffer_init, …).

  python3 tools/firefox-profile-gc.py 'Firefox 2026-08-26 21.11 profile.json.gz'

Record the next capture with Firefox Profiler → Settings →
“Record JS allocations” if you also want JS constructor histograms.
"""

from __future__ import annotations

import gzip
import json
import sys
from collections import Counter


def load(path: str) -> dict:
    opener = gzip.open if path.endswith(".gz") else open
    with opener(path, "rt") as f:
        return json.load(f)


def thread_for_vange(profile: dict) -> tuple[int, dict]:
    best = None
    for i, t in enumerate(profile["threads"]):
        etld = t.get("eTLD+1") or ""
        n = len((t.get("markers") or {}).get("name") or [])
        score = n + (1_000_000 if "vange" in etld else 0)
        if best is None or score > best[0]:
            best = (score, i, t)
    if best is None:
        raise SystemExit("no threads")
    return best[1], best[2]


def walk_stack(si: int, sframe, poff, frames) -> list[int]:
    seen: set[int] = set()
    out: list[int] = []
    while si is not None and si >= 0 and si not in seen:
        seen.add(si)
        out.append(frames[sframe[si]])
        off = poff[si]
        if not off:
            break
        si = si - off
    return out


def main() -> None:
    if len(sys.argv) < 2:
        print(__doc__.strip(), file=sys.stderr)
        sys.exit(2)
    profile = load(sys.argv[1])
    sa = profile["shared"]["stringArray"]
    ft = profile["shared"]["funcTable"]
    fr = profile["shared"]["frameTable"]
    st = profile["shared"]["stackTable"]
    idx, thread = thread_for_vange(profile)
    print(f"thread {idx} {thread.get('name')} {thread.get('eTLD+1')} pid={thread.get('pid')}")
    print(f"features {profile['meta'].get('configuration', {}).get('features')}")

    mk = thread["markers"]
    names = mk["name"]
    data = mk["data"]
    start = mk["startTime"]
    end = mk["endTime"]
    name_c = Counter(sa[n] for n in names)
    print("\nmarkers:")
    for n, c in name_c.most_common(12):
        print(f"  {c:8} {n}")

    events = Counter()
    for i, n in enumerate(names):
        if sa[n] == "DOMEvent":
            events[(data[i] or {}).get("eventType", "?")] += 1
    if events:
        print("\nDOMEvent types:")
        for n, c in events.most_common(8):
            print(f"  {c:8} {n}")

    print("\nGC / CC:")
    for i, n in enumerate(names):
        label = sa[n]
        if label not in ("GCMinor", "GCSlice", "GCMajor", "CC", "CCSlice"):
            continue
        dur = None
        if end[i] and start[i] and end[i] > start[i]:
            gap = end[i] - start[i]
            if gap < 60_000:
                dur = gap
        payload = data[i] or {}
        print(f"  {label} {dur:.1f}ms" if dur else f"  {label}")
        if label == "GCMinor":
            nur = payload.get("nursery") or {}
            print(
                f"    reason={nur.get('reason')} tenured_cells={nur.get('cells_tenured')} "
                f"tenured_bytes={nur.get('bytes_tenured')} nursery_used={nur.get('bytes_used')}"
            )
        elif label == "CC":
            keys = (
                "mReason",
                "mSuspected",
                "mVisitedRefCounted",
                "mVisitedGCed",
                "mFreedRefCounted",
                "mFreedGCed",
                "mMaxSliceTime",
                "mSlices",
            )
            bits = [f"{k}={payload[k]}" for k in keys if k in payload]
            if bits:
                print("    " + " ".join(bits))

    if "jsallocations" not in str(profile["meta"].get("configuration", {})).lower():
        print(
            "\nno JS allocation track. constructors below come from native stacks "
            "on the content thread, not a heap census."
        )

    samples = thread["samples"]
    frames = fr["func"]
    sframe = st["frame"]
    poff = st["prefixOffset"]
    cpu = samples.get("threadCPUDelta") or [1] * samples["length"]
    class_cpu: Counter[str] = Counter()
    rust_cpu: Counter[str] = Counter()
    for i, si in enumerate(samples["stack"]):
        if si is None or si < 0:
            continue
        w = cpu[i] or 1
        funcs = walk_stack(si, sframe, poff, frames)
        for gi in set(funcs):
            name = sa[ft["name"][gi]]
            if "mozilla::dom::" in name or "WebGL" in name or "Abort" in name:
                class_cpu[name.split("(")[0]] += w
            if ".wasm." in name or name.startswith("web-"):
                rust_cpu[name.split("wasm.")[-1][:120]] += w

    print("\nGecko / WebGL classes on sampled stacks (inclusive CPU ns):")
    for n, c in class_cpu.most_common(20):
        print(f"  {c:14} {n}")
    print("\nour wasm on sampled stacks (inclusive CPU ns):")
    for n, c in rust_cpu.most_common(20):
        print(f"  {c:14} {n}")


if __name__ == "__main__":
    main()
