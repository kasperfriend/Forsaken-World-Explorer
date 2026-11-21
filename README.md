# Forsaken World Explorer

An experimental, GPU-based, free-flight viewer for a user-owned **Forsaken World** client, recreated from the architecture and principles of [`kasperfriend/Perfect-World-Explorer`](https://github.com/kasperfriend/Perfect-World-Explorer). It reads world data from a client you already own; it is **not** a game client, editor, server, or gameplay implementation.

> **Work in progress:** Client folder selection (native folder picker), portable root storage, verified `.pck`/`.pkx` archive reading, CPU terrain/lightmap/mask/placement/DDS/static `.bmd` decoders, RAM-first `cache/terrain-v1` and `cache/scenery-v1` streaming, 32 MiB pinned-batch texture caching, and Phases 3–4 `wgpu` terrain + instanced static scenery rendering with free-flight camera and `10%–200%` resolution scaling are implemented. **Visual parity is not verified without a hardware GPU:** offline `naga` validation and Mesa `lavapipe` software offscreen smoke tests prove shader validity and pipeline execution only, not visual parity on a real GPU or client. See [Findings/README.md](Findings/README.md), [Findings/scenery-formats.md](Findings/scenery-formats.md), [Findings/scenery-budget.md](Findings/scenery-budget.md), and [Findings/Status.md](Findings/Status.md) for evidence, limits, and open gates.

## Legal & Non-Affiliation Notice

- Licensed under **GPL-2.0-or-later** (see [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)).
- This project is **not affiliated with, endorsed by, or connected to** the developers or publishers of Forsaken World.
- **No proprietary client files, archives, DLLs, executables, extracted assets, derived caches, or launcher artwork are included or redistributed.** You must supply your own trusted Forsaken World client installation.
- The viewer never modifies your client folder.

## Portable Root Folder Layout

All runtime data lives beside `forsaken-world-explorer.exe` (resolved from the executable's location, not the current working directory). The `settings/`, `logs/`, and `cache/` subdirectories are **created automatically on first run** (they are not pre-populated in the release ZIP):

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

- **One-time legacy migration:** If `<root>/settings/viewer-settings.txt` does not exist yet and an older `%LOCALAPPDATA%\ForsakenWorldExplorer\viewer-settings.txt` exists, it is imported once into `<root>/settings/viewer-settings.txt` and left untouched.
- **Unwritable folder protection:** If `forsaken-world-explorer.exe` is placed in a read-only location (such as `Program Files`), the UI shows a clear banner and lets you choose a writable folder via the native folder picker.

## Free-Flight Controls, World Settings & Diagnostics

- **Movement & Look:** `W`/`A`/`S`/`D` move, `Space`/`E` ascend, `Q`/`Left Ctrl` descend, `Shift` fast boost, `Arrow keys` (or mouse drag) look, `M` World Map placeholder, `Escape` Explorer Menu.
- **Settings → Graphics:** Resolution scale (`10%–200%`, default `100%`; UI stays native), decoded texture cache budget (`FW_TEX_BUDGET_MIB`, default `32 MiB`, pinned visible batch never downscaled or silently dropped), terrain hot RAM budget (`FW_TERRAIN_RAM_MIB`, default `2 MiB`).
- **Settings → World:** Static `.bmd` scenery on/off, camera-centered streaming radius (`32–2048 m`, default `256 m`), max loaded instances (`64–16384`, default `1024`), new models decoded per frame (`1–64`, default `4`), and whole-world loading on/off (default off; warns about high RAM/VRAM use).
- **World View Diagnostics panel (1 Hz):** Displays placements in range, unique models loaded, instances drawn, failed decodes, deferred transparent submeshes, and decoded texture cache count/size vs budget.

### Known Limits (Phase 4)

- **Approximate lighting:** Terrain and static `.bmd` scenery currently use an approximate directional + hemisphere light until regional/time-of-day lighting is decoded in Phase 5.
- **Transparent / blended submeshes:** Submeshes with translucent material alpha are classified and skipped rather than drawn incorrectly; they are deferred to Phase 5.
- **Whole-world loading:** Not verified on full-zone memory use without a real client and hardware GPU.

## Current Status

| Phase | Scope | Status |
| --- | --- | --- |
| Phase 0 | Build/CI skeleton, portable root (`settings/`, `logs/`, `cache/`), native folder picker, `--self-test`, `--smoke-launch`, Drive client inventory | `verified-cpu` |
| Phase 1 | Client folder detection (`update/pem.exe`), verified `.pck`/`.pkx` archive reader, `pck-survey` tool | `verified-cpu` |
| Phase 2 | CPU world decoders (`.trn2`, `.t2bk` v6, `.t2lm`/`.t2lm1`, `.t2mk`, `.ecwld`, `.ecbsd`, DDS `DXT1`/`DXT3`/`DXT5`), `cache/terrain-v1`, `terrain-survey`, `terrain-memory-check` | `verified-cpu` |
| Phase 3 | GPU terrain rendering (`wgpu`), camera-centered streaming (`cache/terrain-v1`), free-flight camera, `10%–200%` resolution scale, `naga` WGSL validator, Mesa `lavapipe` offscreen smoke test | `verified-cpu` / `not-verified-without-gpu` |
| Phase 4 | Static `.bmd` decoder, `.ecwld`/`.ecbsd` transforms, instanced GPU rendering, multi-frame streaming, 32 MiB pinned texture cache, 1 Hz Diagnostics, `scenery-survey` | `verified-cpu` / `not-verified-without-gpu` |
| Phase 5–8 | Sky/lighting/water/transparency, SpeedTree bridge, effects/audio/maps, optional Modules | `not-started` |

See [Findings/Status.md](Findings/Status.md) for per-subsystem evidence and open gates.

## Environment Variables

| Variable | Default | Purpose |
| --- | --- | --- |
| `FW_CACHE_DIR` | `<root>/cache` beside executable | Override directory for derived caches (`terrain-v1`, `scenery-v1`) |
| `FW_CLIENT_DIR` | Unset | Optional override for client folder path (used by local real-data checks; skipped in CI when unset) |
| `FW_MODULES_DIR` | `<root>/Modules` beside executable | Override directory for discovering `module.ini` packages |
| `FW_TERRAIN_RAM_MIB` | `2` | Terrain hot-set RAM budget (MiB) |
| `FW_TEX_BUDGET_MIB` | `32` | Decoded texture cache budget (MiB) |

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
```
