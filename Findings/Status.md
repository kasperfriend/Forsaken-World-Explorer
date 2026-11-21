# Subsystem Status

Status values: `not-started`, `implemented`, `implemented-partial`, `verified-cpu`, `not-verified-without-gpu`, `verified-by-user`.
Only the user can mark a subsystem `verified-by-user` after running it locally on a real Forsaken World client and GPU.

| Phase | Subsystem | Status | Evidence | Open Gates | User Verification Notes |
| --- | --- | --- | --- | --- | --- |
| 0 | Repository skeleton, Cargo package & GPL-2.0-or-later license | `verified-cpu` | `Cargo.toml`, `LICENSE`, `THIRD_PARTY_NOTICES.md`, CI Linux & Windows builds | None (CI gate passed) | Pending |
| 0 | Portable root (`<root>/settings/`, `<root>/logs/`, `<root>/cache/`) & legacy import | `verified-cpu` | `src/settings.rs` & `src/logging.rs` unit tests (round-trip, pruning, unwritable-root detection, one-time legacy import), `tools/check-core.sh`, `--self-test` | Local Windows confirmation of Program Files unwritable prompt | Pending |
| 0 | Desktop window, native folder picker (`rfd`), menu & Settings tabs | `not-verified-without-gpu` | `src/ui.rs` headless `--smoke-launch` & Windows CI startup launch check (native dialog and GPU rendering not interactive in CI) | Native Windows folder picker & live GUI rendering | Pending |
| 0 | Module package discovery (`Modules/*/module.ini`) | `verified-cpu` | `src/modules.rs` unit tests, `tools/check-core.sh`, `--self-test` | None (CI gate passed) | Pending |
| 0 | Drive client inventory | `verified-cpu` | [Findings/client-inventory.md](client-inventory.md) (`speedtreert.dll` PE32/SHA-256 verified; folder/archive census recorded) | `update/pem.exe` SHA-256 marked `pending` ([#3](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/3)) | Pending |
| 1 | Client folder detection & executable fingerprint | `verified-cpu` | `src/client_detect.rs` unit tests + Launcher UI integration | `update/pem.exe` recognized SHA-256 `pending` ([#3](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/3)) | Pending |
| 1 | `.pck` / `.pkx` archive reader & `pck-survey` tool | `verified-cpu` | [Findings/archive-format.md](archive-format.md) (verified on real `configs.pck` 67/67 & `grasses.pck` 103/103); `src/pck.rs` synthetic single/split + fuzz tests; `src/bin/pck-survey.rs` | Real `models.pck`/`models.pkx` (>2 GiB) not yet run locally via `pck-survey` | Pending |
| 2 | Terrain block, lightmap & mask CPU decoders (`.trn2`, `.t2bk`, `.t2lm`, `.t2mk`) | `verified-cpu` | [Findings/world-formats.md](world-formats.md) (verified on real `maps/s0/` 256/256 blocks & 32/32 lightmaps); `src/terrain_block.rs` golden-hash + fuzz tests; `src/bin/terrain-survey.rs` | Zones `s1`–`s40` not yet surveyed locally via `terrain-survey` | Pending |
| 2 | RAM-first lossless terrain cache (`<root>/cache/terrain-v1`, 2 MiB hot set) | `verified-cpu` | `src/terrain_store.rs` unit tests & `src/bin/terrain-memory-check.rs` in CI | None (CI gate passed) | Pending |
| 2 | World placement (`.ecwld`/`.ecbsd`) & DDS (`DXT1`/`DXT3`/`DXT5`) CPU decoders | `verified-cpu` | `src/world_objects.rs` & `src/dxt.rs` golden-hash tests; verified on `maps/s0/s0.ecbsd` (1,883 `.bmd` refs) & `grasses.pck` (45 `DXT3`) | Static `.bmd` mesh geometry decoder deferred to Phase 4 (`building.pck`/`litmodels.pck` >800 MB) | Pending |
| 3 | GPU terrain rendering, free-flight camera & resolution scale | `not-started` | `resolution_scale_pct` setting persisted | Next phase | — |
| 4 | Static scenery, buildings & texture budget | `not-started` | `tex_budget_mib` setting persisted | Blocked on Phase 3 | — |
| 5 | Sky, lighting, fog, water & optional bloom | `not-started` | — | Blocked on Phase 4 | — |
| 6 | Vegetation (SpeedTree 32-bit bridge & grass) | `not-started` | `update/speedtreert.dll` verified 32-bit PE32 ([#6](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/6)) | Blocked on Phase 5 | — |
| 7 | World effects, audio (`.ogg` / `sfx.pck`), maps & navigation | `not-started` | `music/*.ogg`, `resources/sfx.pck`, `resources/gfx.pck` observed ([#7](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/7)) | Blocked on Phase 6 | — |
| 8 | Optional Modules (Walker & Populator) | `not-started` | `Modules/{Walker,Populator}/module.ini` placeholders; `data/elements.data` & `data/task_npc.data` observed ([#8](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/8)) | Blocked on Phase 7 | — |
| 9 | Hardening, user guide & tagged release | `not-started` | `.github/workflows/release.yml`, `tools/package-windows.ps1` | Blocked on Phases 3–8 and user approval | — |
| Deferred | Hardware ray-traced renderer ("Lighter" equivalent) | `not-started` | Explicitly deferred past Phases 0–7 per project brief | Out of scope for Phases 0–7 | — |
