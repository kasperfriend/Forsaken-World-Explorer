# Forsaken World Explorer — Status Table

> **Allowed status strings (strict):**
> - `decoder verified on real data (dump: <file>)`
> - `missing: <reason>`
> - `awaiting user confirmation`
>
> Nothing is marked working until confirmed visually by the user in the Windows artifact on real client files. No fallback colors, approximate lighting, placeholder sky, or synthetic scenes exist in the default path.

## Visible Results (Focus: Items 1 & 2)

| # | Visible Result | Status | Notes |
| --- | --- | --- | --- |
| 1 | Terrain from `maps/s0/` with real heights and textures | `awaiting user confirmation` / `decoder verified on real data (dump: dump-s0.txt)` | `s0.trn2`, `s0_1.t2bk` (256/256 blocks, `68.91..226.16 m`), `s0_1.t2mk` (16 mask areas, 43 layers, `256×256`), and `s0_1.t2lm`/`t2lm1` (`16×512×512` DXT1) verified on real files. Grey fallback removed (opt-in wireframe debug mode off by default). Unified archive path normalizer + `rec_len` fix + `PckArchive::open` error reporting added. |
| 2 | Buildings and rocks from `s0.ecbsd` placements on terrain | `awaiting user confirmation` / `decoder verified on real data (dump: dump-s0.txt)` | `s0.ecwld`/`s0.ecbsd` (`1,884` placements, `0.33 m` median `|dy|` under Angelica mapping) verified on real files. `.bmd` lookup searches all `.pck` archives in `resources/`, groups failures by reason with 5 sample paths, and counts `.bmd` versions. |
| 3 | Trees (SpeedTree DLL) & grass | `awaiting user confirmation` | Out of scope for this round; `s0` has 0 tree placements and 5 grass placements. |
| 4 | Lightmaps & day/night blending | `awaiting user confirmation` / `decoder verified on real data (dump: dump-s0.txt)` | Out of scope for this round; `s0_1.t2lm` and `s0_1.t2lm1` headers/DDS verified. |
| 5 | Water planes | `awaiting user confirmation` | Out of scope for this round; `s0` has 0 water records. |
| 6 | Effects (`ECWLD` / `gfx.pck`) | `missing: ECWLD effect and gfx.pck formats not decoded yet` | Draws nothing. |
| 7 | Sky (`surfaces.pck` skyboxes) | `missing: surfaces.pck skyboxes not decoded yet (Issue #15)` | Draws dark clear background; approximate blue gradient removed from default path. |
| 8 | Minimap & world map tiles | `missing: surfaces/maps and minimap tile formats not decoded yet` | Shows camera coordinates only. |
| 9 | Teleport list | `missing: client teleport destination table not decoded yet` | Zone switcher only. |
| 10 | Audio (`sfx.pck` & loose music) | `missing: audio backend not started` | Out of scope for this round. |

## Core Infrastructure & Input

| Component | Status | Notes |
| --- | --- | --- |
| Mouse capture / release & ESC menu (`src/input.rs`) | `awaiting user confirmation` | 3 CPU unit tests pass; kept unchanged from previous round. |
| Unified PCK path normalizer (`src/pck.rs`) | `decoder verified on real data (dump: dump-s0.txt)` | Byte-for-byte unit test covers all 5 GBK layer paths from real `s0.trn2`. |
| Real-zone single-file dump (`--dump-zone s0`) | `decoder verified on real data (dump: dump-s0.txt)` | Reports Tasks 1–4 plus `Expected for this client`. |
| Diagnostics overlay (1 Hz) | `awaiting user confirmation` | Reports actual drawn terrain blocks, loaded/failed texture counts & bytes, grouped `.bmd` failure reasons, and explicit reasons for every `0` counter. |
