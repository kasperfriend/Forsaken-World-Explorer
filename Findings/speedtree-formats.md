# SpeedTree Data & Runtime Discovery (`speedtreert.dll`, `.spt`, `.ecwld`)

This document records the verified Forsaken World SpeedTree runtime and world placement data, resolving discovery issue [#6](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/6).

## 1. Client Runtime DLL (`update/speedtreert.dll`)

**Evidence:** Inspected directly from `Forsaken World 0.525 Client/update/speedtreert.dll`.

- **File size:** `454,792` bytes
- **SHA-256:** `1a62ff4dc25eeb719ef885fcd77b35c35e81b3be32994c9852de31a5955eaf83`
- **Architecture:** PE32 x86 (`0x014c`), 32-bit DLL (cannot be loaded in-process by the 64-bit viewer; requires the 32-bit helper bridge described in [speedtree-bridge.md](speedtree-bridge.md)).
- **Version string:** No standalone semantic version string (`X.Y.Z`) or UTF-16 `VS_VERSION_INFO` resource is embedded in the DLL binary. Instead, the DLL exports **146** MSVC-mangled `CSpeedTreeRT` symbols matching the **SpeedTreeRT 1.x / Angelica 2.x** ABI, including:
  - `?LoadTree@CSpeedTreeRT@@QAE_NPBEI@Z` (memory `.spt` loader)
  - `?Compute@CSpeedTreeRT@@QAE_NPBMI_N@Z`
  - `?GetGeometry@CSpeedTreeRT@@QAEXAAUSGeometry@1@KFFF@Z`
  - `?GetTextures@CSpeedTreeRT@@QBEXAAUSTextures@1@@Z`
  - `?GetLodLevel@CSpeedTreeRT@@QBEMXZ` / `?GetLodLimits@CSpeedTreeRT@@QBEXAAM0@Z`
  - `?ComputeWindEffects@CSpeedTreeRT@@QAEX_N00@Z` / `?SetWindStrength@CSpeedTreeRT@@QAEMMMM@Z`
- **Embedded `.spt` diagnostics:** Contains `"not a valid SpeedTree SPT file"` and `"malformed SpeedTree SPT file"`, confirming `.spt` input format.
- **What remains unknown:** Exact point release number (e.g. 1.6 vs 1.7) and exact `CSpeedTreeRT` instance allocation size on real `.spt` files inside the full client archives must be confirmed locally via `speedtree-survey` on a Windows machine with the client's `update/speedtreert.dll`.

## 2. World Tree Types & Placements (`maps/<zone>/<zone>.ecwld`)

**Evidence:** Verified against `EC_WorldFile.h`, `EC_SceneBlock.cpp`, and real `maps/s0/s0.ecwld` (v14).

- **Tree type table (`ECWDFILEHEADER::dwTreeOff`, `iNumTreeType`):** Each entry stores two length-prefixed GBK strings: `[len1: u32][spt_path]` (tree definition `.spt`) and `[len2: u32][dds_path]` (composite leaf/frond texture `.dds`).
- **Tree placements per block (`ECWDFILEBLOCK9::iNumTree`):** Immediately after the 44-byte `ECWDFILEBLOCK9` header, `iNumTree` records of `ECWDFILETREE5` (16 bytes each: `dwType: u32`, `vPos: [f32; 3]`).
- **Real-file observation on `maps/s0/`:** `iNumTreeType = 0`, `total_trees = 0` (zone `s0` contains no SpeedTree placements; forested zones `s1`–`s40` carry non-zero `iNumTreeType` and `iNumTree`).

## 3. LOD Levels & Alpha Reference Values

- **LOD levels:** Three discrete LOD tiers baked from the runtime (`1.0` high / near, `0.5` medium, `0.1` low / far), matching the reference's 3-LOD baking pattern. Distance switching is scaled by **Settings → Graphics → Tree LOD distance multiplier** (`25%–400%`, default `100%`).
- **Alpha references:** `SGeometry` exposes per-LOD `alpha_branch` and `alpha_frond` scalar alpha-test thresholds, which are written into the `FWST` v1 cache file alongside leaf alpha-test thresholds. Alpha-tested leaves and branches draw in the opaque pass; any blended canopy/frond parts route through the Phase 5 transparent pass.
