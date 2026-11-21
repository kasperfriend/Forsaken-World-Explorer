# Unsupported Client Paths & Deferred Systems

This document records every Forsaken World client feature, data path, or system that **Forsaken World Explorer** intentionally does not implement, cannot verify yet, or defers.

## Explicitly Out of Scope (Viewer Design)

- **Gameplay, combat, skills, quests, dialogue, economy, and server simulation:** This project is a read-only world viewer, not a game client or server emulator.
- **Server-only mob spawns and NPC navigation routes:** If mob placement or patrol paths live only on the server (as in Perfect World's `npcgen.data`), mobs will remain unspawned and nothing will be invented to fill the gap.
- **In-place modification of client files:** The viewer never writes to the user's Forsaken World installation directory. Patcher (`patcher.exe`), `patchersync/`, `reportbugs/`, `gt/` voice platform, and `tools/launcharc/` are ignored.
- **Hardware ray-traced renderer ("Lighter" equivalent):** Explicitly deferred past Phases 0–7 because hardware ray tracing cannot be tested in CPU-only CI and is not usable on target GPUs.

## Unverified / Pending Format Investigation (Tracked in GitHub Issues)

The following client paths are observed in `Findings/client-inventory.md` but have **no verified parser yet** and therefore remain `not-started`:

1. `update/pem.exe` SHA-256 fingerprint and PE version metadata (tracked in [#3](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/3)).
2. `resources/*.pck` and `resources/models.pkx` archive headers, entry-table encryption keys, and compression parameters (tracked in [#4](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/4)).
3. `maps/s0`–`maps/s40` terrain (`.trn2`, `.t2bk`, `.t2mk`, `.t2lm`, `.t2lm1`, `.t2hc`), world placement (`.ecwld`, `.ecbsd`, `scenelights.dat`, `region.clt`, `precinct.clt`), static meshes, animated skeletons/skins, DDS textures, and GFX effects (tracked in [#5](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/5)).
4. `update/speedtreert.dll` exact SpeedTreeRT SDK version and `.spt` asset compatibility (32-bit PE32 confirmed; SDK version and bridge ABI tracked in [#6](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/6)).
5. `resources/sfx.pck` internal audio encodings and zone music mapping (loose `music/*.ogg` confirmed Ogg Vorbis; tracked in [#7](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/7)).
6. `data/elements.data` and `data/task_npc.data` schemas for NPC placements vs. server-only spawns (tracked in [#8](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/8)).
7. Launcher artwork (`tools/launcher-art.jpg`): omitted unless copyright/redistribution rights are explicitly confirmed in writing.
