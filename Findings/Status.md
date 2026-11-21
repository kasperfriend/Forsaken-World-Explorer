# Subsystem Status

Status values: `not-started`, `implemented`, `implemented-partial`, `verified-cpu`, `verified-by-user`.
Only the user can mark a subsystem `verified-by-user` after running it locally on a real Forsaken World client and GPU.

| Phase | Subsystem | Status | Evidence | Open Gates | User Verification Notes |
| --- | --- | --- | --- | --- | --- |
| 0 | Repository skeleton, Cargo package & GPL-2.0-or-later license | `verified-cpu` | `Cargo.toml`, `LICENSE`, `THIRD_PARTY_NOTICES.md`, CI Linux & Windows builds | None (CI gate passed) | Pending (optional) |
| 0 | Settings persistence (`%LOCALAPPDATA%\ForsakenWorldExplorer`) | `verified-cpu` | `src/settings.rs` unit tests, `tools/check-core.sh`, `--self-test` | None (CI gate passed) | Pending (optional) |
| 0 | Desktop window, menu & Settings tabs | `verified-cpu` | `src/ui.rs` headless `--smoke-launch` & Windows CI 3s startup launch check (GPU visual rendering unverified) | Visual/GPU confirmation on user hardware | Pending (optional) |
| 0 | Module package discovery (`Modules/*/module.ini`) | `verified-cpu` | `src/modules.rs` unit tests, `tools/check-core.sh`, `--self-test` | None (CI gate passed) | Pending (optional) |
| 0 | Drive client inventory | `verified-cpu` | [Findings/client-inventory.md](client-inventory.md) (`speedtreert.dll` PE32/SHA-256 verified; folder/archive census recorded) | `update/pem.exe` SHA-256 marked `pending` ([#3](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/3)) | Pending (optional) |
| 1 | Client folder detection & executable fingerprint | `verified-cpu` | `src/client_detect.rs` unit tests + Launcher UI integration | `update/pem.exe` recognized SHA-256 `pending` ([#3](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/3)) | Pending (optional) |
| 1 | `.pck` / `.pkx` archive reader & `pck-survey` tool | `verified-cpu` | [Findings/archive-format.md](archive-format.md) (verified on real `configs.pck` 67/67 & `grasses.pck` 103/103); `src/pck.rs` synthetic single/split + fuzz tests; `src/bin/pck-survey.rs` | Real `models.pck`/`models.pkx` (>2 GiB) not yet run locally via `pck-survey` | Pending (optional) |
| 2 | Terrain, world placement, static mesh & DDS decoders (CPU) | `not-started` | Zone file extensions observed in `maps/s0/`, `maps/s1/` ([#5](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/5)) | Verify binary layouts in `Findings/` before coding parsers | — |
| 3 | GPU terrain rendering, free-flight camera & resolution scale | `not-started` | `resolution_scale_pct` setting persisted | Blocked on Phase 2 | — |
| 4 | Static scenery, buildings & texture budget | `not-started` | `tex_budget_mib` setting persisted | Blocked on Phase 3 | — |
| 5 | Sky, lighting, fog, water & optional bloom | `not-started` | — | Blocked on Phase 4 | — |
| 6 | Vegetation (SpeedTree 32-bit bridge & grass) | `not-started` | `update/speedtreert.dll` verified 32-bit PE32 ([#6](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/6)) | Blocked on Phase 5 | — |
| 7 | World effects, audio (`.ogg` / `sfx.pck`), maps & navigation | `not-started` | `music/*.ogg`, `resources/sfx.pck`, `resources/gfx.pck` observed ([#7](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/7)) | Blocked on Phase 6 | — |
| 8 | Optional Modules (Walker & Populator) | `not-started` | `Modules/{Walker,Populator}/module.ini` placeholders; `data/elements.data` & `data/task_npc.data` observed ([#8](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/8)) | Blocked on Phase 7 | — |
| 9 | Hardening, user guide & tagged release | `not-started` | `.github/workflows/release.yml`, `tools/package-windows.ps1` | Blocked on Phases 2–8 and user approval | — |
| Deferred | Hardware ray-traced renderer ("Lighter" equivalent) | `not-started` | Explicitly deferred past Phases 0–7 per project brief | Out of scope for Phases 0–7 | — |
