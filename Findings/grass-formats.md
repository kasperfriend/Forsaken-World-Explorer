# Grass Format & Streaming (`ECWDFILEGRASSTYPE`, `ECBSDFILEGRASS7`, `grasses.pck`)

**Evidence:** Verified directly from `ForsakenWorldv260ClientSource/SMElement/SMCommon/EC_WorldFile.h`, real `maps/s0/s0.ecwld` (v14) + `s0.ecbsd` (v15), and `resources/grasses.pck` (103 entries, 45 `DXT3` DDS textures verified in Phase 2).

## 1. Grass Type Table (`ECWDFILEHEADER::dwGrassOff`, `iNumGrassType`)

At `dwGrassOff` in `<zone>.ecwld`, `iNumGrassType` entries are stored sequentially. Each entry consists of:
1. `ECWDFILEGRASSTYPE` (24 bytes):
   - `bAlphaBlend: u8` (+ 3 bytes struct padding) — `0` = alpha-tested, `1` = alpha-blended
   - `iAlphaRef: i32` — alpha-test reference (`0..255`, normalized to `iAlphaRef / 255.0`)
   - `fSize: f32` — base blade size scale
   - `fSizeVar: f32` — size variation (`0.0..1.0`)
   - `fSoftness: f32` — wind softness (`0.0..2.0`)
   - `fSightRange: f32` — authored sight range in meters
2. `[len: u32][GBK texture/model path]` (e.g. `grasses\...`).

**Real-file verification on `maps/s0/s0.ecwld`:** `dwGrassOff = 54468`, `iNumGrassType = 2`. Type 0 decoded directly as `bAlphaBlend = 0`, `iAlphaRef = 84` (`~0.33`), `fSize = 1.0`, `fSizeVar = 0.3`, `fSoftness = 1.0`, `fSightRange = 50.0 m`.

## 2. Grass Area Placements (`ECBSDFILEGRASS7`)

In `.ecwld`, each block's `iNumGrass` `ECWDFILEDATAREF` entries point into `<zone>.ecbsd` where `ECBSDFILEGRASS7` (36 bytes) is stored:
- `dwTypeID: u32`, `vPos: [f32; 3]`, `fGridSize: f32`, `iGridCol: i32`, `iGridRow: i32`, `iSubTerrain: i32`, `iDataIndex: i32`.

**Real-file verification on `maps/s0/`:** `80` grass area references across the 256 blocks.

## 3. Streaming, Density & Wind Animation

- Placements come strictly from client `ECBSDFILEGRASS7` records; no placements are invented for empty areas.
- Camera-centered streaming selects nearby grass areas within `veg_radius_m`, bounded by `veg_max_instances` and `veg_loads_per_frame`.
- **Density setting (`Low` / `Medium` / `High`):** Deterministically subsamples client grass instances (`Low = 35%`, `Medium = 65%`, `High = 100%`) using a stable hash of the placement ID so instances never flicker as the camera moves.
- **Wind animation:** Vertex-shader sine displacement (`sin(time * 2.2 + world_x * 0.17 + world_z * 0.13) * 0.18 * softness * wind_weight`, documented approximation).
