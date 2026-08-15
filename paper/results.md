## Devices

| label | adapter | backend / type | driver | config |
|---|---|---|---|---|
| AMD Radeon 780M Graphics (RADV PHOENIX) | AMD Radeon 780M Graphics (RADV PHOENIX) | Vulkan / IntegratedGpu | Mesa 25.2.8-0ubuntu0.24.04.1 | 1280x800, far 600, 40 frames; shadows ray-traced 1024x1024 |
| AMD Radeon RX 7900 XT (RADV NAVI31) | AMD Radeon RX 7900 XT (RADV NAVI31) | Vulkan / DiscreteGpu | Mesa 26.0.3-1ubuntu1 | 1280x800, far 600, 40 frames; shadows ray-traced 1024x1024 |
| NVIDIA GeForce RTX 5070 | NVIDIA GeForce RTX 5070 | Vulkan / DiscreteGpu | 595.71.05 | 1280x800, far 600, 40 frames; shadows ray-traced 1024x1024 |

## Frame time, avg_ms (ms)

From GPU timestamp queries: the device's own view of how long its work took, with no submission or round trip in it.


### AMD Radeon 780M Graphics (RADV PHOENIX)

| view | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| river @ 0° | 4.8 | 7.7 | 11.1 | 6.9 | 8.5 | 4.3 | 3.8 |
| hangar @ 0° | 4.5 | 8.0 | 8.7 | 29.3 | 8.6 | 4.2 | 3.9 |
| ramp @ 0° | 4.2 | 6.2 | 11.5 | 6.4 | 8.3 | 4.3 | 3.8 |
| portal @ -30° | 4.9 | 8.3 | 7.3 | 7.0 | 13.0 | 4.3 | 3.9 |
| entrance @ -30° | 4.5 | 7.6 | 7.6 | 11.8 | 14.3 | 4.3 | 4.1 |
| river-down @ -30° | 5.2 | 7.8 | 8.6 | 7.1 | 11.1 | 4.5 | 4.2 |
| stash @ -60° | 4.7 | 7.5 | 7.7 | 7.1 | 22.4 | 4.2 | 3.6 |
| copterig charger @ -60° | 5.1 | 8.2 | 10.7 | 7.2 | 16.4 | 4.4 | 4.3 |
| wires @ -60° | 4.7 | 8.4 | 8.5 | 6.6 | 21.4 | 4.0 | 3.8 |
| spiral charger @ -90° | 5.1 | 8.2 | 10.4 | 7.3 | 25.6 | 4.5 | 4.2 |
| gorb charger @ -90° | 4.8 | 6.6 | 8.3 | 6.6 | 26.2 | 3.8 | 3.4 |
| secret @ -90° | 5.1 | 8.1 | 11.8 | 7.3 | 14.1 | 4.3 | 3.8 |

Arithmetic mean over the views at each pitch:

| pitch | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| 0° | 4.491 | 7.317 | 10.430 | 14.170 | 8.447 | 4.284 | 3.832 |
| -30° | 4.876 | 7.923 | 7.844 | 8.649 | 12.822 | 4.367 | 4.047 |
| -60° | 4.859 | 8.010 | 8.975 | 6.962 | 20.053 | 4.198 | 3.901 |
| -90° | 5.002 | 7.637 | 10.155 | 7.068 | 21.990 | 4.207 | 3.835 |

### AMD Radeon RX 7900 XT (RADV NAVI31)

| view | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| river @ 0° | 0.9 | 1.3 | 2.1 | 1.2 | 2.8 | 0.5 | 0.5 |
| hangar @ 0° | 0.8 | 1.3 | 1.7 | 21.5 | 3.0 | 0.5 | 0.5 |
| ramp @ 0° | 0.7 | 1.0 | 2.1 | 1.1 | 3.9 | 0.4 | 0.4 |
| portal @ -30° | 0.6 | 1.3 | 1.5 | 1.2 | 4.8 | 0.5 | 0.5 |
| entrance @ -30° | 0.8 | 1.3 | 1.6 | 2.0 | 5.5 | 0.5 | 0.5 |
| river-down @ -30° | 0.9 | 1.3 | 1.7 | 1.2 | 4.0 | 0.5 | 0.5 |
| stash @ -60° | 0.9 | 1.3 | 1.6 | 1.2 | 7.7 | 0.5 | 0.4 |
| copterig charger @ -60° | 1.0 | 1.4 | 2.2 | 1.2 | 5.1 | 0.5 | 0.5 |
| wires @ -60° | 0.9 | 1.4 | 1.7 | 1.1 | 8.1 | 0.4 | 0.4 |
| spiral charger @ -90° | 1.0 | 1.4 | 2.0 | 1.2 | 7.0 | 0.5 | 0.5 |
| gorb charger @ -90° | 0.9 | 1.1 | 1.7 | 1.1 | 8.5 | 0.5 | 0.4 |
| secret @ -90° | 1.0 | 1.3 | 2.3 | 1.2 | 3.9 | 0.5 | 0.5 |

Arithmetic mean over the views at each pitch:

| pitch | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| 0° | 0.827 | 1.193 | 2.007 | 7.946 | 3.234 | 0.484 | 0.476 |
| -30° | 0.797 | 1.309 | 1.571 | 1.453 | 4.787 | 0.486 | 0.483 |
| -60° | 0.913 | 1.354 | 1.843 | 1.180 | 6.945 | 0.474 | 0.469 |
| -90° | 0.960 | 1.283 | 2.009 | 1.163 | 6.457 | 0.487 | 0.494 |

### NVIDIA GeForce RTX 5070

| view | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| river @ 0° | 0.8 | 1.1 | 2.4 | 1.4 | 2.7 | 0.5 | 0.5 |
| hangar @ 0° | 0.7 | 1.2 | 1.8 | 2.5 | 3.0 | 0.5 | 0.5 |
| ramp @ 0° | 0.6 | 1.0 | 2.6 | 1.3 | 4.1 | 0.5 | 0.5 |
| portal @ -30° | 0.8 | 1.2 | 1.4 | 1.4 | 4.3 | 0.5 | 0.6 |
| entrance @ -30° | 0.7 | 1.1 | 1.5 | 1.9 | 4.8 | 0.5 | 0.5 |
| river-down @ -30° | 0.9 | 1.2 | 1.7 | 1.4 | 3.4 | 0.5 | 0.7 |
| stash @ -60° | 0.8 | 1.1 | 1.4 | 1.3 | 6.2 | 0.5 | 0.5 |
| copterig charger @ -60° | 0.9 | 1.2 | 2.3 | 1.4 | 4.4 | 0.5 | 0.6 |
| wires @ -60° | 0.7 | 1.2 | 1.6 | 1.3 | 6.5 | 0.5 | 0.5 |
| spiral charger @ -90° | 0.9 | 1.2 | 2.0 | 1.3 | 6.6 | 0.5 | 0.6 |
| gorb charger @ -90° | 0.8 | 1.0 | 1.6 | 1.2 | 6.8 | 0.4 | 0.5 |
| secret @ -90° | 0.9 | 1.2 | 2.5 | 1.4 | 3.7 | 0.5 | 0.5 |

Arithmetic mean over the views at each pitch:

| pitch | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| 0° | 0.724 | 1.094 | 2.275 | 1.730 | 3.269 | 0.487 | 0.511 |
| -30° | 0.811 | 1.194 | 1.550 | 1.540 | 4.173 | 0.489 | 0.624 |
| -60° | 0.784 | 1.196 | 1.782 | 1.336 | 5.700 | 0.475 | 0.536 |
| -90° | 0.847 | 1.137 | 2.028 | 1.313 | 5.712 | 0.471 | 0.532 |

## Accuracy: see-through / covers-sky / speckle (%)

Expected to be device-independent; baseline taken from **AMD Radeon 780M Graphics (RADV PHOENIX)** and cross-checked below. `see-through` is solid terrain left as background and `covers-sky` is background filled in — only the first moves when a renderer is really missing geometry, and both move together when the reference is the one disagreeing. `speckle` is what depth agreement cannot see: pixels whose distance disagrees with their own neighbourhood, in excess of the reference doing the same.

| view | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| river @ 0° | 3.6 / 3.2 / 0.2 | 5.1 / 2.9 / 0.1 | 4.1 / 3.0 / 1.1 | 29.4 / 0.6 / 1.6 | 3.0 / 3.3 / 0.1 | 3.4 / 3.2 / 0.0 | 3.1 / 3.2 / 0.0 |
| hangar @ 0° | 7.2 / 6.1 / 0.3 | 7.7 / 5.4 / 0.2 | 7.3 / 6.3 / 2.7 | 74.5 / 1.5 / 1.4 | 6.1 / 6.4 / 0.1 | 16.3 / 5.5 / 0.0 | 6.5 / 6.3 / 0.1 |
| ramp @ 0° | 0.2 / 0.2 / 0.5 | 0.5 / 0.2 / 0.4 | 0.2 / 0.2 / 4.3 | 19.1 / 0.0 / 2.5 | 0.2 / 0.2 / 0.1 | 0.2 / 0.2 / 0.0 | 0.2 / 0.2 / 0.1 |
| portal @ -30° | 2.1 / 4.3 / 1.1 | 1.8 / 4.6 / 0.4 | 2.6 / 3.4 / 1.7 | 4.5 / 3.1 / 4.3 | 1.9 / 4.6 / 0.2 | 2.0 / 3.8 / 0.1 | 1.9 / 4.5 / 0.1 |
| entrance @ -30° | 4.2 / 6.0 / 0.6 | 3.7 / 6.2 / 0.2 | 5.8 / 5.2 / 0.8 | 31.7 / 0.6 / 2.2 | 4.5 / 6.2 / 0.1 | 4.1 / 6.0 / 0.1 | 3.8 / 6.1 / 0.1 |
| river-down @ -30° | 7.7 / 8.6 / 0.8 | 7.0 / 8.9 / 0.3 | 8.2 / 7.7 / 0.7 | 12.3 / 5.3 / 3.8 | 7.1 / 9.0 / 0.1 | 8.1 / 7.7 / 0.1 | 7.2 / 8.8 / 0.1 |
| stash @ -60° | 0.0 / 2.3 / 0.5 | 0.0 / 2.5 / 0.3 | 0.0 / 2.3 / 0.2 | 0.0 / 2.3 / 5.3 | 1.5 / 1.3 / 0.2 | 0.0 / 2.3 / 0.1 | 0.0 / 2.3 / 0.1 |
| copterig charger @ -60° | 0.0 / 8.9 / 0.4 | 0.0 / 9.1 / 0.2 | 0.0 / 9.0 / 0.2 | 0.2 / 8.4 / 4.3 | 1.7 / 6.5 / 0.1 | 0.0 / 9.0 / 0.1 | 0.0 / 9.0 / 0.1 |
| wires @ -60° | 0.0 / 1.8 / 0.9 | 0.0 / 1.8 / 0.4 | 0.0 / 1.8 / 0.3 | 0.0 / 1.8 / 6.4 | 0.7 / 1.2 / 0.2 | 0.0 / 1.8 / 0.1 | 0.0 / 1.8 / 0.1 |
| spiral charger @ -90° | 0.0 / 13.9 / 0.5 | 0.0 / 13.9 / 0.3 | 0.0 / 13.9 / 0.3 | 0.0 / 13.8 / 3.5 | 1.3 / 12.7 / 0.2 | 0.0 / 13.9 / 0.2 | 0.0 / 13.9 / 0.2 |
| gorb charger @ -90° | 0.0 / 0.0 / 0.5 | 0.0 / 0.0 / 0.2 | 0.0 / 0.0 / 0.2 | 0.0 / 0.0 / 4.1 | 0.0 / 0.0 / 0.2 | 0.0 / 0.0 / 0.1 | 0.0 / 0.0 / 0.1 |
| secret @ -90° | 0.1 / 7.0 / 0.5 | 0.0 / 7.0 / 0.2 | 0.0 / 7.0 / 0.2 | 0.3 / 7.0 / 2.6 | 0.0 / 7.0 / 0.2 | 0.0 / 7.0 / 0.1 | 0.0 / 7.0 / 0.1 |

Arithmetic mean over the views at each pitch, see-through / speckle (%):

| pitch | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| 0° | 3.7 / 0.4 | 4.4 / 0.3 | 3.9 / 2.7 | 41.0 / 1.8 | 3.1 / 0.1 | 6.6 / 0.0 | 3.3 / 0.1 |
| -30° | 4.7 / 0.8 | 4.2 / 0.3 | 5.5 / 1.1 | 16.2 / 3.4 | 4.5 / 0.1 | 4.7 / 0.1 | 4.3 / 0.1 |
| -60° | 0.0 / 0.6 | 0.0 / 0.3 | 0.0 / 0.2 | 0.1 / 5.4 | 1.3 / 0.2 | 0.0 / 0.1 | 0.0 / 0.1 |
| -90° | 0.1 / 0.5 | 0.0 / 0.2 | 0.0 / 0.2 | 0.1 / 3.4 | 0.4 / 0.2 | 0.0 / 0.1 | 0.0 / 0.1 |

## Preparation cost (ms, CPU wall time)

`setup` builds pipelines and uploads the terrain texture. `first frame` additionally carries whatever the method builds lazily — for the mesh that is the whole triangulation. `warmup` is every pre-timing frame, which is where an incrementally baked voxel grid actually gets paid for.

| method | setup | first frame | warmup |
|---|---|---|---|
| RayTraced | 9 | 62 | 102 |
| RayVoxel | 12 | 100 | 2647 |
| Sliced | 18 | 87 | 147 |
| Scattered | 10 | 129 | 366 |
| Painter | 17 | 121 | 203 |
| Mesh q=0.0 | 10 | 1479 | 1501 |
| Mesh q=0.75 | 18 | 3406 | 3441 |

## Depth error, p50 / p95 (world units)

Read comparatively. Grazing rays can move their hit point by tens of units for a sub-pixel direction change, while a diagnostic batch also shows scene-dependent common-mode offsets away from the horizon. Inter-method agreement is therefore as important as the absolute error against this reference.

| view | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| river @ 0° | 46.7 / 260.9 | 50.5 / 257.9 | 45.7 / 240.4 | 41.6 / 222.2 | 47.0 / 254.3 | 49.0 / 251.3 | 47.4 / 253.9 |
| hangar @ 0° | 5.1 / 159.1 | 4.6 / 151.2 | 5.7 / 164.4 | 123.0 / 345.1 | 4.7 / 156.3 | 10.6 / 171.9 | 4.7 / 156.6 |
| ramp @ 0° | 9.5 / 79.3 | 9.3 / 81.8 | 9.2 / 85.8 | 19.1 / 147.6 | 9.4 / 84.3 | 10.9 / 80.2 | 9.4 / 79.7 |
| portal @ -30° | 10.4 / 252.8 | 10.9 / 281.2 | 10.3 / 243.5 | 14.8 / 290.5 | 10.8 / 281.1 | 12.0 / 281.5 | 11.0 / 281.4 |
| entrance @ -30° | 14.5 / 253.4 | 14.6 / 256.2 | 14.1 / 244.9 | 12.1 / 100.9 | 14.3 / 248.4 | 14.3 / 260.8 | 14.7 / 256.8 |
| river-down @ -30° | 17.0 / 212.5 | 17.4 / 212.6 | 17.1 / 214.4 | 18.6 / 219.5 | 17.1 / 211.9 | 17.3 / 217.0 | 18.3 / 212.7 |
| stash @ -60° | 18.9 / 145.2 | 18.5 / 136.9 | 18.8 / 144.3 | 26.1 / 179.3 | 18.3 / 144.9 | 28.2 / 167.0 | 19.3 / 145.0 |
| copterig charger @ -60° | 56.3 / 215.5 | 56.0 / 213.8 | 56.2 / 214.6 | 59.6 / 229.3 | 54.4 / 213.0 | 55.7 / 216.9 | 56.1 / 214.7 |
| wires @ -60° | 52.2 / 168.6 | 51.3 / 167.6 | 52.0 / 168.8 | 57.2 / 186.0 | 52.0 / 168.1 | 52.3 / 169.3 | 51.8 / 168.7 |
| spiral charger @ -90° | 45.6 / 134.2 | 45.5 / 123.3 | 45.7 / 126.1 | 47.4 / 158.4 | 44.6 / 118.8 | 50.0 / 153.0 | 45.7 / 131.4 |
| gorb charger @ -90° | 21.7 / 99.5 | 21.7 / 99.6 | 21.8 / 99.6 | 23.6 / 101.6 | 21.8 / 99.7 | 22.0 / 98.9 | 21.8 / 99.5 |
| secret @ -90° | 23.9 / 171.6 | 24.0 / 170.5 | 24.1 / 171.6 | 24.3 / 175.0 | 24.1 / 171.4 | 24.3 / 170.9 | 23.9 / 172.1 |
