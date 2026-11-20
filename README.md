# Forsaken World Explorer

A Rust-based Forsaken World asset explorer, following the modular direction and world-viewer purpose of Perfect World Explorer. The first milestone is client discovery and filesystem inventory: validate the selected install, locate `elementclient.exe`/`data/maps` (case-insensitively), and index world files and packed archives without modifying the client.

## Run

```sh
cargo run -- inspect /path/to/ForsakenWorldClient
cargo run -- inspect /path/to/ForsakenWorldClient --json
cargo test
```

Windows installations usually contain `elementclient.exe`, `data/`, and `maps/` (the map directory may be nested in `data/`). Reports include map candidates (`.map`, `.smd`, `.smo`, `.sma`, `.spt`, `.dds`, `.gfx`, `.ecm`), archive candidates (`.pck`, `.pak`, `.zip`, `.7z`, `.rar`), and extension counts. This is intentionally an inventory stage: it does not claim to decode archives or render world data yet.

## Development direction

1. Build this inventory into an install picker and client-version profile.
2. Reverse-engineer Forsaken World's map/terrain and archive formats against owned client files and the community Godot editor.
3. Implement robust readers and validation fixtures before GPU rendering.
4. Add scene models/buildings, effects, water, atmosphere/sky and shader/material parity as independent modules.

The user's Drive contains a Forsaken World 0.525 client and v260 source tree; the available 0.525 layout includes a `maps` folder. Client source and representative sample files should be used to establish exact format/version differences before promising rendering parity.
