# Per-method tuning

Level fostral, pitch 0°, 400x260, view distance 600, averaged over 4 viewpoints.

Selection rule: the cheapest setting within 1 percentage points of that method's own best error, where error is see-through + speckle.

## RayTraced

| setting | GPU ms | see-through | speckle | error |
|---|---|---|---|---|
| (none) **<-** | 5.16 | 48.4% | 1.8% | 50.3% |

## Painter

| setting | GPU ms | see-through | speckle | error |
|---|---|---|---|---|
| (none) **<-** | 579.48 | 2.3% | 0.2% | 2.5% |

## Sliced

| setting | GPU ms | see-through | speckle | error |
|---|---|---|---|---|
| 32 layers | 3.64 | 82.9% | 0.0% | 82.9% |
| 64 layers | 6.98 | 77.6% | 0.7% | 78.3% |
| 128 layers | 15.46 | 61.2% | 1.8% | 63.0% |
| 256 layers **<-** | 41.09 | 4.7% | 9.3% | 14.0% |
| 512 layers | 46.89 | 4.7% | 9.3% | 14.0% |

## Scattered

| setting | GPU ms | see-through | speckle | error |
|---|---|---|---|---|
| density 1,1,1 | 5.48 | 62.0% | 0.9% | 62.9% |
| density 2,2,2 | 27.35 | 35.8% | 6.9% | 42.7% |
| density 3,3,3 | 77.28 | 25.9% | 5.8% | 31.7% |
| density 4,4,4 **<-** | 151.55 | 21.9% | 4.9% | 26.8% |

## RayVoxel

| setting | GPU ms | see-through | speckle | error |
|---|---|---|---|---|
| grid 4,8,2, 40 steps | 46.27 | 6.4% | 0.3% | 6.7% |
| grid 4,8,2, 100 steps **<-** | 67.20 | 2.5% | 0.4% | 2.9% |
| grid 4,8,2, 200 steps | 87.85 | 2.4% | 0.4% | 2.8% |
| grid 4,8,2, 400 steps | 127.11 | 2.4% | 0.4% | 2.8% |
| grid 2,4,1, 40 steps | — | — | — | not available: panicked at bin/level/headless.rs:315:6: |
| grid 2,4,1, 100 steps | — | — | — | not available: panicked at bin/level/headless.rs:315:6: |
| grid 2,4,1, 200 steps | — | — | — | not available: panicked at bin/level/headless.rs:315:6: |
| grid 2,4,1, 400 steps | — | — | — | not available: panicked at bin/level/headless.rs:315:6: |

## Mesh

| setting | GPU ms | see-through | speckle | error |
|---|---|---|---|---|
| q=0.0 **<-** | 11.44 | 3.0% | 0.1% | 3.1% |
| q=0.25 | 13.34 | 2.6% | 0.1% | 2.7% |
| q=0.5 | 25.54 | 2.5% | 0.1% | 2.6% |
| q=0.75 | 28.46 | 2.5% | 0.1% | 2.6% |
| q=1.0 | 40.51 | 2.5% | 0.1% | 2.6% |

## Chosen

```python
METHODS = [
    ("RayTraced", "RayTraced", [], ...),  # (none)
    ("Painter", "Painted", [], ...),  # (none)
    ("Sliced", "Sliced", ["--slice-layers", "256"], ...),  # 256 layers
    ("Scattered", "Scattered", ["--scatter-density", "4,4,4"], ...),  # density 4,4,4
    ("RayVoxel", "RayVoxelTraced", ["--voxel-size", "4,8,2", "--voxel-steps", "100"], ...),  # grid 4,8,2, 100 steps
    ("Mesh", "Mesh", ["--mesh-quality", "0.0"], ...),  # q=0.0
]
```
