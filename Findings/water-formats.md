# Water Formats (`ECBSDFILEWATER`, `BSData/*.dat`, Water Normal Maps)

This document records how Forsaken World stores water surfaces and water parameters, with evidence per field and open items for local `water-survey` runs.

## 1. World Water Area References (`EC_WorldFile.h`, `EC_SceneBlock.cpp`)

**Evidence:** Verified directly from `ForsakenWorldv260ClientSource/SMElement/SMCommon/EC_WorldFile.h` (`ECWDFILEBLOCK9`, `ECWDFILEDATAREF`, `ECBSDFILEWATER`, `ECWDDTFILEHEADER`) and `SMElementClient/EC_SceneBlock.cpp` (`LoadWater`).

1. Each `.ecwld` block lists `iNumWater` `ECWDFILEDATAREF { dwExportID: u32, dwOffset: u32 }` entries immediately after `iNumTree` tree records and before `iNumOrnament` ornament records.
2. Seeking to `dwOffset` in `<zone>.ecbsd` reads `ECBSDFILEWATER` (8 bytes):
   - `iSubTerrain: i32` — index of the sub-terrain data file (`maps/<zone>/BSData/<iSubTerrain>.dat`)
   - `iDataIndex: i32` — index of the water area inside that `BSData` file (`ECWDDTFILE_VERSION = 2`)
3. **Real-file observation on `maps/s0/`:** Across all 256 blocks of `maps/s0/s0.ecwld`, `iNumWater = 0` (no water areas are placed in `s0`), while `maps/s0/BSData/1.dat` is referenced in `s0.ecbsd`. Other zones (`s1`–`s40`) with lakes/rivers carry non-zero `iNumWater` counts and can be surveyed locally with `water-survey`.

## 2. Water Plane Geometry, Normal Maps & Underwater Parameters

- **Surface definition (`A3DWaterArea` in `BSData/*.dat`):** Stores water area center `(cx, cz)`, height `water_height`, grid cell size `grid_size`, cell counts `(width, height)`, water color (`dwWaterColor`), and wave/flow speed. Individual `BSData/*.dat` water payloads from watery zones (`s1`–`s40`) were not downloaded in CI; the parser supports both `ECBSDFILEWATER` references and `A3DWaterArea` records, and falls back cleanly on unrecognized versions without panicking.
- **Animated normal textures:** Client water normal maps live in `resources/textures.pck` (and `resources/shaders.pck` / `surfaces.pck`). When a client normal map is decoded it is bound directly; when absent (or in synthetic preview), a procedural tileable tangent-space normal map is generated and scrolled across two opposing UV layers (`speed_a = (0.03, 0.02)`, `speed_b = (-0.022, 0.028)` UV/sec — documented default timing).
- **Underwater fog & tint:** Driven directly by `ECBSDFILEBOXAREA15` fields `fUWFogStart`, `fUWFogEnd`, `fUWFogDensity`, `dwUWFogCol` (day) and `fUWFogStart_n`, `fUWFogEnd_n`, `fUWFogDensity_n`, `dwUWFogCol_n` (night), blended by `dn_factor`.
