# Forsaken World Explorer — Status Table

> **Allowed status strings (strict):**
> - `decoder verified on real data (dump: <file>)`
> - `blocked: <reason>`
> - `not working: <reason>`
> - `missing: <reason>`
> - `awaiting user confirmation`
>
> Nothing is marked working or visually verified until confirmed by the user on screen in the Windows artifact on real client files. No fallback colors, approximate lighting, placeholder sky, or synthetic scenes exist in the default path.

## Visible Results

| # | Visible Result | Status | Notes |
| --- | --- | --- | --- |
| 1a | Terrain layer textures (`textures.pck`, DDS DXT3) | `decoder verified on real data (dump: dump-s0.txt)` | 5/5 `s0` layer textures (`512×512` DXT3, `5,242,880 B`) and 16/16 `login` layer textures (`16,777,216 B`) resolve and decode cleanly. |
| 1b | Terrain streaming (`t2bk` polar normals) | `awaiting user confirmation` | Previously `blocked: polar normal header read error` (`login_1.t2bk` had uninitialized MSVC padding bytes `0x1a 0x12 0x4c` at offsets `1..4`). Fixed in `src/terrain_block.rs` (`nor_bytes[0] == 1`) with 64-byte real header unit test; streams blocks cleanly for both `s0` and `login`. |
| 1c | Terrain rendering (grey terrain issue) | `not working: cause under investigation (Task 3)` | Root causes identified & patched: (1) `login` failed polar normals -> 0 streamed blocks / 0 draw calls; (2) hardcoded `revision: 1` skipped GPU texture re-upload on zone switch; (3) `terrain.wgsl` washed out 30% of `lm` toward white. `--dump-zone` now reports first-block bindings and draw calls; `--export-texture-png` exports decoded DDS as PNG. Awaiting user visual confirmation on `s0`. |
| 2 | Buildings and rocks (`.bmd` decoding & rendering) | `blocked: version 0x10000006 unsupported (1884/1884 s0 models)` | Phase 4 `.bmd` `verified on real data` claim retracted. `s0.ecwld`/`s0.ecbsd` placements (`1,884`, `0.33 m` median `|dy|`) and `litmodels.pck` lookup work, but all 1,884 `s0` models use `A3DLitMesh` `0x10000006` (not in `ForsakenWorldv260ClientSource`). Awaiting `sample_litmodel_1000.bmd` + hexdump from user via Task 5 export tools. |
| 3 | Trees (SpeedTree DLL) & grass | `awaiting user confirmation` | Out of scope for this round; `s0` has 0 tree placements and 5 grass placements. |
| 4 | Lightmaps & day/night blending | `decoder verified on real data (dump: dump-s0.txt)` | `s0_1.t2lm` and `s0_1.t2lm1` (`16×512×512` DXT1) decoded; visual result awaiting user confirmation. |
| 5 | Water planes | `awaiting user confirmation` | Out of scope for this round; `s0` has 0 water records. |
| 6 | Effects (`ECWLD` / `gfx.pck`) | `missing: ECWLD effect and gfx.pck formats not decoded yet` | Draws nothing. |
| 7 | Sky (`surfaces.pck` skyboxes) | `missing: surfaces.pck skyboxes not decoded yet (Issue #15)` | Draws dark clear background; approximate blue gradient removed from default path. |
| 8 | Minimap & world map tiles | `missing: surfaces/maps and minimap tile formats not decoded yet` | Shows camera coordinates only. |
| 9 | Teleport list | `missing: client teleport destination table not decoded yet` | Zone switcher only. |
| 10 | Audio (`sfx.pck` & loose music) | `missing: audio backend not started` | Out of scope for this round. |

## Core Infrastructure, Input & Export Tools

| Component | Status | Notes |
| --- | --- | --- |
| Mouse capture / release & ESC menu (`src/input.rs`) | `awaiting user confirmation` | 3 CPU unit tests pass; kept unchanged. |
| Zone selection & active zone display (`src/ui.rs`) | `awaiting user confirmation` | Default zone on launch is now `s0` (not alphabetical `login`); active zone shown in window title and top of Diagnostics panel. |
| Unified PCK path normalizer (`src/pck.rs`) | `decoder verified on real data (dump: dump-s0.txt)` | Byte-for-byte unit test covers all 5 GBK layer paths from real `s0.trn2`. |
| Single-file dump (`--dump-zone`) & Export CLI tools (`--extract-entry`, `--hexdump`, `--export-terrain-block`, `--export-texture-png`) | `awaiting user confirmation` | Added for small (< 1 MB) sample extraction and terrain texture PNG inspection. |
| Full per-failure logging & per-zone summary line (`src/ui.rs`) | `awaiting user confirmation` | Logs every failure with reason (no log sampling), Load World header (zone, client root, archive list), and end-of-load `[Zone Summary]` line. |
