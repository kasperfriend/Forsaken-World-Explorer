# Forsaken World Explorer

An experimental, GPU-based, free-flight viewer for a user-owned **Forsaken World** client, recreated from the architecture and principles of [`kasperfriend/Perfect-World-Explorer`](https://github.com/kasperfriend/Perfect-World-Explorer). It reads world data from a client you already own; it is **not** a game client, editor, server, or gameplay implementation.

> **Work in progress:** Phases 0–6 are implemented and CPU/CI-verified (native folder picker, portable root layout, `.pck`/`.pkx` archive layer, CPU terrain/lightmap/mask/placement/DDS/static `.bmd` decoders, `cache/terrain-v1`, `cache/scenery-v1`, `cache/speedtree-v1`, 32 MiB pinned-batch texture cache, free-flight `wgpu` terrain + opaque/transparent `.bmd` scenery, time-of-day sky/lighting/fog, experimental water, 32-bit SpeedTree bridge helper, and locally streamed grass). **Water is not complete** (see [Findings/water-status.md](Findings/water-status.md)), and **visual parity is not verified without a hardware GPU:** offline `naga` validation and Mesa `lavapipe` software offscreen smoke tests prove shader validity and pipeline execution only, not visual parity on a real GPU or client.

## Legal & Non-Affiliation Notice

- Licensed under **GPL-2.0-or-later** (see [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)).
- This project is **not affiliated with, endorsed by, or connected to** the developers or publishers of Forsaken World.
- **No proprietary client files, archives, DLLs, executables, extracted assets, derived caches, or launcher artwork are included or redistributed.** You must supply your own trusted Forsaken World client installation (including its `update/speedtreert.dll`).
- The viewer never modifies your client folder.

## Portable Root Folder Layout

All runtime data lives beside `forsaken-world-explorer.exe` (resolved from the executable's location, not the current working directory). The `settings/`, `logs/`, and `cache/` subdirectories are **created automatically on first run**:

```text
Forsaken World Explorer/
  forsaken-world-explorer.exe
  tools/
    fw-speedtree-helper.exe  # 32-bit helper built in CI; loads your client's update/speedtreert.dll
  Modules/
    Walker/
      module.ini
    Populator/
      module.ini
  SHA256SUMS.txt
  THIRD_PARTY_NOTICES.md
  settings/                  # created on first run: viewer-settings.txt
  logs/                      # created on first run: per-run fw-explorer-*.log (max 10 files, 2 MiB each)
  cache/                     # created on first run: terrain-v1/, scenery-v1/, speedtree-v1/ (FW_CACHE_DIR)
```

### 32-Bit SpeedTree Helper & Trust Model

- Because the client ships a 32-bit `update/speedtreert.dll` while the viewer is 64-bit, `tools/fw-speedtree-helper.exe` runs out-of-process to load the user's own DLL, bake tree geometry into `<root>/cache/speedtree-v1/`, and exit.
- **User responsibility:** You supply your own trusted client installation containing `update/speedtreert.dll`. The viewer verifies the DLL's SHA-256 fingerprint before launching the helper and refuses to run against an unrecognized or missing DLL (showing a clear notice in the UI and log while keeping the rest of the viewer running).

## Free-Flight Controls, Settings & Diagnostics

- **Movement & Look:** `W`/`A`/`S`/`D` move, `Space`/`E` ascend, `Q`/`Left Ctrl` descend, `Shift` fast boost, `Arrow keys` (or mouse drag) look, `M` World Map placeholder, `Escape` Explorer Menu.
- **Settings → Graphics (persisted under `<root>/settings/`):**
  - Resolution scale: `10%–200%` (default `100%`; UI stays native)
  - Environmental fog: on/off (default `on`), fog distance multiplier `25%–400%` (default `100%`)
  - Water reflection / refraction: on/off (default `on`; disable for weak GPUs), water quality `Low` (25%) / `Medium` (50%, default) / `High` (100%)
  - Time of day: `00:00`–`23:59` slider (default `12:00`) and `Follow real clock` (default `off`)
  - Tree LOD distance multiplier: `25%–400%` (default `100%`)
  - Decoded texture cache budget (`FW_TEX_BUDGET_MIB`, default `32 MiB`) and Terrain hot RAM budget (`FW_TERRAIN_RAM_MIB`, default `2 MiB`)
- **Settings → World (persisted under `<root>/settings/`):**
  - Static `.bmd` scenery (default `on`), transparent `.bmd` scenery (default `on`), scenery streaming radius (`256 m`), max instances (`1024`), models/frame (`4`), whole-world loading (default `off`, memory warning)
  - Trees on/off (default `on`), Grass on/off (default `on`), Grass density (`Low` 35% / `Medium` 65% default / `High` 100%), vegetation streaming radius (`180 m`), max vegetation instances (`2048`), vegetation loads/frame (`4`)
- **World View Diagnostics (1 Hz):** Reports lighting mode, scenery placements/models/instances/transparent drawn & skipped, decode failures, visible water planes & per-frame world render passes, trees in range/loaded, helper status (last run, exit code, duration), grass in range/drawn, `speedtree-v1` cache size, texture cache usage, and GPU timestamp query status.

### Known Limits (Phase 6)

- **Water is not complete:** See [Findings/water-status.md](Findings/water-status.md).
- **Sky dome:** Procedural time-of-day gradient (`approximate`) until `surfaces.pck` skyboxes are decoded ([#15](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/15)).
- **Vegetation wind & LOD:** Uses documented vertex-shader sine wind (`approximate`) and 3 discrete geometry LOD tiers; real `.spt` baking requires local Windows verification with `update/speedtreert.dll`.

## Current Status

| Phase | Scope | Status |
| --- | --- | --- |
| Phase 0–2 | Skeleton, portable root, native folder picker, `.pck`/`.pkx`, CPU world decoders, `cache/terrain-v1` | `verified-cpu` |
| Phase 3–4 | GPU terrain & static `.bmd` scenery, free flight, resolution scale, 32 MiB texture cache, `scenery-survey` | `verified-cpu` / `not-verified-without-gpu` |
| Phase 5 | Sky gradient, day/night lightmaps & `.bmd` vertex lighting, fog, transparent scenery, experimental water, `sky-survey`, `water-survey` | `verified-cpu` / `implemented-partial` / `not-verified-without-gpu` |
| Phase 6 | 32-bit SpeedTree bridge helper, `FWST` v1 cache, tree LOD, locally streamed grass, `speedtree-survey`, `grass-survey` | `verified-cpu` / `implemented-partial` / `not-verified-without-gpu` |
| Phase 7–8 | World effects, audio (`.ogg`/`sfx.pck`), maps & navigation, optional Modules | `not-started` |

See [Findings/Status.md](Findings/Status.md) for per-subsystem evidence and open gates.

## Build & Verify from Source

Install stable Rust from [rustup.rs](https://rustup.rs/), then from the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo check --locked --bins
cargo check --manifest-path tools/speedtree/fw-speedtree-helper/Cargo.toml
./tools/check-core.sh
cargo run --locked --bin shader-validate
cargo run --locked --bin forsaken-world-explorer -- --self-test
cargo run --locked --bin forsaken-world-explorer -- --smoke-launch
cargo run --locked --bin terrain-memory-check
WGPU_BACKEND=vulkan cargo run --locked --bin terrain-render-smoke
```

### Local Client Survey Tools

```sh
cargo run --locked --bin pck-survey -- <path-to-forsaken-world-client> --out reports/archive-survey.md
cargo run --locked --bin terrain-survey -- <path-to-forsaken-world-client> --out reports/terrain-survey.md
cargo run --locked --bin scenery-survey -- <path-to-forsaken-world-client> s0 --out reports/scenery-survey-s0.md
cargo run --locked --bin sky-survey -- <path-to-forsaken-world-client> s0 --out reports/sky-survey-s0.md
cargo run --locked --bin water-survey -- <path-to-forsaken-world-client> s0 --out reports/water-survey-s0.md
cargo run --locked --bin speedtree-survey -- <path-to-forsaken-world-client> --out reports/speedtree-survey.md
cargo run --locked --bin grass-survey -- <path-to-forsaken-world-client> s0 --out reports/grass-survey-s0.md
```
