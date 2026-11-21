# Forsaken World Explorer

An experimental, GPU-based, free-flight viewer for a **user-owned Forsaken World client**. It reads world data from a client you provide; it is not a game client, editor, server, or gameplay implementation.

> **Current status (honest assessment):** No rendered feature is marked working until confirmed visually by the user in the Windows artifact on real client files.
> - **Terrain layer textures:** `decoder verified on real data (dump: dump-s0.txt)` (5/5 `s0` DDS DXT3 layers decoded).
> - **Terrain rendering:** `not working: black output (Task 1)` — fixed `Rgba8UnormSrgb` crushing and double-lighting in `terrain.wgsl`; added `--export-terrain-pngs` for visual/numeric verification.
> - **Buildings (`.bmd`):** `not working: v0x10000006 layout unknown (Task 2)` — `sample_litmodel_1000.bmd` (45,318/45,318 B) traced via `--bmd-trace` and decoded; awaiting user visual check and additional samples.
> - **Counters:** `fixed in this round, pending user check`.
> - See [Findings/Status.md](Findings/Status.md) for the full table.
>
> **No fake worlds in the default path:** The approximate sky gradient and the synthetic preview scene are **not** game worlds and have been removed from the default path.

## Controls (matching Perfect World Explorer)

| Control | Action |
| --- | --- |
| Left-click in 3D view | Capture and hide mouse cursor for looking |
| Escape | Release mouse / open ESC menu; return from Settings, Modules, or Teleport List; close menu |
| W / A / S / D | Fly forward / left / back / right (while mouse captured) |
| Space or E / Q or Left Ctrl | Ascend / descend (while mouse captured) |
| Shift | Faster movement |
| Arrow keys | Look (while mouse captured) |
| M | Toggle World Map |

## Running the Windows Artifact

1. Download `Forsaken-World-Explorer-Windows-x64.zip` from the latest PR workflow run and extract the entire `Forsaken World Explorer` folder together:
   ```text
   Forsaken World Explorer/
     forsaken-world-explorer.exe
     tools/
       fw-speedtree-helper.exe
     Modules/
       Walker/module.ini
       Populator/module.ini
     THIRD_PARTY_NOTICES.md
     SHA256SUMS.txt
   ```
2. Run `forsaken-world-explorer.exe`, click **Browse…** to select your Forsaken World client folder (containing `update/` and `resources/`), confirm the default zone `s0` is selected, and click **Load World**. The active zone name is shown in the window title and at the top of the Diagnostics panel.
3. Settings, per-run logs, and derived caches are stored beside `forsaken-world-explorer.exe` under `settings/`, `logs/`, and `cache/`. Your client folder is never modified.

## Diagnostic Dump & Small-Sample Export Tools

All diagnostic and export commands are built into `forsaken-world-explorer.exe` and produce small outputs (< 1 MB):

```powershell
# Full stage-by-stage zone dump (including first-block terrain bindings & streamed block count)
.\forsaken-world-explorer.exe --client "C:\Path\To\Forsaken World" --dump-zone s0 --out reports\dump-s0.txt

# Export diffuse, day/night lightmaps, unscaled lit output & all layer masks as PNGs + stats
.\forsaken-world-explorer.exe --client "C:\Path\To\Forsaken World" --export-terrain-pngs s0 --out-dir reports\terrain_s0

# Byte-level .bmd field trace (with optional --client to check texture resolution)
.\forsaken-world-explorer.exe --bmd-trace sample_litmodel_1000.bmd --client "C:\Path\To\Forsaken World"

# Extract a single entry from a client .pck archive
.\forsaken-world-explorer.exe --client "C:\Path\To\Forsaken World" --extract-entry litmodels.pck "s0/1/litmodel_1000.bmd" --out sample_litmodel_1000.bmd

# Write a hex + ASCII dump of the first N bytes of any extracted file
.\forsaken-world-explorer.exe --hexdump sample_litmodel_1000.bmd --bytes 256 --out sample_litmodel_1000_hex.txt

# Export decoded terrain block header and first 16 height values
.\forsaken-world-explorer.exe --client "C:\Path\To\Forsaken World" --export-terrain-block s0 0 0 --out s0_block_0_0.txt

# Decode a DDS texture entry from a .pck archive and save as PNG
.\forsaken-world-explorer.exe --client "C:\Path\To\Forsaken World" --export-texture-png textures.pck "<entry path>" --out layer0.png
```

Additional development survey tools (not shipped in the Windows ZIP):

```bash
cargo run --bin pck-survey -- "C:\Path\To\Forsaken World"
cargo run --bin terrain-survey -- --client "C:\Path\To\Forsaken World" --zone s0 --out reports/terrain-survey-s0.md
cargo run --bin scenery-survey -- --client "C:\Path\To\Forsaken World" --zone s0 --out reports/scenery-survey-s0.md
cargo run --bin speedtree-survey -- --client "C:\Path\To\Forsaken World" --out reports/speedtree-survey.md
cargo run --bin grass-survey -- --client "C:\Path\To\Forsaken World" --zone s0 --out reports/grass-survey-s0.md
```

## Build & Test from Source

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo run --locked --bin shader-validate
cargo run --locked --bin forsaken-world-explorer -- --self-test
cargo run --locked --bin forsaken-world-explorer -- --smoke-launch
```

## License

Licensed under **GPL-2.0-or-later** (see [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)). Never commit game client files, archives, DLLs, or derived caches.
