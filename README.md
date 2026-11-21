# Forsaken World Explorer

An experimental, GPU-based, free-flight viewer for a user-owned **Forsaken World** client, recreated from the architecture and principles of [`kasperfriend/Perfect-World-Explorer`](https://github.com/kasperfriend/Perfect-World-Explorer). It reads world data from a client you already own; it is **not** a game client, editor, server, or gameplay implementation.

> **Work in progress (Phase 1 — Client Discovery & Archive Layer):** Client layout validation (`update/pem.exe`, `resources/`, `maps/`) and the verified Forsaken World `.pck`/`.pkx` archive reader + `pck-survey` tool are implemented and CPU-verified. No 3D world rendering or visual parity exists yet; all CI checks run without a GPU. See [Findings/README.md](Findings/README.md) and [Findings/Status.md](Findings/Status.md) for evidence, limits, and open gates.

## Legal & Non-Affiliation Notice

- Licensed under **GPL-2.0-or-later** (see [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)).
- This project is **not affiliated with, endorsed by, or connected to** the developers or publishers of Forsaken World.
- **No proprietary client files, archives, DLLs, executables, extracted assets, derived caches, or launcher artwork are included or redistributed.** You must supply your own trusted Forsaken World client installation.
- The viewer never modifies your client folder. Settings, cache, and logs are stored under `%LOCALAPPDATA%\ForsakenWorldExplorer` (or `FW_CACHE_DIR`).

## Current Status

| Phase | Scope | Status |
| --- | --- | --- |
| Phase 0 | Build/CI skeleton, settings persistence, window/menu, `--self-test`, `--smoke-launch`, Drive client inventory | `verified-cpu` |
| Phase 1 | Client folder detection (`update/pem.exe`), verified `.pck`/`.pkx` archive reader, `pck-survey` tool | `verified-cpu` |
| Phase 2 | CPU world decoders (terrain, placements, static meshes, DDS) | `not-started` |
| Phase 3–8 | GPU rendering, scenery, sky/water, SpeedTree bridge, effects/audio/maps, optional Modules | `not-started` |

See [Findings/Status.md](Findings/Status.md) for per-subsystem evidence and open gates.

## Environment Variables

| Variable | Default | Purpose |
| --- | --- | --- |
| `FW_CACHE_DIR` | `%LOCALAPPDATA%\ForsakenWorldExplorer` (Windows) / `$XDG_CONFIG_HOME/ForsakenWorldExplorer` (Linux) | Root directory for settings, caches, and logs |
| `FW_CLIENT_DIR` | Unset | Optional override for client folder path (used by local real-data checks; skipped in CI when unset) |
| `FW_MODULES_DIR` | `Modules/` beside executable | Override directory for discovering `module.ini` packages |
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
cargo run --locked --bin forsaken-world-explorer -- --self-test
cargo run --locked --bin forsaken-world-explorer -- --smoke-launch
```

### Survey a Local Client's Archives

```sh
cargo run --locked --bin pck-survey -- <path-to-forsaken-world-client> --out archive-survey.md
```

Add `--verify-payloads` to decompress and verify every entry payload across all `.pck`/`.pkx` archives.

## Portable Windows Package

Windows PR artifacts and releases are packaged via `tools/package-windows.ps1` into `Forsaken-World-Explorer-Windows-x64.zip`, containing:

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
```

Run `forsaken-world-explorer.exe --self-test` and `forsaken-world-explorer.exe --smoke-launch` inside the extracted folder to verify settings persistence, `Modules/` discovery, synthetic `.pck`/`.pkx` reading, and UI initialization without needing a GPU or game client.
