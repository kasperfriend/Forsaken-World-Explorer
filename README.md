# Forsaken World Explorer

An experimental, GPU-based, free-flight viewer for a **user-owned Forsaken World client**. It reads world data from a client you provide; it is not a game client, editor, server, or gameplay implementation.

> **Current status (honest assessment):** No rendered feature is marked `working on real data` yet until confirmed visually by the user in the Windows artifact on real client files. Earlier synthetic CI checks and Mesa `lavapipe` smoke tests only prove that shaders and pipelines run on synthetic fixtures — they are not evidence that the game world renders correctly. See [Findings/Status.md](Findings/Status.md) for the exact per-feature status.
>
> **No fake worlds in the default path:** The approximate sky gradient and the synthetic preview scene are **not** game worlds. They have been removed from the default path (the synthetic pipeline test scene is hidden behind a Developer checkbox, off by default). Missing features (skyboxes, effects, map tiles, teleport destinations, audio) are reported explicitly as `missing` or `not started`.

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
2. Run `forsaken-world-explorer.exe`, click **Browse…** to select your Forsaken World client folder (containing `update/` and `resources/`), pick a zone (`s0` by default), and click **Load World**.
3. Settings, per-run logs, and derived caches are stored beside `forsaken-world-explorer.exe` under `settings/`, `logs/`, and `cache/`. Your client folder is never modified.

## Verifying Real Client Decoding (`--dump-zone`)

To inspect exactly what the viewer's readers decode from your client files stage-by-stage (terrain heights, texture dimensions, placement counts, `.bmd` submeshes, vegetation, and missing systems):

```powershell
.\forsaken-world-explorer.exe --client "C:\Path\To\Forsaken World" --dump-zone s0 --out reports\dump-s0.txt
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
