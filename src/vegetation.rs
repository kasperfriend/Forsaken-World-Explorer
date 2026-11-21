//! Phase 6 vegetation subsystem:
//! - SpeedTree 32-bit helper protocol, DLL SHA-256 trust verification, timeout/crash isolation
//! - `FWST` v1 binary tree cache (`<root>/cache/speedtree-v1/`), LOD selection & instance dedup
//! - Client tree (`ECWDFILETREE5`) and grass (`ECWDFILEGRASSTYPE` / `ECBSDFILEGRASS7`) parsers
//! - Camera-centered grass streaming with deterministic density thinning (`Low`/`Medium`/`High`)

use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use sha2::{Digest, Sha256};

use crate::{
    settings::GrassDensity,
    world_objects::{EcbsdHeader, EcwldHeader},
};

pub const SPEEDTREE_CACHE_SUBDIR: &str = "speedtree-v1";
pub const FWST_MAGIC: &[u8; 4] = b"FWST";
pub const FWST_VERSION: u32 = 1;
/// Verified SHA-256 of `Forsaken World 0.525 Client/update/speedtreert.dll` (454,792 bytes).
pub const RECOGNIZED_SPEEDTREE_DLL_SHA256: &str =
    "1a62ff4dc25eeb719ef885fcd77b35c35e81b3be32994c9852de31a5955eaf83";
pub const DEFAULT_HELPER_TIMEOUT_SECS: u64 = 15;

const ECWLD_HEADER_BYTES: usize = 116;
const ECWLD_BLOCK9_BYTES: usize = 44;
const ECWLD_TREE5_BYTES: usize = 16;
const ECWLD_DATAREF_BYTES: usize = 8;
const ECWLD_GRASSTYPE_BYTES: usize = 24;
const ECBSD_GRASS7_BYTES: usize = 36;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct VegVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub wind_weight: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FwstTreeMesh {
    pub tree_type: u32,
    pub lod: u32,
    pub alpha_ref: f32,
    pub wind_approximate: bool,
    pub trunk_vertices: Vec<VegVertex>,
    pub trunk_indices: Vec<u16>,
    pub leaf_vertices: Vec<VegVertex>,
    pub leaf_indices: Vec<u16>,
}

impl FwstTreeMesh {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(FWST_MAGIC);
        out.extend_from_slice(&FWST_VERSION.to_le_bytes());
        out.extend_from_slice(&self.tree_type.to_le_bytes());
        out.extend_from_slice(&self.lod.to_le_bytes());
        out.extend_from_slice(&self.alpha_ref.to_le_bytes());
        out.extend_from_slice(&[u8::from(self.wind_approximate), 0, 0, 0]);
        out.extend_from_slice(&(self.trunk_vertices.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.trunk_indices.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.leaf_vertices.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.leaf_indices.len() as u32).to_le_bytes());
        for v in self.trunk_vertices.iter().chain(&self.leaf_vertices) {
            out.extend_from_slice(bytemuck::bytes_of(v));
        }
        for idx in self.trunk_indices.iter().chain(&self.leaf_indices) {
            out.extend_from_slice(&idx.to_le_bytes());
        }
        out
    }

    pub fn decode(data: &[u8]) -> Result<Self, String> {
        if data.len() < 40 {
            return Err("FWST cache file too small".into());
        }
        if &data[0..4] != FWST_MAGIC {
            return Err("invalid FWST magic".into());
        }
        let ver = u32le(data, 4)?;
        if ver != FWST_VERSION {
            return Err(format!("unsupported FWST version {ver}"));
        }
        let tree_type = u32le(data, 8)?;
        let lod = u32le(data, 12)?;
        let alpha_ref = f32le(data, 16)?;
        let wind_approximate = data[20] != 0;
        let tv_cnt = u32le(data, 24)? as usize;
        let ti_cnt = u32le(data, 28)? as usize;
        let lv_cnt = u32le(data, 32)? as usize;
        let li_cnt = u32le(data, 36)? as usize;
        if tv_cnt > 200_000 || lv_cnt > 200_000 || ti_cnt > 600_000 || li_cnt > 600_000 {
            return Err("FWST geometry counts exceed limits".into());
        }

        let vert_bytes = (tv_cnt + lv_cnt) * std::mem::size_of::<VegVertex>();
        let idx_bytes = (ti_cnt + li_cnt) * 2;
        if 40 + vert_bytes + idx_bytes > data.len() {
            return Err("truncated FWST geometry payload".into());
        }

        let mut p = 40usize;
        let read_verts = |cnt: usize, cursor: &mut usize| -> Result<Vec<VegVertex>, String> {
            let mut out = Vec::with_capacity(cnt);
            for _ in 0..cnt {
                let v = VegVertex {
                    position: [
                        f32le(data, *cursor)?,
                        f32le(data, *cursor + 4)?,
                        f32le(data, *cursor + 8)?,
                    ],
                    normal: [
                        f32le(data, *cursor + 12)?,
                        f32le(data, *cursor + 16)?,
                        f32le(data, *cursor + 20)?,
                    ],
                    uv: [f32le(data, *cursor + 24)?, f32le(data, *cursor + 28)?],
                    wind_weight: f32le(data, *cursor + 32)?,
                };
                *cursor += 36;
                out.push(v);
            }
            Ok(out)
        };
        let trunk_vertices = read_verts(tv_cnt, &mut p)?;
        let leaf_vertices = read_verts(lv_cnt, &mut p)?;

        let read_idxs =
            |cnt: usize, max_v: usize, cursor: &mut usize| -> Result<Vec<u16>, String> {
                let mut out = Vec::with_capacity(cnt);
                for _ in 0..cnt {
                    let idx = u16le(data, *cursor)?;
                    *cursor += 2;
                    if idx as usize >= max_v {
                        return Err(format!("FWST index {idx} out of bounds ({max_v})"));
                    }
                    out.push(idx);
                }
                Ok(out)
            };
        let trunk_indices = read_idxs(ti_cnt, tv_cnt, &mut p)?;
        let leaf_indices = read_idxs(li_cnt, lv_cnt, &mut p)?;

        Ok(Self {
            tree_type,
            lod,
            alpha_ref,
            wind_approximate,
            trunk_vertices,
            trunk_indices,
            leaf_vertices,
            leaf_indices,
        })
    }
}

/// Request passed across the 64-bit viewer -> 32-bit helper CLI protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperRequest {
    pub tree_type: u32,
    pub spt_path: String,
    pub lod: u32,
    pub out_cache_file: PathBuf,
}

impl HelperRequest {
    pub fn to_cli_args(&self, client_root: &Path) -> Vec<String> {
        vec![
            "--client-root".into(),
            client_root.display().to_string(),
            "--tree-type".into(),
            self.tree_type.to_string(),
            "--spt".into(),
            self.spt_path.clone(),
            "--lod".into(),
            self.lod.min(2).to_string(),
            "--out".into(),
            self.out_cache_file.display().to_string(),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperRunReport {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub duration_ms: u64,
    pub stderr_summary: String,
}

/// Verify `<client-root>/update/speedtreert.dll` exists and matches the recognized
/// SHA-256 fingerprint from Phase 0/1 before launching the 32-bit helper.
pub fn verify_client_speedtree_dll(client_root: &Path) -> Result<String, String> {
    let dll = client_root.join("update").join("speedtreert.dll");
    if !dll.is_file() {
        return Err(format!(
            "Client SpeedTree runtime not found at {}",
            dll.display()
        ));
    }
    let bytes = fs::read(&dll).map_err(|e| format!("failed to read {}: {e}", dll.display()))?;
    let digest = Sha256::digest(&bytes);
    let mut hex = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{b:02x}");
    }
    if hex != RECOGNIZED_SPEEDTREE_DLL_SHA256 {
        return Err(format!(
            "Unrecognized speedtreert.dll SHA-256 ({hex}); refusing to launch helper for untrusted DLL"
        ));
    }
    Ok(hex)
}

/// Resolve the 32-bit helper executable strictly relative to the viewer executable root
/// (`<root>/tools/fw-speedtree-helper.exe`). Never accepts an arbitrary user path.
pub fn resolve_helper_path(portable_root: &Path) -> PathBuf {
    #[cfg(windows)]
    let name = "fw-speedtree-helper.exe";
    #[cfg(not(windows))]
    let name = "fw-speedtree-helper";
    portable_root.join("tools").join(name)
}

/// Launch `helper_exe` with `req`, enforcing `timeout` and killing hung processes cleanly.
pub fn run_speedtree_helper(
    helper_exe: &Path,
    client_root: &Path,
    req: &HelperRequest,
    timeout: Duration,
) -> Result<HelperRunReport, String> {
    if !helper_exe.is_file() {
        return Err(format!(
            "SpeedTree 32-bit helper missing at {}",
            helper_exe.display()
        ));
    }
    let start = Instant::now();
    let mut child = Command::new(helper_exe)
        .args(req.to_cli_args(client_root))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn {}: {e}", helper_exe.display()))?;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let dur = start.elapsed().as_millis() as u64;
                let mut stderr_buf = String::new();
                if let Some(mut err_pipe) = child.stderr.take() {
                    use std::io::Read;
                    let _ = err_pipe.read_to_string(&mut stderr_buf);
                }
                return Ok(HelperRunReport {
                    exit_code: status.code(),
                    timed_out: false,
                    duration_ms: dur,
                    stderr_summary: stderr_buf.trim().to_string(),
                });
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(HelperRunReport {
                        exit_code: None,
                        timed_out: true,
                        duration_ms: start.elapsed().as_millis() as u64,
                        stderr_summary: format!("killed after {} ms timeout", timeout.as_millis()),
                    });
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                let _ = child.kill();
                return Err(format!("error polling SpeedTree helper: {e}"));
            }
        }
    }
}

/// Select discrete tree LOD (`0` high, `1` medium, `2` low) by distance and multiplier.
pub fn select_tree_lod(distance_m: f32, lod_dist_mult: f32) -> u32 {
    let mult = lod_dist_mult.clamp(0.25, 4.0);
    let near_cutoff = 90.0 * mult;
    let mid_cutoff = 220.0 * mult;
    if distance_m <= near_cutoff {
        0
    } else if distance_m <= mid_cutoff {
        1
    } else {
        2
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeTypeDef {
    pub type_id: u32,
    pub spt_path: String,
    pub dds_path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreePlacement {
    pub tree_type: u32,
    pub position: [f32; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct GrassTypeDef {
    pub type_id: u32,
    pub alpha_blend: bool,
    pub alpha_ref: i32,
    pub size: f32,
    pub size_var: f32,
    pub softness: f32,
    pub sight_range: f32,
    pub texture_path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GrassPlacement {
    pub export_id: u32,
    pub grass_type: u32,
    pub position: [f32; 3],
    pub grid_size: f32,
    pub grid_cols: i32,
    pub grid_rows: i32,
}

/// Deduplicate visible tree placements by `(tree_type, lod)` so one GPU mesh serves all instances.
pub fn dedup_trees_by_type_and_lod(
    placements: &[TreePlacement],
    camera_pos: Vec3,
    radius_m: f32,
    max_instances: usize,
    lod_dist_mult: f32,
) -> BTreeMap<(u32, u32), Vec<Mat4>> {
    let r_sq = radius_m.max(1.0) * radius_m.max(1.0);
    let mut near: Vec<(f32, &TreePlacement)> = placements
        .iter()
        .filter_map(|t| {
            let d2 = Vec3::from_array(t.position).distance_squared(camera_pos);
            (d2 <= r_sq).then_some((d2, t))
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.truncate(max_instances);

    let mut grouped: BTreeMap<(u32, u32), Vec<Mat4>> = BTreeMap::new();
    for (d2, t) in near {
        let lod = select_tree_lod(d2.sqrt(), lod_dist_mult);
        let mat = Mat4::from_translation(Vec3::from_array(t.position));
        grouped.entry((t.tree_type, lod)).or_default().push(mat);
    }
    grouped
}

/// Deterministic hash-based grass density thinning (never invents placements; only
/// subsamples real client placements according to `GrassDensity`).
pub fn select_visible_grass<'a>(
    placements: &'a [GrassPlacement],
    camera_pos: Vec3,
    radius_m: f32,
    max_instances: usize,
    density: GrassDensity,
) -> Vec<&'a GrassPlacement> {
    let r_sq = radius_m.max(1.0) * radius_m.max(1.0);
    let keep_pct = density.keep_percent();
    let mut near: Vec<(f32, &'a GrassPlacement)> = placements
        .iter()
        .filter(|g| {
            let bucket = (g.export_id.wrapping_mul(2654435761) >> 24) % 100;
            bucket < keep_pct
        })
        .filter_map(|g| {
            let d2 = Vec3::from_array(g.position).distance_squared(camera_pos);
            (d2 <= r_sq).then_some((d2, g))
        })
        .collect();
    near.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then_with(|| a.1.export_id.cmp(&b.1.export_id))
    });
    near.truncate(max_instances);
    near.into_iter().map(|(_, g)| g).collect()
}

/// Deduplicate visible grass placements by `grass_type` so one mesh serves all instances.
pub fn dedup_grass_by_type(selected: &[&GrassPlacement]) -> BTreeMap<u32, Vec<Mat4>> {
    let mut grouped: BTreeMap<u32, Vec<Mat4>> = BTreeMap::new();
    for g in selected {
        let mat = Mat4::from_translation(Vec3::from_array(g.position));
        grouped.entry(g.grass_type).or_default().push(mat);
    }
    grouped
}

/// Parse tree types, tree placements, grass types, and grass placements from `.ecwld` + `.ecbsd`.
#[allow(clippy::type_complexity)]
pub fn parse_ecwld_ecbsd_vegetation(
    wld: &[u8],
    bsd: &[u8],
) -> Result<
    (
        EcwldHeader,
        EcbsdHeader,
        Vec<TreeTypeDef>,
        Vec<TreePlacement>,
        Vec<GrassTypeDef>,
        Vec<GrassPlacement>,
    ),
    String,
> {
    let wh = EcwldHeader::parse(wld)?;
    let bh = EcbsdHeader::parse(bsd)?;
    if wld.len() < ECWLD_HEADER_BYTES {
        return Err("ecwld header truncated".into());
    }
    let tree_off = u32le(wld, 36)? as usize;
    let grass_off = u32le(wld, 40)? as usize;
    let num_tree_types = u32le(wld, 44)? as usize;
    let num_grass_types = u32le(wld, 48)? as usize;

    let mut tree_types = Vec::new();
    let mut p = tree_off;
    for i in 0..num_tree_types {
        if p + 8 > wld.len() {
            break;
        }
        let l1 = u32le(wld, p)? as usize;
        p += 4;
        if p + l1 + 4 > wld.len() {
            break;
        }
        let (s1, _, _) = encoding_rs::GBK.decode(&wld[p..p + l1]);
        p += l1;
        let l2 = u32le(wld, p)? as usize;
        p += 4;
        if p + l2 > wld.len() {
            break;
        }
        let (s2, _, _) = encoding_rs::GBK.decode(&wld[p..p + l2]);
        p += l2;
        tree_types.push(TreeTypeDef {
            type_id: i as u32,
            spt_path: s1.replace('\\', "/"),
            dds_path: s2.replace('\\', "/"),
        });
    }

    let mut grass_types = Vec::new();
    let mut gp = grass_off;
    for i in 0..num_grass_types {
        if gp + ECWLD_GRASSTYPE_BYTES + 4 > wld.len() {
            break;
        }
        let alpha_blend = wld[gp] != 0;
        let alpha_ref = i32le(wld, gp + 4)?;
        let size = f32le(wld, gp + 8)?;
        let size_var = f32le(wld, gp + 12)?;
        let softness = f32le(wld, gp + 16)?;
        let sight_range = f32le(wld, gp + 20)?;
        gp += ECWLD_GRASSTYPE_BYTES;
        let slen = u32le(wld, gp)? as usize;
        gp += 4;
        if gp + slen > wld.len() {
            break;
        }
        let (path, _, _) = encoding_rs::GBK.decode(&wld[gp..gp + slen]);
        gp += slen;
        grass_types.push(GrassTypeDef {
            type_id: i as u32,
            alpha_blend,
            alpha_ref,
            size,
            size_var,
            softness,
            sight_range,
            texture_path: path.replace('\\', "/"),
        });
    }

    let num_blocks = wh.num_blocks as usize;
    let table_end = ECWLD_HEADER_BYTES + num_blocks * 4;
    if table_end > wld.len() {
        return Err("ecwld block table truncated".into());
    }

    let mut trees = Vec::new();
    let mut grasses = Vec::new();
    let mut seen_grass = HashSet::new();

    for bi in 0..num_blocks {
        let boff = u32le(wld, ECWLD_HEADER_BYTES + bi * 4)? as usize;
        if boff + ECWLD_BLOCK9_BYTES > wld.len() {
            continue;
        }
        let num_tree = i32le(wld, boff + 4)?.max(0) as usize;
        let num_water = i32le(wld, boff + 8)?.max(0) as usize;
        let num_grass = i32le(wld, boff + 12)?.max(0) as usize;
        let num_orn = i32le(wld, boff + 16)?.max(0) as usize;
        let num_box = i32le(wld, boff + 20)?.max(0) as usize;

        let mut cur = boff + ECWLD_BLOCK9_BYTES;
        for _ in 0..num_tree {
            if cur + ECWLD_TREE5_BYTES > wld.len() {
                break;
            }
            let dtype = u32le(wld, cur)?;
            let pos = [
                f32le(wld, cur + 4)?,
                f32le(wld, cur + 8)?,
                f32le(wld, cur + 12)?,
            ];
            cur += ECWLD_TREE5_BYTES;
            trees.push(TreePlacement {
                tree_type: dtype,
                position: pos,
            });
        }

        // Skip water, ornaments, box areas to reach grass refs (per EC_SceneBlock::Load order)
        cur += (num_water + num_orn + num_box) * ECWLD_DATAREF_BYTES;
        for _ in 0..num_grass {
            if cur + ECWLD_DATAREF_BYTES > wld.len() {
                break;
            }
            let eid = u32le(wld, cur)?;
            let bsd_off = u32le(wld, cur + 4)? as usize;
            cur += ECWLD_DATAREF_BYTES;
            if seen_grass.insert(eid) && bsd_off + ECBSD_GRASS7_BYTES <= bsd.len() {
                let gtype = u32le(bsd, bsd_off)?;
                let pos = [
                    f32le(bsd, bsd_off + 4)?,
                    f32le(bsd, bsd_off + 8)?,
                    f32le(bsd, bsd_off + 12)?,
                ];
                let grid_size = f32le(bsd, bsd_off + 16)?;
                let grid_cols = i32le(bsd, bsd_off + 20)?;
                let grid_rows = i32le(bsd, bsd_off + 24)?;
                grasses.push(GrassPlacement {
                    export_id: eid,
                    grass_type: gtype,
                    position: pos,
                    grid_size,
                    grid_cols,
                    grid_rows,
                });
            }
        }
    }

    Ok((wh, bh, tree_types, trees, grass_types, grasses))
}

/// Measure total bytes stored in `<cache_root>/speedtree-v1/`.
pub fn speedtree_cache_bytes(cache_root: &Path) -> u64 {
    let dir = cache_root.join(SPEEDTREE_CACHE_SUBDIR);
    let Ok(rd) = fs::read_dir(dir) else {
        return 0;
    };
    rd.flatten()
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

pub mod synthetic {
    use super::*;

    /// Build a synthetic `FwstTreeMesh` (trunk quad + alpha-tested leaf quads) for CPU tests
    /// and the Mesa `lavapipe` offscreen vegetation smoke test.
    pub fn build_synthetic_tree_fwst(tree_type: u32, lod: u32) -> FwstTreeMesh {
        let s = match lod {
            0 => 1.0f32,
            1 => 0.85,
            _ => 0.7,
        };
        let trunk_vertices = vec![
            VegVertex {
                position: [-2.0 * s, 0.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [0.0, 1.0],
                wind_weight: 0.0,
            },
            VegVertex {
                position: [2.0 * s, 0.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [1.0, 1.0],
                wind_weight: 0.0,
            },
            VegVertex {
                position: [1.5 * s, 16.0 * s, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [1.0, 0.0],
                wind_weight: 0.2,
            },
            VegVertex {
                position: [-1.5 * s, 16.0 * s, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [0.0, 0.0],
                wind_weight: 0.2,
            },
        ];
        let trunk_indices = vec![0, 1, 2, 0, 2, 3];

        let leaf_vertices = vec![
            VegVertex {
                position: [-10.0 * s, 12.0 * s, 0.0],
                normal: [0.0, 1.0, 0.0],
                uv: [0.0, 1.0],
                wind_weight: 1.0,
            },
            VegVertex {
                position: [10.0 * s, 12.0 * s, 0.0],
                normal: [0.0, 1.0, 0.0],
                uv: [1.0, 1.0],
                wind_weight: 1.0,
            },
            VegVertex {
                position: [10.0 * s, 28.0 * s, 0.0],
                normal: [0.0, 1.0, 0.0],
                uv: [1.0, 0.0],
                wind_weight: 1.0,
            },
            VegVertex {
                position: [-10.0 * s, 28.0 * s, 0.0],
                normal: [0.0, 1.0, 0.0],
                uv: [0.0, 0.0],
                wind_weight: 1.0,
            },
        ];
        let leaf_indices = vec![0, 1, 2, 0, 2, 3];

        FwstTreeMesh {
            tree_type,
            lod,
            alpha_ref: 0.5,
            wind_approximate: true,
            trunk_vertices,
            trunk_indices,
            leaf_vertices,
            leaf_indices,
        }
    }

    /// Build a crossed-blade grass tuft mesh shared by all instances of a grass type.
    pub fn build_grass_tuft_mesh(size: f32) -> (Vec<VegVertex>, Vec<u16>) {
        let h = size.max(0.5);
        let w = h * 0.7;
        let verts = vec![
            VegVertex {
                position: [-w, 0.0, 0.0],
                normal: [0.0, 1.0, 0.0],
                uv: [0.0, 1.0],
                wind_weight: 0.0,
            },
            VegVertex {
                position: [w, 0.0, 0.0],
                normal: [0.0, 1.0, 0.0],
                uv: [1.0, 1.0],
                wind_weight: 0.0,
            },
            VegVertex {
                position: [w, h, 0.0],
                normal: [0.0, 1.0, 0.0],
                uv: [1.0, 0.0],
                wind_weight: 1.0,
            },
            VegVertex {
                position: [-w, h, 0.0],
                normal: [0.0, 1.0, 0.0],
                uv: [0.0, 0.0],
                wind_weight: 1.0,
            },
            VegVertex {
                position: [0.0, 0.0, -w],
                normal: [0.0, 1.0, 0.0],
                uv: [0.0, 1.0],
                wind_weight: 0.0,
            },
            VegVertex {
                position: [0.0, 0.0, w],
                normal: [0.0, 1.0, 0.0],
                uv: [1.0, 1.0],
                wind_weight: 0.0,
            },
            VegVertex {
                position: [0.0, h, w],
                normal: [0.0, 1.0, 0.0],
                uv: [1.0, 0.0],
                wind_weight: 1.0,
            },
            VegVertex {
                position: [0.0, h, -w],
                normal: [0.0, 1.0, 0.0],
                uv: [0.0, 0.0],
                wind_weight: 1.0,
            },
        ];
        let idxs = vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7];
        (verts, idxs)
    }
}

fn u16le(b: &[u8], off: usize) -> Result<u16, String> {
    let s = b
        .get(off..off + 2)
        .ok_or_else(|| format!("truncated u16 at {off}"))?;
    Ok(u16::from_le_bytes(s.try_into().unwrap()))
}

fn u32le(b: &[u8], off: usize) -> Result<u32, String> {
    let s = b
        .get(off..off + 4)
        .ok_or_else(|| format!("truncated u32 at {off}"))?;
    Ok(u32::from_le_bytes(s.try_into().unwrap()))
}

fn i32le(b: &[u8], off: usize) -> Result<i32, String> {
    let s = b
        .get(off..off + 4)
        .ok_or_else(|| format!("truncated i32 at {off}"))?;
    Ok(i32::from_le_bytes(s.try_into().unwrap()))
}

fn f32le(b: &[u8], off: usize) -> Result<f32, String> {
    let s = b
        .get(off..off + 4)
        .ok_or_else(|| format!("truncated f32 at {off}"))?;
    let v = f32::from_le_bytes(s.try_into().unwrap());
    if !v.is_finite() {
        return Err(format!("non-finite f32 at {off}"));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn fwst_cache_round_trip_and_corruption_rejection() {
        let orig = synthetic::build_synthetic_tree_fwst(7, 1);
        let bytes = orig.encode();
        let decoded = FwstTreeMesh::decode(&bytes).expect("decode valid FWST");
        assert_eq!(decoded, orig);

        assert!(FwstTreeMesh::decode(&[]).is_err());
        assert!(FwstTreeMesh::decode(&[0u8; 40]).is_err());
        assert!(FwstTreeMesh::decode(&bytes[..bytes.len() - 4]).is_err());
    }

    #[test]
    fn tree_lod_selection_and_instance_dedup() {
        assert_eq!(select_tree_lod(40.0, 1.0), 0);
        assert_eq!(select_tree_lod(150.0, 1.0), 1);
        assert_eq!(select_tree_lod(300.0, 1.0), 2);
        // 2x LOD distance multiplier pushes 150m back into LOD 0
        assert_eq!(select_tree_lod(150.0, 2.0), 0);

        let placements = vec![
            TreePlacement {
                tree_type: 1,
                position: [10.0, 0.0, 0.0],
            },
            TreePlacement {
                tree_type: 1,
                position: [25.0, 0.0, 0.0],
            },
            TreePlacement {
                tree_type: 1,
                position: [160.0, 0.0, 0.0],
            },
            TreePlacement {
                tree_type: 2,
                position: [20.0, 0.0, 0.0],
            },
        ];
        let grouped = dedup_trees_by_type_and_lod(&placements, Vec3::ZERO, 200.0, 100, 1.0);
        assert_eq!(grouped.get(&(1, 0)).map(Vec::len), Some(2));
        assert_eq!(grouped.get(&(1, 1)).map(Vec::len), Some(1));
        assert_eq!(grouped.get(&(2, 0)).map(Vec::len), Some(1));
    }

    #[test]
    fn grass_streaming_window_density_and_dedup() {
        let placements: Vec<GrassPlacement> = (0..100u32)
            .map(|i| GrassPlacement {
                export_id: i + 1,
                grass_type: i % 2,
                position: [(i as f32) * 2.0, 0.0, 0.0],
                grid_size: 2.0,
                grid_cols: 4,
                grid_rows: 4,
            })
            .collect();

        let high = select_visible_grass(&placements, Vec3::ZERO, 120.0, 500, GrassDensity::High);
        let med = select_visible_grass(&placements, Vec3::ZERO, 120.0, 500, GrassDensity::Medium);
        let low = select_visible_grass(&placements, Vec3::ZERO, 120.0, 500, GrassDensity::Low);
        assert!(high.len() > med.len() && med.len() > low.len() && !low.is_empty());

        let dedup = dedup_grass_by_type(&high);
        assert_eq!(dedup.len(), 2, "two grass types deduplicated to 2 batches");
    }

    #[test]
    fn fake_helper_canned_output_error_code_and_timeout_kill() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("fw-fake-helper-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let req = HelperRequest {
            tree_type: 3,
            spt_path: "trees/oak.spt".into(),
            lod: 0,
            out_cache_file: dir.join("out.fwst"),
        };

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let write_script = |name: &str, body: &str| -> PathBuf {
                let p = dir.join(name);
                fs::write(&p, body).unwrap();
                fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
                p
            };

            let ok_helper = write_script("ok.sh", "#!/bin/sh\nexit 0\n");
            let r_ok =
                run_speedtree_helper(&ok_helper, &dir, &req, Duration::from_secs(2)).unwrap();
            assert_eq!(r_ok.exit_code, Some(0));
            assert!(!r_ok.timed_out);

            let err_helper = write_script("err.sh", "#!/bin/sh\necho 'bad spt' >&2\nexit 5\n");
            let r_err =
                run_speedtree_helper(&err_helper, &dir, &req, Duration::from_secs(2)).unwrap();
            assert_eq!(r_err.exit_code, Some(5));
            assert!(r_err.stderr_summary.contains("bad spt"));

            let hang_helper = write_script("hang.sh", "#!/bin/sh\nsleep 5\n");
            let r_hang =
                run_speedtree_helper(&hang_helper, &dir, &req, Duration::from_millis(120)).unwrap();
            assert!(r_hang.timed_out);
            assert_eq!(r_hang.exit_code, None);
        }

        // Untrusted / missing DLL must fail cleanly
        assert!(verify_client_speedtree_dll(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
