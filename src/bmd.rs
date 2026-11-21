//! CPU decoder for Forsaken World static `.bmd` meshes (`CELBrushBuilding` + `A3DLitModel`
//! + `A3DLitMesh`).
//!
//! Malformed or truncated inputs always return a typed `BmdError` and never panic.
//! See `Findings/scenery-formats.md` for verified vs inferred field evidence.

use std::fmt;

pub const AFILE_BINARY_MAGIC: u32 = 0x4258_4f4d; // "MOXB" LE
pub const BRUSH_BUILDING_VERSION: u32 = 0x8000_0001;
pub const LITMODEL_VERSION_1: u32 = 0x1000_0001;
pub const LITMODEL_VERSION_2: u32 = 0x1000_0002;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BmdError {
    MissingMagic,
    UnsupportedModelVersion(u32),
    UnsupportedMeshVersion { version: u32, offset: usize },
    Truncated(&'static str),
    InvalidFloat(&'static str),
    InvalidString(&'static str),
    LimitExceeded(&'static str),
    IndexOutOfBounds { index: u16, vertex_count: usize },
}

impl fmt::Display for BmdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingMagic => write!(f, "missing MOXB binary AFile marker"),
            Self::UnsupportedModelVersion(v) => write!(f, "unsupported A3DLitModel version {v:#x}"),
            Self::UnsupportedMeshVersion { version, offset } => write!(
                f,
                "unsupported A3DLitMesh version {version:#x} at byte offset {offset}"
            ),
            Self::Truncated(ctx) => write!(f, "truncated BMD data ({ctx})"),
            Self::InvalidFloat(ctx) => write!(f, "non-finite float in BMD ({ctx})"),
            Self::InvalidString(ctx) => write!(f, "invalid GBK string in BMD ({ctx})"),
            Self::LimitExceeded(ctx) => write!(f, "BMD limit exceeded ({ctx})"),
            Self::IndexOutOfBounds {
                index,
                vertex_count,
            } => write!(
                f,
                "BMD vertex index {index} out of bounds for {vertex_count} vertices"
            ),
        }
    }
}

impl std::error::Error for BmdError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaMode {
    Opaque,
    AlphaTested,
    /// Translucent / blended submeshes are deferred to Phase 5 and never drawn in Phase 4.
    TransparentDeferred,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BmdVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub diffuse: u32,
    pub day_diffuse: u32,
    pub night_diffuse: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BmdMaterial {
    pub name: String,
    pub ambient: [f32; 4],
    pub diffuse: [f32; 4],
    pub emissive: [f32; 4],
    pub specular: [f32; 4],
    pub power: f32,
    pub two_sided: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BmdSubmesh {
    pub name: String,
    pub version: u32,
    pub texture: String,
    pub material: Option<BmdMaterial>,
    pub alpha_mode: AlphaMode,
    pub aabb_min: [f32; 3],
    pub aabb_max: [f32; 3],
    pub vertices: Vec<BmdVertex>,
    pub indices: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BmdModel {
    pub collide_only: bool,
    pub model_version: u32,
    pub scale: [f32; 3],
    pub direction: [f32; 3],
    pub up: [f32; 3],
    pub position: [f32; 3],
    pub submeshes: Vec<BmdSubmesh>,
    pub collision_hull_count: usize,
}

impl BmdModel {
    /// Approximate CPU memory footprint in bytes for cache budgeting.
    pub fn approx_bytes(&self) -> usize {
        let mut total = std::mem::size_of::<Self>();
        for m in &self.submeshes {
            total += std::mem::size_of::<BmdSubmesh>()
                + m.name.len()
                + m.texture.len()
                + m.vertices.len() * std::mem::size_of::<BmdVertex>()
                + m.indices.len() * std::mem::size_of::<u16>();
        }
        total
    }

    pub fn parse(data: &[u8]) -> Result<Self, BmdError> {
        if read_u32(data, 0, "magic")? != AFILE_BINARY_MAGIC {
            return Err(BmdError::MissingMagic);
        }
        let mut p = 4usize;
        let mut collide_only = false;
        let mut model_version = read_u32(data, p, "version")?;
        if model_version == BRUSH_BUILDING_VERSION {
            let flag = *data.get(p + 4).ok_or(BmdError::Truncated("collide_only"))?;
            collide_only = flag != 0;
            p += 5;
            model_version = read_u32(data, p, "model_version")?;
        }
        if model_version != LITMODEL_VERSION_1 && model_version != LITMODEL_VERSION_2 {
            return Err(BmdError::UnsupportedModelVersion(model_version));
        }
        p += 4;

        let scale = read_vec3(data, p, "scale")?;
        let direction = read_vec3(data, p + 12, "direction")?;
        let up = read_vec3(data, p + 24, "up")?;
        let position = read_vec3(data, p + 36, "position")?;
        p += 48;

        let mesh_count = read_u32(data, p, "mesh_count")? as usize;
        p += 4;
        if mesh_count > 4096 {
            return Err(BmdError::LimitExceeded("mesh_count > 4096"));
        }

        let mut submeshes = Vec::with_capacity(mesh_count);
        for _ in 0..mesh_count {
            let ver_offset = p;
            let ver = read_u32(data, p, "mesh_version")?;
            p += 4;
            match ver {
                0x1000_0002..=0x1000_0005 => {}
                other => {
                    return Err(BmdError::UnsupportedMeshVersion {
                        version: other,
                        offset: ver_offset,
                    })
                }
            }

            let name = read_fixed_gbk(data, p, 64, "mesh_name")?;
            let texture = read_fixed_gbk(data, p + 64, 256, "texture_name")?;
            p += 320;

            let num_verts = read_u32(data, p, "num_verts")? as usize;
            let num_faces = read_u32(data, p + 4, "num_faces")? as usize;
            p += 8;
            if num_verts > 500_000 || num_faces > 1_000_000 {
                return Err(BmdError::LimitExceeded("vertex or face count"));
            }

            let stride = if ver >= 0x1000_0004 { 24usize } else { 28usize };
            let has_light_arrays = ver >= 0x1000_0003;
            let mut needed = num_verts
                .checked_mul(stride)
                .and_then(|v| v.checked_add(num_faces.checked_mul(6)?))
                .and_then(|v| v.checked_add(num_verts.checked_mul(12)?))
                .and_then(|v| v.checked_add(48))
                .ok_or(BmdError::LimitExceeded("mesh byte size overflow"))?;
            if has_light_arrays {
                needed = needed
                    .checked_add(
                        num_verts
                            .checked_mul(8)
                            .ok_or(BmdError::LimitExceeded("light array overflow"))?,
                    )
                    .ok_or(BmdError::LimitExceeded("mesh byte size overflow"))?;
            }
            if p.checked_add(needed).is_none_or(|end| end > data.len()) {
                return Err(BmdError::Truncated("mesh payload"));
            }

            let mut vertices = Vec::with_capacity(num_verts);
            for i in 0..num_verts {
                let q = p + i * stride;
                vertices.push(BmdVertex {
                    position: read_vec3(data, q, "vertex_pos")?,
                    diffuse: read_u32(data, q + 12, "vertex_diffuse")?,
                    uv: [
                        read_f32(data, q + 16, "vertex_u")?,
                        read_f32(data, q + 20, "vertex_v")?,
                    ],
                    normal: [0.0, 1.0, 0.0],
                    day_diffuse: 0xffff_ffff,
                    night_diffuse: 0xffff_ffff,
                });
            }
            p += num_verts * stride;

            let mut indices = Vec::with_capacity(num_faces * 3);
            for _ in 0..num_faces * 3 {
                let slice = data
                    .get(p..p + 2)
                    .ok_or(BmdError::Truncated("index buffer"))?;
                let idx = u16::from_le_bytes([slice[0], slice[1]]);
                if idx as usize >= num_verts {
                    return Err(BmdError::IndexOutOfBounds {
                        index: idx,
                        vertex_count: num_verts,
                    });
                }
                indices.push(idx);
                p += 2;
            }

            for v in &mut vertices {
                v.normal = read_vec3(data, p, "vertex_normal")?;
                p += 12;
            }

            if has_light_arrays {
                for v in &mut vertices {
                    v.day_diffuse = read_u32(data, p, "day_diffuse")?;
                    p += 4;
                }
                for v in &mut vertices {
                    v.night_diffuse = read_u32(data, p, "night_diffuse")?;
                    p += 4;
                }
            }

            let _center = read_vec3(data, p, "aabb_center")?;
            let _extents = read_vec3(data, p + 12, "aabb_extents")?;
            let aabb_min = read_vec3(data, p + 24, "aabb_min")?;
            let aabb_max = read_vec3(data, p + 36, "aabb_max")?;
            p += 48;

            let material = if ver >= 0x1000_0005 {
                let tail = data
                    .get(p..)
                    .ok_or(BmdError::Truncated("material header"))?;
                let nul = tail
                    .iter()
                    .position(|&c| c == 0)
                    .ok_or(BmdError::Truncated("material name NUL"))?;
                if nul > 1024 {
                    return Err(BmdError::LimitExceeded("material name > 1024"));
                }
                let mat_name = read_fixed_gbk(data, p, nul, "material_name")?;
                p = p
                    .checked_add(nul + 1)
                    .ok_or(BmdError::LimitExceeded("material offset"))?;
                if p.checked_add(69).is_none_or(|e| e > data.len()) {
                    return Err(BmdError::Truncated("material body"));
                }
                let m = BmdMaterial {
                    name: mat_name,
                    ambient: read_vec4(data, p, "mat_ambient")?,
                    diffuse: read_vec4(data, p + 16, "mat_diffuse")?,
                    emissive: read_vec4(data, p + 32, "mat_emissive")?,
                    specular: read_vec4(data, p + 48, "mat_specular")?,
                    power: read_f32(data, p + 64, "mat_power")?,
                    two_sided: data[p + 68] != 0,
                };
                p += 69;
                Some(m)
            } else {
                None
            };

            let alpha_mode = classify_alpha_mode(&texture, material.as_ref());
            submeshes.push(BmdSubmesh {
                name,
                version: ver,
                texture,
                material,
                alpha_mode,
                aabb_min,
                aabb_max,
                vertices,
                indices,
            });
        }

        // Optional CELBrushBuilding collision trailer
        let mut collision_hull_count = 0usize;
        if p < data.len() {
            let hulls = read_u32(data, p, "hull_count")? as usize;
            p += 4;
            if hulls > 65_536 {
                return Err(BmdError::LimitExceeded("collision hull count"));
            }
            for _ in 0..hulls {
                let cnt = read_u32(data, p, "hull_mesh_list_count")? as usize;
                p += 4;
                let bytes = cnt
                    .checked_mul(4)
                    .ok_or(BmdError::LimitExceeded("hull list overflow"))?;
                p = p
                    .checked_add(bytes)
                    .ok_or(BmdError::LimitExceeded("hull list overflow"))?;
                if p > data.len() {
                    return Err(BmdError::Truncated("hull mesh list"));
                }
            }
            for _ in 0..hulls {
                // CCDBrush: AABB (48 B) + flags (4 B) + side_count (4 B) + side_count * 17 B
                if p.checked_add(56).is_none_or(|e| e > data.len()) {
                    return Err(BmdError::Truncated("collision brush header"));
                }
                let side_count = read_u32(data, p + 52, "brush_sides")? as usize;
                if side_count == 0 || side_count > 256 {
                    return Err(BmdError::LimitExceeded("collision brush side count"));
                }
                p = p
                    .checked_add(56 + side_count * 17)
                    .ok_or(BmdError::LimitExceeded("collision brush overflow"))?;
                if p > data.len() {
                    return Err(BmdError::Truncated("collision brush planes"));
                }
            }
            collision_hull_count = hulls;
        }

        Ok(Self {
            collide_only,
            model_version,
            scale,
            direction,
            up,
            position,
            submeshes,
            collision_hull_count,
        })
    }
}

fn classify_alpha_mode(texture: &str, material: Option<&BmdMaterial>) -> AlphaMode {
    if let Some(m) = material {
        let a = m.diffuse[3];
        if a > 0.01 && a < 0.99 {
            return AlphaMode::TransparentDeferred;
        }
        if m.two_sided {
            return AlphaMode::AlphaTested;
        }
    }
    let lower = texture.to_ascii_lowercase();
    if lower.contains("alpha") || lower.contains("leaf") || lower.contains("foliage") {
        AlphaMode::AlphaTested
    } else {
        AlphaMode::Opaque
    }
}

fn read_u32(b: &[u8], p: usize, ctx: &'static str) -> Result<u32, BmdError> {
    let s = b
        .get(p..p.checked_add(4).ok_or(BmdError::Truncated(ctx))?)
        .ok_or(BmdError::Truncated(ctx))?;
    Ok(u32::from_le_bytes(s.try_into().unwrap()))
}

fn read_f32(b: &[u8], p: usize, ctx: &'static str) -> Result<f32, BmdError> {
    let v = f32::from_bits(read_u32(b, p, ctx)?);
    if !v.is_finite() {
        return Err(BmdError::InvalidFloat(ctx));
    }
    Ok(v)
}

fn read_vec3(b: &[u8], p: usize, ctx: &'static str) -> Result<[f32; 3], BmdError> {
    Ok([
        read_f32(b, p, ctx)?,
        read_f32(b, p + 4, ctx)?,
        read_f32(b, p + 8, ctx)?,
    ])
}

fn read_vec4(b: &[u8], p: usize, ctx: &'static str) -> Result<[f32; 4], BmdError> {
    Ok([
        read_f32(b, p, ctx)?,
        read_f32(b, p + 4, ctx)?,
        read_f32(b, p + 8, ctx)?,
        read_f32(b, p + 12, ctx)?,
    ])
}

fn read_fixed_gbk(b: &[u8], p: usize, n: usize, ctx: &'static str) -> Result<String, BmdError> {
    let s = b
        .get(p..p.checked_add(n).ok_or(BmdError::Truncated(ctx))?)
        .ok_or(BmdError::Truncated(ctx))?;
    let end = s.iter().position(|&c| c == 0).unwrap_or(n);
    let (decoded, _, had_errors) = encoding_rs::GBK.decode(&s[..end]);
    if had_errors {
        return Err(BmdError::InvalidString(ctx));
    }
    Ok(decoded.replace('\\', "/"))
}

/// Synthetic `.bmd` builders matching the documented real binary layout for CPU tests,
/// `--self-test`, and the Mesa `lavapipe` offscreen smoke test.
pub mod synthetic {
    use super::*;

    fn push_fixed(out: &mut Vec<u8>, s: &str, len: usize) {
        let mut buf = vec![0u8; len];
        let bytes = s.as_bytes();
        let n = bytes.len().min(len);
        buf[..n].copy_from_slice(&bytes[..n]);
        out.extend_from_slice(&buf);
    }

    /// Build a valid `v5` `.bmd` building cube (opaque + optional alpha-tested roof submesh)
    /// with optional `0x80000001` brush header and material tails.
    pub fn build_bmd_v5_box(texture_name: &str, half_extent: f32, height: f32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&AFILE_BINARY_MAGIC.to_le_bytes());
        out.extend_from_slice(&BRUSH_BUILDING_VERSION.to_le_bytes());
        out.push(0u8); // collide_only = false
        out.extend_from_slice(&LITMODEL_VERSION_2.to_le_bytes());
        // scale, dir, up, pos
        for v in [
            1.0f32, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&1u32.to_le_bytes()); // 1 submesh

        // Submesh v5
        out.extend_from_slice(&0x1000_0005u32.to_le_bytes());
        push_fixed(&mut out, "tower_body", 64);
        push_fixed(&mut out, texture_name, 256);

        let e = half_extent;
        let h = height;
        let verts: [([f32; 3], [f32; 3], [f32; 2]); 8] = [
            ([-e, 0.0, -e], [-0.577, -0.577, -0.577], [0.0, 1.0]),
            ([e, 0.0, -e], [0.577, -0.577, -0.577], [1.0, 1.0]),
            ([e, h, -e], [0.577, 0.577, -0.577], [1.0, 0.0]),
            ([-e, h, -e], [-0.577, 0.577, -0.577], [0.0, 0.0]),
            ([-e, 0.0, e], [-0.577, -0.577, 0.577], [0.0, 1.0]),
            ([e, 0.0, e], [0.577, -0.577, 0.577], [1.0, 1.0]),
            ([e, h, e], [0.577, 0.577, 0.577], [1.0, 0.0]),
            ([-e, h, e], [-0.577, 0.577, 0.577], [0.0, 0.0]),
        ];
        let indices: [u16; 36] = [
            0, 1, 2, 0, 2, 3, // -Z
            4, 6, 5, 4, 7, 6, // +Z
            0, 4, 5, 0, 5, 1, // -Y
            3, 2, 6, 3, 6, 7, // +Y
            0, 3, 7, 0, 7, 4, // -X
            1, 5, 6, 1, 6, 2, // +X
        ];

        out.extend_from_slice(&(verts.len() as u32).to_le_bytes());
        out.extend_from_slice(&((indices.len() / 3) as u32).to_le_bytes());

        // 24-byte vertex records
        for (pos, _, uv) in &verts {
            for c in pos {
                out.extend_from_slice(&c.to_le_bytes());
            }
            out.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
            out.extend_from_slice(&uv[0].to_le_bytes());
            out.extend_from_slice(&uv[1].to_le_bytes());
        }
        // indices
        for idx in &indices {
            out.extend_from_slice(&idx.to_le_bytes());
        }
        // normals
        for (_, nrm, _) in &verts {
            for c in nrm {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        // day + night vertex light arrays
        for _ in 0..verts.len() {
            out.extend_from_slice(&0xffe8_d8c8u32.to_le_bytes());
        }
        for _ in 0..verts.len() {
            out.extend_from_slice(&0xff60_7090u32.to_le_bytes());
        }
        // AABB (center, extents, mins, maxs)
        for v in [0.0f32, h * 0.5, 0.0, e, h * 0.5, e, -e, 0.0, -e, e, h, e] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        // v5 material tail
        out.extend_from_slice(b"stone_wall\0");
        for v in [
            0.2f32, 0.2, 0.2, 1.0, // ambient
            0.9, 0.85, 0.8, 1.0, // diffuse (opaque)
            0.0, 0.0, 0.0, 1.0, // emissive
            0.1, 0.1, 0.1, 1.0,  // specular
            16.0, // power
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.push(0u8); // two_sided = false
        out
    }

    /// Build a valid `v4` `.bmd` with two submeshes: one opaque rock and one alpha-tested banner
    /// (plus a third translucent submesh to verify `AlphaMode::TransparentDeferred` classification).
    pub fn build_bmd_v4_multi_submesh() -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&AFILE_BINARY_MAGIC.to_le_bytes());
        out.extend_from_slice(&LITMODEL_VERSION_1.to_le_bytes());
        for v in [
            1.5f32, 1.5, 1.5, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&2u32.to_le_bytes()); // 2 submeshes

        let specs = [
            ("rock_base", "textures/rock_01.dds", 0x1000_0004u32, false),
            (
                "foliage_trim",
                "textures/leaf_alpha.dds",
                0x1000_0005u32,
                true,
            ),
        ];
        for (name, tex, ver, with_mat) in specs {
            out.extend_from_slice(&ver.to_le_bytes());
            push_fixed(&mut out, name, 64);
            push_fixed(&mut out, tex, 256);
            out.extend_from_slice(&3u32.to_le_bytes()); // 3 verts
            out.extend_from_slice(&1u32.to_le_bytes()); // 1 face
            for (pos, uv) in [
                ([0.0f32, 0.0, 0.0], [0.0f32, 0.0]),
                ([4.0, 0.0, 0.0], [1.0, 0.0]),
                ([2.0, 4.0, 0.0], [0.5, 1.0]),
            ] {
                for c in pos {
                    out.extend_from_slice(&c.to_le_bytes());
                }
                out.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
                out.extend_from_slice(&uv[0].to_le_bytes());
                out.extend_from_slice(&uv[1].to_le_bytes());
            }
            for idx in [0u16, 1, 2] {
                out.extend_from_slice(&idx.to_le_bytes());
            }
            for _ in 0..3 {
                for c in [0.0f32, 0.0, 1.0] {
                    out.extend_from_slice(&c.to_le_bytes());
                }
            }
            for _ in 0..6 {
                out.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
            }
            for v in [
                2.0f32, 2.0, 0.0, 2.0, 2.0, 0.1, 0.0, 0.0, 0.0, 4.0, 4.0, 0.0,
            ] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            if with_mat {
                out.extend_from_slice(b"glass_window\0");
                for v in [
                    0.1f32, 0.1, 0.1, 1.0, // ambient
                    0.8, 0.9, 1.0, 0.45, // diffuse alpha 0.45 -> TransparentDeferred
                    0.0, 0.0, 0.0, 1.0, // emissive
                    0.5, 0.5, 0.5, 1.0, // specular
                    32.0,
                ] {
                    out.extend_from_slice(&v.to_le_bytes());
                }
                out.push(1u8);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn hash_model(m: &BmdModel) -> String {
        let mut h = Sha256::new();
        h.update(m.model_version.to_le_bytes());
        h.update((m.submeshes.len() as u32).to_le_bytes());
        for sm in &m.submeshes {
            h.update(sm.name.as_bytes());
            h.update(sm.texture.as_bytes());
            for v in &sm.vertices {
                for c in v.position {
                    h.update(c.to_le_bytes());
                }
                for c in v.normal {
                    h.update(c.to_le_bytes());
                }
                for c in v.uv {
                    h.update(c.to_le_bytes());
                }
                h.update(v.day_diffuse.to_le_bytes());
            }
            for idx in &sm.indices {
                h.update(idx.to_le_bytes());
            }
        }
        format!("{:x}", h.finalize())
    }

    #[test]
    fn golden_hash_synthetic_v5_and_v4_bmd_fixtures() {
        let raw_v5 = synthetic::build_bmd_v5_box("textures/stone_tower.dds", 6.0, 18.0);
        let m5 = BmdModel::parse(&raw_v5).expect("parse v5 box");
        assert!(!m5.collide_only);
        assert_eq!(m5.submeshes.len(), 1);
        assert_eq!(m5.submeshes[0].alpha_mode, AlphaMode::Opaque);
        assert_eq!(m5.submeshes[0].vertices.len(), 8);
        assert_eq!(m5.submeshes[0].indices.len(), 36);
        assert_eq!(
            hash_model(&m5),
            "56ed853a7fd86fb6d73ab365b369f11fa94bf3f5b2ae7771ad9c19904f665405"
        );

        let raw_v4 = synthetic::build_bmd_v4_multi_submesh();
        let m4 = BmdModel::parse(&raw_v4).expect("parse v4/v5 multi-submesh");
        assert_eq!(m4.submeshes.len(), 2);
        assert_eq!(m4.submeshes[0].alpha_mode, AlphaMode::Opaque);
        assert_eq!(m4.submeshes[1].alpha_mode, AlphaMode::TransparentDeferred);
        assert_eq!(
            hash_model(&m4),
            "9205bb2ab5c89f258bf178c999b19e885e8f50d7421baad4e72b1f480306e2c5"
        );
    }

    #[test]
    fn malformed_and_fuzz_inputs_return_typed_errors_without_panicking() {
        assert_eq!(BmdModel::parse(&[]), Err(BmdError::Truncated("magic")));
        assert_eq!(BmdModel::parse(&[0u8; 16]), Err(BmdError::MissingMagic));

        // Truncate valid fixture at every byte boundary
        let valid = synthetic::build_bmd_v5_box("textures/a.dds", 2.0, 4.0);
        for cut in 0..valid.len() {
            assert!(
                BmdModel::parse(&valid[..cut]).is_err(),
                "truncated at {cut} must fail cleanly"
            );
        }

        // Deterministic PRNG fuzzing (512 pseudo-random buffers + magic-prefixed buffers)
        let mut seed = 0x5eed_cafe_1234_5678u64;
        let mut next_u8 = || -> u8 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 56) as u8
        };
        for i in 0..512usize {
            let len = (i * 19) % 4096;
            let mut buf: Vec<u8> = (0..len).map(|_| next_u8()).collect();
            if i % 2 == 0 && buf.len() >= 8 {
                buf[0..4].copy_from_slice(&AFILE_BINARY_MAGIC.to_le_bytes());
                buf[4..8].copy_from_slice(&LITMODEL_VERSION_2.to_le_bytes());
            }
            let _ = BmdModel::parse(&buf);
        }
    }
}

#[cfg(test)]
mod version_matrix_tests {
    use super::*;

    fn build_minimal_bmd_for_version(
        with_brush_header: bool,
        model_version: u32,
        mesh_version: u32,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&AFILE_BINARY_MAGIC.to_le_bytes());
        if with_brush_header {
            out.extend_from_slice(&BRUSH_BUILDING_VERSION.to_le_bytes());
            out.push(0u8);
        }
        out.extend_from_slice(&model_version.to_le_bytes());
        for v in [
            1.0f32, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&1u32.to_le_bytes()); // 1 submesh
        out.extend_from_slice(&mesh_version.to_le_bytes());
        out.extend_from_slice(&[0u8; 320]); // name + texture
        out.extend_from_slice(&3u32.to_le_bytes()); // 3 verts
        out.extend_from_slice(&1u32.to_le_bytes()); // 1 face
        let stride = if mesh_version >= 0x1000_0004 { 24 } else { 28 };
        for i in 0..3usize {
            let mut rec = vec![0u8; stride];
            rec[0..4].copy_from_slice(&(i as f32).to_le_bytes());
            rec[12..16].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
            out.extend_from_slice(&rec);
        }
        for idx in [0u16, 1, 2] {
            out.extend_from_slice(&idx.to_le_bytes());
        }
        for _ in 0..3 {
            for c in [0.0f32, 1.0, 0.0] {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        if mesh_version >= 0x1000_0003 {
            for _ in 0..6 {
                out.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
            }
        }
        out.extend_from_slice(&[0u8; 48]); // AABB
        if mesh_version >= 0x1000_0005 {
            out.extend_from_slice(b"mat\0");
            for v in [
                0.2f32, 0.2, 0.2, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0,
                8.0,
            ] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.push(0u8);
        }
        out
    }

    #[test]
    fn reads_every_supported_bmd_header_and_mesh_version() {
        for with_brush in [false, true] {
            for model_ver in [LITMODEL_VERSION_1, LITMODEL_VERSION_2] {
                for mesh_ver in [0x1000_0002u32, 0x1000_0003, 0x1000_0004, 0x1000_0005] {
                    let raw = build_minimal_bmd_for_version(with_brush, model_ver, mesh_ver);
                    let parsed = BmdModel::parse(&raw).unwrap_or_else(|e| {
                        panic!("failed for brush={with_brush} model={model_ver:#x} mesh={mesh_ver:#x}: {e}")
                    });
                    assert_eq!(parsed.model_version, model_ver);
                    assert_eq!(parsed.submeshes.len(), 1);
                    assert_eq!(parsed.submeshes[0].version, mesh_ver);
                    assert_eq!(
                        parsed.submeshes[0].material.is_some(),
                        mesh_ver >= 0x1000_0005
                    );
                }
            }
        }
        // Unsupported mesh version must fail cleanly with UnsupportedMeshVersion
        let bad = build_minimal_bmd_for_version(false, LITMODEL_VERSION_2, 0x1000_0001);
        assert_eq!(
            BmdModel::parse(&bad),
            Err(BmdError::UnsupportedMeshVersion {
                version: 0x1000_0001,
                offset: 60
            })
        );
        // Observed real-client version 0x10000006 must fail cleanly with exact byte offset
        // until verified against real sample_litmodel_1000.bmd bytes.
        let v6 = build_minimal_bmd_for_version(false, LITMODEL_VERSION_2, 0x1000_0006);
        assert_eq!(
            BmdModel::parse(&v6),
            Err(BmdError::UnsupportedMeshVersion {
                version: 0x1000_0006,
                offset: 60
            })
        );
    }
}
