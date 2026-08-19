#!/usr/bin/env python3
"""Pack the web build into a zip itch.io can host as an HTML5 game.

Usage:
    make web
    python3 tools/pack-itch.py --wasm-dir docs --data-dir docs/data-0

    # data zips from a local pack-data run, wasm from `make web`:
    python3 tools/pack-itch.py --wasm-dir docs --data-dir work

Produces `work/vange-rs-web.zip` by default. Upload that file on
https://itch.io as kind "HTML", with `index.html` as the index.

On the itch edit page, set **Embed in page** viewport width/height
(e.g. 1280×800). An unset iframe size gives the canvas a 0×0 drawing
surface and the game looks like it never started.

The zip has `index.html` at the root (itch requires that). It is a
full-viewport play page: canvas, level picker, WASD hint. Blog/About
tabs from the GitHub Pages site are left out — they 404 off vange.rs.

By default only `common.zip` and `fostral.zip` are bundled (~20 MiB).
Pass `--levels glorx,khox` or `--all-levels` to include more worlds
from `--data-dir`. Download the `data-0` release first if `docs/data-0`
is empty:

    gh release download data-0 --dir docs/data-0 --repo kvark/vange-rs
"""

from __future__ import annotations

import argparse
import shutil
import sys
import zipfile
from pathlib import Path

DEFAULT_LEVELS = ("fostral",)
COMMON = "common.zip"

# wasm-bindgen --target web --no-typescript emits these two next to each
# other; the JS module loads the wasm via import.meta.url.
WASM_FILES = ("web.js", "web_bg.wasm")

OPTIONAL_SITE = (
    "embed.js",
    "embed.css",
    "mesh/index.html",
    "voxel/index.html",
    "ray/index.html",
)

LEVELS = [
    ("fostral", "Fostral"),
    ("glorx", "Glorx"),
    ("necross", "Necross"),
    ("khox", "Khox"),
    ("boozeena", "Boozeena"),
    ("weexow", "Weexow"),
    ("xplo", "Xplo"),
    ("hmok", "Hmok"),
    ("threall", "Threall"),
    ("ark-a-znoy", "Ark-a-Znoy"),
]


INDEX_HTML = """<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Rusty Vangers</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        /* itch.io's iframe height comes from the embed viewport. html/body
           at 100% fill that; the canvas stays in normal flow so it is not
           0x0 (absolute + 100% of an empty body collapses the surface). */
        html, body {
            width: 100%; height: 100%;
            background: #0d1117; color: #c9d1d9;
            font-family: 'Segoe UI', system-ui, sans-serif; overflow: hidden;
        }
        canvas {
            width: 100%; height: 100%; display: block;
            background: #1a1a2e; outline: none; touch-action: none;
        }
        #loading {
            position: absolute; top: 50%; left: 50%; transform: translate(-50%, -50%);
            font-family: monospace; font-size: 17px; text-align: center; min-width: 280px;
            z-index: 10;
        }
        #loading.gone { display: none; }
        #loading.error { color: #f85149; }
        #bar { margin-top: 12px; height: 6px; background: #21262d; border-radius: 3px; overflow: hidden; }
        #fill { height: 100%; width: 0; background: #58a6ff; transition: width .15s linear; }
        #fill.indet { width: 30%; animation: indet 1.2s linear infinite; }
        @keyframes indet { 0% { transform: translateX(-100%); } 100% { transform: translateX(400%); } }
        #note { margin-top: 8px; font-size: 13px; color: #8b949e; }
        #level-picker {
            position: absolute; top: 12px; right: 12px; z-index: 10;
            background: rgba(0,0,0,0.7); padding: 6px 10px; border-radius: 6px;
            font-size: 13px; display: flex; gap: 8px; align-items: center;
        }
        #level-picker select {
            background: #161b22; color: #c9d1d9; border: 1px solid #30363d;
            padding: 4px 8px; border-radius: 4px; font-size: 13px; cursor: pointer;
        }
        #hint {
            position: absolute; bottom: 16px; left: 16px; background: rgba(0,0,0,0.7);
            padding: 8px 14px; border-radius: 6px; font-size: 13px; color: #8b949e;
            pointer-events: none;
        }
        /* itch.io plays HTML5 games in a cross-origin iframe. Focusing the
           canvas from script does not focus the iframe itself, so WASD never
           arrives until the player clicks inside this frame. This overlay
           is that click. */
        #click-play {
            position: absolute; inset: 0; z-index: 20;
            background: rgba(13, 17, 23, 0.72);
            display: none; flex-direction: column;
            align-items: center; justify-content: center; gap: 10px;
            cursor: pointer; user-select: none;
        }
        #click-play.show { display: flex; }
        #click-play .big { font-size: 28px; color: #f0f6fc; }
        #click-play .sub { font-size: 14px; color: #8b949e; }
    </style>
</head>
<body>
    <div id="level-picker">
        <label for="level-select">Level:</label>
        <select id="level-select"></select>
    </div>
    <div id="loading">
        <div id="phase">Starting&hellip;</div>
        <div id="bar"><div id="fill" class="indet"></div></div>
        <div id="note"></div>
    </div>
    <div id="click-play">
        <div class="big">Click to play</div>
        <div class="sub">WASD drive &middot; Space brake &middot; Shift turbo</div>
    </div>
    <canvas id="canvas" tabindex="0"></canvas>
    <div id="hint">WASD &ndash; drive &nbsp;|&nbsp; Space &ndash; brake &nbsp;|&nbsp; Q/E &ndash; roll &nbsp;|&nbsp; Alt &ndash; jump &nbsp;|&nbsp; Shift &ndash; turbo</div>
    <script type="module">
        const LEVELS = __LEVELS_JSON__;
        const DEFAULT_LEVEL = 'fostral';

        function readHashLevel() {
            const hash = window.location.hash.replace(/^#/, '');
            for (const pair of hash.split('&')) {
                const [k, v] = pair.split('=');
                if (k === 'level' && v) return decodeURIComponent(v);
            }
            return null;
        }

        const select = document.getElementById('level-select');
        const bundled = new Set(__BUNDLED_JSON__);
        for (const [id, title] of LEVELS) {
            if (!bundled.has(id)) continue;
            const opt = document.createElement('option');
            opt.value = id;
            opt.textContent = title;
            select.appendChild(opt);
        }
        const initial = readHashLevel() || DEFAULT_LEVEL;
        if ([...select.options].some(o => o.value === initial)) select.value = initial;
        window.vangeSelectedLevel = () => select.value;
        window.vangeDataBase = './data-0';
        select.addEventListener('change', () => {
            window.location.hash = 'level=' + select.value;
            window.location.reload();
        });

        const loading = document.getElementById('loading');
        const phase = document.getElementById('phase');
        const fill = document.getElementById('fill');
        const note = document.getElementById('note');
        const canvas = document.getElementById('canvas');
        const clickPlay = document.getElementById('click-play');
        let ready = false;
        let armed = false;

        function inPicker(el) {
            return !!(el && el.closest && el.closest('#level-picker'));
        }

        function grabKeys() {
            window.focus();
            canvas.focus();
        }

        function arm() {
            armed = true;
            clickPlay.classList.remove('show');
            grabKeys();
        }

        window.vangePhase = (label) => {
            phase.textContent = label;
            fill.classList.add('indet');
            fill.style.width = '';
        };
        window.vangeProgress = (label, loaded, total) => {
            phase.textContent = label;
            if (total > 0) {
                fill.classList.remove('indet');
                fill.style.width = (100 * loaded / total).toFixed(1) + '%';
                note.textContent = (loaded / 1048576).toFixed(1) + ' / '
                                 + (total / 1048576).toFixed(1) + ' MB';
            }
        };
        window.vangeProgressDone = () => {
            ready = true;
            loading.classList.remove('error');
            loading.classList.add('gone');
            if (armed) grabKeys();
            else clickPlay.classList.add('show');
        };
        window.vangeProgressError = (message) => {
            ready = true;
            loading.classList.add('error');
            phase.textContent = message;
            fill.style.width = '0';
            fill.classList.remove('indet');
            if (!armed) clickPlay.classList.add('show');
        };

        // Do not preventDefault: that stops the iframe from taking
        // keyboard focus, which is why WASD died after Click to play.
        clickPlay.addEventListener('pointerdown', () => arm());

        // A click on itch.io's own "Run game" button is on the parent
        // page, so it does not arm us. Only a pointer event inside this
        // iframe focuses the frame and lets winit see WASD.
        document.addEventListener('pointerdown', (e) => {
            if (inPicker(e.target)) return;
            arm();
        });
        document.addEventListener('mousemove', (e) => {
            if (!armed || inPicker(e.target)) return;
            if (document.activeElement && document.activeElement.tagName === 'SELECT') return;
            if (document.activeElement !== canvas) grabKeys();
        }, { passive: true });

        // winit listens on the canvas, not the document. If focus drifted
        // to the <select> or the body, steal it back and replay the key
        // (Chrome/itch; Firefox ignores untrusted KeyboardEvents).
        document.addEventListener('keydown', (e) => {
            if (inPicker(e.target) || e.target === select) return;
            if (!armed) {
                if (!ready) return;
                arm();
            }
            if (document.activeElement !== canvas) {
                grabKeys();
                canvas.dispatchEvent(new KeyboardEvent(e.type, {
                    code: e.code, key: e.key, keyCode: e.keyCode,
                    bubbles: false, cancelable: true,
                }));
            }
            if (e.code === 'Space' || e.code.startsWith('Arrow')) {
                e.preventDefault();
            }
        });

        import('./web.js').then(({ default: init }) => init()).catch((e) => {
            if (!String(e).includes('Using exceptions for control flow')) {
                window.vangeProgressError(String(e));
                throw e;
            }
        });
    </script>
</body>
</html>
"""


def die(msg: str) -> None:
    sys.exit(f"error: {msg}")


def find_data_dir(explicit: Path | None) -> Path:
    if explicit is not None:
        if not explicit.is_dir():
            die(f"data dir {explicit} does not exist")
        return explicit
    repo = Path(__file__).resolve().parents[1]
    for candidate in (repo / "docs" / "data-0", repo / "work"):
        if (candidate / COMMON).is_file():
            return candidate
    die(
        "no data dir with common.zip. Pass --data-dir, run "
        "`python3 tools/pack-data.py ...`, or "
        "`gh release download data-0 --dir docs/data-0`"
    )
    raise AssertionError


def write_index(dest: Path, bundled: list[str]) -> None:
    import json

    html = INDEX_HTML.replace("__LEVELS_JSON__", json.dumps(LEVELS))
    html = html.replace("__BUNDLED_JSON__", json.dumps(bundled))
    dest.write_text(html, encoding="utf-8")


def pack(
    wasm_dir: Path,
    data_dir: Path,
    out_zip: Path,
    levels: list[str],
    include_routes: bool,
) -> None:
    for name in WASM_FILES:
        if not (wasm_dir / name).is_file():
            die(f"missing {wasm_dir / name} — run `make web` first")
    common = data_dir / COMMON
    if not common.is_file():
        die(f"missing {common}")

    staging = out_zip.parent / (out_zip.stem + "-staging")
    if staging.exists():
        shutil.rmtree(staging)
    staging.mkdir(parents=True)

    for name in WASM_FILES:
        shutil.copy2(wasm_dir / name, staging / name)

    if include_routes:
        repo_docs = Path(__file__).resolve().parents[1] / "docs"
        for rel in OPTIONAL_SITE:
            src = repo_docs / rel
            if src.is_file():
                dest = staging / rel
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(src, dest)

    data_out = staging / "data-0"
    data_out.mkdir()
    shutil.copy2(common, data_out / COMMON)
    bundled: list[str] = []
    for lid in levels:
        src = data_dir / f"{lid}.zip"
        if not src.is_file():
            die(f"missing level archive {src}")
        shutil.copy2(src, data_out / src.name)
        bundled.append(lid)

    write_index(staging / "index.html", bundled)

    out_zip.parent.mkdir(parents=True, exist_ok=True)
    if out_zip.exists():
        out_zip.unlink()
    with zipfile.ZipFile(out_zip, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as zf:
        for path in sorted(staging.rglob("*")):
            if path.is_dir():
                continue
            zf.write(path, arcname=path.relative_to(staging).as_posix())

    size_mb = out_zip.stat().st_size / (1024 * 1024)
    print(f"wrote {out_zip} ({size_mb:.1f} MiB)")
    print("levels:", ", ".join(bundled))
    print("itch.io: HTML project, upload this zip, index file = index.html")
    shutil.rmtree(staging)


def main() -> None:
    repo = Path(__file__).resolve().parents[1]
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--wasm-dir", type=Path, default=repo / "docs",
                    help="directory with web.js and web_bg.wasm (default: docs/)")
    ap.add_argument("--data-dir", type=Path, default=None,
                    help="directory with common.zip and <level>.zip")
    ap.add_argument("--out", type=Path, default=repo / "work" / "vange-rs-web.zip",
                    help="output zip path (default: work/vange-rs-web.zip)")
    ap.add_argument("--levels", default=",".join(DEFAULT_LEVELS),
                    help="comma-separated level ids to bundle (default: fostral)")
    ap.add_argument("--all-levels", action="store_true",
                    help="bundle every <id>.zip present in --data-dir")
    ap.add_argument("--with-routes", action="store_true",
                    help="also include /mesh /voxel /ray pages (needs embed.js)")
    args = ap.parse_args()

    data_dir = find_data_dir(args.data_dir)
    if args.all_levels:
        levels = sorted(
            p.stem
            for p in data_dir.glob("*.zip")
            if p.name != COMMON
        )
        if not levels:
            die(f"no level zips in {data_dir}")
    else:
        levels = [s.strip() for s in args.levels.split(",") if s.strip()]
        if not levels:
            die("no levels selected")

    pack(args.wasm_dir, data_dir, args.out, levels, args.with_routes)


if __name__ == "__main__":
    main()
