# Forsaken World Explorer

An experimental, GPU-based, free-flight viewer for a user-owned **Forsaken World** client, recreated from the architecture and principles of [`kasperfriend/Perfect-World-Explorer`](https://github.com/kasperfriend/Perfect-World-Explorer). It reads world data from a client you already own; it is **not** a game client, editor, server, or gameplay implementation.

> **Work in progress:** Client folder selection (native folder picker), portable root storage, verified `.pck`/`.pkx` archive reading, CPU terrain/lightmap/mask/placement/DDS decoders, and the RAM-first `cache/terrain-v1` (2 MiB default hot set) are implemented and CPU-verified. No 3D GPU rendering or visual parity is claimed yet; all CI checks run without a hardware GPU. See [Findings/README.md](Findings/README.md) and [Findings/Status.md](Findings/Status.md) for evidence, limits, and open gates.

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
  cache/                   # created on first run: terrain-v1/ (overridable via FW_CACHE_DIR)
```

- **One-time legacy migration:** If `<root>/settings/viewer-settings.txt` does not exist yet and an older `%LOCALAPPDATA%\ForsakenWorldExplorer\viewer-settings.txt` exists, it is imported once into `<root>/settings/viewer-settings.txt` and left untouched.
- **Unwritable folder protection:** If `forsaken-world-explorer.exe` is placed in a read-only location (such as `Program Files`), the UI shows a clear banner and lets you choose a writable folder via the native folder picker.

## Current Status

| Phase | Scope | Status |
| --- | --- | --- |
| Phase 0 | Build/CI skeleton, portable root (`settings/`, `logs/`, `cache/`), native folder picker, `--self-test`, `--smoke-launch`, Drive client inventory | `verified-cpu` |
| Phase 1 | Client folder detection (`update/pem.exe`), verified `.pck`/`.pkx` archive reader, `pck-survey` tool | `verified-cpu` |
| Phase 2 | CPU world decoders (`.trn2`, `.t2bk` v6, `.t2lm`/`.t2lm1`, `.t2mk`, `.ecwld`, `.ecbsd`, DDS `DXT1`/`DXT3`/`DXT5`), `cache/terrain-v1`, `terrain-survey`, `terrain-memory-check` | `verified-cpu` |
| Phase 3–8 | GPU terrain rendering & free flight, static scenery, sky/water, SpeedTree bridge, effects/audio/maps, optional Modules | `not-started` |

See [Findings/Status.md](Findings/Status.md) for per-subsystem evidence and open gates.

## Environment Variables

| Variable | Default | Purpose |
| --- | --- | --- |
| `FW_CACHE_DIR` | `<root>/cache` beside executable | Override directory for derived caches (such as `terrain-v1`) |
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
cargo run --locked --bin forsaken-world-explorer -- --self-test
cargo run --locked --bin forsaken-world-explorer -- --smoke-launch
cargo run --locked --bin terrain-memory-check
```

### Local Client Survey Tools

```sh
cargo run --locked --bin pck-survey -- <path-to-forsaken-world-client> --out archive-survey.md
cargo run --locked --bin terrain-survey -- <path-to-forsaken-world-client> --out terrain-survey.md
```
