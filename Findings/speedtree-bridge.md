# 32-Bit SpeedTree Bridge Architecture (`tools/speedtree/`)

## 1. Design Adopted from Reference

We inspected `kasperfriend/Perfect-World-Explorer`'s `tools/speedtree/bake.cpp` and `src/tree_source.rs` and adopted the following core design principles:

1. **Out-of-process 32-bit helper:** Because the client ships a 32-bit `update/speedtreert.dll` while the viewer is 64-bit, a dedicated 32-bit helper executable (`tools/fw-speedtree-helper.exe`) loads the user's own `update/speedtreert.dll` at runtime, bakes tree geometry and LODs, writes a cache file under `<root>/cache/speedtree-v1/`, and exits.
2. **Zero DLL redistribution:** `update/speedtreert.dll` is never bundled, copied into the repo, or modified.
3. **Cache-only viewer reads:** The 64-bit viewer never loads 32-bit DLLs directly and reads only validated `FWST` v1 cache files from `<root>/cache/speedtree-v1/`.

## 2. Helper Build & Packaging

- **Source location:** `tools/speedtree/fw-speedtree-helper/` (standalone zero-dependency Rust crate).
- **Build command (Windows CI):**
  ```sh
  cargo build --manifest-path tools/speedtree/fw-speedtree-helper/Cargo.toml --release --target i686-pc-windows-msvc
  ```
- **Packaged location:** Placed at `tools/fw-speedtree-helper.exe` inside `Forsaken-World-Explorer-Windows-x64.zip`, with its SHA-256 listed in `SHA256SUMS.txt`. No prebuilt binary is committed to the repository.

## 3. Protocol & Binary Cache Format (`FWST` v1)

**CLI invocation (launched by the 64-bit viewer):**
```text
fw-speedtree-helper.exe --client-root <dir> --tree-type <id> --spt <path> --lod <0|1|2> --out <cache-file>
```
- The helper **never writes into `<client-root>`**. It writes only to `--out` (under `<root>/cache/speedtree-v1/`, respecting `FW_CACHE_DIR`).
- Exit codes: `0` success, `2` invalid CLI args, `3` missing/unloadable `update/speedtreert.dll`, `4` missing expected `CSpeedTreeRT` export, `5` `.spt` read/bake failure, `6` cache write failure.

**Binary cache layout (`FWST` v1):**
- Magic `"FWST"` (4 bytes), version `u32 = 1`, `tree_type: u32`, `lod: u32`, `alpha_ref: f32`, `wind_approx: u8` (`1` = shader wind approximation, `0` = runtime-sampled motion), `reserved: [u8; 3]`.
- `trunk_vert_count: u32`, `trunk_idx_count: u32`, `leaf_vert_count: u32`, `leaf_idx_count: u32`.
- Vertex records (36 bytes each: `pos: [f32; 3]`, `normal: [f32; 3]`, `uv: [f32; 2]`, `wind_weight: f32`) followed by `u16` index buffers for trunk and leaf geometry.

## 4. Trust Model, Timeout & Crash Isolation

- **DLL trust check:** Before invoking the helper on a client folder, the viewer hashes `<client-root>/update/speedtreert.dll` (SHA-256) and verifies it against the recognized client fingerprint (`1a62ff4dc25eeb719ef885fcd77b35c35e81b3be32994c9852de31a5955eaf83`). The viewer refuses to launch the helper against an unrecognized or missing DLL and logs a clear warning instead.
- **Helper path check:** The viewer resolves `tools/fw-speedtree-helper.exe` strictly relative to the viewer's own executable root (never from an arbitrary user-supplied path).
- **Timeout & crash handling:** Each helper invocation is polled with a configurable timeout (default `15 s`). A hung helper is killed (`child.kill()`), non-zero exit codes are logged per tree type, and the viewer continues rendering the rest of the world without that tree.
- **CI testing:** The real helper is never run against `speedtreert.dll` in CI (the proprietary DLL is not present). Unit tests exercise protocol encode/decode, fake helper executables returning canned `FWST` output, error exit codes, hung-process timeout kills, and crash handling.
