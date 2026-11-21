# Terrain Formats & Real-Data Diagnostics (`.trn2`, `.t2bk`, `.t2mk`, `.t2lm`, `.t2lm1`)

This document records the verified Forsaken World terrain layout, block-index coordinate mapping, lightmap/mask decode statistics, and the root cause of the black-terrain-with-blobs issue on `maps/s0/`.

## 1. Block-Index Mapping & `(0, 0)` Height Resolution

In Angelica3D centered world coordinates (`s0.trn2`: `1024 × 1024 m`, `block_size = 64 m`, `block_grid = 32`, `16 × 16` blocks):

```text
x = -world_width / 2  + col * block_size + gx * (block_size / block_grid)
z = +world_length / 2 - row * block_size - gz * (block_size / block_grid)

col = floor((x + world_width / 2) / block_size)   ∈ [0..15]
row = floor((world_length / 2 - z) / block_size)  ∈ [0..15]
block_idx = row * block_cols + col                ∈ [0..255]
```

This resolves the height difference between `s0_block_0_0.txt` and `height(0.00, 0.00)` in `dump-s0.txt`:

- `--export-terrain-block s0 0 0` exports **block `(col=0, row=0)`** (`block_idx = 0`), which is the **northwest corner** of the map (`X ∈ [-512, -448]`, `Z ∈ [+448, +512]`, `height_range = [189.456, 201.444] m`, `h[0] = 189.456 m`).
- World origin **`(X=0.00, Z=0.00)`** maps to `col = floor(512 / 64) = 8`, `row = floor(512 / 64) = 8`, **`block_idx = 8 * 16 + 8 = 136`** (`X ∈ [0, +64]`, `Z ∈ [-64, 0]`, `mask_area = 10`, `height_range = [133.838, 146.299] m`, vertex `[0, 0]` = **`139.300 m`**).

Both numbers come from the exact same `s0_1.t2bk` file and coordinate mapping; `(0, 0)` in `--export-terrain-block` is a block grid index, whereas `(0.00, 0.00)` in `height(0.00, 0.00)` is a world-space meter coordinate in block `(8, 8)`.

## 2. Why Terrain Rendered Black With Orange & Light Blobs (Root Cause)

Quantitative checks on real `maps/s0/` files (`s0_1.t2lm`, `s0_1.t2lm1`, `s0_1.t2mk`) and the GPU pipeline (`src/terrain_gpu.rs`, `src/shaders/terrain.wgsl`, `src/shaders/blit.wgsl`) identified two compounding defects:

1. **sRGB decode without linear-to-sRGB encode (primary cause):**
   - `TerrainViewportRenderer::upload_rgba_view` created the composited terrain texture with `wgpu::TextureFormat::Rgba8UnormSrgb`, causing the GPU sampler to apply `pow(x, 2.2)` on read.
   - However, `scene_color` is `wgpu::TextureFormat::Rgba8Unorm` (linear) and `src/shaders/blit.wgsl` copies `scene_color` verbatim without linear-to-sRGB encoding.
   - A normal soil/grass pixel (`diffuse ≈ 0.45`, `t2lm ≈ 0.56`, product `0.252`) was crushed to `0.252^2.2 = 0.048` (`~12/255`), while only bright orange lava (`layer[4]`: `彦古罗马战场岩浆01.dds`) and bright highlights survived `pow(x, 2.2)` — producing black terrain with orange and light blobs.
   - **Fix:** Changed `upload_rgba_view` to `wgpu::TextureFormat::Rgba8Unorm` to match the linear `scene_color` target and verbatim blit.
2. **Double lighting in `terrain.wgsl` (secondary cause):**
   - `terrain.wgsl` multiplied the already-lit `lm` (`diffuse × t2lm`) a second time by a synthetic hemisphere factor `shade = 0.40 + 0.60 * ndotl`, crushing mid-tones further toward black.
   - **Fix:** Removed the synthetic `shade` multiplier in `terrain.wgsl`. The shader now outputs the **unscaled** `diffuse × lightmap` product (`in.color.rgb * lm`). Note: DirectX 9 / Angelica3D terrain lightmaps may use a `2×` (`D3DTOP_MODULATE2X`) scale, but because the exact engine shader constant is not confirmed in `ForsakenWorldv260ClientSource`, the viewer and `--export-terrain-pngs` use the **unscaled** product and label it `unscaled`.

## 3. Verified Real-File Decode Properties (`maps/s0/`)

- **DXT1 Lightmaps (`s0_1.t2lm`, `s0_1.t2lm1`):** `ver=2`, 16 areas, `512×512` DXT1 (`131,200` B/area). Both 4-color (`c0 > c1`) and 3-color (`c0 <= c1`) blocks are present (`area[0]`: `13,396` 4-color, `2,988` 3-color; `area[5]`: `13,318` / `3,066`; `area[10]`: `12,539` / `3,845`), with `zero_frac = 0.0000` and non-black channel means (`R 152–207`, `G 142–182`, `B 133–161`).
- **Layer Masks (`s0_1.t2mk`):** `ver=2`, 16 mask areas, 5 `trn2` layers, 43 total area-layer entries, `256×256` 8-bit masks (`0..255` unsigned bytes). Layer `0` in each mask area has `data_len = 0` (base layer, weight `255/255`); subsequent layers map `layer_index -> trn2.texture_paths[layer_index]` and alpha-blend in order (`dst * (1 - m/255) + src * (m/255)`).
