# Static Scenery & Building Formats (`.ecwld`, `.ecbsd`, `.bmd`)

This document records the binary formats used for static scenery and building placements in **Forsaken World**, distinguishing directly verified fields from synthetic-fixture-tested and real-corpus-unverified fields.

## 1. World Ornament Placements (`maps/<zone>/<zone>.ecwld` + `<zone>.ecbsd`)

**Evidence:** Verified directly against `ForsakenWorldv260ClientSource/SMElement/SMCommon/EC_WorldFile.h`, `SMElementClient/EC_SceneBlock.cpp`, `SMElementClient/EC_Ornament.cpp`, and real `maps/s0/s0.ecwld` (v14) + `maps/s0/s0.ecbsd` (v15) from `Forsaken World 0.525 Client`.

- `.ecwld` v14: 116-byte `ECWDFILEHEADER` -> `iNumBlock * u32` block offsets -> 44-byte `ECWDFILEBLOCK9` -> `iNumTree * 16` (`ECWDFILETREE5`) -> `iNumWater * 8` (`ECWDFILEDATAREF`) -> `iNumOrnament * 8` (`ECWDFILEDATAREF { dwExportID, dwOffset }`).
- `.ecbsd` v15 at `dwOffset`: 40-byte `ECBSDFILEORNAMENT13` (`vPos[3]`, `vDir[3]`, `vUp[3]`, `bReflect`, `bRefract`, `bOptimize`, pad) followed immediately by `[len: u32][path bytes]` (`.lmd` mapped to `.bmd` per `EC_Ornament.cpp`).
- **Real-file verification on `maps/s0/`:** 2,737 block refs / **1,884** unique `dwExportID` placements, **0** non-`.bmd`/`.lmd` paths.

## 2. Static Mesh Format (`.bmd` — `CELBrushBuilding` + `A3DLitModel` + `A3DLitMesh`)

**Evidence:** `CELBrushBuilding` container header (`0x80000001` + `bCollideOnly`) and collision hull trailer verified directly from `EL_BrushBuilding.cpp`. Inner `A3DLitModel` / `A3DLitMesh` layouts are covered by CI synthetic fixtures for every supported version (`src/bmd.rs::version_matrix_tests`).

### Supported Version Matrix (CI Synthetic Fixtures vs Real Corpus)

| Header / Mesh Version | Synthetic CI Fixture | Real Corpus (`litmodels.pck` / `building.pck`) | Notes |
| --- | --- | --- | --- |
| `CELBrushBuilding` `0x80000001` (`bCollideOnly`) | Tested (`src/bmd.rs`) | Verified in `EL_BrushBuilding.cpp` | Optional 5-byte prefix after `MOXB` |
| `A3DLitModel` `0x10000001` | Tested (`src/bmd.rs`) | Unverified on full archive | Identical 48-byte transform header |
| `A3DLitModel` `0x10000002` | Tested (`src/bmd.rs`) | Unverified on full archive | Identical 48-byte transform header |
| `A3DLitMesh` `0x10000002` | Tested (`src/bmd.rs`) | Unverified on full archive | 28 B vertex stride, no day/night light arrays, no material tail |
| `A3DLitMesh` `0x10000003` | Tested (`src/bmd.rs`) | Unverified on full archive | 28 B vertex stride, day/night light arrays, no material tail |
| `A3DLitMesh` `0x10000004` | Tested (`src/bmd.rs`) | Unverified on full archive | 24 B vertex stride, day/night light arrays, no material tail |
| `A3DLitMesh` `0x10000005` | Tested (`src/bmd.rs`) | Unverified on full archive | 24 B vertex stride, day/night light arrays, 69 B + name material tail |
| `A3DLitMesh` `0x10000001` / `>= 0x10000006` | Rejected cleanly (`UnsupportedMeshVersion`) | Unverified on full archive | Returned as typed `BmdError`, never panics |

## 3. Alpha Handling & Phase 5 Transparent Scenery

- **Opaque & alpha-tested submeshes:** Drawn in the opaque pass (`alpha_cutoff = 0.0` or `0.5` discard) with depth test and depth write enabled.
- **Transparent / alpha-blended submeshes (`AlphaMode::TransparentDeferred`):** Now drawn in **Phase 5** after the opaque pass and before water, sorted back-to-front per instance and per submesh, with alpha blending enabled and depth write disabled (toggled via **Settings → World → Transparent scenery**).
- **Per-vertex day/night lighting:** Submeshes with `v3`–`v5` vertex light arrays blend `day_diffuse` and `night_diffuse` using the `EC_SunMoon` time-of-day `dn_factor`. Meshes without vertex light arrays (`v2`) fall back to directional + hemisphere lighting and are labeled `approximate` in Diagnostics.
