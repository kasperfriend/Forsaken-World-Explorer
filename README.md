# Forsaken World Explorer

An experimental, GPU-based, free-flight viewer for a user-owned **Forsaken World** client, recreated from the architecture and principles of [`kasperfriend/Perfect-World-Explorer`](https://github.com/kasperfriend/Perfect-World-Explorer). It reads world data from a client you already own; it is **not** a game client, editor, server, or gameplay implementation.

> **Work in progress (Phase 0 — Skeleton & Client Inventory):** No 3D world rendering, archive parsing, or visual parity exists yet. Every automated check in CI runs CPU-only; visual and GPU behavior cannot be verified without a local GPU and real client files. See [Findings/README.md](Findings/README.md) and [Findings/Status.md](Findings/Status.md) for evidence, limits, and open gates.

## Legal & Non-Affiliation Notice

- Licensed under **GPL-2.0-or-later** (see [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)).
- This project is **not affiliated with, endorsed by, or connected to** the developers or publishers of Forsaken World.
- **No proprietary client files, archives, DLLs, executables, extracted assets, derived caches, or launcher artwork are included or redistributed.** You must supply your own trusted Forsaken World client installation.
- The viewer never modifies your client folder. Settings, cache, and logs are stored under `%LOCALAPPDATA%\ForsakenWorldExplorer` (or `FW_CACHE_DIR`).

## Current Status

| Phase | Scope | Status |
| --- | --- | --- |
| Phase 0 | Build/CI skeleton, settings persistence, window/menu placeholders, `--self-test`, Drive client inventory | `verified-cpu` (pending user local Windows verification) |
| Phase 1 | Client folder detection (`update/pem.exe` fingerprint) & `.pck`/`.pkx` archive layer | `not-started` |
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
cargo clippy --all-targets -- -D warnings
cargo test --locked --bin forsaken-world-explorer
cargo check --locked --bins
./tools/check-core.sh
cargo run --locked --bin forsaken-world-explorer -- --self-test
```

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

Run `forsaken-world-explorer.exe --self-test` inside the extracted folder to verify settings persistence and executable-adjacent `Modules/` discovery without needing a GPU or game client.
