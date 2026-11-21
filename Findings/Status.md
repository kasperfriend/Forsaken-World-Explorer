# Forsaken World Explorer — Status Table

> **Allowed status strings (strict):**
> - `decoder verified on real data (file + offset evidence)`
> - `not working: <reason>`
> - `missing: <reason>`
> - `fixed in this round, pending user check`
> - `awaiting user confirmation`
>
> A feature is only marked working when the user confirms it on screen in the Windows artifact on real client files. Never report verified based on synthetic fixtures alone.

## Visible Results

| # | Visible Result | Status | Notes |
| --- | --- | --- | --- |
| 1a | Terrain layer textures (`textures.pck`, DDS DXT3) | `decoder verified on real data (dump-s0.txt)` | 5/5 `s0` layer textures (`512×512` DXT3, `5,242,880 B`) decode cleanly. |
| 1b | Terrain rendering (s0) | `not working: black output (Task 1)` | Root cause found & patched: `upload_rgba_view` used `Rgba8UnormSrgb` into linear `Rgba8Unorm` `scene_color` (crushing mid-tones via `pow(x, 2.2)`) and `terrain.wgsl` applied a second synthetic `shade` multiplier. Added `--export-terrain-pngs` + channel stats; awaiting user visual check. |
| 2 | Buildings and rocks (`.bmd` `0x10000006`) | `not working: v0x10000006 layout unknown (Task 2)` | `sample_litmodel_1000.bmd` (45,318/45,318 B, 12 submeshes, 28 collision hulls) traced and implemented (`mesh_flags_v6: u8` after AABB, bit meaning `unknown`). Awaiting user test on real client + 2 additional `.bmd` samples (`litmodel_1001.bmd` and one `building.pck` model). |
| 3 | Trees (SpeedTree DLL) & grass | `awaiting user confirmation` | Out of scope for this round; `s0` has 0 tree placements and 5 grass placements. |
| 4 | Lightmaps & day/night blending | `decoder verified on real data (dump-s0.txt)` | `s0_1.t2lm` and `s0_1.t2lm1` (`16×512×512` DXT1, `zero_frac=0.0000`) verified; unscaled `diffuse × lightmap` used until client modulate scale is confirmed. |
| 5 | Water planes | `awaiting user confirmation` | Out of scope for this round; `s0` has 0 water records. |
| 6 | Effects (`ECWLD` / `gfx.pck`) | `missing: ECWLD effect and gfx.pck formats not decoded yet` | Draws nothing. |
| 7 | Sky (`surfaces.pck` skyboxes) | `missing: surfaces.pck skyboxes not decoded yet (Issue #15)` | Draws dark clear background. |
| 8 | Minimap & world map tiles | `missing: surfaces/maps and minimap tile formats not decoded yet` | Shows camera coordinates only. |
| 9 | Teleport list | `missing: client teleport destination table not decoded yet` | Zone switcher only. |
| 10 | Audio (`sfx.pck` & loose music) | `missing: audio backend not started` | Out of scope for this round. |

## Core Infrastructure & Diagnostics

| Component | Status | Notes |
| --- | --- | --- |
| Placement & Diagnostics counters (`src/scenery.rs`, `src/ui.rs`) | `fixed in this round, pending user check` | Enforces `total = not_in_range + in_range` and `in_range = loaded + failed + pending` (each placement counted once), verified by CI unit test. |
| Mouse capture / release & ESC menu (`src/input.rs`) | `awaiting user confirmation` | Kept unchanged. |
| Zone selection (`s0` default) & active zone title | `awaiting user confirmation` | Confirmed by user on `s0`. |
| Diagnostic & Export CLI (`--dump-zone`, `--export-terrain-pngs`, `--bmd-trace`, `--extract-entry`, `--hexdump`, `--export-terrain-block`, `--export-texture-png`) | `fixed in this round, pending user check` | All outputs `< 1 MB`. |
