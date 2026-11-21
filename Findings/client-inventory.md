# Forsaken World Client Inventory

**Evidence source:** Direct inspection of the user-shared Google Drive folder `Games/ForsakenWorld` (client folder `Forsaken World 0.525 Client` and source folder `ForsakenWorldv260ClientSource`) on 2026-11-21. No client files, archives, DLLs, or proprietary source code are committed to this repository.

## 1. Recognized Client Build & Version Fingerprint

- **Drive client folder name:** `Forsaken World 0.525 Client`
- **Included notes (`UltraWidePerspective NOTES.txt`, 507 B):** Identifies the build as a **Forsaken World Blood Harvest** client tested against an **FW v260 Blood Harvest** server VM, launched via `update/pem.exe`.
- **Main client executable:** `update/pem.exe`
  - **Size:** `10,703,784` bytes (`~10.21 MiB`)
  - **Last modified (Drive metadata):** `2016-02-05T14:38:50Z`
  - **SHA-256:** **Not yet computed remotely** — the automated Drive download connector enforces a 10 MB single-file transfer limit (`10,703,784` bytes exceeds the limit). The user must run `Get-FileHash update\pem.exe -Algorithm SHA256` (Windows) or `sha256sum update/pem.exe` (Linux) locally so Phase 1 can record the exact recognized executable hash.
- **Additional client copies in Drive:** Only one Forsaken World client folder (`Forsaken World 0.525 Client`) and one Forsaken World source folder (`ForsakenWorldv260ClientSource`) exist under `Games/ForsakenWorld`.

## 2. Top-Level Client Folder Layout (`Forsaken World 0.525 Client`)

Unlike Perfect World 1.4.2 (which places `elementclient.exe` and `.pck` archives under `element/`), this Forsaken World client places the game executable and runtime DLLs in `update/`, `.pck`/`.pkx` archives in `resources/`, data tables in `data/`, zone maps in `maps/`, and music in `music/`.

### Root Files (9 files, 3,000,837 bytes total)

| File | Extension | Size (bytes) | Notes |
| --- | --- | ---: | --- |
| `patcher.exe` | `.exe` | 594,776 | Client updater/launcher |
| `uninstall.exe` | `.exe` | 540,672 | Uninstaller |
| `uninstall.dat` | `.dat` | 1,008,280 | Uninstaller data |
| `uninstall.ini` | `.ini` | 738 | Uninstaller config |
| `SkinLib.dll` | `.dll` | 278,528 | Patcher UI skin library |
| `sysinfo.dll` | `.dll` | 28,672 | System info helper |
| `p2sp_4th_lib.dll` | `.dll` | 418,208 | Patcher P2P library |
| `packdll.dll` | `.dll` | 90,528 | Patcher archive helper |
| `UltraWidePerspective NOTES.txt` | `.txt` | 507 | Third-party packager notes |

### Top-Level Subfolders (18 folders)

| Folder | Direct Children Observed | Primary File Types | Observed Role / Notes |
| --- | --- | --- | --- |
| `update/` | 6 subfolders (`config`, `gameConfig`, `logs`, `RoleSettings`, `server`, `video`), 13 files (17,239,650 B) | `.exe`, `.dll`, `.lua`, `.txt` | **Main executable directory**: contains `pem.exe` (10,703,784 B), `speedtreert.dll` (454,792 B), `elementskill.dll` (4,580,184 B), `zlibwapi.dll`, `arcsdk.dll`, `dbghelp.dll`, `ftdriver.dll`, `gtembed.dll`, `ifc22.dll`, `immwrapper.dll`, `unicows.dll`, `server.lua`, `stroutpck.txt` |
| `resources/` | 1 subfolder (`help`), 14 archive files (6,903,729,518 B / ~6.43 GiB) | `.pck` (13), `.pkx` (1) | **Client asset archives** (see §3 below) |
| `data/` | 50+ files (paginated in Drive) | `.data`, `.data1`–`.data30`, `.dat`, `.bak` | Contains `elements.data` (109,435,342 B), `elements.data.bak` (60,964,596 B), `task_npc.data` (20,172 B), `tasks.data` + split `tasks.data1`..`tasks.data30`, `path.data` (1,684,392 B), `TaskBitOrder.dat`, `kwar.dat`, `s*.dat`, `f*.dat` |
| `maps/` | 38 subfolders (`world`, `login`, `s0`–`s40`) | `.trn2`, `.t2bk`, `.t2mk`, `.t2lm`, `.t2lm1`, `.t2hc`, `.ecwld`, `.ecbsd`, `.cld`, `.oc`, `.clt`, `.dat`, `.cfg`, `.data`, `.txt` | Zone terrain and world placement directories (see §4 below) |
| `music/` | 100+ files (paginated in Drive; >200 MB observed across first two pages) | `.ogg` | Loose Ogg Vorbis zone music tracks |
| `font/` | 3 files (10,469,656 B) | `.ttf` | `fzl2jw.ttf` (4,126,448 B), `fzlbjw.ttf` (3,679,712 B), `fzxh1jw.ttf` (2,663,496 B) |
| `cursors/` | 22 files (189,032 B) | `.ani` (17), `.cur` (5) | Animated and static Windows cursors |
| `gt/` | 1 subfolder (`config`), 6 files (2,917,854 B) | `.exe`, `.dll`, `.ini` | Voice/platform helper (`gtplatform.exe`, `gtdemo.exe`, `gtbiz.dll`, `gtnettrans.dll`, `gtvoice.dll`, `log.ini`) |
| `skin/` | 2 subfolders (`image`, `sounds`), 3 files (82,202 B) | `.xml`, `.txt` | Patcher skin (`mainuni.xml`, `string.txt`, `stringtab.txt`) |
| `patchersync/` | 2 subfolders (`Logs`, `resources`), 8 files (1,484,457 B) | `.exe`, `.dll`, `.bmp`, `.dat`, `.sw`, `.bat` | Patcher synchronization files |
| `reportbugs/` | 2 files (1,118,208 B) | `.exe`, `.dll` | Crash reporter (`reportbugs.exe`, `mfc42.dll`) |
| `rolesettings/` | 4 subfolders (`gamesave`, `GMLog`, `Layout`, `Team`), 6 files (20,593 B) | `.ini`, `.data`, `.txt` | Client user settings (`systemsettings.ini`, `suggestsettings.ini`, `dyn_tasks.data`, `currentserver.data`) |
| `Logs/` | 7 files (57,317 B) | `.log`, `.bin` | Runtime logs (`A3D.log`, `AF.log`, `AM.log`, `EC.log`, `Imm.log`, `s2ccmd.log`, `s2ccmdlog.bin`) |
| `help/` | 2 subfolders (`book`, `script`) | (subfolders not recursed yet) | In-game help content |
| `info/` | 1 subfolder (`html`) | (subfolder not recursed yet) | Launcher/patcher HTML info |
| `tools/` | 1 subfolder (`launcharc`) | (subfolder not recursed yet) | Arc launcher helper |
| `cache/` | 0 files | — | Empty runtime cache directory |
| `screenshots/` | 0 files | — | Empty screenshots directory |

## 3. Archive Inventory (`resources/`)

All 14 archive files in `resources/` were enumerated directly from Drive metadata:

| Archive | Size (bytes) | Size (MiB) | Notes |
| --- | ---: | ---: | --- |
| `building.pck` | 834,598,694 | 795.94 | Static buildings / scenery |
| `configs.pck` | 834,575 | 0.80 | Client configuration tables |
| `gfx.pck` | 213,680,382 | 203.78 | Visual effects and particles |
| `grasses.pck` | 2,577,365 | 2.46 | Grass meshes and assets |
| `interfaces.pck` | 11,245,757 | 10.72 | UI layouts and assets |
| `litmodels.pck` | 924,774,372 | 881.93 | Lit static models |
| `loddata.pck` | 57,410,755 | 54.75 | LOD geometry/data |
| `models.pck` | 2,147,483,392 | 2048.00 | Split 2 GiB archive (part 1) |
| `models.pkx` | 1,162,789,541 | 1108.92 | Split archive continuation (part 2) |
| `script.pck` | 5,724,389 | 5.46 | Client scripts |
| `sfx.pck` | 722,247,333 | 688.79 | Sound effects |
| `shaders.pck` | 8,225,973 | 7.84 | Client shaders |
| `surfaces.pck` | 690,293,053 | 658.31 | Textures, UI surfaces, map tiles |
| `textures.pck` | 123,887,337 | 118.15 | World/model textures |
| **Total (14 files)** | **6,903,729,518** | **6,583.91** | **Header/keys/compression unverified until Phase 1** |

*Unverified in Phase 0:* Exact `.pck`/`.pkx` header magic, entry table XOR/mask keys, version tag, and compression method are **not assumed** from Perfect World and will be verified directly in Phase 1 before writing any parser.

## 4. Map & Terrain File Families (`maps/`)

Inspected `maps/world/`, `maps/s0/`, and `maps/s1/`:

- `maps/world/` contains only `world_targets.txt` (2 B). Unlike Perfect World (whose main overworld lives in `maps/world/`), Forsaken World organizes playable zones into numbered directories `maps/s0/` through `maps/s40/` (36 numbered zone folders plus `login/` and `world/`).
- `maps/s0/` and `maps/s1/` contain Angelica 2-family file extensions:
  - Terrain: `*.trn2`, `*_N.t2bk`, `*_N.t2mk`, `*_N.t2lm`, `*_N.t2lm1`, `*.t2hc`, `terrain.cfg`
  - World & placement: `*.ecwld`, `*.ecbsd`, `*.cld`, `*.oc`, `scenelights.dat`, `region.clt`, `precinct.clt`, `flyareas.data`, plus `bsdata/` and `movemaps/` subfolders.
- *Unverified in Phase 0:* Binary record layouts, version numbers, and which zone(s) constitute the primary overworld are deferred to Phase 2.

## 5. SpeedTree Runtime Verification (`update/speedtreert.dll`)

Downloaded `update/speedtreert.dll` into the temporary analysis environment (not committed) and inspected its PE header and export table directly:

- **File:** `update/speedtreert.dll`
- **Size:** `454,792` bytes
- **SHA-256:** `1a62ff4dc25eeb719ef885fcd77b35c35e81b3be32994c9852de31a5955eaf83`
- **PE Header:** `PE\0\0`, `Machine = 0x014c` (**x86 32-bit**), `OptionalHeader.Magic = 0x010b` (**PE32**), `TimeDateStamp = 1122542702` (`2005-07-28 UTC`)
- **Exports verified:** `CSpeedTreeRT` C++ class symbols (`??0CSpeedTreeRT@@QAE@XZ`, `SGeometry`, `SIndexed`, `SLeaf`, `SBillboard`, `STextures`, `?Authorize@CSpeedTreeRT@@SAXPBD@Z`, etc.).
- **Conclusion:** Forsaken World **does** ship a **32-bit** `speedtreert.dll` runtime in `update/`. A 64-bit viewer will therefore require a 32-bit bridge helper in Phase 6 (never bundling the proprietary DLL).

## 6. Drive Source Tree (`ForsakenWorldv260ClientSource`) & Licensing Note

- **Top-level folders:** `SMElement/`, `SMReportBugs/`, `SMTaskTemplEditor/`.
- **Key subfolders under `SMElement/`:** `SMElementClient`, `SMCommon`, `SMElementData`, `GfxCommon`, `SMCHBasedCD`, `AUIInterface`, `ElementLocalize`, `SMElementEditor`, `SMElementSkill`, `SMAutoTerrainEditor`, `SMFWEditorLib`, `LuaWrapper`, `MODELVIEW`, `include`, `lib`, `Tools`.
- **Version note:** The source folder is labeled `v260` while the client folder is labeled `0.525` (noting compatibility with an `FW v260` server VM). Any constants or struct layouts read from the source must still be cross-checked against the actual binary files in `Forsaken World 0.525 Client`.
- **Licensing restriction:** `ForsakenWorldv260ClientSource` is proprietary C++ game engine source code (not open-source licensed). **No source code from Drive may be copied into this GPL-2.0-or-later repository.** It may be consulted only as read-only reference material to verify file formats and constants, with every format fact independently documented in `Findings/`.
