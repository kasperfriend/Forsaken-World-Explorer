# Findings Index

Every format claim, client layout fact, and verification limit for **Forsaken World Explorer** is recorded here with its evidence source and what remains unverified.

## Documents

- [client-inventory.md](client-inventory.md) — Top-level folder layout, file types, counts, sizes, `speedtreert.dll` PE32 verification, `pem.exe` fingerprint status (`pending`), and Drive source-tree overview from the inspected Google Drive client (`Forsaken World 0.525 Client` / `ForsakenWorldv260ClientSource`).
- [archive-format.md](archive-format.md) — Verified Forsaken World `.pck` / `.pkx` binary layout, guard keys (`0xa508bdc4` / `0x853567c8`), XOR entry-table keys (`0x21c31a3f` / `0x399f3a1a`), GBK path encoding, zlib compression, and split `.pkx` continuation model.
- [world-formats.md](world-formats.md) — Verified Forsaken World `.trn2` (v1), `.t2bk` (v6 36-byte block header, heights, polar normals, day/night vertex colours), `.t2lm`/`.t2lm1` (v2 512x512 DXT1 DDS lightmaps), `.t2mk` (v2 masks), `.ecwld` (v14), `.ecbsd` (v15 `.bmd` placements), and DDS (`DXT1`/`DXT3`) formats from `maps/s0/` and `resources/grasses.pck`.
- [gpu-stack.md](gpu-stack.md) — Graphics crate and backend decision (`wgpu 24`, `eframe 0.31`, `glam 0.29`, `naga 24`), resolution-scaling pipeline (`10%–200%` 3D pass with native UI overlay), and CPU/lavapipe verification limits.
- [scenery-formats.md](scenery-formats.md) — Verified `.ecwld` (v14) + `.ecbsd` (v15) `ECBSDFILEORNAMENT13` placement layout, `CELBrushBuilding` + `A3DLitModel`/`A3DLitMesh` `.bmd` version fixture vs corpus matrix, and Phase 5 transparent/lighting updates.
- [scenery-budget.md](scenery-budget.md) — Camera-centered multi-frame scenery streaming, instance deduplication, `<root>/cache/scenery-v1/` caching, 32 MiB pinned-batch texture cache enforcement, transparent scenery counters, and unmeasured real-world memory items.
- [sky-fog-formats.md](sky-fog-formats.md) — Verified `EC_SunMoon.h` day/night transition constants (`03:00..07:00`, `18:00..21:00`, `12:00` midday) and `ECBSDFILEBOXAREA15` regional sky/fog/lighting/underwater struct verified on `maps/s0/`.
- [water-formats.md](water-formats.md) — Verified `ECBSDFILEWATER` references in `.ecwld`/`.ecbsd`, `BSData/*.dat` sub-terrain water storage, animated normal map timing, and underwater fog parameters.
- [water-status.md](water-status.md) — Per-subfeature Phase 5 water status table (normals, planar reflection, refraction, shoreline edge blend, underwater tint/fog, culling/scissor).
- [speedtree-formats.md](speedtree-formats.md) — Verified `update/speedtreert.dll` PE32 x86 exports (146 `CSpeedTreeRT` symbols, SHA-256), `.ecwld` tree type/placement tables, 3 LOD tiers, and alpha reference data (resolves [#6](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/6)).
- [speedtree-bridge.md](speedtree-bridge.md) — 32-bit out-of-process `tools/fw-speedtree-helper.exe` architecture adopted from reference, `FWST` v1 cache format (`<root>/cache/speedtree-v1/`), DLL SHA-256 trust check, timeout/crash isolation, and reproducible `i686-pc-windows-msvc` build flags.
- [grass-formats.md](grass-formats.md) — Verified `ECWDFILEGRASSTYPE` (`iAlphaRef=84`, `fSize=1.0`, `fSoftness=1.0`, `fSightRange=50.0`) and `ECBSDFILEGRASS7` placements on `maps/s0/` (`2` types, `80` placements), plus density thinning and sine wind approximation.
- [vegetation-status.md](vegetation-status.md) — Per-subfeature Phase 6 vegetation status table (SpeedTree helper, tree LOD, wind, grass streaming, grass density).
- [Status.md](Status.md) — Subsystem status table (`implemented`, `implemented-partial`, `verified-cpu`, `not-verified-without-gpu`, `verified-by-user`, `not-started`), evidence links, open gates, and user verification notes.
- [Unsupported-client-paths.md](Unsupported-client-paths.md) — Client features, server-only data, and deferred systems not handled by this viewer, with reasons and linked issues.

## Status Legend

- `not-started` — No implementation yet.
- `implemented` — Code written, not yet verified on CPU tests or by the user.
- `implemented-partial` — Partially implemented or contains documented approximations.
- `verified-cpu` — Verified by automated CPU-only unit, fixture, or CI checks; visual/GPU output is **not** verified.
- `not-verified-without-gpu` — Requires a live GPU and desktop session for visual/interactive verification.
- `verified-by-user` — Confirmed locally by the user on a real Forsaken World client and GPU. Only the user can assign this status.
