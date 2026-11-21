# Sky, Lighting & Environmental Fog Formats (`EC_SunMoon`, `ECBSDFILEBOXAREA15`, `A3DSkySphere`)

This document records the verified Forsaken World sky, time-of-day, lighting, and fog data formats with source-file line citations.

## 1. Client Sky Rendering Path (`EC_World.cpp` + `ECBSDFILEBOXAREA15`)

- **Source citations (`ForsakenWorldv260ClientSource/SMElement/SMElementClient/EC_World.cpp`):**
  - `EC_World.cpp:865–888` (`CECWorld::LoadSky`): instantiates `A3DSkySphere`, initializes it with 3 textures (`Cap`, `SurroundF`, `SurroundB`), sets `SetFlySpeedU(0.003f)` and `SetFlySpeedV(0.003f)`, and attaches it to `A3DEngine::SetSky`.
  - `EC_World.cpp:1367–1374` (`CECWorld::Render`): renders `m_pA3DSky->Render()` **first**, behind terrain.
- **BoxArea sky texture paths (`EC_WorldFile.h`, `ECBSDFILEBOXAREA15`, verified on real `maps/s0/s0.ecbsd`):**
  - Offset `+272`: `fCapTileSpeed` (`f32`, `8.0` on `s0`), `+276`: `fCapRotSpeed` (`f32`, `0.0` on `s0`).
  - Offset `+280`: **6 length-prefixed GBK sky texture paths** (3 Day: `Cap`, `SurroundF`, `SurroundB`; 3 Night: `Cap`, `SurroundF`, `SurroundB`):
    - `export_id = 2` (`iDefArea = 1`, default zone sky at `s0.ecbsd` offset `215`):
      - Day & Night Cap: `希腊天空\静谧花园顶.bmp`
      - Day & Night SurroundF: `希腊天空\巨人峰夜天1.bmp`
      - Day & Night SurroundB: `希腊天空\巨人峰夜天2.bmp`
    - `export_id = 53` (`iDefArea = 0`, regional override at `s0.ecbsd` offset `5184`):
      - Day & Night Cap: `登录界面天空\城堡白天顶.bmp`
      - Day & Night SurroundF: `S0天空\S0侧1.bmp`
      - Day & Night SurroundB: `S0天空\S0侧2.bmp`
- **No synthetic sky fallback:** `src/shaders/sky.wgsl` renders ONLY when all 3 real client sky textures resolve and decode (`.dds` or `.bmp`) from `surfaces.pck`/`textures.pck`. If any sky texture is missing, the sky pass is skipped (plain clear color) and the UI/dump reports `Sky: missing: <reason>`.

## 2. Time-of-Day & Day/Night Blend (`EC_SunMoon.h`)

**Evidence:** Directly verified from `ForsakenWorldv260ClientSource/SMElement/SMElementClient/EC_SunMoon.h`.

- `NIGHT_DAY_START = 3.0 / 24.0` (`03:00`) -> `NIGHT_DAY_END = 7.0 / 24.0` (`07:00`): `dn_factor` ramps `1.0 -> 0.0`.
- `DAY_NIGHT_START = 18.0 / 24.0` (`18:00`) -> `DAY_NIGHT_END = 21.0 / 24.0` (`21:00`): `dn_factor` ramps `0.0 -> 1.0`.
- `MID_NOON = 12.0 / 24.0`, `SUN_MAX_PITCH = 70°`, `MOON_MAX_PITCH = 60°`.
