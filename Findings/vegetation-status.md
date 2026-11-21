# Vegetation Subsystem Status (Phase 6)

Status values: `implemented`, `implemented-partial`, `verified-cpu`, `not-verified-without-gpu`, `not-started`.

| Vegetation Sub-Feature | Status | Details & Known Limits |
| --- | --- | --- |
| SpeedTree 32-bit helper source & Windows CI build (`tools/speedtree/fw-speedtree-helper`) | `verified-cpu` | Built in Windows CI for `i686-pc-windows-msvc` and packaged at `tools/fw-speedtree-helper.exe` with SHA-256 in `SHA256SUMS.txt`; real DLL execution not run in CI |
| Helper protocol, DLL SHA-256 trust check, timeout & crash isolation | `verified-cpu` | Unit-tested in `src/vegetation.rs` and `--self-test` using fake helper executables (canned output, error exit codes, timeout kill, crash recovery) |
| `FWST` v1 tree cache (`<root>/cache/speedtree-v1/`), LOD selection & instance dedup | `verified-cpu` | Unit-tested on synthetic tree fixtures; real `.spt` baking requires local Windows test with `update/speedtreert.dll` |
| Tree rendering (trunk + alpha-tested leaves + transparent canopy) | `not-verified-without-gpu` | `src/shaders/vegetation.wgsl` validated by `naga`; Mesa `lavapipe` renders synthetic tree trunk + alpha-tested leaves (pipeline execution only, not visual parity) |
| Tree wind & leaf motion | `implemented-partial` | Vertex-shader sine wind approximation (`approximate` labeled in Diagnostics and README until runtime-sampled leaf-rocking tables are baked from real `.spt` files) |
| Grass type/placement decoding (`ECWDFILEGRASSTYPE`, `ECBSDFILEGRASS7`), streaming & density | `verified-cpu` | Verified on `maps/s0/` (`2` grass types, `80` grass refs); density thinning (`Low`/`Medium`/`High`) and dedup unit-tested |
| Grass GPU rendering & wind animation | `not-verified-without-gpu` | `src/shaders/vegetation.wgsl` validated by `naga`; Mesa `lavapipe` renders synthetic grass patch (pipeline execution only, not visual parity) |
| Survey tools (`speedtree-survey`, `grass-survey`) | `verified-cpu` | Compiled & checked in Linux and Windows CI; real client run pending local user test |
