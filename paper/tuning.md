# Per-method tuning

Level fostral, pitch 0°, 400x260, view distance 600, averaged over 3 viewpoints.

Selection rule: the cheapest setting within 1 percentage points of that method's own best error, where error is see-through + speckle.

## RayTraced

| setting | GPU ms | see-through | speckle | error | depth p50 |
|---|---|---|---|---|---|
| 16 steps | 0.24 | 6.4% | 0.5% | 6.9% | 1.3u |
| 32 steps | 0.31 | 3.4% | 0.5% | 3.9% | 0.6u |
| 64 steps | 0.52 | 1.8% | 0.5% | 2.3% | 0.3u |
| 128 steps **<-** | 0.87 | 0.8% | 0.4% | 1.2% | 0.2u |
| 256 steps | 1.29 | 0.4% | 0.3% | 0.6% | 0.1u |

## Painter

| setting | GPU ms | see-through | speckle | error | depth p50 |
|---|---|---|---|---|---|
| (none) **<-** | 2.59 | 0.0% | 0.1% | 0.1% | 0.0u |

## Sliced

| setting | GPU ms | see-through | speckle | error | depth p50 |
|---|---|---|---|---|---|
| 32 layers | 0.31 | 15.5% | 1.2% | 16.7% | 26.9u |
| 64 layers | 0.52 | 9.7% | 3.4% | 13.1% | 14.9u |
| 128 layers | 0.80 | 6.8% | 6.4% | 13.2% | 5.8u |
| 256 layers | 1.11 | 4.4% | 6.9% | 11.3% | 2.6u |
| 512 layers **<-** | 1.54 | 3.0% | 3.5% | 6.5% | 1.3u |

## Scattered

| setting | GPU ms | see-through | speckle | error | depth p50 |
|---|---|---|---|---|---|
| density 1,1,1 | 0.15 | 69.4% | 0.3% | 69.7% | 43.8u |
| density 2,2,2 | 0.61 | 50.2% | 2.2% | 52.4% | 66.8u |
| density 3,3,3 | 1.61 | 40.5% | 4.5% | 45.0% | 78.1u |
| density 4,4,4 **<-** | 2.34 | 33.9% | 5.3% | 39.2% | 76.9u |

## RayVoxel

| setting | GPU ms | see-through | speckle | error | depth p50 |
|---|---|---|---|---|---|
| grid 4,8,2, 40 steps | 0.79 | 2.3% | 0.1% | 2.5% | 0.5u |
| grid 4,8,2, 100 steps **<-** | 1.17 | 0.1% | 0.2% | 0.3% | 0.5u |
| grid 4,8,2, 200 steps | 1.75 | 0.1% | 0.2% | 0.3% | 0.5u |
| grid 4,8,2, 400 steps | 2.56 | 0.1% | 0.2% | 0.3% | 0.5u |
| grid 2,4,1, 40 steps | 0.93 | 3.3% | 0.1% | 3.4% | 0.5u |
| grid 2,4,1, 100 steps | 1.20 | 0.1% | 0.2% | 0.3% | 0.5u |
| grid 2,4,1, 200 steps | 1.64 | 0.1% | 0.2% | 0.3% | 0.5u |
| grid 2,4,1, 400 steps | 2.69 | 0.1% | 0.2% | 0.3% | 0.5u |

## Mesh

| setting | GPU ms | see-through | speckle | error | depth p50 |
|---|---|---|---|---|---|
| q=0.0 | 0.11 | 1.8% | 0.1% | 1.8% | 0.7u |
| q=0.25 | 0.12 | 1.7% | 0.1% | 1.8% | 0.7u |
| q=0.5 **<-** | 0.17 | 0.9% | 0.1% | 1.0% | 0.5u |
| q=0.75 | 0.22 | 0.6% | 0.1% | 0.7% | 0.4u |
| q=1.0 | 0.31 | 0.6% | 0.1% | 0.7% | 0.3u |

## Chosen

```python
METHODS = [
    ("RayTraced", "RayTraced", ["--ray-steps", "128"], ...),  # 128 steps
    ("Painter", "Painted", [], ...),  # (none)
    ("Sliced", "Sliced", ["--slice-layers", "512"], ...),  # 512 layers
    ("Scattered", "Scattered", ["--scatter-density", "4,4,4"], ...),  # density 4,4,4
    ("RayVoxel", "RayVoxelTraced", ["--voxel-size", "4,8,2", "--voxel-steps", "100"], ...),  # grid 4,8,2, 100 steps
    ("Mesh", "Mesh", ["--mesh-quality", "0.5"], ...),  # q=0.5
]
```
\n## Mesh at publication resolution\n\nLevel fostral, pitch 0°, 1280x800, view distance 600, averaged over 3 viewpoints.\n\n| setting | GPU ms | see-through | speckle | error | depth p50 |\n|---|---|---|---|---|---|\n| q=0.0 | 0.44 | 3.9% | 0.0% | 3.9% | 1.8u |\n| q=0.25 **<-** | 0.51 | 0.7% | 0.0% | 0.8% | 0.7u |\n| q=0.5 | 0.59 | 0.3% | 0.1% | 0.3% | 0.4u |\n| q=0.75 | 0.78 | 0.2% | 0.1% | 0.3% | 0.4u |\n| q=1.0 | 1.05 | 0.2% | 0.1% | 0.3% | 0.3u |\n\nThe one-percentage-point selection rule chooses q=0.25 at publication resolution.\n
