# Static Scenery & Building Formats (`.ecwld`, `.ecbsd`, `.bmd`)

This document records the binary formats used for static scenery and building placements in **Forsaken World**, distinguishing directly verified fields from inferred or corpus-unverified fields.

## 1. World Ornament Placements (`maps/<zone>/<zone>.ecwld` + `<zone>.ecbsd`)

**Evidence:** Verified directly against `ForsakenWorldv260ClientSource/SMElement/SMCommon/EC_WorldFile.h`, `SMElementClient/EC_SceneBlock.cpp`, `SMElementClient/EC_Ornament.cpp`, and real `maps/s0/s0.ecwld` (v14) + `maps/s0/s0.ecbsd` (v15) from `Forsaken World 0.525 Client`.

### `.ecwld` Block Directory (v14)
1. `ECWDFILEHEADER` (116 bytes): `dwIdentify = "ECWD"` (`0x45435744` LE), `dwVersion = 14`, `fWorldWid`, `fWorldLen`, `fBlockSize`, `iBlockGrid`, `iNumBlock`, `iWorldBlkRow`, `iWorldBlkCol`, `dwTreeOff`, `dwGrassOff`, `iNumTreeType`, `iNumGrassType`, `dwFlags`, `aReserved[60]`.
2. Block offset table: `iNumBlock` little-endian `u32` offsets immediately at byte 116.
3. Each block begins with `ECWDFILEBLOCK9` (44 bytes): `iRowInWorld: u16`, `iColInWorld: u16`, followed by 10 `i32` counts: `iNumTree`, `iNumWater`, `iNumGrass`, `iNumOrnament`, `iNumBoxArea`, `iNumEffect`, `iNumECModel`, `iNumCritter`, `iNumBezier`, `iNumSound`.
4. Immediately following `ECWDFILEBLOCK9`:
   - `iNumTree` records of `ECWDFILETREE5` (16 bytes each: `dwType: u32`, `vPos: [f32; 3]`).
   - `iNumWater` records of `ECWDFILEDATAREF` (8 bytes each: `dwExportID: u32`, `dwOffset: u32`).
   - `iNumOrnament` records of `ECWDFILEDATAREF` (8 bytes each: `dwExportID: u32`, `dwOffset: u32`).

### `.ecbsd` Ornament Record (`ECBSDFILEORNAMENT13`, v15)
Seeking to `dwOffset` in `<zone>.ecbsd` reads:
1. `ECBSDFILEORNAMENT13` (40 bytes):
   - `vPos: [f32; 3]` (world position, meters)
   - `vDir: [f32; 3]` (forward direction vector)
   - `vUp: [f32; 3]` (up direction vector)
   - `bReflect: u8`, `bRefract: u8`, `bOptimize: u8`, `_pad: u8` (struct alignment padding to 40 bytes)
2. `strModelFile`: `[len: u32][ASCII/GBK path bytes]` (no trailing NUL).
   - Per `EC_Ornament.cpp`, if the stored extension is `.lmd`, the client replaces the `l` with `b` to load the exported `.bmd` file.

**Real-file verification on `maps/s0/`:** Parsing all 256 blocks of `s0.ecwld` -> `s0.ecbsd` yields **2,737** block ornament references and **1,884** unique `dwExportID` placements, with **0** non-`.bmd`/`.lmd` paths.

### Placement Transform Convention
- Right-handed orthonormal basis built from `vDir` and `vUp` (`right = normalize(cross(up, dir))`, re-orthogonalized `up = cross(dir, right)`), combined with `.bmd` model scale (`[sx, sy, sz]`) and translation `vPos`.
- **Open item:** Whether any real `s0` `.bmd` assets carry pre-baked world-space vertex positions (as in some Perfect World `litmodel_*.bmd` exports where `vPos = (0, 0, 0)`) vs local-space vertices placed at `vPos` must be confirmed locally on the full `litmodels.pck` archive via `scenery-survey` and visual inspection.

## 2. Static Mesh Format (`.bmd` — `CELBrushBuilding` + `A3DLitModel` + `A3DLitMesh`)

**Evidence:** Container header and collision trailer verified directly from `ForsakenWorldv260ClientSource/SMElement/SMCommon/EL_BrushBuilding.cpp` and `EL_BrushBuilding.h`. Inner `A3DLitModel` / `A3DLitMesh` binary layout is shared across Angelica 2.x clients (`Perfect-World-Explorer` reference); individual `.bmd` files inside `resources/litmodels.pck` and `resources/building.pck` (>800 MB) were not downloaded or inspected locally in CI.

| Section | Layout | Verification Status |
| --- | --- | --- |
| Binary AFile marker | `u32` `"MOXB"` (`0x42584f4d` LE) | Verified (`EL_BrushBuilding.cpp` / `AFileImage`) |
| Optional brush header | If `u32 == 0x80000001`: read `bCollideOnly: u8`; else rewind 4 bytes | Verified (`EL_BrushBuilding.cpp:288-296`) |
| `A3DLitModel` header | `version: u32` (`0x10000001` or `0x10000002`), `scale [f32; 3]`, `dir [f32; 3]`, `up [f32; 3]`, `pos [f32; 3]`, `num_meshes: u32` | Inferred from Angelica 2.x `A3DLitModel::Load`; verify on real `litmodels.pck` via `scenery-survey` |
| `A3DLitMesh` header | `mesh_ver: u32` (`0x10000002`..=`0x10000005`), `name: [u8; 64]` (GBK), `texture: [u8; 256]` (GBK), `num_verts: u32`, `num_faces: u32` | Inferred from Angelica 2.x `A3DLitMesh::Load`; verify on real `litmodels.pck` via `scenery-survey` |
| Vertex stream | v4/v5: 24 B/vert (`pos [f32; 3]`, `diffuse u32`, `uv [f32; 2]`); v2/v3: 28 B/vert (trailing 4 B extra field) | Inferred from Angelica 2.x; synthetic golden-hash tested |
| Index stream | `num_faces * 3` little-endian `u16` vertex indices | Inferred from Angelica 2.x; bounds-checked |
| Normals & light arrays | `normals: [[f32; 3]; num_verts]`; for v3+: `day_diffuse: [u32; num_verts]`, `night_diffuse: [u32; num_verts]`; `aabb: 48 B` | Inferred from Angelica 2.x; synthetic golden-hash tested |
| Material tail (v5+) | NUL-terminated GBK name, `ambient/diffuse/emissive/specular: [f32; 4]`, `power: f32`, `two_sided: u8` | Inferred from Angelica 2.x `A3DMaterial::Load` |
| Collision trailer | Optional `num_hulls: u32`, per-hull mesh-index lists, `CCDBrush` records | Verified (`EL_BrushBuilding.cpp:309-316`) |

## 3. Alpha Handling & Deferred Items

- **Opaque & alpha-tested submeshes:** Drawn in Phase 4 (`diffuse.a >= 0.99` -> opaque; `alpha_test` flag or `DXT3`/`DXT5` cutout material -> alpha-tested with `discard` at `alpha < 0.5`).
- **Transparent / alpha-blended submeshes:** Submeshes with translucent material alpha (`0.0 < diffuse.a < 0.99`) are classified as `AlphaMode::TransparentDeferred` and **not drawn** in Phase 4 so they are never rendered with broken depth sorting. They are deferred to Phase 5.
- **Scene lighting:** Uses an approximate directional sun + hemisphere ambient light until full regional/time-of-day lighting data is decoded in Phase 5.
- **Open items to verify locally with `scenery-survey`:**
  1. Confirm whether any `.bmd` files in Forsaken World `litmodels.pck` / `building.pck` use a mesh version newer than `0x10000005` (e.g., `0x10000006` with secondary UV / lightmap channel).
  2. Confirm texture path resolution prefixes inside `textures.pck` / `litmodels.pck` / `building.pck`.
