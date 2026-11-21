# Static Scenery Streaming & Memory Budgets

This document explains how **Forsaken World Explorer** bounds CPU RAM, GPU memory, and per-frame load spikes for static `.bmd` scenery and decoded textures, and states what has not been measured without a real client and GPU.

## 1. Camera-Centered Streaming & Multi-Frame Pacing

| Setting | Default | Range | Purpose |
| --- | --- | --- | --- |
| `scenery_enabled` | `true` | `bool` | Toggle static `.bmd` rendering in World View |
| `transparent_scenery_enabled` | `true` | `bool` | Toggle back-to-front blended transparent `.bmd` submeshes (drawn after opaque, before water) |
| `scenery_radius_m` | `256` m | `32..=2048` m | Camera-centered radius for loading/unloading placements |
| `scenery_max_instances` | `1024` | `64..=16384` | Hard cap on simultaneously active placement instances |
| `scenery_loads_per_frame` | `4` | `1..=64` | Maximum new unique `.bmd` models decoded per frame to bound frame-time spikes |
| `scenery_whole_world` | `false` | `bool` | Load all zone placements ignoring radius cap (**warning:** high RAM/VRAM use; unverified on large zones) |

- Placements within `scenery_radius_m` are sorted nearest-first by squared distance to the camera and truncated to `scenery_max_instances`.
- Each frame, at most `scenery_loads_per_frame` uncached models are decoded from archive/cache so moving across a city boundary spreads CPU decode cost over multiple frames instead of stalling one frame.
- **Instance deduplication:** Repeated placements of the same `.bmd` path share **one** decoded mesh (and one GPU vertex/index buffer + texture set). Each placement contributes only its 64-byte `mat4x4<f32>` instance transform.
- **Transparent submesh diagnostics:** The 1 Hz Diagnostics panel reports both `Transparent submeshes drawn` and `Transparent submeshes skipped` (e.g., when transparent scenery is toggled off).

## 2. Content-Addressed Scenery Cache (`<root>/cache/scenery-v1/`)

- Decoded `.bmd` geometry is cached in RAM and backed on disk under `<root>/cache/scenery-v1/` (honoring `FW_CACHE_DIR`), keyed by SHA-256 of the normalized asset path.
- Corrupted or missing `.bmd` files return typed `BmdError` values, increment **Failed to decode** in Diagnostics, log a warning to `<root>/logs/`, and never panic or halt rendering.

## 3. Decoded Texture Cache (`FW_TEX_BUDGET_MIB`, Default 32 MiB)

- **Default budget:** `32 MiB` (`DEFAULT_TEX_BUDGET_MIB`), configurable via Settings → Graphics or `FW_TEX_BUDGET_MIB`.
- **Sharing:** Every texture path (case-insensitive, normalized `/` separators) is decoded once into RGBA8 and shared across all submeshes and placements that reference it.
- **Pinned-batch rule:** Textures referenced by the currently visible draw batch are pinned and **never** silently downscaled or dropped; if pinned bytes exceed budget, the exceedance is counted and logged.

## 4. What Is Not Measured Yet

- Real-world peak VRAM, water pass overhead, and frame-time distributions on full Forsaken World cities (`maps/s0/`–`s40`) cannot be measured in CI without the 6.9 GB client archives and a hardware GPU.
- Whole-world loading (`scenery_whole_world = true`) has not been profiled on large multi-thousand-placement zones.
