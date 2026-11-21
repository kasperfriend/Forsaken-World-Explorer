# Forsaken World Explorer — Status Table

> **Allowed status strings (strict):**
> - `decoder verified on real data (file + offset)`
> - `working (user confirmed)`
> - `not working`
> - `missing: <reason>`
>
> A feature is only marked `working (user confirmed)` after the user confirms it on screen in the Windows artifact on real client files. No fallback colors, synthetic sky, or approximations exist in the default path.

## Visible Results

| # | Visible Result | Status | Notes |
| --- | --- | --- | --- |
| 1a | Buildings (`s0`, 1,884 placements, `A3DLitMesh 0x10000006`) | `working (user confirmed)` | Confirmed on `s0` by user (`1,884/1,884` resolved, `2,993` submeshes, `64/64` sampled textures). |
| 1b | Trees (SpeedTree DLL) | `working (user confirmed)` | Confirmed on `s0` by user. |
| 2 | Sky (`A3DSkySphere` from `ECBSDFILEBOXAREA15` + `surfaces.pck`/`textures.pck`) | `not working` | `EC_World.cpp:865-888` & `s0.ecbsd +280` sky paths decoded and wired into `sky.wgsl` (BMP/DDS). Draws nothing (`Sky: missing: <reason>`) if sky textures are absent. Awaiting user confirmation + `--list-entries surfaces.pck --filter sky`. |
| 3 | Ground detail (terrain textures & splatting) | `not working` | Replaced blurry 512×512 whole-zone diffuse bake with real multi-layer GPU shader splatting (`512×512` mipmapped `texture_2d_array`, `t2mk` `scale_u = [0.1, 0.1, 0.0556, 0.0833, 0.0667]`, trilinear + 16× aniso). Awaiting user visual confirmation. |
| 4 | Terrain layer textures (`textures.pck`, DDS DXT3) | `decoder verified on real data (dump-s0.txt + s0_1.t2mk bytes 84..224)` | 5/5 `s0` layer DDS (`512×512` DXT3) + 28-byte `t2mk` layer scales verified. |
| 5 | Lightmaps (`s0_1.t2lm` / `s0_1.t2lm1`) | `decoder verified on real data (dump-s0.txt)` | Raw `t2lm`/`t2lm1` (`16×512×512` DXT1) multiplied unscaled with GPU splat diffuse. |
| 6 | Water planes | `not working` | Out of scope for this round. |
| 7 | Effects (`ECWLD` / `gfx.pck`) | `missing: ECWLD effect and gfx.pck formats not decoded yet` | Draws nothing. |
| 8 | Minimap & world map tiles | `missing: surfaces/maps and minimap tile formats not decoded yet` | Shows camera coordinates only. |
| 9 | Teleport list | `missing: client teleport destination table not decoded yet` | Zone switcher only. |
| 10 | Audio (`sfx.pck` & loose music) | `missing: audio backend not started` | Out of scope for this round. |

## Performance, Counters & Tooling

| Component | Status | Notes |
| --- | --- | --- |
| Movement smoothness / lag fix (`src/terrain_gpu.rs`, `src/scenery.rs`, `src/ui.rs`) | `not working` | Fixed 60×/sec GPU texture & buffer re-upload storm (`gpu_tex_cache` + unchanged-batch revision skip), moved `.bmd`/DDS decoding to bounded `SceneryDecodeWorker` thread, added per-stage avg/max frame timing to 1 Hz Diagnostics & `--bench-flight`. Awaiting user feel check on `s0` & `s40`. |
| Placement & drawn counters (`src/scenery.rs`, `src/ui.rs`) | `not working` | Enforces `total = not_in_range + in_range`, `in_range = loaded + failed + pending`, and `drawn <= loaded` with explicit reasons (`collide_only`, `trans_off`, `empty`), verified by CI unit tests. Awaiting user check on `s40`. |
| Mouse capture / release & ESC menu (`src/input.rs`) | `working (user confirmed)` | Kept unchanged. |
