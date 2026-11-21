# Terrain Formats & Real-Data Diagnostics (`.trn2`, `.t2bk`, `.t2mk`, `.t2lm`, `.t2lm1`)

This document records the verified Forsaken World terrain layout, block-index coordinate mapping, per-layer tiling scales from `s0_1.t2mk`, GPU mipmap/sampler settings, and mask-to-layer verification.

## 1. Block-Index Mapping & `(0, 0)` Height Resolution

In Angelica3D centered world coordinates (`s0.trn2`: `1024 × 1024 m`, `block_size = 64 m`, `block_grid = 32`, `16 × 16` blocks):

```text
col = floor((x + world_width / 2) / block_size)   ∈ [0..15]
row = floor((world_length / 2 - z) / block_size)  ∈ [0..15]
block_idx = row * block_cols + col                ∈ [0..255]
```

- Block `(0, 0)` (`block_idx = 0`) is the northwest corner (`X ∈ [-512, -448]`, `Z ∈ [+448, +512]`, `height_range = [189.456, 201.444] m`).
- World origin `(0.00, 0.00)` lies in block `(col=8, row=8)` (`block_idx = 136`, `X ∈ [0, +64]`, `Z ∈ [-64, 0]`, `mask_area = 10`, `h[0] = 139.300 m`).

## 2. Blurry Ground Root Cause & Real Multi-Layer GPU Splatting Fix

- **Root cause of blurry, muddy ground:** The previous renderer baked the entire `1024 m × 1024 m` zone into a single `512 × 512` diffuse texture (`2.0 m/texel`, `mip_level_count = 1`), throwing away 99% of the `512 × 512` layer DDS detail.
- **Verified per-layer tiling table in `s0_1.t2mk` (bytes `84..224`, `5 × 28 = 140` bytes between offset table and `area[0]`):**
  - `layer[0]` (`彦古罗马战场土01.dds`): `proj_axis=1 (Y)`, `scale_u = scale_v = 0.1000` (1 tile / 10 m = `6.4` repeats / 64 m block)
  - `layer[1]` (`彦古罗马战场草01.dds`): `proj_axis=1 (Y)`, `scale_u = scale_v = 0.1000` (1 tile / 10 m)
  - `layer[2]` (`彦古罗马战场山01.dds`): `proj_axis=1 (Y)`, `scale_u = scale_v = 0.0556` (1 tile / 18 m)
  - `layer[3]` (`彦古罗马战场岩浆石头01.dds`): `proj_axis=1 (Y)`, `scale_u = scale_v = 0.0833` (1 tile / 12 m)
  - `layer[4]` (`彦古罗马战场岩浆01.dds`): `proj_axis=1 (Y)`, `scale_u = scale_v = 0.0667` (1 tile / 15 m)
- **GPU Terrain Shader (`src/shaders/terrain.wgsl`) & Sampler (`src/terrain_gpu.rs`):**
  - Uploads all real `512 × 512` layer DDS textures as a `texture_2d_array<f32>` with a **full 10-level 2×2 box-filter mipmap chain** (`512 -> 1`, no invented detail).
  - Sampler: `address_mode_u/v = Repeat`, `mag_filter = Linear`, `min_filter = Linear`, `mipmap_filter = Linear`, `anisotropy_clamp = 16`.
  - Samples each layer at `world_pos.xz * scale_u[i]` (~`2 cm/texel`, **100× sharper** than the old `512×512` whole-zone bake), blends layers `1..7` over base layer `0` using the real `t2mk` 8-bit masks (`0..255` normalized to `0.0..1.0` in GPU `Rgba8Unorm`), and multiplies by the unscaled raw `t2lm`/`t2lm1` lightmap (`Rgba8Unorm`, no sRGB double-decode).
  - Note: Client source (`EC_World.cpp`) delegates terrain rendering to precompiled `A3DTerrain2`, so anisotropic level (`16`) and unscaled `diffuse × lightmap` are explicitly documented; the baked whole-zone composite is now only used as a debug export in `--export-terrain-pngs`.
