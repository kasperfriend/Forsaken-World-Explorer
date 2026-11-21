# Sky, Lighting & Environmental Fog Formats (`EC_SunMoon`, `ECBSDFILEBOXAREA15`)

This document records the verified Forsaken World sky, time-of-day, lighting, and fog data formats, with evidence per field and explicit notes where defaults or approximations remain.

## 1. Time-of-Day & Day/Night Blend (`EC_SunMoon.h`)

**Evidence:** Directly verified from `ForsakenWorldv260ClientSource/SMElement/SMElementClient/EC_SunMoon.h`.

- Time-of-day is stored as a normalized day fraction `m_vTimeOfTheDay` in `[0.0, 1.0)` (`0.0 = 00:00`, `1.0 = 24:00`, i.e. `hours / 24.0`), wrapping cleanly across midnight.
- **Client transition constants:**
  - `NIGHT_DAY_START = 3.0 / 24.0` (`03:00`) -> `NIGHT_DAY_END = 7.0 / 24.0` (`07:00`): `dn_factor` ramps from `1.0` (full night) to `0.0` (full day).
  - `DAWN_TIME = 3.5 / 24.0` (`03:30`), `MID_NOON = 12.0 / 24.0` (`12:00` — viewer default time-of-day), `SUN_SET = 21.0 / 24.0` (`21:00`).
  - `DAY_NIGHT_START = 18.0 / 24.0` (`18:00`) -> `DAY_NIGHT_END = 21.0 / 24.0` (`21:00`): `dn_factor` ramps from `0.0` (full day) to `1.0` (full night).
  - `SUN_MAX_PITCH = 70°`, `MOON_MAX_PITCH = 60°`.
- Both terrain day/night lightmaps (`t2lm` + `t2lm1`) and static `.bmd` per-vertex `day_diffuse` / `night_diffuse` colors blend using this exact `dn_factor` (`mix(day, night, dn_factor)`).

## 2. Regional Sky, Lighting & Fog Box Areas (`ECBSDFILEBOXAREA15`)

**Evidence:** Directly verified from `ForsakenWorldv260ClientSource/SMElement/SMCommon/EC_WorldFile.h` (`struct ECBSDFILEBOXAREA14` / `ECBSDFILEBOXAREA15`) and real `maps/s0/s0.ecwld` (v14) + `maps/s0/s0.ecbsd` (v15).

In `.ecwld` (v9+), each block's `iNumBoxArea` entries are `ECWDFILEDATAREF { dwExportID, dwOffset }` records pointing into `<zone>.ecbsd`. At `dwOffset`, `<zone>.ecbsd` (v15) stores `ECBSDFILEBOXAREA15`:

| Offset (bytes) | Field(s) | Type | Meaning |
| --- | --- | --- | --- |
| `0..8` | `iDefArea`, `dwAreaFlags` | `i32`, `u32` | Non-zero marks the default zone-wide atmosphere box |
| `8..68` | `vPos`, `vExt`, `vAxisX`, `vAxisY`, `vAxisZ` | `5 × [f32; 3]` | Oriented bounding box center, half-extents, and basis |
| `68..84` | `fFogStart`, `fFogEnd`, `fFogDensity`, `dwFogColor` | `3 × f32`, `u32` | Daylight environmental fog parameters (ARGB) |
| `84..100` | `fUWFogStart`, `fUWFogEnd`, `fUWFogDensity`, `dwUWFogCol` | `3 × f32`, `u32` | Daylight underwater fog parameters (ARGB) |
| `100..116` | `fFogStart_n`, `fFogEnd_n`, `fFogDensity_n`, `dwFogColor_n` | `3 × f32`, `u32` | Night environmental fog parameters (ARGB) |
| `116..144` | `vLightDir`, `dwAmbient`, `dwDirLtCol`, `dwAmbient_n`, `dwDirLtCol_n` | `[f32; 3]`, `4 × u32` | Directional light vector & day/night ambient + sun colors |
| `144..220` | `vSkySpeed`, `dwTrnLtScale`, `dwTransTime`, rain/snow, `fSunPower`, cloud colors, night UW fog | mixed | Sky cloud motion, sun power, night underwater fog (`fUWFogStart_n..dwUWFogCol_n`) |

**Real-file verification on `maps/s0/`:** `s0.ecwld` + `s0.ecbsd` contains **2** unique box areas:
- `dwExportID = 2` (`iDefArea = 1`, half-extents `512 × 512`, zone-wide default atmosphere)
- `dwExportID = 53` (`iDefArea = 0`, regional atmosphere override inside `s0`)

## 3. Documented Approximations

- **Sky dome rendering:** Uses a procedural time-of-day zenith/horizon gradient + sun/moon disc driven by `EC_SunMoon` pitch and `ECBSDFILEBOXAREA15` day/night fog & cloud colors (`approximate` until full client skybox texture sets from `surfaces.pck` are decoded).
- **Scenery without vertex light arrays (`v2` `.bmd`):** Falls back to directional + hemisphere lighting driven by `ECBSDFILEBOXAREA15` `vLightDir` / `dwAmbient` / `dwDirLtCol` and is labeled `approximate` in the UI.
- **Scenery with `v3`–`v5` `.bmd` vertex light arrays:** Uses the real decoded `day_diffuse` / `night_diffuse` colors blended by `dn_factor` (labeled `client day/night` in Diagnostics).
