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

## Placement Height & Axis-Mapping Verification (`maps/s0/`)

Verified across all **1,884** unique ornament placements in `maps/s0/s0.ecwld` (v14) + `maps/s0/s0.ecbsd` (v15) against the decoded `maps/s0/s0_1.t2bk` (v6, 256/256 blocks) heightfield using the renderer's exact `sample_angelica_height` sampler (no fudge offset applied):

```text
| Candidate Mapping | Median |dy| | Min |dy| | Max |dy| | >1m | >5m | >20m |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Angelica (x, y, z) on Z=+512-row*64-gz*2 | 0.33 m | 0.00 m | 139.30 m | 404 | 170 | 38 |
| X negated (-x, y, z) | 5.41 m | 0.00 m | 139.30 m | 1777 | 1143 | 238 |
| X/Z swapped (z, y, x) | 17.16 m | 0.00 m | 144.82 m | 1660 | 1466 | 829 |
| X&Z negated (-x, y, -z) | 22.06 m | 0.00 m | 148.28 m | 1805 | 1697 | 1210 |
| Z negated (x, y, -z) | 25.17 m | 0.02 m | 139.30 m | 1828 | 1784 | 1347 |
| Non-flipped terrain Z=-512+row*64+gz*2 | 25.17 m | 0.02 m | 139.30 m | 1828 | 1784 | 1347 |
| Y/Z swapped (x, z, y) | 187.73 m | 0.24 m | 495.55 m | 1864 | 1801 | 1681 |
```

### Evidence & Outlier Explanation
- **Winning mapping:** Angelica `(x, y, z)` on `Z = +world_length/2 - row*block_size - gz*step` produces a **0.33 m** median `|dy|` across all 1,884 placements (**1,480 / 1,884** within 1 m; **1,714 / 1,884** within 5 m), beating every alternative sign/axis mapping by 16× to 560×.
- **Outliers (`|dy| > 20 m`, 38 of 1,884 — left unchanged with zero fudge offset):**
  - `id=2275` at `(0.00, 0.00, 0.00)` (`terrain_h = 139.30 m`, `dy = -139.30 m`, `1\litmodel_3218.bmd`): origin-baked world-space composite mesh whose geometry carries its own world-space offset.
  - Perimeter arena cliff/wall/arch structures (`id=72`, `id=479`, `id=4`, `id=48`, `id=73`, `id=75`, `id=38`, `id=74`, `id=128`) whose authored pivots sit below steep perimeter cliffs or elevated above the arena floor. No fudge offset is added.
