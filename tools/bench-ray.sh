#!/usr/bin/env bash
# Drives the ray-traced (WebGL fallback) terrain bench at three camera
# elevations. Mirrors `bench-voxel.sh` but uses --terrain RayTraced so
# the same Fostral region is rendered through the fragment-only path
# the WebGL2 build falls back to.
#
# Outputs: snapshots/ray-bench/<stage>-<angle>.{png,json}
#
# Usage:
#   tools/bench-ray.sh <stage>

set -euo pipefail

STAGE="${1:?stage name required}"
OUT_DIR="snapshots/ray-bench"
LEVEL_ZIP="docs/data-0/fostral.zip"
COMMON_ZIP="docs/data-0/common.zip"

# Same target as the voxel bench so PNGs are comparable across paths.
TARGET="576,896,80"
DISTANCE="350"
WIDTH="640"
HEIGHT="480"
WARMUP="40"
FRAMES="60"

mkdir -p "$OUT_DIR"

for ELEV in 30 60 90; do
    OUT_PNG="${OUT_DIR}/${STAGE}-${ELEV}.png"
    OUT_JSON="${OUT_DIR}/${STAGE}-${ELEV}.json"
    echo "=== ${STAGE} @ ${ELEV}deg -> ${OUT_PNG} ==="
    cargo run --release --bin level --quiet -- \
        --snapshot "$OUT_PNG" \
        --bench-out "$OUT_JSON" \
        --terrain RayTraced \
        --shadow-ray \
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
        grep -E '"(min|avg|max)_ms"' "$f" | tr -d ',' | tr '\n' ' '
        echo
    fi
done
