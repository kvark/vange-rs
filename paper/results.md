## Devices

| label | adapter | backend / type | driver | config |
|---|---|---|---|---|
| AMD Radeon 780M Graphics (RADV PHOENIX) | AMD Radeon 780M Graphics (RADV PHOENIX) | Vulkan / IntegratedGpu | Mesa 25.2.8-0ubuntu0.24.04.1 | 1280x800, far 600, 40 frames; shadows ray-traced 1024x1024 |
| AMD Radeon RX 7900 XT (RADV NAVI31) | AMD Radeon RX 7900 XT (RADV NAVI31) | Vulkan / DiscreteGpu | Mesa 26.0.3-1ubuntu1 | 1280x800, far 600, 40 frames; shadows ray-traced 1024x1024 |
| Apple M3 | Apple M3 | Metal / IntegratedGpu | ? | 1280x800, far 600, 40 frames; shadows ray-traced 1024x1024 |
| Intel(R) Graphics (RPL-U) | Intel(R) Graphics (RPL-U) | Vulkan / IntegratedGpu | Mesa 26.0.3-1ubuntu1 | 1280x800, far 600, 40 frames; shadows ray-traced 1024x1024 |
| NVIDIA GeForce RTX 5070 | NVIDIA GeForce RTX 5070 | Vulkan / DiscreteGpu | 595.71.05 | 1280x800, far 600, 40 frames; shadows ray-traced 1024x1024 |

## Frame time, avg_ms (ms)

**Mixed timing.** Vulkan rows use GPU timestamp queries. Metal uses its retained CPU submit-and-wait average because encoder timestamps did not reliably bracket this multipass workload. The Metal values include the round trip and must not be compared directly with Vulkan GPU times.


### AMD Radeon 780M Graphics (RADV PHOENIX)

| view | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| river @ 0° | 3.7 | 7.7 | 11.1 | 6.8 | 8.5 | 4.3 | 3.8 |
| hangar @ 0° | 3.6 | 8.0 | 8.7 | 29.3 | 8.6 | 4.3 | 3.8 |
| ramp @ 0° | 3.3 | 6.2 | 11.5 | 6.4 | 8.2 | 4.3 | 4.2 |
| portal @ -30° | 3.8 | 8.4 | 7.3 | 7.0 | 13.0 | 4.3 | 3.8 |
| entrance @ -30° | 3.6 | 7.6 | 7.6 | 11.8 | 14.3 | 4.2 | 4.3 |
| river-down @ -30° | 4.0 | 7.9 | 8.6 | 7.1 | 11.1 | 4.5 | 3.9 |
| stash @ -60° | 3.6 | 7.5 | 7.7 | 7.1 | 22.4 | 4.1 | 3.8 |
| copterig charger @ -60° | 3.8 | 8.2 | 10.7 | 7.2 | 16.3 | 4.5 | 4.2 |
| wires @ -60° | 3.7 | 8.4 | 8.5 | 6.7 | 21.4 | 4.0 | 3.8 |
| spiral charger @ -90° | 3.9 | 8.2 | 10.4 | 7.3 | 25.6 | 4.2 | 4.1 |
| gorb charger @ -90° | 3.6 | 6.6 | 8.3 | 6.6 | 26.2 | 3.9 | 3.4 |
| secret @ -90° | 3.9 | 8.1 | 11.8 | 7.3 | 14.1 | 4.4 | 3.9 |

Arithmetic mean over the views at each pitch:

| pitch | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| 0° | 3.552 | 7.310 | 10.442 | 14.172 | 8.472 | 4.309 | 3.950 |
| -30° | 3.793 | 7.943 | 7.819 | 8.656 | 12.811 | 4.324 | 4.029 |
| -60° | 3.717 | 8.000 | 8.958 | 6.983 | 20.048 | 4.163 | 3.929 |
| -90° | 3.804 | 7.636 | 10.153 | 7.079 | 21.998 | 4.146 | 3.813 |

### AMD Radeon RX 7900 XT (RADV NAVI31)

| view | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| river @ 0° | 0.6 | 1.3 | 2.1 | 1.2 | 2.8 | 0.5 | 0.5 |
| hangar @ 0° | 0.5 | 1.3 | 1.7 | 21.6 | 3.0 | 0.5 | 0.5 |
| ramp @ 0° | 0.3 | 1.0 | 2.2 | 1.1 | 3.9 | 0.5 | 0.5 |
| portal @ -30° | 0.6 | 1.4 | 1.5 | 1.2 | 4.9 | 0.5 | 0.5 |
| entrance @ -30° | 0.6 | 1.2 | 1.6 | 2.1 | 5.5 | 0.5 | 0.5 |
| river-down @ -30° | 0.7 | 1.3 | 1.7 | 1.2 | 4.0 | 0.6 | 0.5 |
| stash @ -60° | 0.6 | 1.3 | 1.6 | 1.2 | 7.8 | 0.5 | 0.4 |
| copterig charger @ -60° | 0.6 | 1.4 | 2.2 | 1.2 | 5.0 | 0.6 | 0.5 |
| wires @ -60° | 0.6 | 1.4 | 1.8 | 1.1 | 8.1 | 0.5 | 0.5 |
| spiral charger @ -90° | 0.6 | 1.4 | 2.1 | 1.2 | 7.0 | 0.5 | 0.6 |
| gorb charger @ -90° | 0.6 | 1.1 | 1.7 | 1.1 | 8.6 | 0.5 | 0.4 |
| secret @ -90° | 0.6 | 1.3 | 2.3 | 1.2 | 3.9 | 0.5 | 0.5 |

Arithmetic mean over the views at each pitch:

| pitch | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| 0° | 0.474 | 1.193 | 2.021 | 7.977 | 3.255 | 0.491 | 0.478 |
| -30° | 0.605 | 1.313 | 1.599 | 1.505 | 4.828 | 0.517 | 0.478 |
| -60° | 0.593 | 1.354 | 1.833 | 1.180 | 6.954 | 0.501 | 0.473 |
| -90° | 0.617 | 1.276 | 2.027 | 1.177 | 6.466 | 0.473 | 0.498 |

### Apple M3

| view | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| river @ 0° | 7.6 | 11.4 | 12.8 | 11.4 | 16.5 | 6.0 | 6.5 |
| hangar @ 0° | 4.6 | 11.4 | 11.5 | 99.8 | 17.8 | 5.5 | 6.6 |
| ramp @ 0° | 4.5 | 9.4 | 12.6 | 10.3 | 17.7 | 5.3 | 5.3 |
| portal @ -30° | 5.2 | 12.6 | 10.2 | 11.5 | 34.3 | 5.6 | 6.5 |
| entrance @ -30° | 5.2 | 11.5 | 11.5 | 14.0 | 38.0 | 5.4 | 5.4 |
| river-down @ -30° | 5.6 | 11.4 | 12.7 | 11.0 | 25.4 | 5.6 | 6.5 |
| stash @ -60° | 5.2 | 11.5 | 10.2 | 10.2 | 54.8 | 5.4 | 5.4 |
| copterig charger @ -60° | 5.7 | 12.6 | 14.0 | 11.5 | 36.6 | 6.5 | 6.6 |
| wires @ -60° | 5.1 | 12.7 | 11.5 | 11.3 | 58.2 | 5.8 | 5.5 |
| spiral charger @ -90° | 5.2 | 12.2 | 11.4 | 11.5 | 35.5 | 5.2 | 6.5 |
| gorb charger @ -90° | 5.1 | 10.2 | 10.2 | 10.2 | 36.8 | 5.4 | 5.3 |
| secret @ -90° | 5.7 | 11.4 | 16.5 | 11.5 | 21.6 | 5.7 | 6.5 |

Arithmetic mean over the views at each pitch:

| pitch | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| 0° | 5.606 | 10.740 | 12.292 | 40.515 | 17.338 | 5.584 | 6.132 |
| -30° | 5.334 | 11.825 | 11.442 | 12.155 | 32.564 | 5.534 | 6.136 |
| -60° | 5.337 | 12.261 | 11.869 | 10.975 | 49.864 | 5.884 | 5.811 |
| -90° | 5.327 | 11.291 | 12.710 | 11.046 | 31.304 | 5.405 | 6.078 |

### Intel(R) Graphics (RPL-U)

| view | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| river @ 0° | 9.8 | 33.9 | 55.7 | 72.5 | 44.3 | 11.7 | 12.0 |
| hangar @ 0° | 8.6 | 34.0 | 39.9 | 148.8 | 44.6 | 11.3 | 12.0 |
| ramp @ 0° | 8.1 | 22.4 | 48.7 | 91.7 | 46.5 | 10.0 | 10.6 |
| portal @ -30° | 10.1 | 29.5 | 26.6 | 87.2 | 60.7 | 11.4 | 11.9 |
| entrance @ -30° | 9.2 | 27.0 | 27.4 | 98.2 | 64.8 | 10.1 | 10.9 |
| river-down @ -30° | 11.0 | 28.1 | 32.7 | 83.0 | 46.3 | 11.5 | 12.0 |
| stash @ -60° | 9.4 | 26.1 | 25.0 | 84.5 | 92.2 | 10.3 | 10.9 |
| copterig charger @ -60° | 10.9 | 28.6 | 39.2 | 100.6 | 55.1 | 11.9 | 12.6 |
| wires @ -60° | 9.3 | 28.5 | 28.4 | 78.1 | 92.4 | 10.4 | 11.0 |
| spiral charger @ -90° | 10.9 | 28.1 | 32.3 | 70.6 | 122.0 | 10.9 | 12.1 |
| gorb charger @ -90° | 9.8 | 23.2 | 28.0 | 55.8 | 115.3 | 10.1 | 10.6 |
| secret @ -90° | 11.3 | 27.9 | 45.7 | 73.1 | 68.7 | 11.5 | 12.1 |

Arithmetic mean over the views at each pitch:

| pitch | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| 0° | 8.846 | 30.105 | 48.097 | 104.304 | 45.108 | 11.019 | 11.535 |
| -30° | 10.131 | 28.183 | 28.909 | 89.467 | 57.296 | 11.032 | 11.560 |
| -60° | 9.874 | 27.705 | 30.874 | 87.719 | 79.882 | 10.852 | 11.499 |
| -90° | 10.692 | 26.385 | 35.341 | 66.507 | 101.991 | 10.817 | 11.595 |

### NVIDIA GeForce RTX 5070

| view | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| river @ 0° | 0.5 | 1.1 | 2.4 | 1.4 | 2.7 | 0.5 | 0.6 |
| hangar @ 0° | 0.4 | 1.2 | 1.8 | 2.5 | 3.0 | 0.5 | 0.6 |
| ramp @ 0° | 0.4 | 1.0 | 2.6 | 1.3 | 4.1 | 0.5 | 0.5 |
| portal @ -30° | 0.5 | 1.2 | 1.4 | 1.4 | 4.3 | 0.5 | 0.7 |
| entrance @ -30° | 0.4 | 1.1 | 1.5 | 1.9 | 4.8 | 0.5 | 0.6 |
| river-down @ -30° | 0.5 | 1.2 | 1.7 | 1.4 | 3.4 | 0.5 | 0.6 |
| stash @ -60° | 0.4 | 1.1 | 1.4 | 1.3 | 6.2 | 0.5 | 0.6 |
| copterig charger @ -60° | 0.5 | 1.2 | 2.3 | 1.4 | 4.4 | 0.5 | 0.6 |
| wires @ -60° | 0.4 | 1.2 | 1.6 | 1.3 | 6.5 | 0.5 | 0.5 |
| spiral charger @ -90° | 0.5 | 1.2 | 2.0 | 1.3 | 6.5 | 0.5 | 0.6 |
| gorb charger @ -90° | 0.4 | 1.0 | 1.6 | 1.2 | 6.8 | 0.4 | 0.7 |
| secret @ -90° | 0.5 | 1.2 | 2.5 | 1.4 | 3.7 | 0.5 | 0.6 |

Arithmetic mean over the views at each pitch:

| pitch | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| 0° | 0.417 | 1.089 | 2.265 | 1.721 | 3.257 | 0.484 | 0.544 |
| -30° | 0.465 | 1.189 | 1.546 | 1.534 | 4.159 | 0.487 | 0.614 |
| -60° | 0.449 | 1.189 | 1.775 | 1.329 | 5.682 | 0.474 | 0.559 |
| -90° | 0.481 | 1.133 | 2.019 | 1.305 | 5.686 | 0.470 | 0.602 |

## Accuracy: see-through / covers-sky / speckle (%)

Expected to be device-independent; baseline taken from **AMD Radeon 780M Graphics (RADV PHOENIX)** and cross-checked below. `see-through` is solid terrain left as background and `covers-sky` is background filled in — only the first moves when a renderer is really missing geometry, and both move together when the reference is the one disagreeing. `speckle` is what depth agreement cannot see: pixels whose distance disagrees with their own neighbourhood, in excess of the reference doing the same.

| view | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| river @ 0° | 3.9 / 3.1 / 0.2 | 5.1 / 2.9 / 0.1 | 4.1 / 3.0 / 1.1 | 29.4 / 0.6 / 1.6 | 3.0 / 3.3 / 0.1 | 3.4 / 3.2 / 0.0 | 3.1 / 3.2 / 0.0 |
| hangar @ 0° | 8.1 / 6.1 / 0.4 | 7.7 / 5.4 / 0.2 | 7.3 / 6.3 / 2.7 | 74.5 / 1.5 / 1.4 | 6.1 / 6.4 / 0.1 | 16.3 / 5.5 / 0.0 | 6.5 / 6.3 / 0.1 |
| ramp @ 0° | 0.3 / 0.2 / 0.3 | 0.5 / 0.2 / 0.4 | 0.2 / 0.2 / 4.3 | 19.1 / 0.0 / 2.5 | 0.2 / 0.2 / 0.1 | 0.2 / 0.2 / 0.0 | 0.2 / 0.2 / 0.1 |
| portal @ -30° | 2.4 / 3.9 / 1.0 | 1.8 / 4.6 / 0.4 | 2.6 / 3.4 / 1.7 | 4.5 / 3.1 / 4.3 | 1.9 / 4.6 / 0.2 | 2.0 / 3.8 / 0.1 | 1.9 / 4.5 / 0.1 |
| entrance @ -30° | 5.1 / 5.2 / 0.5 | 3.7 / 6.2 / 0.2 | 5.8 / 5.2 / 0.8 | 31.7 / 0.6 / 2.2 | 4.5 / 6.2 / 0.1 | 4.1 / 6.0 / 0.1 | 3.8 / 6.1 / 0.1 |
| river-down @ -30° | 8.2 / 8.0 / 0.8 | 7.0 / 8.9 / 0.3 | 8.2 / 7.7 / 0.7 | 12.3 / 5.3 / 3.8 | 7.1 / 9.0 / 0.1 | 8.1 / 7.7 / 0.1 | 7.2 / 8.8 / 0.1 |
| stash @ -60° | 0.0 / 2.3 / 0.6 | 0.0 / 2.5 / 0.3 | 0.0 / 2.3 / 0.2 | 0.0 / 2.3 / 5.3 | 1.5 / 1.3 / 0.2 | 0.0 / 2.3 / 0.1 | 0.0 / 2.3 / 0.1 |
| copterig charger @ -60° | 0.1 / 8.8 / 0.5 | 0.0 / 9.1 / 0.2 | 0.0 / 9.0 / 0.2 | 0.2 / 8.4 / 4.3 | 1.7 / 6.5 / 0.1 | 0.0 / 9.0 / 0.1 | 0.0 / 9.0 / 0.1 |
| wires @ -60° | 0.0 / 1.8 / 0.8 | 0.0 / 1.8 / 0.4 | 0.0 / 1.8 / 0.3 | 0.0 / 1.8 / 6.4 | 0.7 / 1.2 / 0.2 | 0.0 / 1.8 / 0.1 | 0.0 / 1.8 / 0.1 |
| spiral charger @ -90° | 0.0 / 13.9 / 0.7 | 0.0 / 13.9 / 0.3 | 0.0 / 13.9 / 0.3 | 0.0 / 13.8 / 3.5 | 1.3 / 12.7 / 0.2 | 0.0 / 13.9 / 0.2 | 0.0 / 13.9 / 0.2 |
| gorb charger @ -90° | 0.0 / 0.0 / 0.6 | 0.0 / 0.0 / 0.2 | 0.0 / 0.0 / 0.2 | 0.0 / 0.0 / 4.1 | 0.0 / 0.0 / 0.2 | 0.0 / 0.0 / 0.1 | 0.0 / 0.0 / 0.1 |
| secret @ -90° | 0.2 / 7.0 / 0.6 | 0.0 / 7.0 / 0.2 | 0.0 / 7.0 / 0.2 | 0.3 / 7.0 / 2.6 | 0.0 / 7.0 / 0.2 | 0.0 / 7.0 / 0.1 | 0.0 / 7.0 / 0.1 |

Arithmetic mean over the views at each pitch, see-through / speckle (%):

| pitch | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| 0° | 4.1 / 0.3 | 4.4 / 0.3 | 3.9 / 2.7 | 41.0 / 1.8 | 3.1 / 0.1 | 6.6 / 0.0 | 3.3 / 0.1 |
| -30° | 5.2 / 0.8 | 4.2 / 0.3 | 5.5 / 1.1 | 16.2 / 3.4 | 4.5 / 0.1 | 4.7 / 0.1 | 4.3 / 0.1 |
| -60° | 0.0 / 0.6 | 0.0 / 0.3 | 0.0 / 0.2 | 0.1 / 5.4 | 1.3 / 0.2 | 0.0 / 0.1 | 0.0 / 0.1 |
| -90° | 0.1 / 0.6 | 0.0 / 0.2 | 0.0 / 0.2 | 0.1 / 3.4 | 0.4 / 0.2 | 0.0 / 0.1 | 0.0 / 0.1 |

> **Cross-device threshold crossings.** Accuracy should not normally vary with the adapter; inspect these rows before deciding whether the spread is material.

> - Scattered `see_through` at hangar @ 0°: AMD Radeon 780M Graphics (RADV PHOENIX) 74.5% vs Apple M3 75.2%
> - Scattered `depth_p50` at hangar @ 0°: AMD Radeon 780M Graphics (RADV PHOENIX) 123.0u vs Apple M3 124.3u
> - Scattered `depth_p95` at hangar @ 0°: AMD Radeon 780M Graphics (RADV PHOENIX) 345.1u vs Apple M3 348.0u

## Preparation cost (ms, CPU wall time)

`setup` builds pipelines and uploads the terrain texture. `first frame` additionally carries whatever the method builds lazily — for the mesh that is the whole triangulation. `warmup` is every pre-timing frame, which is where an incrementally baked voxel grid actually gets paid for.

| method | setup | first frame | warmup |
|---|---|---|---|
| RayTraced | 9 | 56 | 87 |
| RayVoxel | 12 | 99 | 2644 |
| Sliced | 10 | 88 | 148 |
| Scattered | 11 | 128 | 367 |
| Painter | 10 | 121 | 202 |
| Mesh q=0.0 | 11 | 1427 | 1452 |
| Mesh q=0.75 | 10 | 3532 | 3566 |

## Depth error, p50 / p95 (world units)

Read comparatively. Grazing rays can move their hit point by tens of units for a sub-pixel direction change, while a diagnostic batch also shows scene-dependent common-mode offsets away from the horizon. Inter-method agreement is therefore as important as the absolute error against this reference.

| view | RayTraced | RayVoxel | Sliced | Scattered | Painter | Mesh q=0.0 | Mesh q=0.75 |
|---|---|---|---|---|---|---|---|
| river @ 0° | 46.0 / 259.2 | 50.5 / 257.9 | 45.7 / 240.4 | 41.6 / 222.2 | 47.0 / 254.3 | 49.0 / 251.3 | 47.4 / 253.9 |
| hangar @ 0° | 5.5 / 179.5 | 4.6 / 151.2 | 5.7 / 164.4 | 123.0 / 345.1 | 4.7 / 156.3 | 10.6 / 171.9 | 4.7 / 156.6 |
| ramp @ 0° | 9.8 / 75.4 | 9.3 / 81.8 | 9.2 / 85.8 | 19.1 / 147.6 | 9.4 / 84.3 | 10.9 / 80.2 | 9.4 / 79.7 |
| portal @ -30° | 9.8 / 248.4 | 10.9 / 281.2 | 10.3 / 243.5 | 14.8 / 290.5 | 10.8 / 281.1 | 12.0 / 281.5 | 11.0 / 281.4 |
| entrance @ -30° | 14.4 / 248.5 | 14.6 / 256.2 | 14.1 / 244.9 | 12.1 / 100.9 | 14.3 / 248.4 | 14.3 / 260.8 | 14.7 / 256.8 |
| river-down @ -30° | 17.9 / 216.1 | 17.4 / 212.6 | 17.1 / 214.4 | 18.6 / 219.5 | 17.1 / 211.9 | 17.3 / 217.0 | 18.3 / 212.7 |
| stash @ -60° | 19.3 / 146.3 | 18.5 / 136.9 | 18.8 / 144.3 | 26.1 / 179.3 | 18.3 / 144.9 | 28.2 / 167.0 | 19.3 / 145.0 |
| copterig charger @ -60° | 56.4 / 217.6 | 56.0 / 213.8 | 56.2 / 214.6 | 59.6 / 229.3 | 54.4 / 213.0 | 55.7 / 216.9 | 56.1 / 214.7 |
| wires @ -60° | 52.5 / 168.4 | 51.3 / 167.6 | 52.0 / 168.8 | 57.2 / 186.0 | 52.0 / 168.1 | 52.3 / 169.3 | 51.8 / 168.7 |
| spiral charger @ -90° | 45.5 / 142.6 | 45.5 / 123.3 | 45.7 / 126.1 | 47.4 / 158.4 | 44.6 / 118.8 | 50.0 / 153.0 | 45.7 / 131.4 |
| gorb charger @ -90° | 21.1 / 99.4 | 21.7 / 99.6 | 21.8 / 99.6 | 23.6 / 101.6 | 21.8 / 99.7 | 22.0 / 98.9 | 21.8 / 99.5 |
| secret @ -90° | 23.8 / 174.5 | 24.0 / 170.5 | 24.1 / 171.6 | 24.3 / 175.0 | 24.1 / 171.4 | 24.3 / 170.9 | 23.9 / 172.1 |
