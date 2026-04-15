#!/usr/bin/env python3
"""Package a Vangers installation into release zips for vange-rs web.

Usage:
    python3 tools/pack-data.py --root PATH_TO_VANGERS --out PATH_TO_OUTPUT [options]

Produces one zip per level plus a `common.zip` holding cross-level
assets, laid out to match the layout expected by `src/data.rs`:

    <out>/
      common.zip            game.lst, wrlds.dat, car.prm, common.prm,
                            resource/m3d/, resource/pal/ — the files
                            the web build actually reads. Everything
                            else under --root (cutscene videos, menu
                            art, music, original binaries, editor
                            scripts…) is dropped by default because the
                            web build doesn't use it and the full data
                            is ~180 MiB on the wire.
      fostral.zip           contents of the Fostral level directory,
                            flat (no leading `fostral/` prefix)
      necross.zip           ...
      ...

A level directory is any directory containing a `world.ini` file; the
directory's own name becomes the level id (the zip basename). This
matches the URL scheme the web client uses:

    https://github.com/kvark/vange-rs/releases/download/data-0/<id>.zip

Pass `--full-common` to revert to the old behaviour of packing every
non-level file; useful if you want to host the full Vangers data set
at the same release tag.

Upload the resulting zips as assets to the `data-0` release (e.g. via
`gh release upload data-0 *.zip`).
"""

from __future__ import annotations

import argparse
import os
import sys
import zipfile
from pathlib import Path

# Which paths under --root are kept in the default (web-minimal)
# common.zip. Everything outside this list is dropped unless
# --full-common is passed.
#
# Drivers for this list come from a read of the runtime loaders:
#   - bin/web/main.rs::spawn_default_agent  — game.lst, car.prm, m3d,
#     per-vehicle .prm, default.prm
#   - WebApp::build                         — common.prm
# The palette dir is included defensively; nothing in the web build
# currently reads it, but it's ~10 KiB and levels reference palettes
# by relative path.
WEB_KEEP_FILES = {
    "game.lst",
    "wrlds.dat",
    "car.prm",
    "common.prm",
    "worlds.prm",
    "escaves.prm",
    "spots.prm",
    "bunches.prm",
    "tabutask.prm",
}
WEB_KEEP_PREFIXES = (
    "resource/m3d/",
    "resource/pal/",
)


def find_level_dirs(root: Path) -> dict[str, Path]:
    """Map level id -> level dir. Level id is the directory name.

    A level is any directory containing a `world.ini` (case-insensitive).
    Duplicates (same basename in two paths) abort the script; the user
    must disambiguate.
    """
    levels: dict[str, Path] = {}
    for dirpath, dirnames, filenames in os.walk(root):
        # Case-insensitive match for world.ini.
        if any(f.lower() == "world.ini" for f in filenames):
            d = Path(dirpath)
            lid = d.name.lower()
            if lid in levels:
                sys.exit(
                    f"error: two level directories share the name {lid!r}:\n"
                    f"  {levels[lid]}\n  {d}\n"
                    f"rename one or pass --levels to pick explicitly."
                )
            levels[lid] = d
    return levels


def pack_level(level_id: str, level_dir: Path, out_path: Path, verbose: bool) -> None:
    """Zip the contents of `level_dir` directly into `out_path`.

    Files are stored without the `<level_id>/` prefix so the client's
    VFS can load them with keys like `"world.ini"` / `"output.vmc"`.
    """
    if verbose:
        print(f"  building {out_path.name} from {level_dir}")
    with zipfile.ZipFile(out_path, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as zf:
        for p in sorted(level_dir.rglob("*")):
            if p.is_dir():
                continue
            # Path inside the archive: relative to the level dir.
            arcname = p.relative_to(level_dir).as_posix()
            if verbose:
                print(f"    + {arcname}")
            zf.write(p, arcname=arcname)


def keep_for_web(arcname: str) -> bool:
    """Whether a file at `arcname` (forward-slash path relative to
    --root) should go into the minimal common.zip."""
    if arcname in WEB_KEEP_FILES:
        return True
    return any(arcname.startswith(p) for p in WEB_KEEP_PREFIXES)


def pack_common(
    root: Path,
    level_dirs: set[Path],
    out_path: Path,
    full: bool,
    verbose: bool,
) -> None:
    """Zip the cross-level assets into `out_path`.

    By default only the files the web build actually reads are kept
    (see `WEB_KEEP_FILES` / `WEB_KEEP_PREFIXES`). Passing `full=True`
    restores the "everything not inside a level dir" behaviour, handy
    when a release also needs to serve the full Vangers data set.
    """
    level_dirs_abs = {d.resolve() for d in level_dirs}
    mode = "full" if full else "web-minimal"
    if verbose:
        print(f"  building {out_path.name} ({mode} cross-level assets)")
    kept = 0
    dropped = 0
    dropped_bytes = 0
    with zipfile.ZipFile(out_path, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as zf:
        for p in sorted(root.rglob("*")):
            if p.is_dir():
                continue
            # Skip anything under a level directory.
            if any(anc in level_dirs_abs for anc in p.resolve().parents):
                continue
            arcname = p.relative_to(root).as_posix()
            if not full and not keep_for_web(arcname):
                dropped += 1
                dropped_bytes += p.stat().st_size
                continue
            if verbose:
                print(f"    + {arcname}")
            zf.write(p, arcname=arcname)
            kept += 1
    if not full:
        print(
            f"    kept {kept} file(s); dropped {dropped} non-web "
            f"file(s) ({human_bytes(dropped_bytes)})"
        )


def human_bytes(n: int) -> str:
    for unit in ("B", "KiB", "MiB", "GiB"):
        if n < 1024:
            return f"{n:.1f} {unit}" if unit != "B" else f"{n} B"
        n /= 1024
    return f"{n:.1f} TiB"


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Package Vangers data for the vange-rs web release.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    ap.add_argument(
        "--root",
        required=True,
        type=Path,
        help="path to the Vangers installation (contains game.lst)",
    )
    ap.add_argument(
        "--out",
        required=True,
        type=Path,
        help="output directory for the zip assets (created if missing)",
    )
    ap.add_argument(
        "--levels",
        help="comma-separated level ids to include (default: all discovered)",
    )
    ap.add_argument(
        "--skip-common",
        action="store_true",
        help="do not build common.zip (useful when only re-packing levels)",
    )
    ap.add_argument(
        "--full-common",
        action="store_true",
        help="pack every non-level file into common.zip (default is to "
        "include only the small set the web build actually reads, "
        "trimming ~170 MiB of videos/music/menu art from a typical "
        "Vangers install).",
    )
    ap.add_argument(
        "-v",
        "--verbose",
        action="store_true",
        help="list every file as it is added",
    )
    args = ap.parse_args()

    root = args.root.expanduser().resolve()
    if not root.is_dir():
        sys.exit(f"error: --root is not a directory: {root}")

    out = args.out.expanduser().resolve()
    out.mkdir(parents=True, exist_ok=True)

    print(f"Scanning {root}")
    levels = find_level_dirs(root)
    if not levels:
        sys.exit("error: no world.ini found anywhere under --root")

    selected: dict[str, Path]
    if args.levels:
        wanted = {s.strip().lower() for s in args.levels.split(",") if s.strip()}
        unknown = wanted - set(levels)
        if unknown:
            sys.exit(f"error: unknown level(s): {', '.join(sorted(unknown))}")
        selected = {k: levels[k] for k in wanted}
    else:
        selected = levels

    print(f"Found {len(levels)} level(s); packing {len(selected)}:")
    for lid in sorted(selected):
        print(f"  - {lid}  ({selected[lid].relative_to(root)})")

    for lid, d in sorted(selected.items()):
        out_path = out / f"{lid}.zip"
        pack_level(lid, d, out_path, args.verbose)
        print(f"    -> {out_path.name}  {human_bytes(out_path.stat().st_size)}")

    if not args.skip_common:
        out_path = out / "common.zip"
        # Pass all discovered level dirs (even ones not selected) so
        # common.zip never duplicates level data that lives elsewhere.
        pack_common(
            root,
            set(levels.values()),
            out_path,
            full=args.full_common,
            verbose=args.verbose,
        )
        print(f"    -> {out_path.name}  {human_bytes(out_path.stat().st_size)}")

    print("\nDone. Upload with:")
    tag = "data-0"
    print(f"  gh release upload {tag} {out}/*.zip --repo kvark/vange-rs")
    return 0


if __name__ == "__main__":
    sys.exit(main())
