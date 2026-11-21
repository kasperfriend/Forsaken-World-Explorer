# Forsaken World `.pck` / `.pkx` Archive Format

**Evidence source:** Direct binary inspection and full entry + payload decompression of two real archives from the Google Drive client (`Forsaken World 0.525 Client`):
- `resources/configs.pck` (`834,575` bytes) — **67 / 67 entries and payloads** decompressed and verified with zero errors.
- `resources/grasses.pck` (`2,577,365` bytes) — **103 / 103 entries and payloads** decompressed and verified with zero errors.

## 1. Binary Layout (Angelica v2.0.2 with Forsaken World Keys)

Forsaken World uses the Angelica v2 (`0x00020002`) package structure, **but with different footer guard keys and different XOR entry-table keys** than Perfect World 1.4.2.

### Safe Header (bytes `0..12`, little-endian)

| Offset | Type | Value / Meaning | Verified On |
| --- | --- | --- | --- |
| `0x00` | `u32` | `0x4dca23ef` (`SAFE_GUARD0`) | `configs.pck`, `grasses.pck` |
| `0x04` | `u32` | Logical end offset (`834,575` in `configs.pck`; `2,577,365` in `grasses.pck`) | `configs.pck`, `grasses.pck` |
| `0x08` | `u32` | `0x56a089b7` (`SAFE_GUARD1`) | `configs.pck`, `grasses.pck` |

### Footer (`end - 280 .. end`, 280 bytes, little-endian)

| Footer Offset | Type | Forsaken World 0.525 Value | Perfect World 1.4.2 (for comparison) |
| --- | --- | --- | --- |
| `+0` | `u32` | **`0xa508bdc4`** (`FOOTER_GUARD0`) | `0xfdfdfeee` |
| `+4` | `u32` | `0x00020002` (`version`) | `0x00020002` / `0x00020001` |
| `+8` | `u32` | `entry_table_offset ^ 0x21c31a3f` | `entry_table_offset ^ 0xa8937462` |
| `+12` | `u32` | `flags` (`0x00000000` observed; bit `0x80000000` = encrypted) | `flags` |
| `+16..+268` | `[u8; 252]` | NUL-padded ASCII description (`"Angelica File Package, Perfect World..."`) | Same |
| `+268` | `u32` | **`0x853567c8`** (`FOOTER_GUARD1`) | `0xf00dbeef` |
| `+272` | `u32` | `entry_count` (`67` in `configs.pck`; `103` in `grasses.pck`) | `entry_count` |
| `+276` | `u32` | `0x00020002` (`version_tail`) | `0x00020002` / `0x00020001` |

### Entry Table (`entry_table_offset .. end - 280`)

Each of the `entry_count` sequential records starts with an 8-byte masked length header:

- `ENTRY_MASK = 0x21c31a3f`
- `ENTRY_CHECK = 0x399f3a1a`
- `w0 = record_len ^ ENTRY_MASK`
- `w1 = record_len ^ ENTRY_MASK ^ ENTRY_CHECK` (so `w0 ^ w1 == ENTRY_CHECK`)

Followed by `record_len` bytes (`1..=276`):
- If `record_len == 276`: raw 276-byte entry struct.
- If `record_len < 276`: zlib-compressed stream decompressing to exactly `276` bytes.

Decompressed 276-byte entry record:

| Offset | Type | Meaning |
| --- | --- | --- |
| `0..260` | `[u8; 260]` | NUL-terminated GBK/GB18030 relative path (backslash-separated, e.g. `autofamilyconfigs\autoterrain.cfg`) |
| `260..264` | `u32` | Logical payload offset in archive |
| `264..268` | `u32` | Uncompressed payload size in bytes |
| `268..272` | `u32` | Stored (compressed) payload size in bytes |
| `272..276` | `u32` | Extra / reserved `u32` (`0` observed across all 170 entries) |

### Payload Storage

- Located at `[offset .. offset + compressed]` in the logical archive stream.
- If `compressed == size`, stored uncompressed; otherwise standard zlib stream decompressing to `size` bytes.

## 2. Split `.pck` + `.pkx` Archives (`models.pck` + `models.pkx`)

- In `Forsaken World 0.525 Client/resources/`, `models.pck` is exactly `2,147,483,392` bytes (`0x7FFFFF00`, Angelica's 2 GiB − 256 B split threshold) and `models.pkx` is `1,162,789,541` bytes.
- Combined logical length: `3,310,272,933` bytes (`0xC54E79A5`), which fits in the 32-bit `end` field.
- `src/pck.rs` treats a `.pck` + `.pkx` pair as one contiguous logical byte stream (`0..pck_size` in `.pck`, `pck_size..pck_size+pkx_size` in `.pkx`, including reads spanning the split boundary).
- **Verification limit:** Split `.pck` + `.pkx` reading is verified on synthetic fixtures (`pck::tests::round_trip_split_pck_and_pkx_across_boundary`). Real `models.pck` / `models.pkx` (`~3.1 GiB`) exceeds the 10 MB Drive connector limit and has not been read directly yet; running `cargo run --bin pck-survey -- <client-root>` locally will verify all 14 archives.

## 3. Sample Verified Entries

- `resources/configs.pck`: `autofamilyconfigs\autoterrain.cfg` (607 B), `autofamilyconfigs\sky.lst` (170 B), `autofamilyconfigs\trees.lst` (9,804 B), `actions_npc.txt` (543 B), `always_show_gfx_list.txt` (13,493 B).
- `resources/grasses.pck`: GBK-encoded paths containing `.dds` textures and `.mox` grass meshes (103/103 verified).
