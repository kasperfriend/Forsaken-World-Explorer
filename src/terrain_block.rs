//! CPU decoders for Forsaken World `.trn2`, `.t2bk` (v5 & v6), `.t2lm`/`.t2lm1`, and `.t2mk`.
//!
//! Verified against real `Forsaken World 0.525 Client` files in `maps/s0/`
//! (see `Findings/world-formats.md`).

use flate2::{Decompress, FlushDecompress, Status};

pub const TRN2_MAGIC: u32 = 0x5452_4e32; // "2NRT" LE
pub const T2BK_MAGIC: u32 = 0x5442_4b42; // "BKBT" LE
pub const T2LM_MAGIC: u32 = 0x5432_4c4d; // "ML2T" LE
pub const T2MK_MAGIC: u32 = 0x544d_4b42; // "BKMT" LE

pub const FLAG_VERTEX_COLOUR: u8 = 0x01;
pub const FLAG_COMPRESSED: u8 = 0x04;
pub const FLAG_COMPRESSED_NORMALS: u8 = 0x10;
pub const FLAG_POLAR_NORMALS: u8 = 0x20;

fn i16le(b: &[u8], off: usize) -> Result<i16, String> {
    let s = b
        .get(off..off + 2)
        .ok_or_else(|| format!("truncated i16 at {off}"))?;
    Ok(i16::from_le_bytes(s.try_into().unwrap()))
}

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

fn inflate_exact(raw: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    let mut decomp = Decompress::new(true);
    let mut out = Vec::with_capacity(expected.saturating_add(1));
    let status = decomp
        .decompress_vec(raw, &mut out, FlushDecompress::Finish)
        .map_err(|e| format!("terrain zlib decode failed: {e}"))?;
    if out.len() != expected {
        return Err(format!(
            "terrain zlib size mismatch: got {}, expected {expected}",
            out.len()
        ));
    }
    if status != Status::StreamEnd {
        return Err("truncated terrain zlib stream".into());
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Trn2Header {
    pub version: u32,
    pub width: f32,
    pub length: f32,
    pub block_size: f32,
    pub block_grid: u32,
    pub num_blocks: u32,
    pub block_rows: u32,
    pub block_cols: u32,
    pub mask_areas: u32,
    pub mask_size: u32,
    pub sub_terrain_count: u32,
    pub texture_paths: Vec<String>,
}

impl Trn2Header {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        if data.len() < 56 {
            return Err("trn2 file too small".into());
        }
        if u32le(data, 0)? != TRN2_MAGIC {
            return Err("invalid TRN2 magic".into());
        }
        let version = u32le(data, 4)?;
        let width = f32le(data, 8)?;
        let length = f32le(data, 12)?;
        let block_size = f32le(data, 16)?;
        let block_grid = u32le(data, 20)?;
        let num_blocks = u32le(data, 24)?;
        let block_rows = u32le(data, 28)?;
        let block_cols = u32le(data, 32)?;
        let num_textures = u32le(data, 36)? as usize;
        let mask_areas = u32le(data, 40)?;
        let mask_size = u32le(data, 48)?;
        let sub_terrain_count = u32le(data, 52)?;
        let mut texture_paths = Vec::new();
        if data.len() >= 76 && num_textures <= 256 {
            let tex_off = u32le(data, 72)? as usize;
            if tex_off >= 76 && tex_off < data.len() {
                let mut cur = tex_off;
                for _ in 0..num_textures {
                    if cur + 4 > data.len() {
                        break;
                    }
                    let slen = u32le(data, cur)? as usize;
                    cur += 4;
                    if slen == 0 || slen > 512 || cur + slen > data.len() {
                        break;
                    }
                    let (decoded, _, _) = encoding_rs::GB18030.decode(&data[cur..cur + slen]);
                    texture_paths.push(decoded.into_owned());
                    cur += slen;
                }
            }
        }
        if block_grid == 0 || block_grid > 128 {
            return Err(format!("unsupported trn2 block_grid {block_grid}"));
        }
        if !width.is_finite() || !length.is_finite() || !block_size.is_finite() {
            return Err("non-finite trn2 dimensions".into());
        }
        Ok(Self {
            version,
            width,
            length,
            block_size,
            block_grid,
            num_blocks,
            block_rows,
            block_cols,
            mask_areas,
            mask_size,
            sub_terrain_count,
            texture_paths,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedBlock {
    pub row: i16,
    pub col: i16,
    pub mask_area: i16,
    pub lod_scale: f32,
    pub grid: u32,
    pub heights: Vec<f32>,
    /// Quantized unit normals `[nx, ny, nz]` in `[-127, 127]`.
    pub normals: Vec<[i8; 3]>,
    pub day_colors: Option<Vec<[u8; 4]>>,
    pub night_colors: Option<Vec<[u8; 4]>>,
}

#[derive(Debug, Clone)]
pub struct T2bkFile {
    pub version: u32,
    pub flags: u8,
    pub offsets: Vec<u32>,
}

impl T2bkFile {
    pub fn parse_header(data: &[u8]) -> Result<Self, String> {
        if data.len() < 16 {
            return Err("t2bk file too small".into());
        }
        if u32le(data, 0)? != T2BK_MAGIC {
            return Err("invalid T2BK magic".into());
        }
        let version = u32le(data, 4)?;
        if !(5..=6).contains(&version) {
            return Err(format!("unsupported T2BK version {version}"));
        }
        let num_blocks = u32le(data, 8)? as usize;
        if num_blocks == 0 || num_blocks > 65_536 {
            return Err(format!("invalid T2BK block count {num_blocks}"));
        }
        let flags = (u32le(data, 12)? & 0xff) as u8;
        let table_end = 16usize
            .checked_add(num_blocks * 4)
            .ok_or_else(|| "t2bk offset table overflow".to_string())?;
        if table_end > data.len() {
            return Err("t2bk offset table truncated".into());
        }
        let mut offsets = Vec::with_capacity(num_blocks);
        for i in 0..num_blocks {
            let off = u32le(data, 16 + i * 4)?;
            if (off as usize) < table_end || (off as usize) >= data.len() {
                return Err(format!("t2bk block {i} offset {off} out of range"));
            }
            offsets.push(off);
        }
        Ok(Self {
            version,
            flags,
            offsets,
        })
    }

    pub fn decode_block(
        &self,
        data: &[u8],
        index: usize,
        grid: u32,
    ) -> Result<DecodedBlock, String> {
        let &off_u32 = self
            .offsets
            .get(index)
            .ok_or_else(|| format!("block index {index} out of bounds"))?;
        let off = off_u32 as usize;
        let header_len = if self.version >= 6 { 36 } else { 32 };
        if off + header_len > data.len() {
            return Err(format!("t2bk block {index} header truncated"));
        }

        let mask_area = i16le(data, off)?;
        let row = i16le(data, off + 4)?;
        let col = i16le(data, off + 6)?;
        let lod_scale = f32le(data, off + 8)?;

        let (sz_hei, sz_nor, sz_diff, sz_diff1) = if self.version >= 6 {
            (
                u32le(data, off + 16)? as usize,
                u32le(data, off + 20)? as usize,
                u32le(data, off + 24)? as usize,
                u32le(data, off + 28)? as usize,
            )
        } else {
            (
                u32le(data, off + 16)? as usize,
                u32le(data, off + 20)? as usize,
                u32le(data, off + 24)? as usize,
                0usize,
            )
        };

        let verts = ((grid as usize) + 1) * ((grid as usize) + 1);
        let hei_start = off + header_len;
        let nor_start = hei_start
            .checked_add(sz_hei)
            .ok_or_else(|| "height section overflow".to_string())?;
        let diff_start = nor_start
            .checked_add(sz_nor)
            .ok_or_else(|| "normal section overflow".to_string())?;
        let diff1_start = diff_start
            .checked_add(sz_diff)
            .ok_or_else(|| "day colour section overflow".to_string())?;
        let block_end = diff1_start
            .checked_add(sz_diff1)
            .ok_or_else(|| "night colour section overflow".to_string())?;
        if block_end > data.len() {
            return Err(format!("t2bk block {index} extends past EOF"));
        }

        let hei_bytes = if self.flags & FLAG_COMPRESSED != 0 {
            inflate_exact(&data[hei_start..nor_start], verts * 4)?
        } else {
            if sz_hei != verts * 4 {
                return Err("uncompressed height size mismatch".into());
            }
            data[hei_start..nor_start].to_vec()
        };
        let mut heights = Vec::with_capacity(verts);
        for &chunk in hei_bytes.as_chunks::<4>().0 {
            let h = f32::from_le_bytes(chunk);
            if !h.is_finite() {
                return Err(format!("non-finite height in block {index}"));
            }
            heights.push(h);
        }

        let normals = if self.flags & FLAG_POLAR_NORMALS != 0 {
            let expected_nor = 8 + verts * 2;
            let nor_bytes = if self.flags & FLAG_COMPRESSED_NORMALS != 0 {
                inflate_exact(&data[nor_start..diff_start], expected_nor)?
            } else {
                if sz_nor != expected_nor {
                    return Err("uncompressed polar normal size mismatch".into());
                }
                data[nor_start..diff_start].to_vec()
            };
            // Verified byte-for-byte on real maps/s0/s0_1.t2bk and maps/login/login_1.t2bk:
            // offset 0 is a 1-byte u8 version/flag (0x01), offsets 1..4 are 3 bytes of
            // uninitialized MSVC struct padding (0x00,0x00,0x00 in s0_1.t2bk; 0x1a,0x12,0x4c
            // = 0x4c121a01 in login_1.t2bk), and offsets 4..8 are u32le vertex count.
            let nor_ver = nor_bytes[0];
            let nor_raw_hdr = u32le(&nor_bytes, 0)?;
            let nor_cnt = u32le(&nor_bytes, 4)? as usize;
            if nor_ver != 1 || nor_cnt != verts {
                return Err(format!(
                    "block {index}: unexpected polar normal header (ver={nor_ver}, raw_hdr={nor_raw_hdr:#010x}, cnt={nor_cnt}, expected {verts})"
                ));
            }
            decode_polar_normals(&nor_bytes[8..])
        } else {
            let expected_nor = verts * 12;
            let nor_bytes = if self.flags & FLAG_COMPRESSED_NORMALS != 0 {
                inflate_exact(&data[nor_start..diff_start], expected_nor)?
            } else {
                if sz_nor != expected_nor {
                    return Err("uncompressed vector normal size mismatch".into());
                }
                data[nor_start..diff_start].to_vec()
            };
            nor_bytes
                .as_chunks::<12>()
                .0
                .iter()
                .map(|c| {
                    let x = f32::from_le_bytes(c[0..4].try_into().unwrap());
                    let y = f32::from_le_bytes(c[4..8].try_into().unwrap());
                    let z = f32::from_le_bytes(c[8..12].try_into().unwrap());
                    quantize_normal(x, y, z)
                })
                .collect()
        };

        let day_colors = if sz_diff > 0 && (self.flags & FLAG_VERTEX_COLOUR != 0) {
            let raw = inflate_exact(&data[diff_start..diff1_start], verts * 4)?;
            Some(raw.as_chunks::<4>().0.to_vec())
        } else {
            None
        };

        let night_colors = if sz_diff1 > 0 && (self.flags & FLAG_VERTEX_COLOUR != 0) {
            let raw = inflate_exact(&data[diff1_start..block_end], verts * 4)?;
            Some(raw.as_chunks::<4>().0.to_vec())
        } else {
            None
        };

        Ok(DecodedBlock {
            row,
            col,
            mask_area,
            lod_scale,
            grid,
            heights,
            normals,
            day_colors,
            night_colors,
        })
    }
}

fn quantize_normal(x: f32, y: f32, z: f32) -> [i8; 3] {
    let q = |v: f32| (v.clamp(-1.0, 1.0) * 127.0).round() as i8;
    [q(x), q(y), q(z)]
}

fn decode_polar_normals(bytes: &[u8]) -> Vec<[i8; 3]> {
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for &pair in bytes.as_chunks::<2>().0 {
        let packed = u16::from_le_bytes(pair);
        let pitch_idx = (packed & 0xff) as f32;
        let yaw_idx = ((packed >> 8) & 0xff) as f32;
        let theta = (pitch_idx / 255.0) * std::f32::consts::FRAC_PI_2;
        let phi = (yaw_idx / 256.0) * std::f32::consts::TAU;
        let sin_t = theta.sin();
        let nx = sin_t * phi.cos();
        let ny = theta.cos();
        let nz = sin_t * phi.sin();
        out.push(quantize_normal(nx, ny, nz));
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct T2lmHeader {
    pub version: u32,
    pub num_areas: u32,
    pub lm_size: u32,
    pub flags: u32,
    pub data_size: u32,
    pub offsets: Vec<u32>,
}

impl T2lmHeader {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        if data.len() < 24 {
            return Err("t2lm file too small".into());
        }
        if u32le(data, 0)? != T2LM_MAGIC {
            return Err("invalid T2LM magic".into());
        }
        let version = u32le(data, 4)?;
        if version != 2 {
            return Err(format!("unsupported T2LM version {version}"));
        }
        let num_areas = u32le(data, 8)?;
        let lm_size = u32le(data, 12)?;
        let flags = u32le(data, 16)?;
        let data_size = u32le(data, 20)?;
        let table_end = 24usize
            .checked_add((num_areas as usize) * 4)
            .ok_or_else(|| "t2lm offset table overflow".to_string())?;
        if table_end > data.len() {
            return Err("t2lm offset table truncated".into());
        }
        let mut offsets = Vec::with_capacity(num_areas as usize);
        for i in 0..num_areas as usize {
            let off = u32le(data, 24 + i * 4)?;
            let end = (off as usize)
                .checked_add(data_size as usize)
                .ok_or_else(|| "t2lm entry overflow".to_string())?;
            if (off as usize) < table_end || end > data.len() {
                return Err(format!("t2lm area {i} out of bounds"));
            }
            offsets.push(off);
        }
        Ok(Self {
            version,
            num_areas,
            lm_size,
            flags,
            data_size,
            offsets,
        })
    }

    pub fn area_bytes<'a>(&self, data: &'a [u8], area: usize) -> Result<&'a [u8], String> {
        let &off = self
            .offsets
            .get(area)
            .ok_or_else(|| format!("t2lm area {area} out of range"))?;
        let start = off as usize;
        let end = start + self.data_size as usize;
        data.get(start..end)
            .ok_or_else(|| format!("t2lm area {area} truncated"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct T2mkHeader {
    pub version: u32,
    pub num_areas: u32,
    pub num_layers_field: u32,
    pub compressed: u32,
    pub offsets: Vec<u32>,
}

impl T2mkHeader {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        if data.len() < 20 {
            return Err("t2mk file too small".into());
        }
        if u32le(data, 0)? != T2MK_MAGIC {
            return Err("invalid T2MK magic".into());
        }
        let version = u32le(data, 4)?;
        if version != 2 {
            return Err(format!("unsupported T2MK version {version}"));
        }
        let num_areas = u32le(data, 8)?;
        let num_layers_field = u32le(data, 12)?;
        let compressed = u32le(data, 16)?;
        let table_end = 20usize
            .checked_add((num_areas as usize) * 4)
            .ok_or_else(|| "t2mk offset table overflow".to_string())?;
        if table_end > data.len() {
            return Err("t2mk offset table truncated".into());
        }
        let mut offsets = Vec::with_capacity(num_areas as usize);
        for i in 0..num_areas as usize {
            let off = u32le(data, 20 + i * 4)?;
            if (off as usize) < table_end || (off as usize) >= data.len() {
                return Err(format!("t2mk area {i} offset {off} out of range"));
            }
            offsets.push(off);
        }
        Ok(Self {
            version,
            num_areas,
            num_layers_field,
            compressed,
            offsets,
        })
    }
}

/// Synthetic terrain fixture builders for deterministic CPU unit and golden-hash tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct T2mkLayerRef {
    pub layer_index: u32,
    pub data_offset: u32,
    pub data_len: u32,
}

impl T2mkHeader {
    /// Parse the layer list for one mask area (`0..num_areas`).
    pub fn area_layers(&self, data: &[u8], area: usize) -> Result<Vec<T2mkLayerRef>, String> {
        let &off_u32 = self
            .offsets
            .get(area)
            .ok_or_else(|| format!("t2mk area {area} out of range"))?;
        let off = off_u32 as usize;
        if off + 4 > data.len() {
            return Err(format!("t2mk area {area} header truncated"));
        }
        let count = u32le(data, off)? as usize;
        if count > 64 {
            return Err(format!(
                "t2mk area {area} layer count {count} exceeds limit"
            ));
        }
        let table_end = off + 4 + count * 12;
        if table_end > data.len() {
            return Err(format!("t2mk area {area} layer table truncated"));
        }
        let mut layers = Vec::with_capacity(count);
        for i in 0..count {
            let base = off + 4 + i * 12;
            layers.push(T2mkLayerRef {
                layer_index: u32le(data, base)?,
                data_offset: u32le(data, base + 4)?,
                data_len: u32le(data, base + 8)?,
            });
        }
        Ok(layers)
    }

    /// Decode an 8-bit alpha mask for one layer. Returns `(side_px, alpha_bytes)`.
    /// Base layers (`data_len == 0`) return `(1, vec![255])`.
    pub fn decode_layer_mask(
        &self,
        data: &[u8],
        layer: &T2mkLayerRef,
    ) -> Result<(u32, Vec<u8>), String> {
        if layer.data_len == 0 {
            return Ok((1, vec![255]));
        }
        let start = layer.data_offset as usize;
        let end = start
            .checked_add(layer.data_len as usize)
            .ok_or_else(|| "t2mk layer offset overflow".to_string())?;
        let slice = data
            .get(start..end)
            .ok_or_else(|| format!("t2mk layer payload {start}..{end} out of bounds"))?;
        let raw = if self.compressed != 0 {
            let mut decomp = Decompress::new(true);
            let mut out = Vec::with_capacity(256 * 256);
            let mut in_pos = 0usize;
            let mut buf = [0u8; 8192];
            loop {
                let before_in = decomp.total_in();
                let before_out = decomp.total_out();
                let status = decomp
                    .decompress(&slice[in_pos..], &mut buf, FlushDecompress::None)
                    .map_err(|e| format!("t2mk zlib decode failed: {e}"))?;
                in_pos += (decomp.total_in() - before_in) as usize;
                let produced = (decomp.total_out() - before_out) as usize;
                out.extend_from_slice(&buf[..produced]);
                if out.len() > 1024 * 1024 {
                    return Err("t2mk mask exceeds 1 MiB safety limit".into());
                }
                if status == Status::StreamEnd {
                    break;
                }
                if in_pos >= slice.len() && produced == 0 {
                    break;
                }
            }
            out
        } else {
            slice.to_vec()
        };
        let side = (raw.len() as f64).sqrt().round() as u32;
        if (side as usize) * (side as usize) != raw.len() || side == 0 {
            return Err(format!(
                "t2mk decoded mask length {} is not a square",
                raw.len()
            ));
        }
        Ok((side, raw))
    }
}

pub mod synthetic {
    use super::*;
    use flate2::{write::ZlibEncoder, Compression};
    use std::io::Write;

    fn compress(bytes: &[u8]) -> Vec<u8> {
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(bytes).expect("zlib write");
        enc.finish().expect("zlib finish")
    }

    pub fn build_t2bk_v6(grid: u32, num_blocks: usize) -> Vec<u8> {
        let verts = ((grid as usize) + 1) * ((grid as usize) + 1);
        let mut blocks_bytes = Vec::new();
        let mut offsets = Vec::with_capacity(num_blocks);
        let table_end = 16 + num_blocks * 4;

        for idx in 0..num_blocks {
            offsets.push((table_end + blocks_bytes.len()) as u32);
            let mut hei_raw = Vec::with_capacity(verts * 4);
            let mut nor_raw = Vec::with_capacity(8 + verts * 2);
            nor_raw.extend_from_slice(&1u32.to_le_bytes());
            nor_raw.extend_from_slice(&(verts as u32).to_le_bytes());
            let mut diff0_raw = Vec::with_capacity(verts * 4);
            let mut diff1_raw = Vec::with_capacity(verts * 4);

            for v in 0..verts {
                let h = 100.0 + (idx as f32) * 2.5 + ((v % 17) as f32) * 0.25;
                hei_raw.extend_from_slice(&h.to_le_bytes());
                let packed = (((v * 7) & 0xff) as u16) << 8 | ((v % 32) as u16);
                nor_raw.extend_from_slice(&packed.to_le_bytes());
                diff0_raw.extend_from_slice(&[200, 210, 220, 255]);
                diff1_raw.extend_from_slice(&[80, 90, 120, 255]);
            }

            let c_hei = compress(&hei_raw);
            let c_nor = compress(&nor_raw);
            let c_d0 = compress(&diff0_raw);
            let c_d1 = compress(&diff1_raw);

            let row = (idx / 4) as i16;
            let col = (idx % 4) as i16;
            blocks_bytes.extend_from_slice(&0i16.to_le_bytes());
            blocks_bytes.extend_from_slice(&0i16.to_le_bytes());
            blocks_bytes.extend_from_slice(&row.to_le_bytes());
            blocks_bytes.extend_from_slice(&col.to_le_bytes());
            blocks_bytes.extend_from_slice(&0.25f32.to_le_bytes());
            blocks_bytes.extend_from_slice(&1u32.to_le_bytes());
            blocks_bytes.extend_from_slice(&(c_hei.len() as u32).to_le_bytes());
            blocks_bytes.extend_from_slice(&(c_nor.len() as u32).to_le_bytes());
            blocks_bytes.extend_from_slice(&(c_d0.len() as u32).to_le_bytes());
            blocks_bytes.extend_from_slice(&(c_d1.len() as u32).to_le_bytes());
            blocks_bytes.extend_from_slice(&0u32.to_le_bytes());
            blocks_bytes.extend_from_slice(&c_hei);
            blocks_bytes.extend_from_slice(&c_nor);
            blocks_bytes.extend_from_slice(&c_d0);
            blocks_bytes.extend_from_slice(&c_d1);
        }

        let mut out = Vec::with_capacity(table_end + blocks_bytes.len());
        out.extend_from_slice(&T2BK_MAGIC.to_le_bytes());
        out.extend_from_slice(&6u32.to_le_bytes());
        out.extend_from_slice(&(num_blocks as u32).to_le_bytes());
        let flags =
            (FLAG_VERTEX_COLOUR | FLAG_COMPRESSED | FLAG_COMPRESSED_NORMALS | FLAG_POLAR_NORMALS)
                as u32;
        out.extend_from_slice(&flags.to_le_bytes());
        for off in offsets {
            out.extend_from_slice(&off.to_le_bytes());
        }
        out.extend_from_slice(&blocks_bytes);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn decodes_synthetic_v6_t2bk_and_matches_golden_hash() {
        let bytes = synthetic::build_t2bk_v6(8, 2);
        let t2bk = T2bkFile::parse_header(&bytes).expect("parse header");
        assert_eq!(t2bk.version, 6);
        assert_eq!(t2bk.offsets.len(), 2);

        let block0 = t2bk.decode_block(&bytes, 0, 8).expect("decode block 0");
        assert_eq!(block0.heights.len(), 81);
        assert_eq!(block0.normals.len(), 81);
        assert_eq!(block0.day_colors.as_ref().unwrap().len(), 81);
        assert_eq!(block0.night_colors.as_ref().unwrap().len(), 81);

        // Deterministic golden hash over decoded heights + normals
        let mut hasher = Sha256::new();
        for h in &block0.heights {
            hasher.update(h.to_le_bytes());
        }
        for n in &block0.normals {
            hasher.update([n[0] as u8, n[1] as u8, n[2] as u8]);
        }
        let digest = format!("{:x}", hasher.finalize());
        assert_eq!(
            digest,
            "87caef4c070d61934514ad6d21f1117aa78493632b37490ad61e907b942cc015"
        );
    }

    #[test]
    fn malformed_t2bk_returns_error_without_panicking() {
        assert!(T2bkFile::parse_header(&[]).is_err());
        assert!(T2bkFile::parse_header(&[0u8; 32]).is_err());
        let mut seed: u64 = 0x1234_5678_9abc_def0;
        for case in 0..16 {
            let mut buf = vec![0u8; 64 + case * 19];
            for b in &mut buf {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                *b = (seed >> 33) as u8;
            }
            assert!(T2bkFile::parse_header(&buf).is_err());
        }
    }

    #[test]
    fn polar_normal_header_with_real_login_1_t2bk_padding_bytes() {
        // Exact first 64 bytes of decompressed polar normal stream from real
        // maps/login/login_1.t2bk block 0 (ver=6, num_blocks=256, flags=0x35):
        // byte 0 = 0x01 (version), bytes 1..4 = 0x1a 0x12 0x4c (MSVC struct padding,
        // previously misread as u32le 1276254721), bytes 4..8 = 0x00000441 (1089 verts).
        const REAL_LOGIN_1_T2BK_BLK0_NOR_64: [u8; 64] = [
            0x01, 0x1a, 0x12, 0x4c, 0x41, 0x04, 0x00, 0x00, 0x00, 0x00, 0x40, 0x06, 0x40, 0x02,
            0x40, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00,
            0x40, 0x01, 0x40, 0x04, 0x40, 0x08, 0x40, 0x0c, 0x40, 0x11, 0x40, 0x13, 0x40, 0x15,
            0x40, 0x14, 0x40, 0x12, 0x40, 0x0f, 0x40, 0x0b,
        ];
        assert_eq!(REAL_LOGIN_1_T2BK_BLK0_NOR_64[0], 1);
        assert_eq!(
            u32le(&REAL_LOGIN_1_T2BK_BLK0_NOR_64, 0).unwrap(),
            1_276_254_721
        );
        assert_eq!(u32le(&REAL_LOGIN_1_T2BK_BLK0_NOR_64, 4).unwrap(), 1089);

        // Build a full 32x32-grid (1089-vertex) uncompressed polar normal block using
        // the exact 64-byte real header + payload prefix and verify decode_block succeeds.
        let grid = 32u32;
        let verts = 1089usize;
        let mut nor_raw = vec![0u8; 8 + verts * 2];
        nor_raw[..64].copy_from_slice(&REAL_LOGIN_1_T2BK_BLK0_NOR_64);
        let hei_raw = vec![0u8; verts * 4];

        let table_end = 16usize + 4;
        let mut out = Vec::new();
        out.extend_from_slice(&T2BK_MAGIC.to_le_bytes());
        out.extend_from_slice(&6u32.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&(FLAG_POLAR_NORMALS as u32).to_le_bytes());
        out.extend_from_slice(&(table_end as u32).to_le_bytes());
        // 36-byte block v6 header
        out.extend_from_slice(&0i16.to_le_bytes()); // mask_area
        out.extend_from_slice(&0i16.to_le_bytes()); // pad
        out.extend_from_slice(&0i16.to_le_bytes()); // row
        out.extend_from_slice(&0i16.to_le_bytes()); // col
        out.extend_from_slice(&0.25f32.to_le_bytes()); // lod_scale
        out.extend_from_slice(&1u32.to_le_bytes()); // flags
        out.extend_from_slice(&(hei_raw.len() as u32).to_le_bytes());
        out.extend_from_slice(&(nor_raw.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&hei_raw);
        out.extend_from_slice(&nor_raw);

        let t2bk = T2bkFile::parse_header(&out).expect("parse header");
        let blk = t2bk
            .decode_block(&out, 0, grid)
            .expect("decode block with login_1.t2bk padding bytes");
        assert_eq!(blk.heights.len(), verts);
        assert_eq!(blk.normals.len(), verts);
        // Verify first few decoded quantized normals are non-zero unit-ish vectors
        let n0 = blk.normals[0];
        let mag0 =
            ((n0[0] as f32).powi(2) + (n0[1] as f32).powi(2) + (n0[2] as f32).powi(2)).sqrt();
        assert!(
            mag0 > 100.0 && mag0 <= 130.0,
            "unexpected normal magnitude {mag0}"
        );
    }
}
