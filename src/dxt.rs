//! DDS header parser and CPU BC1 (DXT1) / BC2 (DXT3) / BC3 (DXT5) block decoder.
//!
//! Verified against Forsaken World `maps/s0/s0_1.t2lm` (`DXT1`) and `resources/grasses.pck` (`DXT3`).

pub const DDS_MAGIC: &[u8; 4] = b"DDS ";
pub const DDS_HEADER_LEN: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DdsFormat {
    Dxt1,
    Dxt3,
    Dxt5,
    Bgra8,
    Rgba8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DdsSurface<'a> {
    pub width: u32,
    pub height: u32,
    pub mip_count: u32,
    pub format: DdsFormat,
    /// Top-level mip raw bytes (BC1/BC2/BC3 blocks or uncompressed pixels).
    pub top_mip: &'a [u8],
}

fn u32le(b: &[u8], off: usize) -> Result<u32, String> {
    let s = b
        .get(off..off + 4)
        .ok_or_else(|| format!("truncated DDS u32 at {off}"))?;
    Ok(u32::from_le_bytes(s.try_into().unwrap()))
}

pub fn parse_dds(data: &[u8]) -> Result<DdsSurface<'_>, String> {
    if data.len() < DDS_HEADER_LEN {
        return Err("DDS buffer smaller than 128-byte header".into());
    }
    if &data[0..4] != DDS_MAGIC {
        return Err("invalid DDS magic".into());
    }
    if u32le(data, 4)? != 124 {
        return Err("invalid DDS header size".into());
    }
    let height = u32le(data, 12)?;
    let width = u32le(data, 16)?;
    let mip_count = u32le(data, 28)?.max(1);
    if width == 0 || height == 0 || width > 16384 || height > 16384 {
        return Err(format!("invalid DDS dimensions {width}x{height}"));
    }

    let pf_flags = u32le(data, 80)?;
    let fourcc = &data[84..88];
    let rgb_bits = u32le(data, 88)?;
    let r_mask = u32le(data, 92)?;

    let format = if pf_flags & 0x4 != 0 {
        match fourcc {
            b"DXT1" => DdsFormat::Dxt1,
            b"DXT3" => DdsFormat::Dxt3,
            b"DXT5" => DdsFormat::Dxt5,
            other => return Err(format!("unsupported DDS FourCC {:?}", other)),
        }
    } else if rgb_bits == 32 {
        if r_mask == 0x00ff_0000 {
            DdsFormat::Bgra8
        } else {
            DdsFormat::Rgba8
        }
    } else {
        return Err(format!(
            "unsupported uncompressed DDS pixel format ({rgb_bits} bpp)"
        ));
    };

    let top_len = expected_mip_bytes(width, height, format)?;
    let top_mip = data
        .get(DDS_HEADER_LEN..DDS_HEADER_LEN + top_len)
        .ok_or_else(|| "DDS top mip truncated".to_string())?;

    Ok(DdsSurface {
        width,
        height,
        mip_count,
        format,
        top_mip,
    })
}

pub fn expected_mip_bytes(width: u32, height: u32, format: DdsFormat) -> Result<usize, String> {
    let w = width as usize;
    let h = height as usize;
    match format {
        DdsFormat::Dxt1 => {
            let bw = w.div_ceil(4);
            let bh = h.div_ceil(4);
            bw.checked_mul(bh)
                .and_then(|n| n.checked_mul(8))
                .ok_or_else(|| "DXT1 size overflow".into())
        }
        DdsFormat::Dxt3 | DdsFormat::Dxt5 => {
            let bw = w.div_ceil(4);
            let bh = h.div_ceil(4);
            bw.checked_mul(bh)
                .and_then(|n| n.checked_mul(16))
                .ok_or_else(|| "DXT3/5 size overflow".into())
        }
        DdsFormat::Bgra8 | DdsFormat::Rgba8 => w
            .checked_mul(h)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| "RGBA8 size overflow".into()),
    }
}

fn rgb565_to_rgb8(c: u16) -> [u8; 3] {
    let r = (((c >> 11) & 0x1f) as u32 * 255 + 15) / 31;
    let g = (((c >> 5) & 0x3f) as u32 * 255 + 31) / 63;
    let b = ((c & 0x1f) as u32 * 255 + 15) / 31;
    [r as u8, g as u8, b as u8]
}

fn decode_bc1_color_block(block: &[u8], four_color_always: bool) -> [[u8; 4]; 16] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let rgb0 = rgb565_to_rgb8(c0);
    let rgb1 = rgb565_to_rgb8(c1);
    let mut palette = [[0u8; 4]; 4];
    palette[0] = [rgb0[0], rgb0[1], rgb0[2], 255];
    palette[1] = [rgb1[0], rgb1[1], rgb1[2], 255];
    if four_color_always || c0 > c1 {
        for ch in 0..3 {
            palette[2][ch] = ((2 * rgb0[ch] as u16 + rgb1[ch] as u16) / 3) as u8;
            palette[3][ch] = ((rgb0[ch] as u16 + 2 * rgb1[ch] as u16) / 3) as u8;
        }
        palette[2][3] = 255;
        palette[3][3] = 255;
    } else {
        for ch in 0..3 {
            palette[2][ch] = ((rgb0[ch] as u16 + rgb1[ch] as u16) / 2) as u8;
        }
        palette[2][3] = 255;
        palette[3] = [0, 0, 0, 0];
    }

    let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    let mut pixels = [[0u8; 4]; 16];
    for (i, px) in pixels.iter_mut().enumerate() {
        let code = ((indices >> (i * 2)) & 0x3) as usize;
        *px = palette[code];
    }
    pixels
}

/// Decode the top mip of a parsed DDS surface to tightly-packed RGBA8 pixels.
pub fn decode_rgba8(surface: &DdsSurface<'_>) -> Vec<u8> {
    let w = surface.width as usize;
    let h = surface.height as usize;
    let mut out = vec![0u8; w * h * 4];
    match surface.format {
        DdsFormat::Rgba8 => {
            out.copy_from_slice(surface.top_mip);
        }
        DdsFormat::Bgra8 => {
            for (dst, src) in out
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(surface.top_mip.as_chunks::<4>().0.iter())
            {
                dst[0] = src[2];
                dst[1] = src[1];
                dst[2] = src[0];
                dst[3] = src[3];
            }
        }
        DdsFormat::Dxt1 | DdsFormat::Dxt3 | DdsFormat::Dxt5 => {
            let bw = w.div_ceil(4);
            let bh = h.div_ceil(4);
            let stride = if surface.format == DdsFormat::Dxt1 {
                8
            } else {
                16
            };
            for by in 0..bh {
                for bx in 0..bw {
                    let b = &surface.top_mip[(by * bw + bx) * stride..][..stride];
                    let mut pixels = match surface.format {
                        DdsFormat::Dxt1 => decode_bc1_color_block(b, false),
                        DdsFormat::Dxt3 => {
                            let mut px = decode_bc1_color_block(&b[8..16], true);
                            let alpha_bits = u64::from_le_bytes(b[0..8].try_into().unwrap());
                            for (i, p) in px.iter_mut().enumerate() {
                                let a4 = ((alpha_bits >> (i * 4)) & 0xf) as u8;
                                p[3] = (a4 << 4) | a4;
                            }
                            px
                        }
                        DdsFormat::Dxt5 => {
                            let mut px = decode_bc1_color_block(&b[8..16], true);
                            let a0 = b[0] as u16;
                            let a1 = b[1] as u16;
                            let mut lut = [0u8; 8];
                            lut[0] = a0 as u8;
                            lut[1] = a1 as u8;
                            if a0 > a1 {
                                for k in 1..=6 {
                                    lut[k + 1] = (((7 - k) as u16 * a0 + k as u16 * a1) / 7) as u8;
                                }
                            } else {
                                for k in 1..=4 {
                                    lut[k + 1] = (((5 - k) as u16 * a0 + k as u16 * a1) / 5) as u8;
                                }
                                lut[6] = 0;
                                lut[7] = 255;
                            }
                            let mut bits: u64 = 0;
                            for i in 0..6 {
                                bits |= (b[2 + i] as u64) << (i * 8);
                            }
                            for (i, p) in px.iter_mut().enumerate() {
                                p[3] = lut[((bits >> (i * 3)) & 0x7) as usize];
                            }
                            px
                        }
                        _ => unreachable!(),
                    };
                    for py in 0..4 {
                        let y = by * 4 + py;
                        if y >= h {
                            continue;
                        }
                        for px_x in 0..4 {
                            let x = bx * 4 + px_x;
                            if x >= w {
                                continue;
                            }
                            let dst = (y * w + x) * 4;
                            out[dst..dst + 4].copy_from_slice(&pixels[py * 4 + px_x]);
                            pixels[py * 4 + px_x] = [0; 4];
                        }
                    }
                }
            }
        }
    }
    out
}

pub mod synthetic {
    use super::*;

    pub fn build_dds(width: u32, height: u32, fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut hdr = vec![0u8; DDS_HEADER_LEN];
        hdr[0..4].copy_from_slice(DDS_MAGIC);
        hdr[4..8].copy_from_slice(&124u32.to_le_bytes());
        hdr[8..12].copy_from_slice(&0x1007u32.to_le_bytes());
        hdr[12..16].copy_from_slice(&height.to_le_bytes());
        hdr[16..20].copy_from_slice(&width.to_le_bytes());
        hdr[28..32].copy_from_slice(&1u32.to_le_bytes());
        hdr[76..80].copy_from_slice(&32u32.to_le_bytes());
        hdr[80..84].copy_from_slice(&0x4u32.to_le_bytes());
        hdr[84..88].copy_from_slice(fourcc);
        hdr.extend_from_slice(payload);
        hdr
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn decodes_synthetic_dxt1_and_dxt3_matching_golden_hash() {
        // 4x4 DXT1 block: pure red (0xf800) to pure blue (0x001f)
        let dxt1_block = [0x00, 0xf8, 0x1f, 0x00, 0xe4, 0xe4, 0xe4, 0xe4];
        let dds1 = synthetic::build_dds(4, 4, b"DXT1", &dxt1_block);
        let surf1 = parse_dds(&dds1).expect("parse DXT1");
        assert_eq!(surf1.format, DdsFormat::Dxt1);
        let rgba1 = decode_rgba8(&surf1);
        assert_eq!(rgba1.len(), 64);
        assert_eq!(&rgba1[0..4], &[255, 0, 0, 255]);
        assert_eq!(&rgba1[4..8], &[0, 0, 255, 255]);

        // 4x4 DXT3 block with half alpha (0x88) + same color block
        let mut dxt3_block = [0x88u8; 16];
        dxt3_block[8..16].copy_from_slice(&dxt1_block);
        let dds3 = synthetic::build_dds(4, 4, b"DXT3", &dxt3_block);
        let surf3 = parse_dds(&dds3).expect("parse DXT3");
        assert_eq!(surf3.format, DdsFormat::Dxt3);
        let rgba3 = decode_rgba8(&surf3);
        assert_eq!(rgba3[3], 0x88);

        let digest = format!("{:x}", Sha256::digest(&rgba3));
        assert_eq!(
            digest,
            "295d5ff8ac9a71aece6c10f6be2221a38b16798200d9272afc8d9d4387db7509"
        );
    }

    #[test]
    fn rejects_malformed_dds() {
        assert!(parse_dds(&[]).is_err());
        assert!(parse_dds(&[0u8; 128]).is_err());
        let bad = synthetic::build_dds(4, 4, b"DXT1", &[0u8; 4]);
        assert!(parse_dds(&bad).is_err());
    }
}
