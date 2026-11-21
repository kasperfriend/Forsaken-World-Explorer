# Forsaken World Explorer — Honest Status Table

> **Notice (2026-11-21 reset):** Earlier status tables marked rendering, scenery, sky, water, grass, and trees as `verified-cpu` or `implemented` based on synthetic fixtures and Mesa `lavapipe` smoke tests. When tested on real client data, the Phase 5 artifact showed flat green terrain on a fake blue sky with no buildings, no trees, no effects, and broken mouse look. All overstated claims have been withdrawn below.
>
> **Allowed status values:**
> - `working on real data` — confirmed visually by the user in the Windows artifact on real client files (currently **none**).
> - `decoder verified on real data` — CPU decoder was executed on real Forsaken World client files and produced verified dump output.
> - `pipeline runs on synthetic fixture` — shader/GPU pipeline compiled and ran on a synthetic fixture (proves pipeline execution only, NOT that the game world renders).
> - `not working` — withdrawn from earlier claims; awaiting user confirmation on real client data after root-cause fixes.
> - `missing` — client format or asset not yet decoded; viewer draws nothing for this part and reports it as missing.
> - `not started` — work has not begun.
>
> **Important:** The approximate sky gradient and the synthetic preview scene are **not** game worlds. They have been removed from the default path (the synthetic pipeline test scene is hidden behind a collapsible Developer checkbox, off by default).

## Ten Required Visible Results (in verification order)

| # | Visible Result | Status | Honest Evidence / Gap |
| --- | --- | --- | --- |
| 1 | Terrain from `maps/s0/` with real heights and textures | `not working` (awaiting user test); terrain/mask/lightmap `decoder verified on real data` | Real `s0.trn2`, `s0_1.t2bk` (256/256 blocks, `68.91..226.16 m`, `height(0,0)=139.30 m`), `s0_1.t2mk` (16 mask areas, 43 layers, `256×256` masks), and `s0_1.t2lm`/`t2lm1` (16×`512×512` DXT1) decoded on real files. Fixed Angelica centered coordinates (`[-512..+512]`, `Z = +512 - row*64 - gz*2`), removed green fallback, and fixed `textures.pck` prefix stripping. Needs user verification in Windows artifact. |
| 2 | Buildings and rocks from `s0.ecbsd` placements at correct terrain positions | `not working` (awaiting user test); placements `decoder verified on real data` | Real `s0.ecwld`/`s0.ecbsd` decoded (`1,884` unique placements, `0.33 m` median delta from Angelica terrain height). Fixed `litmodels.pck` prefix lookup (`s0/1/litmodel_*.bmd` first) and removed fake tan texture fallback. Needs user verification in Windows artifact with real `litmodels.pck`/`textures.pck`. |
| 3 | Trees from SpeedTree runtime DLL (32-bit helper) and grass from client placements | `not working` (awaiting user test); grass placements & DLL fingerprint `decoder verified on real data` | `s0` has `0` tree placements and `2` grass types (`5` unique placements); `update/speedtreert.dll` SHA-256 verified (`1a62ff4d...`). 32-bit helper builds in Windows CI but has not been run against live `.spt` files. |
| 4 | Lightmaps and day/night blending with time-of-day slider | `not working` (awaiting user test); `s0_1.t2lm`/`t2lm1` `decoder verified on real data` | Both 16-area `512×512` DXT1 lightmap files decode cleanly and stitch into day/night terrain textures; visual result awaiting user verification. |
| 5 | Water planes from client water data (reflection & refraction) | `pipeline runs on synthetic fixture` | `s0` has `0` water planes (`ECBSDFILEWATER`). Water pipeline executes on Mesa `lavapipe` synthetic fixture only; not tested on a real zone with water. |
| 6 | Effects (`ECWLD` effects and fire/glows from `gfx.pck`) | `missing` | Client effect record and `gfx.pck` formats are not decoded yet. Viewer draws nothing and reports `Effects: missing`. |
| 7 | Sky from client skybox textures (`surfaces.pck`) | `missing` ([Issue #15](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/15)) | `surfaces.pck` skybox textures are not decoded yet. Approximate blue sky gradient has been removed from the default real-world path; viewer clears to dark background and reports `Sky: missing`. |
| 8 | Minimap and world map from client map tiles | `missing` | `surfaces/maps` and minimap tile formats are not decoded yet. UI shows camera coordinates and reports map tiles as `missing`. |
| 9 | Teleport list from client teleport data | `missing` | Client teleport destination table is not decoded yet. ESC → Teleport List provides zone switching and reports destinations as `missing`. |
| 10 | Audio (zone music, ambience, effects) from `sfx.pck` & loose music | `not started` | Audio backend and `sfx.pck` playback not yet started. |

## Core Infrastructure & Input

| Component | Status | Notes |
| --- | --- | --- |
| Mouse capture / release & ESC menu input state machine (`src/input.rs`) | `pipeline runs on synthetic fixture` (CPU unit tests pass; awaiting Windows user test) | Left-click captures/hides cursor (`CursorGrab::Locked`), mouse look & WASD apply only while captured, Escape releases cursor and unwinds `Settings`/`Modules`/`Teleports` → `Main` → `Closed`, `M` toggles World Map, focus loss releases cursor. |
| Real-zone diagnostic dump (`--dump-zone <zone>`) | `decoder verified on real data` | Verified on real `maps/s0/` files; prints stage-by-stage counts, height probes, texture paths, and placement deltas. |
| End-to-end real-loader CI fixture test (`tests/real_loader_e2e.rs`) | `pipeline runs on synthetic fixture` | Exercises real `TerrainSession::open_zone` and `zone_dump::dump_zone` on a tiny real-format fixture (not synthetic preview). |
| Mesa `lavapipe` offscreen render smoke (`terrain-render-smoke`) | `pipeline runs on synthetic fixture` | Proves WGSL shaders and `wgpu` pipelines compile and execute on CPU software Vulkan; does NOT prove the viewer works on game data. |
| Portable root layout (`<root>/{settings,logs,cache}/`) & native folder picker | `pipeline runs on synthetic fixture` | Runs in Windows CI; awaiting user re-verification on real client. |
