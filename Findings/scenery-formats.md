# Static Scenery & Building Formats (`.ecwld`, `.ecbsd`, `.bmd`)

This document records the binary formats used for static scenery and building placements in **Forsaken World**, distinguishing source-confirmed fields from byte-consistent and unknown fields.

## 1. World Ornament Placements (`maps/<zone>/<zone>.ecwld` + `<zone>.ecbsd`)

**Evidence:** Verified directly against `ForsakenWorldv260ClientSource/SMElement/SMCommon/EC_WorldFile.h`, `SMElementClient/EC_SceneBlock.cpp`, `SMElementClient/EC_Ornament.cpp`, and real `maps/s0/s0.ecwld` (v14) + `maps/s0/s0.ecbsd` (v15) from `Forsaken World 0.525 Client`.

- `.ecwld` v14: 116-byte `ECWDFILEHEADER` -> `iNumBlock * u32` block offsets -> 44-byte `ECWDFILEBLOCK9` -> `iNumTree * 16` (`ECWDFILETREE5`) -> `iNumWater * 8` (`ECWDFILEDATAREF`) -> `iNumOrnament * 8` (`ECWDFILEDATAREF { dwExportID, dwOffset }`).
- `.ecbsd` v15 at `dwOffset`: 40-byte `ECBSDFILEORNAMENT13` (`vPos[3]`, `vDir[3]`, `vUp[3]`, `bReflect`, `bRefract`, `bOptimize`, pad) followed immediately by `[len: u32][path bytes]` (`.lmd` mapped to `.bmd` per `EC_Ornament.cpp`).
- **Real-file verification on `maps/s0/`:** 2,737 block refs / **1,884** unique `dwExportID` placements, **0** non-`.bmd`/`.lmd` paths.

## 2. Static Mesh Format (`.bmd` — `CELBrushBuilding` + `A3DLitModel` + `A3DLitMesh`)

**Evidence & Real-File Verification (`sample_litmodel_1000.bmd`, 45,318 bytes from `s0/1/litmodel_1000.bmd`):**
- `EL_BrushBuilding.cpp` (`ForsakenWorldv260ClientSource/SMElement/`) confirms the `0x80000001` + `bCollideOnly` container header and `CCDBrush` collision hull trailer.
- Angelica3D engine source (`A3DLitModel.cpp` / `A3DLitMesh`) is not included in `SMElement/`.
- Byte-level tracing (`--bmd-trace sample_litmodel_1000.bmd`) confirmed that `A3DLitMesh v0x10000006` is identical to `0x10000005` except for **1 extra byte** (`mesh_flags_v6: u8`, value `0x00` on all 12 submeshes of `sample_litmodel_1000.bmd`) immediately after `num_verts`/`num_faces` and before the vertex buffer. Immediately after the 48-byte AABB, the NUL-terminated `"MATERIAL: \0"` string and 69-byte `A3DMaterial` follow with zero gap.
- Consumes **45,318 / 45,318 bytes (`remaining = 0`)** across all **12 submeshes** (`658` vertices, `996` indices) and **28 `CCDBrush` collision hulls**.
- `0x10000002`–`0x10000005` parsing paths are unchanged.

### Field-by-Field Evidence Table (`sample_litmodel_1000.bmd`, 45,318 B)

| Byte Offset | Size | Field | Value Read (`sample_litmodel_1000.bmd`) | Evidence | Confidence |
| --- | ---: | --- | --- | --- | --- |
| `0x0000` | 4 B | `magic` | `"MOXB"` (`0x42584f4d`) | Angelica `AFile` binary marker | `source-confirmed` |
| `0x0004` | 4 B | `brush_building_version` | `0x80000001` | `EL_BrushBuilding.cpp` (`CELBrushBuilding::Load`) | `source-confirmed` |
| `0x0008` | 1 B | `bCollideOnly` | `0` | `EL_BrushBuilding.cpp` (`CELBrushBuilding::Load`) | `source-confirmed` |
| `0x0009` | 4 B | `model_version` | `0x10000002` | `A3DLitModel` v2 header | `byte-consistent` |
| `0x000d` | 12 B | `scale` | `(1.0, 1.0, 1.0)` | Unit scale vec3 | `byte-consistent` |
| `0x0019` | 12 B | `direction` | `(-0.3146, 0.0, 0.9492)` | Unit direction vec3 (`|d|=1.0`) | `byte-consistent` |
| `0x0025` | 12 B | `up` | `(0.0, 1.0, 0.0)` | Unit up vec3 | `byte-consistent` |
| `0x0031` | 12 B | `position` | `(-13.06, 166.83, -175.03)` | World position inside `s0` bounds | `byte-consistent` |
| `0x003d` | 4 B | `mesh_count` | `12` | Matches all 12 `0x10000006` submesh markers in file | `byte-consistent` |
| `0x0041` (`65`) | 4 B | `mesh[0].version` | `0x10000006` | Repeated at all 12 submesh boundaries | `byte-consistent` |
| `0x0045` (`69`) | 64 B | `mesh[0].name` | `"Object03"` | NUL-padded GBK string | `byte-consistent` |
| `0x0085` (`133`) | 256 B | `mesh[0].texture` | `"Building\textures\n\孟古罗马战场建筑19.dds"` | NUL-padded GBK `.dds` path | `byte-consistent` |
| `0x0185` (`389`) | 8 B | `num_verts`, `num_faces` | `30`, `18` (`54` indices) | All indices `< 30`, exact stride alignment to next submesh | `byte-consistent` |
| `0x018d` (`397`) | 1 B | `mesh_flags_v6` | `0x00` (all 12 submeshes) | Required 1-byte shift before vertices; aligns `v[0]=(4.640, 9.926, 6.961)`, `diffuse=0xff3d4153`, `uv=(0.0005, 0.0005)` and exact AABB max `(4.640, 9.926, 6.961)`; bit semantics unconfirmed | `unknown` (offset/size `byte-consistent`) |
| `0x018e` (`398`) | `nv*24` B | `vertices` | 30 × 24 B (`pos[3], diffuse, uv[2]`) | Finite floats matching AABB bounds to `0.000`, `uv ∈ [0,1]`, `diffuse=0xff3d4153` | `byte-consistent` |
| `0x045e` (`1118`) | `nf*6` B | `indices` | 54 × `u16` (`2, 3, 0, 0, 1, 2, ...`) | Every index `< num_verts` across all 12 submeshes | `byte-consistent` |
| `0x04ca` (`1226`) | `nv*12` B | `normals` | 30 × `vec3` | Finite unit-length normals | `byte-consistent` |
| `0x0632` (`1586`) | `nv*8` B | `day_diffuse`, `night_diffuse` | 30 × 2 × `u32` | Per-vertex baked day/night ARGB colors | `byte-consistent` |
| `0x0722` (`1826`) | 48 B | `aabb` | `min=(-7.064, 2.105, -2.756)`, `max=(4.640, 9.926, 6.961)` | Matches `v[0]=(4.640, 9.926, 6.961)` to 3 decimal places | `byte-consistent` |
| `0x0752` (`1874`) | 11 B | `material_name` | `"MATERIAL: \0"` | Exact ASCII NUL-terminated prefix immediately after AABB on all 12 submeshes | `byte-consistent` |
| `0x075d` (`1885`) | 69 B | `A3DMaterial` | `ambient/diffuse/emissive/specular (vec4), power (f32), two_sided (u8)` | Exact 69-byte alignment to `mesh[1]` at `0x07a2` (`1954`) | `byte-consistent` |
| `0x8ebd` (`36541`) | 8,777 B | `CELBrushBuilding` collision trailer | `28` hull mesh index lists + `28` `CCDBrush` hulls | `EL_BrushBuilding.cpp`; ends at byte `45,318 / 45,318` (`remaining = 0`) | `source-confirmed` |

### Supported Version Matrix (CI Synthetic Fixtures vs Real Corpus)

| Header / Mesh Version | Synthetic CI Fixture | Real Corpus (`litmodels.pck` / `building.pck`) | Notes |
| --- | --- | --- | --- |
| `CELBrushBuilding` `0x80000001` (`bCollideOnly`) | Tested (`src/bmd.rs`) | Verified in `EL_BrushBuilding.cpp` & `sample_litmodel_1000.bmd` | Optional 5-byte prefix after `MOXB` |
| `A3DLitModel` `0x10000001` / `0x10000002` | Tested (`src/bmd.rs`) | `0x10000002` verified in `sample_litmodel_1000.bmd` | 48-byte transform header |
| `A3DLitMesh` `0x10000002`–`0x10000005` | Tested (`src/bmd.rs`) | Unverified on full archive | Unchanged |
| `A3DLitMesh` `0x10000006` | Tested (`src/bmd.rs`) | **Decoder verified on `sample_litmodel_1000.bmd` (45,318/45,318 B)** | 1-byte `mesh_flags_v6` after AABB; visual rendering awaiting user check |

## 3. Placement Height & Axis-Mapping Verification (`maps/s0/`)

Verified across all **1,884** unique ornament placements in `maps/s0/s0.ecwld` (v14) + `maps/s0/s0.ecbsd` (v15) against the decoded `maps/s0/s0_1.t2bk` (v6, 256/256 blocks) heightfield using `sample_angelica_height` (no fudge offset applied):

- **Winning mapping:** Angelica `(x, y, z)` on `Z = +world_length/2 - row*block_size - gz*step` produces a **0.33 m** median `|dy|` across all 1,884 placements (**1,480 / 1,884** within 1 m; **1,714 / 1,884** within 5 m).
