//! CPU decoders for Forsaken World `.ecwld` (v14) headers and `.ecbsd` (v15) static
//! `.bmd` placement scans.
//!
//! Verified against `maps/s0/s0.ecwld` and `maps/s0/s0.ecbsd` (see `Findings/world-formats.md`).

pub const ECWLD_MAGIC: u32 = 0x4543_5744; // "DWCE" LE
pub const ECBSD_MAGIC0: u32 = 0x4258_4f4d; // "MOXB" LE
pub const ECBSD_MAGIC1: u32 = 0x4253_4446; // "FDSB" LE

fn u32le(b: &[u8], off: usize) -> Result<u32, String> {
    let s = b
        .get(off..off + 4)
        .ok_or_else(|| format!("truncated u32 at {off}"))?;
    Ok(u32::from_le_bytes(s.try_into().unwrap()))
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

/// Extract length-prefixed `.bmd` placement records (`[len: u32][ascii path][pos: 3*f32]`)
/// from a validated `.ecbsd` buffer.
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
        wld.extend_from_slice(&256u32.to_le_bytes());
        wld.extend_from_slice(&16u32.to_le_bytes());
        wld.extend_from_slice(&16u32.to_le_bytes());
        wld.resize(48, 0);
        let parsed = EcwldHeader::parse(&wld).expect("parse ecwld");
        assert_eq!(parsed.version, 14);
        assert_eq!(parsed.num_blocks, 256);

        let mut bsd = Vec::new();
        bsd.extend_from_slice(&ECBSD_MAGIC0.to_le_bytes());
        bsd.extend_from_slice(&ECBSD_MAGIC1.to_le_bytes());
        bsd.extend_from_slice(&15u32.to_le_bytes());
        bsd.extend_from_slice(&1u32.to_le_bytes());
        let path = b"1\\litmodel_37.bmd";
        bsd.extend_from_slice(&(path.len() as u32).to_le_bytes());
        bsd.extend_from_slice(path);
        for v in [-220.1f32, 83.9, 221.1] {
            bsd.extend_from_slice(&v.to_le_bytes());
        }
        let (hdr, items) = scan_ecbsd_bmd_placements(&bsd).expect("scan ecbsd");
        assert_eq!(hdr.version, 15);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].asset_path, "1\\litmodel_37.bmd");

        let mut hasher = Sha256::new();
        hasher.update(items[0].asset_path.as_bytes());
        for c in items[0].position {
            hasher.update(c.to_le_bytes());
        }
        assert_eq!(
            format!("{:x}", hasher.finalize()),
            "a8a6365646e06131743089c51c1c76187d8d6de787aefa13708a3dd58690c8c8"
        );
    }

    #[test]
    fn rejects_invalid_world_headers() {
        assert!(EcwldHeader::parse(&[0u8; 48]).is_err());
        assert!(scan_ecbsd_bmd_placements(&[0u8; 32]).is_err());
    }
}
