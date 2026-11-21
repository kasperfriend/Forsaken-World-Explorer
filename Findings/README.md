# Findings Index

Every format claim, client layout fact, and verification limit for **Forsaken World Explorer** is recorded here with its evidence source and what remains unverified.

## Documents

- [client-inventory.md](client-inventory.md) — Top-level folder layout, file types, counts, sizes, `speedtreert.dll` PE32 verification, `pem.exe` fingerprint status (`pending`), and Drive source-tree overview from the inspected Google Drive client (`Forsaken World 0.525 Client` / `ForsakenWorldv260ClientSource`).
- [archive-format.md](archive-format.md) — Verified Forsaken World `.pck` / `.pkx` binary layout, guard keys (`0xa508bdc4` / `0x853567c8`), XOR entry-table keys (`0x21c31a3f` / `0x399f3a1a`), GBK path encoding, zlib compression, and split `.pkx` continuation model.
- [world-formats.md](world-formats.md) — Verified Forsaken World `.trn2` (v1), `.t2bk` (v6 36-byte block header, heights, polar normals, day/night vertex colours), `.t2lm`/`.t2lm1` (v2 512x512 DXT1 DDS lightmaps), `.t2mk` (v2 masks), `.ecwld` (v14), `.ecbsd` (v15 `.bmd` placements), and DDS (`DXT1`/`DXT3`) formats from `maps/s0/` and `resources/grasses.pck`.
- [Status.md](Status.md) — Subsystem status table (`implemented`, `implemented-partial`, `verified-cpu`, `verified-by-user`, `not-started`), evidence links, open gates, and user verification notes.
- [Unsupported-client-paths.md](Unsupported-client-paths.md) — Client features, server-only data, and deferred systems not handled by this viewer, with reasons and linked issues.

## Status Legend

- `not-started` — No implementation yet.
- `implemented` — Code written, not yet verified on CPU tests or by the user.
- `implemented-partial` — Partially implemented or contains documented approximations.
- `verified-cpu` — Verified by automated CPU-only unit, fixture, or CI checks; visual/GPU output is **not** verified.
- `verified-by-user` — Confirmed locally by the user on a real Forsaken World client and GPU. Only the user can assign this status.
