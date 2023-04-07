## Khox
Using regular export:
```bash
cargo run --bin convert --release -- "c:\Program Files (x86)\GOG Galaxy\Games\Vangers\data\thechain\khox\world.ini" tmp\khox.obj
```

Simplify produces a bunch of disjoint triangles:
```bash
"C:\Program Files\simplify.exe" tmp\khox.obj tmp\khox-8.obj 0.125
```
Meshopt is fast and produces something reasonable:
```bash
"C:\Program Files\gltfpack.exe" -i tmp\khox.obj -o tmp\khox.gltf -si 0.125
```
Compressonator takes a long time to process:
```bash
"C:\Program Files\Compressonator\bin\CLI\compressonatorcli.exe" -meshopt -simplifyMeshLOD 8 tmp\khox.obj tmp\khox-opt.obj
```

## Fostral
Using chunked export to produce 8 chunks of size 2K by 2K:
```bash
cargo run --bin convert --release -- -c 8 "c:\Program Files (x86)\GOG Galaxy\Games\Vangers\data\thechain\fostral\world.ini" tmp\fostral.obj
```