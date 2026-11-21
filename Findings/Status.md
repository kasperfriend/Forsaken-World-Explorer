# Subsystem Status

Status values: `not-started`, `implemented`, `implemented-partial`, `verified-cpu`, `verified-by-user`.
Only the user can mark a subsystem `verified-by-user` after running it locally on a real Forsaken World client and GPU.

| Phase | Subsystem | Status | Evidence | Open Gates | User Verification Notes |
| --- | --- | --- | --- | --- | --- |
| 0 | Repository skeleton, Cargo package & GPL-2.0-or-later license | `verified-cpu` | `Cargo.toml`, `LICENSE`, `THIRD_PARTY_NOTICES.md`, CI Linux & Windows builds | User local smoke check of Windows ZIP artifact | Pending |
| 0 | Settings persistence (`%LOCALAPPDATA%\ForsakenWorldExplorer`) | `verified-cpu` | `src/settings.rs` unit tests, `tools/check-core.sh`, `--self-test` | User local confirmation on Windows | Pending |
| 0 | Desktop window, menu & Settings placeholders | `implemented` | `src/ui.rs` compiles on Linux & Windows (no GPU visual verification in CI) | User local launch confirmation (cannot verify GUI/GPU without display/GPU) | Pending |
| 0 | Module package discovery (`Modules/*/module.ini`) | `verified-cpu` | `src/modules.rs` unit tests, `tools/check-core.sh`, `--self-test` | User local confirmation on packaged ZIP | Pending |
| 0 | Drive client inventory | `verified-cpu` | `Findings/client-inventory.md` (`speedtreert.dll` PE32/SHA-256 verified; folder/archive census recorded) | `update/pem.exe` SHA-256 exceeds 10 MB Drive MCP download limit; user must hash locally | Pending |
| 1 | Client folder detection & executable fingerprint | `not-started` | `Findings/client-inventory.md` | Needs local `pem.exe` SHA-256 & Phase 1 PR | — |
| 1 | `.pck`/`.pkx` archive reader & survey tool | `not-started` | `resources/` archive census in `Findings/client-inventory.md` | Must verify header/XOR keys/compression in Phase 1 before coding parser | — |
| 2 | Terrain, world placement, static mesh & DDS decoders (CPU) | `not-started` | Zone file extensions observed in `maps/s0/`, `maps/s1/` | Blocked on Phase 1 archive layer and binary format verification | — |
| 3 | GPU terrain rendering, free-flight camera & resolution scale | `not-started` | `resolution_scale_pct` setting persisted in Phase 0 | Blocked on Phase 2 | — |
| 4 | Static scenery, buildings & texture budget | `not-started` | `tex_budget_mib` setting persisted in Phase 0 | Blocked on Phase 3 | — |
| 5 | Sky, lighting, fog, water & optional bloom | `not-started` | — | Blocked on Phase 4 | — |
| 6 | Vegetation (SpeedTree 32-bit bridge & grass) | `not-started` | `update/speedtreert.dll` verified 32-bit PE32 in `Findings/client-inventory.md` | Blocked on Phase 5 & `.spt` format verification | — |
| 7 | World effects, audio (`.ogg` / `sfx.pck`), maps & navigation | `not-started` | `music/*.ogg` and `resources/sfx.pck`, `resources/gfx.pck` observed | Blocked on Phase 6 | — |
| 8 | Optional Modules (Walker & Populator) | `not-started` | `Modules/Walker/module.ini`, `Modules/Populator/module.ini` placeholders; `data/elements.data` & `data/task_npc.data` observed | Blocked on Phase 7 & NPC/character data verification | — |
| 9 | Hardening, user guide & tagged release | `not-started` | `.github/workflows/release.yml`, `tools/package-windows.ps1` | Blocked on Phases 1–8 and user approval | — |
| Deferred | Hardware ray-traced renderer ("Lighter" equivalent) | `not-started` | Explicitly deferred past Phases 0–7 per project brief | Out of scope for Phases 0–7 | — |
