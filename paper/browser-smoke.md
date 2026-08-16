# Browser WebGPU smoke test

Validated 2026-08-15 from clean revision `f65421597fc4566b8546e2e7b507016ba7aee59a`.

## Build

```bash
cargo build --target wasm32-unknown-unknown --features web --bin web --release
wasm-bindgen target/wasm32-unknown-unknown/release/web.wasm \
  --out-dir work/web-smoke --target web --no-typescript
```

The build used the repository's pinned `wasm-bindgen` 0.2.117 schema. The raw
Wasm SHA-256 was
`c6b6570d91cb7809e2d95506ca99cf4086e6677e927ac98742bfad6fa699ee6e`.

## Run

Firefox 152.0.5 was driven headlessly through geckodriver at 1280×800. The
test forced `dom.webgpu.enabled`, requested a WebGPU adapter, then loaded the
WebGPU-only `/voxel/#level=fostral` route from a local HTTP server. Firefox
reported:

- adapter: AMD Radeon 890M Graphics (RADV STRIX1)
- backend: Vulkan
- driver: RADV / Mesa 26.1.4
- maximum 2D texture dimension: 16384
- features used by the adapter include timestamp queries and BC compression

The 18.6 MB level archive and 1.1 MB common archive loaded from the local
site. The canvas reached 1280×714 and rendered Fostral through the voxel path;
the in-canvas diagnostics reported `Backend: WebGPU`. The local screenshot is
`work/web-smoke-firefox-page.png`, SHA-256
`d71a61dd1601f8d408cbf86a8d179918762b26a370deb9fce31c71abca2c930b`.
It remains untracked with the other derived Fostral imagery pending the data
grant.

This is an execution and visual-parity smoke test, not a browser timing run.
The performance tables remain native Vulkan/Metal measurements using the same
wgpu/WGSL source and validation model.
