# Forsaken World Explorer

A Rust desktop world-explorer built by porting the established Perfect World Explorer renderer, asset pipeline, module system and release workflow. This repository now carries the upstream application's terrain, model, foliage, sky, water, effects, shader/GPU, UI, diagnostics and module infrastructure together with its PR checks and Windows packaging workflows. Executable target: `forsaken-world-explorer`.

## Build and run

```sh
cargo build --release --bin forsaken-world-explorer
cargo test --bin forsaken-world-explorer
cargo check --bins
```

On Windows, use the same toolchain and setup as documented in `.github/workflows/pr-checks.yml` and `.github/workflows/manual-release.yml`. The CI workflows check Rust source and tests, build the Windows executable, and stage release packages. A usable game client is required at runtime; game files are not redistributed. See `Modules/` for the modular add-on system and `tools/` for packaging/validation support.

## Important format/port status

This is the **renderer and workflow foundation**, not yet a verified Forsaken World renderer. The inherited scene readers and rendering code were written for Perfect World formats; changing the product name or packaging does not make those formats interchangeable. The Forsaken World client in the user's Drive has a 0.525 installation with `maps/` and a v260 source tree. The next essential engineering step is to characterize these client builds and adapt the filesystem/archive, map/terrain, object/model, effect, environment, water and shader readers against those actual files. Until those version-specific readers are ported and validated, do not treat a successful launch or a rendered scene as Forsaken World format compatibility.

No client binaries/assets are committed. The retained third-party source and license notices are documented in `LICENSE` and `THIRD_PARTY_NOTICES.md`; upstream-derived Rust code remains GPL-2.0-or-later.
