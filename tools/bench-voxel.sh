#!/usr/bin/env bash
# Drives the voxel-terrain reftest/benchmark at three camera elevations.
#
# Renders into `snapshots/voxel-bench/<stage>-<angle>.png` and writes
# per-frame timing summaries into `snapshots/voxel-bench/<stage>-<angle>.json`,
# so each pass produces a comparable image+number for the same Fostral region.
#
# Usage:
#   tools/bench-voxel.sh <stage>
# Example:
#   tools/bench-voxel.sh baseline
#   tools/bench-voxel.sh c1-fix
#
# Requires `docs/data-0/{fostral,common}.zip`. CI builds these via the
# data-0 release; locally they ship next to the demo build.

set -euo pipefail

STAGE="${1:?stage name required (e.g. baseline, c1-fix)}"
OUT_DIR="snapshots/voxel-bench"
LEVEL_ZIP="docs/data-0/fostral.zip"
COMMON_ZIP="docs/data-0/common.zip"

# Locked benchmark location: Fostral citadel cluster around (576, 896).
# Has clearly visible double-level terrain and the canonical tunnel mouths,
# so C4-class fixes (low_alt straddling) show up here.
TARGET="576,896,80"
DISTANCE="350"
WIDTH="640"
HEIGHT="480"
# Warmup needs to be high enough to fully drain the bake queue. With the
# C1 budget bug, only one rect per frame is processed and a Fostral-sized
# level needs ~50–80 frames just to bake; round up generously so the timed
# frames measure steady-state.
WARMUP="300"
FRAMES="60"

mkdir -p "$OUT_DIR"

for ELEV in 30 60 90; do
    OUT_PNG="${OUT_DIR}/${STAGE}-${ELEV}.png"
    OUT_JSON="${OUT_DIR}/${STAGE}-${ELEV}.json"
    echo "=== ${STAGE} @ ${ELEV}deg -> ${OUT_PNG} ==="
    cargo run --release --bin level --quiet -- \
        --snapshot "$OUT_PNG" \
        --bench-out "$OUT_JSON" \
        --terrain RayVoxelTraced \
        --level-zip "$LEVEL_ZIP" \
        --common-zip "$COMMON_ZIP" \
        --width "$WIDTH" \
        --height "$HEIGHT" \
        --cam-target "$TARGET" \
        --cam-distance "$DISTANCE" \
        --cam-elev "$ELEV" \
        --warmup "$WARMUP" \
        --frames "$FRAMES"
done

echo
echo "Summary:"
for ELEV in 30 60 90; do
    f="${OUT_DIR}/${STAGE}-${ELEV}.json"
    if [ -f "$f" ]; then
        printf "  %s @ %s°: " "$STAGE" "$ELEV"
        # cheap JSON read — avoid jq dependency
        grep -E '"(min|avg|max)_ms"' "$f" | tr -d ',' | tr '\n' ' '
        echo
    fi
done
