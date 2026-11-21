# Static Scenery Streaming & Memory Budgets

This document explains how **Forsaken World Explorer** bounds CPU RAM, GPU memory, and per-frame load spikes for static `.bmd` scenery and decoded textures, and states what has not been measured without a real client and GPU.

## 1. Camera-Centered Streaming & Multi-Frame Pacing

| Setting | Default | Range | Purpose |
| --- | --- | --- | --- |
| `scenery_enabled` | `true` | `bool` | Toggle static `.bmd` rendering in World View |
| `scenery_radius_m` | `256` m | `32..=2048` m | Camera-centered radius for loading/unloading placements |
| `scenery_max_instances` | `1024` | `64..=16384` | Hard cap on simultaneously active placement instances |
| `scenery_loads_per_frame` | `4` | `1..=64` | Maximum new unique `.bmd` models decoded per frame to bound frame-time spikes |
| `scenery_whole_world` | `false` | `bool` | Load all zone placements ignoring radius cap (**warning:** high RAM/VRAM use; unverified on large zones) |

- Placements within `scenery_radius_m` are sorted nearest-first by squared distance to the camera and truncated to `scenery_max_instances`.
- Each frame, at most `scenery_loads_per_frame` uncached models are decoded from archive/cache so moving across a city boundary spreads CPU decode cost over multiple frames instead of stalling one frame.
- **Instance deduplication:** Repeated placements of the same `.bmd` path share **one** decoded mesh (and one GPU vertex/index buffer + texture set). Each placement contributes only its 64-byte `mat4x4<f32>` instance transform.

## 2. Content-Addressed Scenery Cache (`<root>/cache/scenery-v1/`)

- Decoded `.bmd` geometry is cached in RAM up to a configurable hot budget and backed on disk under `<root>/cache/scenery-v1/` (honoring `FW_CACHE_DIR`), keyed by SHA-256 of the source asset path and version tag.
- Corrupted or missing `.bmd` files return typed `BmdError` values, increment the **Failed to decode** counter in the Diagnostics panel, log a warning to `<root>/logs/`, and never panic or halt rendering.

## 3. Decoded Texture Cache (`FW_TEX_BUDGET_MIB`, Default 32 MiB)

- **Default budget:** `32 MiB` (`DEFAULT_TEX_BUDGET_MIB`), configurable via Settings → Graphics or `FW_TEX_BUDGET_MIB`.
- **Sharing:** Every texture path (case-insensitive, normalized `/` separators) is decoded once into RGBA8 and shared across all submeshes and placements that reference it.
- **Pinned-batch rule:** Before rendering a frame, all textures referenced by the currently visible draw batch are **pinned**.
  - LRU eviction removes only **unpinned** textures when resident bytes exceed the budget.
  - Pinned textures are **never** silently dropped or downscaled to fit the budget.
  - If the pinned set alone exceeds `tex_budget_mib`, the cache keeps every pinned texture intact, increments the budget-exceeded counter, and emits a warning log entry (rate-limited per exceedance transition).

## 4. What Is Not Measured Yet

- Real-world peak VRAM and frame-time distributions on full Forsaken World cities (e.g. Freedom Harbor / `maps/s0/` with all 1,884 placements) cannot be measured in CI without the 6.9 GB client archives and a hardware GPU.
- Whole-world loading (`scenery_whole_world = true`) has not been profiled on large multi-thousand-placement zones.
- Users can measure real zone model counts, decode failure rates, unique textures, and estimated GPU memory locally using `scenery-survey`.
