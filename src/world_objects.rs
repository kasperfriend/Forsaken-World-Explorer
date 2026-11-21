//! CPU decoders for Forsaken World `.ecwld` (v14) headers, `.ecbsd` (v15) static
//! `.bmd` ornament placements, and deterministic placement-to-transform math.
//!
//! Verified against `maps/s0/s0.ecwld`, `maps/s0/s0.ecbsd`, `EC_WorldFile.h`,
//! `EC_SceneBlock.cpp`, and `EC_Ornament.cpp` (see `Findings/scenery-formats.md`).

use std::collections::HashSet;

use glam::{Mat4, Vec3};

pub const ECWLD_MAGIC: u32 = 0x4543_5744; // "DWCE" LE
pub const ECBSD_MAGIC0: u32 = 0x4258_4f4d; // "MOXB" LE
pub const ECBSD_MAGIC1: u32 = 0x4253_4446; // "FDSB" LE

const ECWLD_HEADER_BYTES: usize = 116;
const ECWLD_BLOCK9_BYTES: usize = 44;
const ECWLD_TREE5_BYTES: usize = 16;
const ECWLD_DATAREF_BYTES: usize = 8;
const ECBSD_ORNAMENT13_BYTES: usize = 40;

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
    Ok(f32::from_le_bytes(s.try_into().unwrap()))
}

#[derive(Debug, Clone, PartialEq)]
pub struct EcwldHeader {
    pub version: u32,
    pub world_width: f32,
    pub world_length: f32,
    pub block_size: f32,
    pub block_grid: u32,
    pub num_blocks: u32,
    pub block_rows: u32,
    pub block_cols: u32,
}

impl EcwldHeader {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        if data.len() < 48 {
            return Err("ecwld file too small".into());
        }
        if u32le(data, 0)? != ECWLD_MAGIC {
            return Err("invalid ECWLD magic".into());
        }
        let version = u32le(data, 4)?;
        let world_width = f32le(data, 8)?;
        let world_length = f32le(data, 12)?;
        let block_size = f32le(data, 16)?;
        let block_grid = u32le(data, 20)?;
        let num_blocks = u32le(data, 24)?;
        let block_rows = u32le(data, 28)?;
        let block_cols = u32le(data, 32)?;
        if !world_width.is_finite() || !world_length.is_finite() || !block_size.is_finite() {
            return Err("non-finite ECWLD dimensions".into());
        }
        Ok(Self {
            version,
            world_width,
            world_length,
            block_size,
            block_grid,
            num_blocks,
            block_rows,
            block_cols,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EcbsdHeader {
    pub version: u32,
    pub sub_terrain_count: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StaticPlacement {
    pub asset_path: String,
    pub position: [f32; 3],
}

/// Full static scenery placement parsed from `.ecwld` + `.ecbsd` (`ECBSDFILEORNAMENT13`).
#[derive(Debug, Clone, PartialEq)]
pub struct OrnamentPlacement {
    pub export_id: u32,
    pub asset_path: String,
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub up: [f32; 3],
    pub scale: [f32; 3],
    pub reflect: bool,
    pub refract: bool,
    pub optimize: bool,
}

impl OrnamentPlacement {
    /// Normalize the stored ornament path to forward slashes and ensure `.lmd`
    /// references map to their exported `.bmd` file (matching `EC_Ornament.cpp`).
    pub fn normalized_bmd_path(&self) -> String {
        let mut s = self.asset_path.replace('\\', "/");
        if s.len() >= 4 && s[s.len() - 4..].eq_ignore_ascii_case(".lmd") {
            let prefix_len = s.len() - 4;
            s.replace_range(prefix_len.., ".bmd");
        }
        s
    }

    /// Build the 4x4 world transform matrix from stored position, direction, up, and scale.
    pub fn transform_matrix(&self) -> Mat4 {
        placement_transform_matrix(self.position, self.direction, self.up, self.scale)
    }
}

/// Deterministic placement-to-transform matrix from `(position, direction, up, scale)`.
///
/// Builds a right-handed orthonormal basis with `+Z = forward`, `+Y = up`, `+X = cross(up, dir)`,
/// applies non-uniform scale `(sx, sy, sz)` along the local axes, and places translation in column 3.
pub fn placement_transform_matrix(
    position: [f32; 3],
    direction: [f32; 3],
    up: [f32; 3],
    scale: [f32; 3],
) -> Mat4 {
    let dir_raw = Vec3::from_array(direction);
    let forward = if dir_raw.length_squared() > 1e-10 {
        dir_raw.normalize()
    } else {
        Vec3::Z
    };
    let up_raw = Vec3::from_array(up);
    let up_hint = if up_raw.length_squared() > 1e-10 {
        up_raw.normalize()
    } else {
        Vec3::Y
    };
    let right_raw = up_hint.cross(forward);
    let right = if right_raw.length_squared() > 1e-10 {
        right_raw.normalize()
    } else {
        let fallback = if forward.y.abs() < 0.99 {
            Vec3::Y
        } else {
            Vec3::X
        };
        fallback.cross(forward).normalize()
    };
    let ortho_up = forward.cross(right).normalize();

    let sx = if scale[0].is_finite() && scale[0] != 0.0 {
        scale[0]
    } else {
        1.0
    };
    let sy = if scale[1].is_finite() && scale[1] != 0.0 {
        scale[1]
    } else {
        1.0
    };
    let sz = if scale[2].is_finite() && scale[2] != 0.0 {
        scale[2]
    } else {
        1.0
    };

    Mat4::from_cols(
        (right * sx).extend(0.0),
        (ortho_up * sy).extend(0.0),
        (forward * sz).extend(0.0),
        Vec3::from_array(position).extend(1.0),
    )
}

impl EcbsdHeader {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        if data.len() < 16 {
            return Err("ecbsd file too small".into());
        }
        if u32le(data, 0)? != ECBSD_MAGIC0 || u32le(data, 4)? != ECBSD_MAGIC1 {
            return Err("invalid ECBSD magic".into());
        }
        let version = u32le(data, 8)?;
        let sub_terrain_count = u32le(data, 12)?;
        Ok(Self {
            version,
            sub_terrain_count,
        })
    }
}

/// Parse all deduplicated ornament placements from an `.ecwld` (v9+) + `.ecbsd` (v13+) pair
/// using the exact block offset table and `ECBSDFILEORNAMENT13` binary layout.
pub fn parse_ecwld_ecbsd_ornaments(
    wld: &[u8],
    bsd: &[u8],
) -> Result<(EcwldHeader, EcbsdHeader, Vec<OrnamentPlacement>), String> {
    let wld_hdr = EcwldHeader::parse(wld)?;
    let bsd_hdr = EcbsdHeader::parse(bsd)?;
    if wld.len() < ECWLD_HEADER_BYTES {
        return Err("ecwld header truncated (< 116 bytes)".into());
    }
    let num_blocks = wld_hdr.num_blocks as usize;
    let table_end = ECWLD_HEADER_BYTES
        .checked_add(
            num_blocks
                .checked_mul(4)
                .ok_or("ecwld block table overflow")?,
        )
        .ok_or("ecwld block table overflow")?;
    if table_end > wld.len() {
        return Err("ecwld block offset table truncated".into());
    }

    let mut seen_ids = HashSet::new();
    let mut placements = Vec::new();
    for bi in 0..num_blocks {
        let boff = u32le(wld, ECWLD_HEADER_BYTES + bi * 4)? as usize;
        if boff + ECWLD_BLOCK9_BYTES > wld.len() {
            return Err(format!("ecwld block {bi} header truncated at {boff}"));
        }
        let _row = u16le(wld, boff)?;
        let _col = u16le(wld, boff + 2)?;
        let num_tree = i32le(wld, boff + 4)?;
        let num_water = i32le(wld, boff + 8)?;
        let _num_grass = i32le(wld, boff + 12)?;
        let num_ornament = i32le(wld, boff + 16)?;
        if num_tree < 0 || num_water < 0 || num_ornament < 0 {
            return Err(format!("negative object count in ecwld block {bi}"));
        }

        let mut p = boff + ECWLD_BLOCK9_BYTES;
        p = p
            .checked_add((num_tree as usize) * ECWLD_TREE5_BYTES)
            .and_then(|v| v.checked_add((num_water as usize) * ECWLD_DATAREF_BYTES))
            .ok_or("ecwld block offset overflow")?;
        let orn_end = p
            .checked_add((num_ornament as usize) * ECWLD_DATAREF_BYTES)
            .ok_or("ecwld ornament table overflow")?;
        if orn_end > wld.len() {
            return Err(format!("ecwld block {bi} ornament refs truncated"));
        }

        for _ in 0..num_ornament {
            let export_id = u32le(wld, p)?;
            let bsd_off = u32le(wld, p + 4)? as usize;
            p += ECWLD_DATAREF_BYTES;
            if !seen_ids.insert(export_id) {
                continue;
            }
            if bsd_off + ECBSD_ORNAMENT13_BYTES + 4 > bsd.len() {
                return Err(format!(
                    "ecbsd ornament {export_id} truncated at offset {bsd_off}"
                ));
            }
            let pos = [
                f32le(bsd, bsd_off)?,
                f32le(bsd, bsd_off + 4)?,
                f32le(bsd, bsd_off + 8)?,
            ];
            let dir = [
                f32le(bsd, bsd_off + 12)?,
                f32le(bsd, bsd_off + 16)?,
                f32le(bsd, bsd_off + 20)?,
            ];
            let up = [
                f32le(bsd, bsd_off + 24)?,
                f32le(bsd, bsd_off + 28)?,
                f32le(bsd, bsd_off + 32)?,
            ];
            if pos.iter().chain(&dir).chain(&up).any(|v| !v.is_finite()) {
                return Err(format!("non-finite transform in ornament {export_id}"));
            }
            let reflect = bsd[bsd_off + 36] != 0;
            let refract = bsd[bsd_off + 37] != 0;
            let optimize = bsd[bsd_off + 38] != 0;

            let slen = u32le(bsd, bsd_off + ECBSD_ORNAMENT13_BYTES)? as usize;
            if slen == 0 || slen > 512 {
                return Err(format!(
                    "invalid ornament {export_id} path length {slen} at {bsd_off}"
                ));
            }
            let sstart = bsd_off + ECBSD_ORNAMENT13_BYTES + 4;
            let send = sstart
                .checked_add(slen)
                .ok_or("ecbsd ornament string overflow")?;
            let path_bytes = bsd
                .get(sstart..send)
                .ok_or_else(|| format!("ecbsd ornament {export_id} path truncated"))?;
            let (decoded, _, _) = encoding_rs::GBK.decode(path_bytes);

            placements.push(OrnamentPlacement {
                export_id,
                asset_path: decoded.into_owned(),
                position: pos,
                direction: dir,
                up,
                scale: [1.0, 1.0, 1.0],
                reflect,
                refract,
                optimize,
            });
        }
    }
    Ok((wld_hdr, bsd_hdr, placements))
}

/// Extract length-prefixed `.bmd` placement records from a standalone `.ecbsd` buffer
/// when `.ecwld` is unavailable (used by Phase 2 surveys).
pub fn scan_ecbsd_bmd_placements(
    data: &[u8],
) -> Result<(EcbsdHeader, Vec<StaticPlacement>), String> {
    let header = EcbsdHeader::parse(data)?;
    let mut placements = Vec::new();
    let mut i = 16usize;
    while i + 16 < data.len() {
        let len = u32::from_le_bytes(data[i..i + 4].try_into().unwrap()) as usize;
        if (5..=260).contains(&len) && i + 4 + len + 12 <= data.len() {
            let path_bytes = &data[i + 4..i + 4 + len];
            if path_bytes.iter().all(|&b| (0x20..0x7f).contains(&b)) {
                if let Ok(path) = std::str::from_utf8(path_bytes) {
                    if path.len() >= 4
                        && path[path.len() - 4..].eq_ignore_ascii_case(".bmd")
                        && !path.contains(' ')
                    {
                        let pos_off = i + 4 + len;
                        let px = f32le(data, pos_off)?;
                        let py = f32le(data, pos_off + 4)?;
                        let pz = f32le(data, pos_off + 8)?;
                        if px.is_finite()
                            && py.is_finite()
                            && pz.is_finite()
                            && px.abs() < 1e6
                            && py.abs() < 1e6
                            && pz.abs() < 1e6
                        {
                            placements.push(StaticPlacement {
                                asset_path: path.to_string(),
                                position: [px, py, pz],
                            });
                            i = pos_off + 12;
                            continue;
                        }
                    }
                }
            }
        }
        i += 1;
    }
    Ok((header, placements))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn parses_synthetic_ecwld_and_ecbsd_placements() {
        let mut wld = Vec::new();
        wld.extend_from_slice(&ECWLD_MAGIC.to_le_bytes());
        wld.extend_from_slice(&14u32.to_le_bytes());
        wld.extend_from_slice(&1024.0f32.to_le_bytes());
        wld.extend_from_slice(&1024.0f32.to_le_bytes());
        wld.extend_from_slice(&64.0f32.to_le_bytes());
        wld.extend_from_slice(&32u32.to_le_bytes());
        wld.extend_from_slice(&1u32.to_le_bytes()); // 1 block
        wld.extend_from_slice(&1u32.to_le_bytes());
        wld.extend_from_slice(&1u32.to_le_bytes());
        wld.resize(ECWLD_HEADER_BYTES, 0);
        let blk_off = (ECWLD_HEADER_BYTES + 4) as u32;
        wld.extend_from_slice(&blk_off.to_le_bytes());
        // ECWDFILEBLOCK9 (44 bytes): 0 trees, 0 water, 0 grass, 2 ornament refs (1 duplicate export_id)
        wld.extend_from_slice(&0u16.to_le_bytes());
        wld.extend_from_slice(&0u16.to_le_bytes());
        for count in [0i32, 0, 0, 2, 0, 0, 0, 0, 0, 0] {
            wld.extend_from_slice(&count.to_le_bytes());
        }
        // Two refs pointing to same export_id 42 at bsd_off=16
        wld.extend_from_slice(&42u32.to_le_bytes());
        wld.extend_from_slice(&16u32.to_le_bytes());
        wld.extend_from_slice(&42u32.to_le_bytes());
        wld.extend_from_slice(&16u32.to_le_bytes());

        let mut bsd = Vec::new();
        bsd.extend_from_slice(&ECBSD_MAGIC0.to_le_bytes());
        bsd.extend_from_slice(&ECBSD_MAGIC1.to_le_bytes());
        bsd.extend_from_slice(&15u32.to_le_bytes());
        bsd.extend_from_slice(&1u32.to_le_bytes());
        // ECBSDFILEORNAMENT13 at offset 16
        for v in [-220.1f32, 83.9, 221.1, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            bsd.extend_from_slice(&v.to_le_bytes());
        }
        bsd.extend_from_slice(&[1, 0, 1, 0]); // reflect, refract, optimize, pad
        let path = b"1\\litmodel_37.lmd";
        bsd.extend_from_slice(&(path.len() as u32).to_le_bytes());
        bsd.extend_from_slice(path);

        let (wh, bh, items) = parse_ecwld_ecbsd_ornaments(&wld, &bsd).expect("parse ornaments");
        assert_eq!(wh.version, 14);
        assert_eq!(bh.version, 15);
        assert_eq!(items.len(), 1, "duplicate export_id must be deduplicated");
        assert_eq!(items[0].normalized_bmd_path(), "1/litmodel_37.bmd");
        assert!(items[0].reflect && !items[0].refract && items[0].optimize);

        let mut hasher = Sha256::new();
        hasher.update(items[0].normalized_bmd_path().as_bytes());
        for c in items[0].position {
            hasher.update(c.to_le_bytes());
        }
        assert_eq!(
            format!("{:x}", hasher.finalize()),
            "a758335632353666eefdf0ee812f9679b55962040655b71c360ea2837791233f"
        );
    }

    #[test]
    fn placement_transform_applies_position_rotation_and_scale() {
        // Facing +X (90 deg yaw), up +Y, scale (2, 3, 4), pos (10, 20, 30)
        let m = placement_transform_matrix(
            [10.0, 20.0, 30.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [2.0, 3.0, 4.0],
        );
        // Local +Z maps to world +X scaled by 4 -> (14, 20, 30)
        let p_fwd = m.transform_point3(Vec3::Z);
        assert!((p_fwd - Vec3::new(14.0, 20.0, 30.0)).length() < 1e-4);
        // Local +Y maps to world +Y scaled by 3 -> (10, 23, 30)
        let p_up = m.transform_point3(Vec3::Y);
        assert!((p_up - Vec3::new(10.0, 23.0, 30.0)).length() < 1e-4);
        // Local +X = cross(up=(0,1,0), fwd=(1,0,0)) = (0,0,-1) scaled by 2 -> (10, 20, 28)
        let p_right = m.transform_point3(Vec3::X);
        assert!((p_right - Vec3::new(10.0, 20.0, 28.0)).length() < 1e-4);
    }

    #[test]
    fn rejects_invalid_world_headers() {
        assert!(EcwldHeader::parse(&[0u8; 48]).is_err());
        assert!(scan_ecbsd_bmd_placements(&[0u8; 32]).is_err());
        assert!(parse_ecwld_ecbsd_ornaments(&[0u8; 32], &[0u8; 32]).is_err());
    }
}
