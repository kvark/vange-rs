# Per-method tuning

Level fostral, pitch 0°, 400x260, view distance 600, averaged over 3 viewpoints.

Selection rule: the cheapest setting within 1 percentage points of that method's own best error, where error is see-through + speckle.

## RayTraced

| setting | GPU ms | see-through | speckle | error | depth p50 |
|---|---|---|---|---|---|
| (none) **<-** | 0.11 | 64.5% | 1.6% | 66.1% | 12.7u |

## Painter

| setting | GPU ms | see-through | speckle | error | depth p50 |
|---|---|---|---|---|---|
| (none) **<-** | 2.70 | 4.0% | 0.5% | 4.5% | 17.6u |

## Sliced

| setting | GPU ms | see-through | speckle | error | depth p50 |
|---|---|---|---|---|---|
| 32 layers | 0.19 | 17.2% | 2.4% | 19.7% | 54.9u |
| 64 layers | 0.32 | 13.1% | 4.8% | 17.8% | 39.6u |
| 128 layers | 0.48 | 10.1% | 8.2% | 18.3% | 27.6u |
| 256 layers | 0.67 | 7.1% | 10.9% | 18.0% | 20.8u |
| 512 layers **<-** | 1.44 | 5.4% | 4.4% | 9.8% | 20.2u |

## Scattered

| setting | GPU ms | see-through | speckle | error | depth p50 |
|---|---|---|---|---|---|
| density 1,1,1 | 0.19 | 79.3% | 0.3% | 79.6% | 90.2u |
| density 2,2,2 | 0.78 | 66.2% | 5.6% | 71.8% | 97.8u |
| density 3,3,3 | 2.48 | 59.4% | 4.4% | 63.8% | 105.7u |
| density 4,4,4 **<-** | 3.84 | 53.2% | 4.1% | 57.3% | 106.6u |

## RayVoxel

| setting | GPU ms | see-through | speckle | error | depth p50 |
|---|---|---|---|---|---|
| grid 4,8,2, 40 steps **<-** | 0.75 | 4.8% | 0.6% | 5.5% | 17.4u |
| grid 4,8,2, 100 steps | 0.94 | 4.1% | 0.7% | 4.8% | 17.7u |
| grid 4,8,2, 200 steps | 1.19 | 4.1% | 0.7% | 4.8% | 17.7u |
| grid 4,8,2, 400 steps | 1.38 | 4.1% | 0.7% | 4.8% | 17.7u |
| grid 2,4,1, 40 steps | 0.72 | 6.4% | 0.5% | 6.9% | 17.0u |
| grid 2,4,1, 100 steps | 1.02 | 4.1% | 0.6% | 4.7% | 17.7u |
| grid 2,4,1, 200 steps | 1.10 | 4.1% | 0.6% | 4.7% | 17.7u |
| grid 2,4,1, 400 steps | 1.57 | 4.1% | 0.6% | 4.7% | 17.7u |

## Mesh

| setting | GPU ms | see-through | speckle | error | depth p50 |
|---|---|---|---|---|---|
| q=0.0 **<-** | 0.11 | 4.3% | 0.1% | 4.4% | 20.5u |
| q=0.25 | 0.11 | 4.4% | 0.1% | 4.5% | 22.6u |
| q=0.5 | 0.15 | 4.3% | 0.2% | 4.4% | 20.4u |
| q=0.75 | 0.19 | 4.2% | 0.2% | 4.4% | 20.7u |
| q=1.0 | 0.23 | 4.2% | 0.2% | 4.4% | 18.0u |

## Chosen

```python
METHODS = [
    ("RayTraced", "RayTraced", [], ...),  # (none)
    ("Painter", "Painted", [], ...),  # (none)
    ("Sliced", "Sliced", ["--slice-layers", "512"], ...),  # 512 layers
    ("Scattered", "Scattered", ["--scatter-density", "4,4,4"], ...),  # density 4,4,4
    ("RayVoxel", "RayVoxelTraced", ["--voxel-size", "4,8,2", "--voxel-steps", "40"], ...),  # grid 4,8,2, 40 steps
    ("Mesh", "Mesh", ["--mesh-quality", "0.0"], ...),  # q=0.0
]
```
