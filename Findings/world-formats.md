# Forsaken World World, Terrain & Texture Formats

**Evidence source:** Direct binary inspection and full CPU decoding of real `Forsaken World 0.525 Client` files from Google Drive (`maps/s0/` and `resources/grasses.pck`) on 2026-11-21.

## 1. Terrain Descriptor (`*.trn2`)

Verified on `maps/s0/s0.trn2` (`413` bytes):

| Offset | Type | Observed Value (`s0.trn2`) | Meaning |
| --- | --- | --- | --- |
| `0x00` | `u32` | `0x54524e32` (`TRN2`) | Magic |
| `0x04` | `u32` | `1` | Format version |
| `0x08` | `f32` | `1024.0` | Terrain width (world units) |
| `0x0c` | `f32` | `1024.0` | Terrain length (world units) |
| `0x10` | `f32` | `64.0` | Block size (world units) |
| `0x14` | `u32` | `32` | Block grid cells per edge (`33 x 33 = 1,089` vertices per block) |
| `0x18` | `u32` | `256` | Blocks per sub-terrain (`16 x 16`) |
| `0x1c` | `u32` | `16` | Block rows |
| `0x20` | `u32` | `16` | Block columns |
| `0x24` | `u32` | `5` | Sub-terrain grid / LOD parameter |
| `0x28` | `u32` | `16` | Mask area count (`4 x 4`) |
| `0x2c` | `u32` | `4` | Mask grid / area stride |
| `0x30` | `u32` | `128` | Mask size |
| `0x34` | `u32` | `1` | Sub-terrain count (`s0_1`) |

## 2. Terrain Blocks (`*_N.t2bk`)

Verified on `maps/s0/s0_1.t2bk` (`1,857,222` bytes) — **all 256 / 256 blocks decoded** with `sum(block_spans) = 1,856,182 == file_len - (16 + 256 * 4)` (exact to the byte; decoded elevation range `[68.91, 226.16]`):

- **File header (`0..16`):** `magic = 0x54424b42` (`T2BK`), `version = 6`, `num_blocks = 256`, `compress_flags = 0x35` (`0x01 | 0x04 | 0x10 | 0x20`).
- **Offset table (`16 .. 16 + num_blocks * 4`):** `num_blocks` little-endian `u32` byte offsets (`offsets[0] = 1040`).
- **v6 Block Header (`36` bytes — 4 bytes longer than Perfect World 1.4.2's v5 32-byte header):**
  - `+0`: `row: i16`, `+2`: `col: i16`, `+4`: `mask_area: i16`, `+6`: `block_flags: u8`, `+7`: `pad: u8`
  - `+8`: `lod_scale: f32` (`0.2944` in block 0)
  - `+12`: `u32` (`1` observed in v6)
  - `+16`: `size_hei: u32` (zlib-compressed `(grid + 1)^2 * 4` = `4,356` B `f32` heights)
  - `+20`: `size_nor: u32` (zlib-compressed `2,186` B polar normals: `u32 version = 1`, `u32 count = 1089`, followed by `1089` 2-byte spherical angle pairs)
  - `+24`: `size_diff: u32` (zlib-compressed `4,356` B day per-vertex RGBA)
  - `+28`: `size_diff1: u32` (zlib-compressed `4,356` B night per-vertex RGBA)
  - `+32`: `u32` (`0` observed)
- **Block span:** `36 + size_hei + size_nor + size_diff + size_diff1` matches `offsets[i + 1] - offsets[i]` across all 256 blocks.

## 3. Day & Night Lightmaps (`*_N.t2lm`, `*_N.t2lm1`)

Verified on `maps/s0/s0_1.t2lm` (`2,099,288` B) and `maps/s0/s0_1.t2lm1` (`2,099,288` B) — **32 / 32 lightmaps verified**:

- **Header (`0..24`):** `magic = 0x54324c4d` (`T2LM`), `version = 2`, `num_areas = 16`, `lm_size = 512`, `flags = 1` (DXT1 DDS), `data_size = 131,200`.
- **Offset table (`24 .. 24 + 16 * 4 = 88`):** `16` `u32` offsets (`88 + i * 131,200`).
- **Entry payload (`131,200` B each):** Standard `128`-byte `DDS ` header (`512x512`, FourCC `DXT1`) followed by `131,072` B of BC1 blocks.

## 4. Terrain Masks (`*_N.t2mk`)

Verified on `maps/s0/s0_1.t2mk` (`387,445` bytes):

- **Header (`0..20`):** `magic = 0x544d4b42` (`T2MK`), `version = 2`, `num_areas = 16`, `layer_count_or_flags = 5`, `compressed = 1`.
- **Offset table (`20 .. 20 + 16 * 4`):** `16` `u32` offsets into per-mask-area records (`first_off = 224`).

## 5. World & Static Object Placements (`*.ecwld`, `*.ecbsd`)

Verified on `maps/s0/s0.ecwld` (`54,648` B) and `maps/s0/s0.ecbsd` (`128,015` B):

- `s0.ecwld`: `magic = 0x45435744` (`ECWD`), `version = 14`, world `1024.0 x 1024.0`, `block_size = 64.0`, `block_grid = 32`, `num_blocks = 256` (`16 x 16`).
- `s0.ecbsd`: `magic = 0x42584f4d, 0x42534446` (`MOXBFDSB`), `version = 15`, `num_sub_terrains = 1`; contains `1,884` length-prefixed `.bmd` asset path references (`1\litmodel_*.bmd`) paired with world transform vectors (`pos`, `dir`, `up`).

## 6. DDS Texture Compression Formats

- `s0_1.t2lm` / `s0_1.t2lm1`: **`DXT1` (BC1)** (32 / 32 verified).
- `resources/grasses.pck`: **`DXT3` (BC2)** (45 / 45 verified).
- `src/dxt.rs` parses `DDS ` headers and supports direct raw BC1/BC2/BC3 block access plus CPU RGBA8 decoding for `DXT1`, `DXT3`, `DXT5`, and uncompressed 32-bit RGBA/BGRA surfaces.

## 7. Verification Limits

- Only zone `s0` (`s0.trn2`, `s0_1.t2bk`, `s0_1.t2lm`, `s0_1.t2lm1`, `s0_1.t2mk`, `s0.ecwld`, `s0.ecbsd`) has been inspected directly from Drive. Other zones (`s1`–`s40`) and static `.bmd` mesh geometry inside `building.pck`/`litmodels.pck` (>800 MB each) will be verified via `terrain-survey` and Phase 4 tooling on a full local client.
