## Devices

| label | adapter | backend / type | driver | config |
|---|---|---|---|---|
| AMD Radeon 780M Graphics (RADV PHOENIX) | AMD Radeon 780M Graphics (RADV PHOENIX) | Vulkan / IntegratedGpu | Mesa 25.2.8-0ubuntu0.24.04.1 | 1280x800, far 600, 40 frames; shadows ray-traced 1024x1024 |
| AMD Radeon RX 7900 XT (RADV NAVI31) | AMD Radeon RX 7900 XT (RADV NAVI31) | Vulkan / DiscreteGpu | Mesa 26.0.3-1ubuntu1 | 1280x800, far 600, 40 frames; shadows ray-traced 1024x1024 |
| Intel(R) Graphics (RPL-U) | Intel(R) Graphics (RPL-U) | Vulkan / IntegratedGpu | Mesa 26.0.3-1ubuntu1 | 1280x800, far 600, 40 frames; shadows ray-traced 1024x1024 |
| NVIDIA GeForce RTX 5070 | NVIDIA GeForce RTX 5070 | Vulkan / DiscreteGpu | 595.71.05 | 1280x800, far 600, 40 frames; shadows ray-traced 1024x1024 |
| Apple M3 | Apple M3 | Metal / IntegratedGpu | ? | 1280x800, far 600, 40 frames; shadows ray-traced 1024x1024 |

## Frame time, avg_ms (ms)

**Mixed timing.** Vulkan rows use GPU timestamp queries. Metal uses its retained CPU submit-and-wait average because encoder timestamps did not reliably bracket this multipass workload. The Metal values include the round trip and must not be compared directly with Vulkan GPU times.


### AMD Radeon 780M Graphics (RADV PHOENIX)

| view | RayTraced | RayVoxel | Sliced | Scattered | Painted | Mesh q=0.5 |
|---|---|---|---|---|---|---|
| river @ 0° | 4.8 | 8.6 | 11.2 | 6.8 | 8.5 | 4.1 |
| hangar @ 0° | 4.5 | 9.5 | 8.7 | 29.3 | 8.6 | 4.0 |
| ramp @ 0° | 4.4 | 6.3 | 11.5 | 6.4 | 8.2 | 4.1 |
| portal @ -30° | 4.7 | 9.8 | 7.2 | 7.0 | 13.0 | 4.0 |
| entrance @ -30° | 4.7 | 8.5 | 7.6 | 11.8 | 14.3 | 4.2 |
| river-down @ -30° | 5.2 | 9.1 | 8.6 | 7.1 | 11.2 | 4.2 |
| stash @ -60° | 4.8 | 7.9 | 7.7 | 7.1 | 22.4 | 4.0 |
| copterig charger @ -60° | 5.0 | 8.4 | 10.7 | 7.2 | 16.4 | 4.3 |
| wires @ -60° | 4.7 | 8.5 | 8.5 | 6.6 | 21.4 | 3.7 |
| spiral charger @ -90° | 5.1 | 8.4 | 10.4 | 7.3 | 25.6 | 4.2 |
| gorb charger @ -90° | 4.8 | 6.6 | 8.2 | 6.6 | 26.3 | 3.8 |
| secret @ -90° | 5.3 | 8.2 | 11.8 | 7.3 | 14.1 | 4.2 |

Arithmetic mean over the views at each pitch:

| pitch | RayTraced | RayVoxel | Sliced | Scattered | Painted | Mesh q=0.5 |
|---|---|---|---|---|---|---|
| 0° | 4.562 | 8.143 | 10.437 | 14.179 | 8.441 | 4.072 |
| -30° | 4.871 | 9.123 | 7.809 | 8.658 | 12.853 | 4.152 |
| -60° | 4.838 | 8.254 | 8.974 | 6.970 | 20.058 | 3.974 |
| -90° | 5.044 | 7.762 | 10.142 | 7.072 | 22.005 | 4.081 |

### AMD Radeon RX 7900 XT (RADV NAVI31)

| view | RayTraced | RayVoxel | Sliced | Scattered | Painted | Mesh q=0.5 |
|---|---|---|---|---|---|---|
| river @ 0° | 0.6 | 1.4 | 1.9 | 1.0 | 2.5 | 0.4 |
| hangar @ 0° | 0.5 | 1.6 | 1.5 | 21.8 | 2.8 | 0.4 |
| ramp @ 0° | 0.5 | 1.1 | 2.0 | 0.9 | 3.8 | 0.4 |
| portal @ -30° | 0.7 | 1.6 | 1.2 | 1.0 | 4.8 | 0.4 |
| entrance @ -30° | 0.6 | 1.4 | 1.3 | 1.9 | 5.3 | 0.4 |
| river-down @ -30° | 0.7 | 1.5 | 1.5 | 1.0 | 3.8 | 0.4 |
| stash @ -60° | 0.6 | 1.4 | 1.4 | 1.0 | 7.6 | 0.4 |
| copterig charger @ -60° | 0.7 | 1.4 | 1.9 | 1.0 | 4.9 | 0.4 |
| wires @ -60° | 0.6 | 1.4 | 1.5 | 0.9 | 8.0 | 0.4 |
| spiral charger @ -90° | 0.7 | 1.4 | 1.9 | 1.0 | 6.8 | 0.4 |
| gorb charger @ -90° | 0.6 | 1.1 | 1.4 | 0.9 | 8.5 | 0.3 |
| secret @ -90° | 0.7 | 1.4 | 2.1 | 1.0 | 3.7 | 0.4 |

Arithmetic mean over the views at each pitch:

| pitch | RayTraced | RayVoxel | Sliced | Scattered | Painted | Mesh q=0.5 |
|---|---|---|---|---|---|---|
| 0° | 0.574 | 1.365 | 1.781 | 7.897 | 3.035 | 0.384 |
| -30° | 0.660 | 1.522 | 1.343 | 1.315 | 4.658 | 0.395 |
| -60° | 0.658 | 1.415 | 1.604 | 0.991 | 6.837 | 0.385 |
| -90° | 0.693 | 1.308 | 1.790 | 0.974 | 6.332 | 0.383 |

### Intel(R) Graphics (RPL-U)

| view | RayTraced | RayVoxel | Sliced | Scattered | Painted | Mesh q=0.5 |
|---|---|---|---|---|---|---|
| river @ 0° | 18.4 | 36.8 | 54.3 | 72.5 | 43.6 | 11.8 |
| hangar @ 0° | 14.9 | 40.0 | 39.9 | 148.9 | 46.8 | 12.9 |
| ramp @ 0° | 14.0 | 27.3 | 69.3 | 91.8 | 46.5 | 10.5 |
| portal @ -30° | 18.2 | 42.3 | 29.5 | 87.2 | 68.1 | 11.6 |
| entrance @ -30° | 16.2 | 36.9 | 36.9 | 98.3 | 72.4 | 10.5 |
| river-down @ -30° | 20.0 | 39.6 | 37.8 | 83.0 | 46.4 | 12.2 |
| stash @ -60° | 16.8 | 33.3 | 27.5 | 84.7 | 101.4 | 10.6 |
| copterig charger @ -60° | 19.6 | 35.0 | 45.7 | 100.8 | 55.6 | 12.5 |
| wires @ -60° | 16.4 | 35.0 | 31.4 | 78.2 | 102.7 | 10.7 |
| spiral charger @ -90° | 20.1 | 35.6 | 38.2 | 74.3 | 148.0 | 11.6 |
| gorb charger @ -90° | 17.9 | 29.1 | 31.8 | 60.2 | 142.1 | 10.7 |
| secret @ -90° | 25.7 | 36.0 | 58.0 | 78.5 | 82.0 | 11.9 |

Arithmetic mean over the views at each pitch:

| pitch | RayTraced | RayVoxel | Sliced | Scattered | Painted | Mesh q=0.5 |
|---|---|---|---|---|---|---|
| 0° | 15.751 | 34.681 | 54.498 | 104.370 | 45.628 | 11.745 |
| -30° | 18.106 | 39.596 | 34.730 | 89.523 | 62.300 | 11.426 |
| -60° | 17.591 | 34.433 | 34.862 | 87.927 | 86.566 | 11.256 |
| -90° | 21.227 | 33.561 | 42.657 | 70.993 | 124.020 | 11.388 |

### NVIDIA GeForce RTX 5070

| view | RayTraced | RayVoxel | Sliced | Scattered | Painted | Mesh q=0.5 |
|---|---|---|---|---|---|---|
| river @ 0° | 0.8 | 1.3 | 2.4 | 1.4 | 2.7 | 0.5 |
| hangar @ 0° | 0.7 | 1.5 | 1.8 | 2.5 | 3.0 | 0.5 |
| ramp @ 0° | 0.6 | 1.0 | 2.6 | 1.3 | 4.1 | 0.5 |
| portal @ -30° | 0.8 | 1.5 | 1.4 | 1.4 | 4.3 | 0.5 |
| entrance @ -30° | 0.7 | 1.3 | 1.5 | 1.9 | 4.8 | 0.5 |
| river-down @ -30° | 0.9 | 1.5 | 1.7 | 1.4 | 3.4 | 0.5 |
| stash @ -60° | 0.8 | 1.2 | 1.4 | 1.3 | 6.2 | 0.5 |
| copterig charger @ -60° | 0.9 | 1.2 | 2.3 | 1.4 | 4.4 | 0.5 |
| wires @ -60° | 0.7 | 1.2 | 1.6 | 1.3 | 6.5 | 0.5 |
| spiral charger @ -90° | 0.9 | 1.2 | 2.0 | 1.3 | 6.5 | 0.5 |
| gorb charger @ -90° | 0.8 | 1.0 | 1.6 | 1.2 | 6.8 | 0.5 |
| secret @ -90° | 0.9 | 1.2 | 2.5 | 1.4 | 3.7 | 0.5 |

Arithmetic mean over the views at each pitch:

| pitch | RayTraced | RayVoxel | Sliced | Scattered | Painted | Mesh q=0.5 |
|---|---|---|---|---|---|---|
| 0° | 0.722 | 1.251 | 2.259 | 1.721 | 3.258 | 0.503 |
| -30° | 0.809 | 1.419 | 1.544 | 1.532 | 4.155 | 0.501 |
| -60° | 0.782 | 1.226 | 1.774 | 1.327 | 5.676 | 0.493 |
| -90° | 0.845 | 1.146 | 2.018 | 1.304 | 5.679 | 0.496 |

### Apple M3

| view | RayTraced | RayVoxel | Sliced | Scattered | Painted | Mesh q=0.5 |
|---|---|---|---|---|---|---|
| river @ 0° | 10.8 | 12.7 | 12.7 | 11.5 | 16.5 | 5.3 |
| hangar @ 0° | 7.5 | 14.2 | 11.4 | 99.8 | 17.8 | 5.5 |
| ramp @ 0° | 6.4 | 10.2 | 12.7 | 10.3 | 16.5 | 5.2 |
| portal @ -30° | 7.6 | 14.0 | 10.2 | 10.2 | 34.7 | 6.0 |
| entrance @ -30° | 7.6 | 12.7 | 10.2 | 12.7 | 38.0 | 5.3 |
| river-down @ -30° | 8.9 | 14.0 | 12.7 | 11.5 | 25.3 | 6.5 |
| stash @ -60° | 7.6 | 12.2 | 10.7 | 10.7 | 54.8 | 6.1 |
| copterig charger @ -60° | 9.2 | 12.2 | 15.0 | 10.7 | 36.5 | 6.6 |
| wires @ -60° | 7.7 | 12.2 | 10.7 | 10.7 | 58.0 | 6.2 |
| spiral charger @ -90° | 9.2 | 12.2 | 12.2 | 10.7 | 35.1 | 6.3 |
| gorb charger @ -90° | 8.5 | 10.7 | 10.7 | 11.3 | 37.9 | 5.5 |
| secret @ -90° | 9.0 | 12.7 | 16.5 | 11.5 | 21.6 | 6.7 |

Arithmetic mean over the views at each pitch:

| pitch | RayTraced | RayVoxel | Sliced | Scattered | Painted | Mesh q=0.5 |
|---|---|---|---|---|---|---|
| 0° | 8.219 | 12.378 | 12.284 | 40.524 | 16.944 | 5.307 |
| -30° | 8.057 | 13.551 | 11.017 | 11.443 | 32.664 | 5.936 |
| -60° | 8.172 | 12.173 | 12.110 | 10.681 | 49.750 | 6.302 |
| -90° | 8.915 | 11.862 | 13.124 | 11.161 | 31.530 | 6.135 |

## Within-run timing uncertainty

Each cell is the fixed-fixture mean over twelve scenes ± an approximate 95% interval propagated from the 40 within-scene frame samples. It measures frame-to-frame noise in this session, not driver-to-driver or session-to-session variation. Metal rows use retained CPU submit-and-wait samples.

| device | RayTraced | RayVoxel | Sliced | Scattered | Painted | Mesh q=0.5 |
|---|---|---|---|---|---|---|
| AMD Radeon 780M Graphics (RADV PHOENIX) | 4.829 ± 0.116 | 8.320 ± 0.008 | 9.341 ± 0.120 | 9.220 ± 0.042 | 15.839 ± 0.085 | 4.070 ± 0.050 |
| AMD Radeon RX 7900 XT (RADV NAVI31) | 0.646 ± 0.007 | 1.403 ± 0.001 | 1.629 ± 0.010 | 2.794 ± 0.003 | 5.216 ± 0.002 | 0.387 ± 0.003 |
| Intel(R) Graphics (RPL-U) | 18.169 ± 0.103 | 35.568 ± 0.063 | 41.687 ± 0.424 | 88.203 ± 0.159 | 79.629 ± 0.444 | 11.454 ± 0.041 |
| NVIDIA GeForce RTX 5070 | 0.790 ± 0.000 | 1.260 ± 0.001 | 1.899 ± 0.000 | 1.471 ± 0.000 | 4.692 ± 0.001 | 0.498 ± 0.001 |
| Apple M3 | 8.341 ± 0.032 | 12.491 ± 0.014 | 12.134 ± 0.016 | 18.452 ± 0.022 | 32.722 ± 0.036 | 5.920 ± 0.068 |

## Accuracy: see-through / covers-sky / speckle (%)

Expected to be device-independent; baseline taken from **AMD Radeon 780M Graphics (RADV PHOENIX)** and cross-checked below. `see-through` is solid terrain left as background and `covers-sky` is background filled in — only the first moves when a renderer is really missing geometry, and both move together when the reference is the one disagreeing. `speckle` is what depth agreement cannot see: pixels whose distance disagrees with their own neighbourhood, in excess of the reference doing the same.

| view | RayTraced | RayVoxel | Sliced | Scattered | Painted | Mesh q=0.5 |
|---|---|---|---|---|---|---|
| river @ 0° | 0.6 / 0.0 / 0.2 | 0.0 / 0.1 / 0.1 | 1.3 / 0.0 / 1.1 | 29.0 / 0.0 / 1.6 | 0.0 / 0.2 / 0.1 | 0.3 / 0.1 / 0.0 |
| hangar @ 0° | 1.2 / 0.0 / 0.4 | 0.1 / 0.1 / 0.2 | 1.2 / 0.1 / 2.9 | 73.2 / 0.0 / 1.4 | 0.0 / 0.2 / 0.1 | 0.5 / 0.1 / 0.1 |
| ramp @ 0° | 0.1 / 0.0 / 0.5 | 0.0 / 0.0 / 0.4 | 0.1 / 0.0 / 4.2 | 19.1 / 0.0 / 2.5 | 0.0 / 0.0 / 0.1 | 0.0 / 0.0 / 0.1 |
| portal @ -30° | 0.5 / 0.0 / 1.2 | 0.0 / 0.1 / 0.3 | 1.8 / 0.0 / 1.8 | 4.1 / 0.1 / 4.5 | 0.0 / 0.2 / 0.1 | 0.5 / 0.1 / 0.1 |
| entrance @ -30° | 0.6 / 0.0 / 0.6 | 0.0 / 0.1 / 0.2 | 2.9 / 0.0 / 1.6 | 33.4 / 0.0 / 2.1 | 0.8 / 0.1 / 0.1 | 0.2 / 0.1 / 0.1 |
| river-down @ -30° | 0.8 / 0.1 / 0.8 | 0.1 / 0.3 / 0.3 | 2.2 / 0.0 / 0.8 | 8.6 / 0.1 / 4.0 | 0.0 / 0.3 / 0.1 | 0.5 / 0.2 / 0.1 |
| stash @ -60° | 0.0 / 0.0 / 0.5 | 0.0 / 0.0 / 0.3 | 0.0 / 0.0 / 0.2 | 0.1 / 0.0 / 5.6 | 2.6 / 0.0 / 0.1 | 0.0 / 0.0 / 0.1 |
| copterig charger @ -60° | 0.1 / 0.0 / 0.6 | 0.0 / 0.0 / 0.2 | 0.0 / 0.0 / 0.2 | 0.8 / 0.0 / 4.8 | 4.2 / 0.0 / 0.1 | 0.1 / 0.0 / 0.1 |
| wires @ -60° | 0.0 / 0.0 / 0.9 | 0.0 / 0.0 / 0.3 | 0.0 / 0.0 / 0.3 | 0.0 / 0.0 / 6.7 | 1.3 / 0.0 / 0.2 | 0.0 / 0.0 / 0.1 |
| spiral charger @ -90° | 0.0 / 0.0 / 0.6 | 0.0 / 0.0 / 0.3 | 0.0 / 0.0 / 0.3 | 0.1 / 0.0 / 4.1 | 2.5 / 0.0 / 0.3 | 0.0 / 0.0 / 0.2 |
| gorb charger @ -90° | 0.0 / 0.0 / 0.5 | 0.0 / 0.0 / 0.2 | 0.0 / 0.0 / 0.2 | 0.0 / 0.0 / 4.1 | 0.0 / 0.0 / 0.2 | 0.0 / 0.0 / 0.1 |
| secret @ -90° | 0.1 / 0.0 / 0.6 | 0.0 / 0.0 / 0.2 | 0.0 / 0.0 / 0.2 | 0.3 / 0.0 / 2.8 | 0.0 / 0.0 / 0.2 | 0.0 / 0.0 / 0.2 |

Arithmetic mean over the views at each pitch, see-through / speckle (%):

| pitch | RayTraced | RayVoxel | Sliced | Scattered | Painted | Mesh q=0.5 |
|---|---|---|---|---|---|---|
| 0° | 0.6 / 0.4 | 0.0 / 0.2 | 0.9 / 2.7 | 40.4 / 1.8 | 0.0 / 0.1 | 0.3 / 0.1 |
| -30° | 0.6 / 0.9 | 0.0 / 0.2 | 2.3 / 1.4 | 15.4 / 3.5 | 0.3 / 0.1 | 0.4 / 0.1 |
| -60° | 0.1 / 0.7 | 0.0 / 0.3 | 0.0 / 0.2 | 0.3 / 5.7 | 2.7 / 0.1 | 0.0 / 0.1 |
| -90° | 0.1 / 0.5 | 0.0 / 0.3 | 0.0 / 0.2 | 0.1 / 3.7 | 0.8 / 0.2 | 0.0 / 0.1 |

> **Cross-device threshold crossings.** Accuracy should not normally vary with the adapter; inspect these rows before deciding whether the spread is material.

> - Sliced `depth_p50` at hangar @ 0°: AMD Radeon 780M Graphics (RADV PHOENIX) 0.3u vs NVIDIA GeForce RTX 5070 1.0u
> - Sliced `depth_p95` at hangar @ 0°: AMD Radeon 780M Graphics (RADV PHOENIX) 35.1u vs NVIDIA GeForce RTX 5070 41.3u
> - Scattered `see_through` at hangar @ 0°: AMD Radeon 780M Graphics (RADV PHOENIX) 73.2% vs Apple M3 74.0%
> - Scattered `depth_p50` at hangar @ 0°: AMD Radeon 780M Graphics (RADV PHOENIX) 109.8u vs Apple M3 112.0u
> - Scattered `depth_p95` at hangar @ 0°: AMD Radeon 780M Graphics (RADV PHOENIX) 347.2u vs Apple M3 350.4u

## Preparation cost (ms, CPU wall time)

`setup` builds pipelines and uploads the terrain texture. `first frame` additionally carries whatever the method builds lazily — for the mesh that is the whole triangulation. `warmup` is every pre-timing frame, which is where an incrementally baked voxel grid actually gets paid for.

| method | setup | first frame | warmup |
|---|---|---|---|
| RayTraced | 9 | 60 | 101 |
| RayVoxel | 12 | 124 | 3163 |
| Sliced | 17 | 88 | 144 |
| Scattered | 10 | 127 | 362 |
| Painted | 18 | 119 | 200 |
| Mesh q=0.5 | 10 | 2464 | 2488 |

## Depth error, p50 / p95 (world units)

Read comparatively. Grazing rays can move their hit point by tens of units for a sub-pixel direction change, while a diagnostic batch also shows scene-dependent common-mode offsets away from the horizon. Inter-method agreement is therefore as important as the absolute error against this reference.

| view | RayTraced | RayVoxel | Sliced | Scattered | Painted | Mesh q=0.5 |
|---|---|---|---|---|---|---|
| river @ 0° | 0.1 / 4.8 | 0.5 / 0.9 | 0.4 / 8.0 | 28.3 / 195.3 | 0.0 / 0.9 | 0.3 / 5.2 |
| hangar @ 0° | 0.1 / 80.0 | 0.5 / 1.1 | 0.3 / 35.1 | 109.8 / 347.2 | 0.0 / 0.9 | 0.4 / 8.9 |
| ramp @ 0° | 0.1 / 28.5 | 0.5 / 1.1 | 0.6 / 10.3 | 16.3 / 147.7 | 0.0 / 0.4 | 0.6 / 32.6 |
| portal @ -30° | 0.2 / 31.9 | 0.6 / 1.2 | 0.2 / 11.6 | 0.9 / 267.3 | 0.0 / 0.1 | 0.3 / 5.1 |
| entrance @ -30° | 0.2 / 7.1 | 0.6 / 1.4 | 0.5 / 7.4 | 1.7 / 69.9 | 0.0 / 0.1 | 0.6 / 4.3 |
| river-down @ -30° | 0.3 / 6.0 | 0.7 / 2.1 | 0.2 / 8.0 | 0.3 / 131.6 | 0.0 / 0.3 | 0.4 / 6.3 |
| stash @ -60° | 0.1 / 5.0 | 0.5 / 2.2 | 0.1 / 1.1 | 0.2 / 94.0 | 0.0 / 1.0 | 0.6 / 6.9 |
| copterig charger @ -60° | 0.1 / 4.6 | 0.5 / 1.3 | 0.1 / 1.5 | 0.5 / 177.3 | 0.0 / 1.7 | 0.7 / 4.1 |
| wires @ -60° | 0.2 / 12.3 | 0.6 / 2.4 | 0.1 / 1.4 | 0.4 / 145.3 | 0.0 / 1.6 | 0.7 / 4.8 |
| spiral charger @ -90° | 14.3 / 133.4 | 14.4 / 131.6 | 14.5 / 131.3 | 15.1 / 143.1 | 14.0 / 128.4 | 14.9 / 136.5 |
| gorb charger @ -90° | 8.6 / 81.9 | 8.5 / 81.4 | 8.6 / 81.1 | 9.7 / 90.0 | 8.6 / 81.1 | 8.5 / 81.6 |
| secret @ -90° | 10.5 / 163.1 | 10.5 / 162.2 | 10.5 / 162.1 | 10.7 / 167.7 | 10.6 / 162.0 | 10.6 / 162.4 |
