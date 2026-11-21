# Forsaken World Explorer

An experimental, GPU-based, free-flight viewer for a user-owned **Forsaken World** client, recreated from the architecture and principles of [`kasperfriend/Perfect-World-Explorer`](https://github.com/kasperfriend/Perfect-World-Explorer). It reads world data from a client you already own; it is **not** a game client, editor, server, or gameplay implementation.

> **Work in progress:** Phases 0–5 are implemented and CPU/CI-verified (native folder picker, portable root layout, `.pck`/`.pkx` archive layer, CPU terrain/lightmap/mask/placement/DDS/static `.bmd` decoders, `cache/terrain-v1` & `cache/scenery-v1`, 32 MiB pinned-batch texture cache, free-flight `wgpu` terrain + opaque/transparent `.bmd` scenery, time-of-day sky/lighting/fog, and experimental water). **Water is not complete** (see [Findings/water-status.md](Findings/water-status.md)), and **visual parity is not verified without a hardware GPU:** offline `naga` validation and Mesa `lavapipe` software offscreen smoke tests prove shader validity and pipeline execution only, not visual parity on a real GPU or client.

## Legal & Non-Affiliation Notice

- Licensed under **GPL-2.0-or-later** (see [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)).
- This project is **not affiliated with, endorsed by, or connected to** the developers or publishers of Forsaken World.
- **No proprietary client files, archives, DLLs, executables, extracted assets, derived caches, or launcher artwork are included or redistributed.** You must supply your own trusted Forsaken World client installation.
- The viewer never modifies your client folder.

## Portable Root Folder Layout

All runtime data lives beside `forsaken-world-explorer.exe` (resolved from the executable's location, not the current working directory). The `settings/`, `logs/`, and `cache/` subdirectories are **created automatically on first run**:

```text
Forsaken World Explorer/
  forsaken-world-explorer.exe
  Modules/
    Walker/
      module.ini
    Populator/
      module.ini
  SHA256SUMS.txt
  THIRD_PARTY_NOTICES.md
  settings/                # created on first run: viewer-settings.txt
  logs/                    # created on first run: per-run fw-explorer-*.log (max 10 files, 2 MiB each)
  cache/                   # created on first run: terrain-v1/, scenery-v1/ (overridable via FW_CACHE_DIR)
```

## Free-Flight Controls, Settings & Diagnostics

- **Movement & Look:** `W`/`A`/`S`/`D` move, `Space`/`E` ascend, `Q`/`Left Ctrl` descend, `Shift` fast boost, `Arrow keys` (or mouse drag) look, `M` World Map placeholder, `Escape` Explorer Menu.
- **Settings → Graphics (persisted under `<root>/settings/`):**
  - Resolution scale: `10%–200%` (default `100%`; UI stays native)
  - Environmental fog: on/off (default `on`), fog distance multiplier `25%–400%` (default `100%`)
  - Water reflection: on/off (default `on`, disable for weak GPUs)
  - Water refraction: on/off (default `on`, disable for weak GPUs)
  - Water quality: `Low` (25%), `Medium` (50%, default), `High` (100% of scaled 3D viewport)
  - Time of day: `00:00`–`23:59` slider (default `12:00` matching client `MID_NOON`) and `Follow real clock` (default `off`)
  - Decoded texture cache budget: `FW_TEX_BUDGET_MIB` (default `32 MiB`, pinned visible batch never silently dropped)
  - Terrain hot RAM budget: `FW_TERRAIN_RAM_MIB` (default `2 MiB`)
- **Settings → World (persisted under `<root>/settings/`):**
  - Static `.bmd` scenery: on/off (default `on`)
  - Transparent `.bmd` scenery: on/off (default `on`, sorted back-to-front, depth write off)
  - Streaming radius: `32–2048 m` (default `256 m`)
  - Max loaded instances: `64–16384` (default `1024`)
  - New models decoded per frame: `1–64` (default `4`)
  - Whole-world scenery loading: on/off (default `off`; warns about high RAM/VRAM use)
- **World View Diagnostics (1 Hz):** Reports lighting mode (`client day/night` vs `approximate fallback`), placements in range, unique models loaded, opaque/alpha-test instances drawn, transparent submeshes drawn & skipped, decode failures, visible water planes, per-frame world render pass count, decoded texture cache usage, and GPU timestamp query support.

### Known Limits (Phase 5)

- **Water is not complete:** Dual-layer animated normals, scissored planar reflection/refraction, shoreline edge fade, and underwater fog/tint are implemented-partial (see [Findings/water-status.md](Findings/water-status.md)). Shoreline wave foam strips, caustics, and watery-zone `BSData/*.dat` verification remain open.
- **Sky dome:** Uses a procedural time-of-day zenith/horizon gradient + sun disc driven by `EC_SunMoon` and `ECBSDFILEBOXAREA15` colors (`approximate` until full `surfaces.pck` skybox textures are decoded).
- **Scenery fallback lighting:** `v3`–`v5` `.bmd` submeshes blend real per-vertex day/night light arrays; `v2` meshes without vertex light arrays use approximate directional + hemisphere lighting (labeled in Diagnostics).
- **Whole-world loading:** Not verified on full-zone memory use without a real client and hardware GPU.

## Current Status

| Phase | Scope | Status |
| --- | --- | --- |
| Phase 0–2 | Skeleton, portable root, native folder picker, `.pck`/`.pkx`, CPU world decoders, `cache/terrain-v1` | `verified-cpu` |
| Phase 3 | GPU terrain rendering (`wgpu`), free flight, `10%–200%` resolution scale, `naga` & Mesa `lavapipe` CI gates | `verified-cpu` / `not-verified-without-gpu` |
| Phase 4 | Static `.bmd` decoder (all versions fixture-tested), instancing, multi-frame streaming, 32 MiB texture cache, `scenery-survey` | `verified-cpu` / `not-verified-without-gpu` |
| Phase 5 | Sky gradient, day/night lightmap & `.bmd` vertex lighting blend, environmental fog, transparent `.bmd` scenery, experimental water, `sky-survey` & `water-survey` | `verified-cpu` / `implemented-partial` / `not-verified-without-gpu` |
| Phase 6–8 | Vegetation (SpeedTree 32-bit bridge & grass), effects/audio/maps, optional Modules | `not-started` |

See [Findings/Status.md](Findings/Status.md) for per-subsystem evidence and open gates.

## Build & Verify from Source

Install stable Rust from [rustup.rs](https://rustup.rs/), then from the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo check --locked --bins
./tools/check-core.sh
cargo run --locked --bin shader-validate
cargo run --locked --bin forsaken-world-explorer -- --self-test
cargo run --locked --bin forsaken-world-explorer -- --smoke-launch
cargo run --locked --bin terrain-memory-check
WGPU_BACKEND=vulkan cargo run --locked --bin terrain-render-smoke
```

### Local Client Survey Tools

```sh
cargo run --locked --bin pck-survey -- <path-to-forsaken-world-client> --out archive-survey.md
cargo run --locked --bin terrain-survey -- <path-to-forsaken-world-client> --out terrain-survey.md
cargo run --locked --bin scenery-survey -- <path-to-forsaken-world-client> s0 --out scenery-survey-s0.md
cargo run --locked --bin sky-survey -- <path-to-forsaken-world-client> s0 --out sky-survey-s0.md
cargo run --locked --bin water-survey -- <path-to-forsaken-world-client> s0 --out water-survey-s0.md
```
