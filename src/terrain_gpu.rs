//! GPU world renderer using `wgpu`:
//! - Sky dome/gradient (`sky.wgsl`)
//! - Streamed terrain with day/night `t2lm` + `t2lm1` lightmap blending (`terrain.wgsl`)
//! - Instanced opaque/alpha-tested & back-to-front transparent `.bmd` scenery (`scenery.wgsl`)
//! - Experimental animated water with scissored visible-only reflection & refraction passes,
//!   shoreline edge blend, and underwater tint (`water.wgsl`)
//! - Resolution scaling (`10%–200%`) with native `egui` UI overlay (`blit.wgsl`)

use std::{
    env, fs,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_TERRAIN_TEX_REV: AtomicU64 = AtomicU64::new(1);

fn next_terrain_tex_revision() -> u64 {
    NEXT_TERRAIN_TEX_REV.fetch_add(1, Ordering::Relaxed)
}

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

use crate::{
    atmosphere::{self, BoxAreaAtmosphere, ScissorRect, WaterPlane},
    bmd::{self, AlphaMode, BmdModel},
    camera::{scaled_render_target, streaming_blocks, streaming_blocks_angelica, FreeCamera},
    dxt,
    settings::WaterQuality,
    terrain_block::{self, T2bkFile, T2lmHeader, T2mkHeader, Trn2Header},
    terrain_store::{TerrainPage, TerrainStore, DEFAULT_HOT_RAM_BYTES},
    tex_cache::DecodedTexture,
    vegetation::{self, FwstTreeMesh, VegVertex},
    world_objects::placement_transform_matrix,
    zone_dump::{open_all_resource_pcks, texture_lookup_candidates_for_archive},
};

pub const SKY_WGSL: &str = include_str!("shaders/sky.wgsl");
pub const TERRAIN_WGSL: &str = include_str!("shaders/terrain.wgsl");
pub const SCENERY_WGSL: &str = include_str!("shaders/scenery.wgsl");
pub const VEGETATION_WGSL: &str = include_str!("shaders/vegetation.wgsl");
pub const WATER_WGSL: &str = include_str!("shaders/water.wgsl");
pub const BLIT_WGSL: &str = include_str!("shaders/blit.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct TerrainVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
    pub uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SceneryVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub day_color: [f32; 4],
    pub night_color: [f32; 4],
    pub uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SceneryInstance {
    pub model_cols: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SceneryMaterialUniform {
    pub diffuse_rgba: [f32; 4],
    /// x = alpha_cutoff (0.0 opaque/blended, 0.5 alpha-tested)
    /// y = has_vertex_lighting (1.0 client day/night, 0.0 approximate fallback)
    pub params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct VegMaterialUniform {
    pub base_color: [f32; 4],
    /// x = alpha_cutoff (0.0 trunk, 0.5 alpha-tested leaves/grass), y = wind_amplitude
    pub params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct WaterVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub edge_factor: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct WaterUniform {
    pub water_color: [f32; 4],
    /// x = refl_enabled, y = refr_enabled, z = wave_scale, w = shore_fade
    pub flags: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],
    /// xyz = normalized sun dir, w = dn_factor (0=day, 1=night)
    pub sun_dir: [f32; 4],
    /// rgb = active fog color, a = elapsed time seconds
    pub fog_color: [f32; 4],
    /// x = fog_start, y = fog_end, z = fog_enabled (0/1), w = underwater (0/1)
    pub fog_params: [f32; 4],
    /// rgb = sky zenith, a = water_height
    pub sky_zenith: [f32; 4],
    /// rgb = sky horizon, a = camera_y
    pub sky_horizon: [f32; 4],
    /// rgb = sun disc color, a = 1.0
    pub sun_color: [f32; 4],
}

impl CameraUniform {
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        view_proj: Mat4,
        camera_pos: Vec3,
        hours: f32,
        elapsed_sec: f32,
        fog_enabled: bool,
        fog_mult: f32,
        box_area: Option<&BoxAreaAtmosphere>,
        water_height: Option<f32>,
    ) -> Self {
        let dn = atmosphere::day_night_factor(hours);
        let sun = atmosphere::sun_direction_for_time(hours);
        let default_box = BoxAreaAtmosphere::default();
        let area = box_area.unwrap_or(&default_box);

        let underwater = water_height.is_some_and(|wh| camera_pos.y < wh);
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let lerp3 = |a: [f32; 3], b: [f32; 3], t: f32| {
            [
                lerp(a[0], b[0], t),
                lerp(a[1], b[1], t),
                lerp(a[2], b[2], t),
            ]
        };

        let (fog_start, fog_end, fog_rgb) = if underwater {
            (
                area.uw_fog_start,
                area.uw_fog_end,
                atmosphere::unpack_argb_rgb(area.uw_fog_color),
            )
        } else {
            let mult = fog_mult.clamp(0.25, 4.0);
            let s = lerp(area.day_fog_start, area.night_fog_start, dn) * mult;
            let e = lerp(area.day_fog_end, area.night_fog_end, dn) * mult;
            let c = lerp3(
                atmosphere::unpack_argb_rgb(area.day_fog_color),
                atmosphere::unpack_argb_rgb(area.night_fog_color),
                dn,
            );
            (s, e.max(s + 1.0), c)
        };

        let (zenith, horizon, sun_col) = atmosphere::sky_colors_for_time(hours, Some(area));
        Self {
            view_proj: view_proj.to_cols_array_2d(),
            sun_dir: [sun.x, sun.y, sun.z, dn],
            fog_color: [fog_rgb[0], fog_rgb[1], fog_rgb[2], elapsed_sec],
            fog_params: [
                fog_start,
                fog_end,
                if fog_enabled || underwater { 1.0 } else { 0.0 },
                if underwater { 1.0 } else { 0.0 },
            ],
            sky_zenith: [
                zenith[0],
                zenith[1],
                zenith[2],
                water_height.unwrap_or(-1e6),
            ],
            sky_horizon: [horizon[0], horizon[1], horizon[2], camera_pos.y],
            sun_color: [sun_col[0], sun_col[1], sun_col[2], 1.0],
        }
    }
}

/// One deduplicated static submesh batch (one vertex/index buffer shared across N instances).
#[derive(Clone, Debug)]
pub struct SceneryDrawBatch {
    pub vertices: Vec<SceneryVertex>,
    pub indices: Vec<u16>,
    pub instances: Vec<SceneryInstance>,
    pub diffuse_rgba: [f32; 4],
    pub alpha_cutoff: f32,
    pub is_transparent: bool,
    pub has_vertex_lighting: bool,
    pub sort_center: Vec3,
    pub sort_id: u64,
    pub texture: Option<Arc<DecodedTexture>>,
}

/// Complete set of deduplicated opaque and back-to-front sorted transparent scenery batches.
#[derive(Clone, Debug, Default)]
pub struct SceneryDrawSnapshot {
    pub opaque_batches: Vec<SceneryDrawBatch>,
    pub transparent_batches: Vec<SceneryDrawBatch>,
    pub revision: u64,
}

/// One instanced vegetation draw batch (SpeedTree trunk/leaves or grass tuft type).
#[derive(Clone, Debug)]
pub struct VegetationDrawBatch {
    pub vertices: Vec<VegVertex>,
    pub indices: Vec<u16>,
    pub instances: Vec<SceneryInstance>,
    pub base_color: [f32; 4],
    pub alpha_cutoff: f32,
    pub wind_amplitude: f32,
    pub texture: Option<Arc<DecodedTexture>>,
}

/// Complete set of instanced tree and grass batches for the current frame.
#[derive(Clone, Debug, Default)]
pub struct VegetationDrawSnapshot {
    pub batches: Vec<VegetationDrawBatch>,
    pub revision: u64,
}

/// Build trunk + alpha-tested leaf draw batches from a cached `FwstTreeMesh`.
pub fn build_tree_batches(
    mesh: &FwstTreeMesh,
    transforms: &[Mat4],
    trunk_color: [f32; 4],
    leaf_color: [f32; 4],
    leaf_tex: Option<Arc<DecodedTexture>>,
) -> Vec<VegetationDrawBatch> {
    if transforms.is_empty() {
        return Vec::new();
    }
    let instances: Vec<SceneryInstance> = transforms
        .iter()
        .map(|m| SceneryInstance {
            model_cols: m.to_cols_array_2d(),
        })
        .collect();
    let mut out = Vec::new();
    if !mesh.trunk_vertices.is_empty() && !mesh.trunk_indices.is_empty() {
        out.push(VegetationDrawBatch {
            vertices: mesh.trunk_vertices.clone(),
            indices: mesh.trunk_indices.clone(),
            instances: instances.clone(),
            base_color: trunk_color,
            alpha_cutoff: 0.0,
            wind_amplitude: if mesh.wind_approximate { 0.12 } else { 0.0 },
            texture: None,
        });
    }
    if !mesh.leaf_vertices.is_empty() && !mesh.leaf_indices.is_empty() {
        out.push(VegetationDrawBatch {
            vertices: mesh.leaf_vertices.clone(),
            indices: mesh.leaf_indices.clone(),
            instances,
            base_color: leaf_color,
            alpha_cutoff: mesh.alpha_ref.clamp(0.0, 0.95),
            wind_amplitude: if mesh.wind_approximate { 0.35 } else { 0.0 },
            texture: leaf_tex,
        });
    }
    out
}

/// Water render configuration for the current frame.
#[derive(Clone, Debug)]
pub struct WaterRenderSnapshot {
    pub planes: Vec<WaterPlane>,
    pub reflection_enabled: bool,
    pub refraction_enabled: bool,
    pub quality: WaterQuality,
}

impl Default for WaterRenderSnapshot {
    fn default() -> Self {
        Self {
            planes: Vec::new(),
            reflection_enabled: true,
            refraction_enabled: true,
            quality: WaterQuality::Medium,
        }
    }
}

fn unpack_argb_to_rgba_f32(argb: u32) -> [f32; 4] {
    let a = ((argb >> 24) & 0xff) as f32 / 255.0;
    let r = ((argb >> 16) & 0xff) as f32 / 255.0;
    let g = ((argb >> 8) & 0xff) as f32 / 255.0;
    let b = (argb & 0xff) as f32 / 255.0;
    [r, g, b, a]
}

/// Convert a parsed `BmdModel` + its deduplicated world instance transforms into opaque and
/// per-instance back-to-front sortable transparent draw batches.
/// Returns `(opaque_batches, transparent_batches, approximate_lighting_submeshes, transparent_skipped)`.
pub fn build_scenery_batches_for_model<F>(
    model: &BmdModel,
    transforms: &[Mat4],
    transparent_enabled: bool,
    base_sort_id: u64,
    mut resolve_tex: F,
) -> (Vec<SceneryDrawBatch>, Vec<SceneryDrawBatch>, usize, usize)
where
    F: FnMut(&str) -> Option<Arc<DecodedTexture>>,
{
    if model.collide_only || transforms.is_empty() {
        return (Vec::new(), Vec::new(), 0, 0);
    }
    let instances: Vec<SceneryInstance> = transforms
        .iter()
        .map(|m| SceneryInstance {
            model_cols: m.to_cols_array_2d(),
        })
        .collect();

    let mut opaque = Vec::new();
    let mut transparent = Vec::new();
    let mut approx_count = 0usize;
    let mut trans_skipped = 0usize;

    for (sm_idx, sm) in model.submeshes.iter().enumerate() {
        if sm.vertices.is_empty() || sm.indices.is_empty() {
            continue;
        }
        let is_transparent = sm.alpha_mode == AlphaMode::TransparentDeferred;
        if is_transparent && !transparent_enabled {
            trans_skipped += transforms.len();
            continue;
        }

        // v3–v5 carry per-vertex day/night diffuse arrays; v2 falls back to approximate directional lighting
        let has_vertex_lighting = sm.version >= 0x1000_0003;
        if !has_vertex_lighting {
            approx_count += 1;
        }

        let alpha_cutoff = if sm.alpha_mode == AlphaMode::AlphaTested {
            0.5
        } else {
            0.0
        };
        let diffuse_rgba = sm
            .material
            .as_ref()
            .map(|m| m.diffuse)
            .unwrap_or([1.0, 1.0, 1.0, 1.0]);
        let vertices: Vec<SceneryVertex> = sm
            .vertices
            .iter()
            .map(|v| SceneryVertex {
                position: v.position,
                normal: v.normal,
                day_color: unpack_argb_to_rgba_f32(v.day_diffuse),
                night_color: unpack_argb_to_rgba_f32(v.night_diffuse),
                uv: v.uv,
            })
            .collect();
        let texture = if sm.texture.is_empty() {
            None
        } else {
            resolve_tex(&sm.texture)
        };

        if is_transparent {
            // Emit one sortable entry per instance and per submesh for accurate back-to-front sorting
            let local_center =
                (Vec3::from_array(sm.aabb_min) + Vec3::from_array(sm.aabb_max)) * 0.5;
            for (inst_i, &mat) in transforms.iter().enumerate() {
                let world_center = mat.transform_point3(local_center);
                transparent.push(SceneryDrawBatch {
                    vertices: vertices.clone(),
                    indices: sm.indices.clone(),
                    instances: vec![SceneryInstance {
                        model_cols: mat.to_cols_array_2d(),
                    }],
                    diffuse_rgba,
                    alpha_cutoff: 0.0,
                    is_transparent: true,
                    has_vertex_lighting,
                    sort_center: world_center,
                    sort_id: base_sort_id.wrapping_add(((sm_idx as u64) << 20) + (inst_i as u64)),
                    texture: texture.clone(),
                });
            }
        } else {
            opaque.push(SceneryDrawBatch {
                vertices,
                indices: sm.indices.clone(),
                instances: instances.clone(),
                diffuse_rgba,
                alpha_cutoff,
                is_transparent: false,
                has_vertex_lighting,
                sort_center: Vec3::ZERO,
                sort_id: base_sort_id + (sm_idx as u64),
                texture,
            });
        }
    }
    (opaque, transparent, approx_count, trans_skipped)
}

/// Build an indexed triangle mesh for one terrain block from a streamed `TerrainPage`.
pub fn build_block_mesh(
    page: &TerrainPage,
    day_colors: Option<&[[u8; 4]]>,
    row: u32,
    col: u32,
    block_size: f32,
    grid: u32,
) -> (Vec<TerrainVertex>, Vec<u32>) {
    let side = (grid as usize) + 1;
    let step = block_size / (grid as f32);
    let origin_x = (col as f32) * block_size;
    let origin_z = (row as f32) * block_size;

    let mut vertices = Vec::with_capacity(side * side);
    for gz in 0..side {
        for gx in 0..side {
            let idx = gz * side + gx;
            let h = page.heights.get(idx).copied().unwrap_or(0.0);
            let n = page.normals.get(idx).copied().unwrap_or([0, 127, 0]);
            let normal = [
                n[0] as f32 / 127.0,
                n[1] as f32 / 127.0,
                n[2] as f32 / 127.0,
            ];
            let color = if let Some(cols) = day_colors.and_then(|c| c.get(idx)) {
                [
                    cols[0] as f32 / 255.0,
                    cols[1] as f32 / 255.0,
                    cols[2] as f32 / 255.0,
                    cols[3] as f32 / 255.0,
                ]
            } else {
                let t = ((h - 80.0) / 160.0).clamp(0.0, 1.0);
                [0.32 + 0.35 * t, 0.48 + 0.25 * t, 0.26 + 0.20 * t, 1.0]
            };
            vertices.push(TerrainVertex {
                position: [
                    origin_x + (gx as f32) * step,
                    h,
                    origin_z + (gz as f32) * step,
                ],
                normal,
                color,
                uv: [(gx as f32) / (grid as f32), (gz as f32) / (grid as f32)],
            });
        }
    }

    let mut indices = Vec::with_capacity((grid as usize) * (grid as usize) * 6);
    for gz in 0..(grid as usize) {
        for gx in 0..(grid as usize) {
            let i0 = (gz * side + gx) as u32;
            let i1 = i0 + 1;
            let i2 = ((gz + 1) * side + gx) as u32;
            let i3 = i2 + 1;
            indices.extend_from_slice(&[i0, i2, i1, i1, i2, i3]);
        }
    }

    (vertices, indices)
}

/// Build an indexed triangle mesh for one terrain block in Angelica centered world coordinates:
/// `x = -world_width/2 + col*block_size + gx*step`, `z = +world_length/2 - row*block_size - gz*step`.
/// Verified to match real `s0.ecbsd` placements within 0.33 m median.
/// Uses global zone UVs `(0..1, 0..1)` so stitched real layer+mask+lightmap textures align cleanly.
/// Never synthesizes fallback green height colors.
#[allow(clippy::too_many_arguments)]
pub fn build_block_mesh_angelica(
    page: &TerrainPage,
    row: u32,
    col: u32,
    block_rows: u32,
    block_cols: u32,
    block_size: f32,
    grid: u32,
) -> (Vec<TerrainVertex>, Vec<u32>) {
    let side = (grid as usize) + 1;
    let step = block_size / (grid as f32);
    let half_w = (block_cols as f32) * block_size * 0.5;
    let half_l = (block_rows as f32) * block_size * 0.5;
    let origin_x = -half_w + (col as f32) * block_size;
    let origin_z = half_l - (row as f32) * block_size;
    let cols_f = (block_cols.max(1)) as f32;
    let rows_f = (block_rows.max(1)) as f32;

    let mut vertices = Vec::with_capacity(side * side);
    for gz in 0..side {
        for gx in 0..side {
            let idx = gz * side + gx;
            let h = page.heights.get(idx).copied().unwrap_or(0.0);
            let n = page.normals.get(idx).copied().unwrap_or([0, 127, 0]);
            let normal = [
                n[0] as f32 / 127.0,
                n[1] as f32 / 127.0,
                n[2] as f32 / 127.0,
            ];
            let u = ((col as f32) + (gx as f32) / (grid as f32)) / cols_f;
            let v = ((row as f32) + (gz as f32) / (grid as f32)) / rows_f;
            vertices.push(TerrainVertex {
                position: [
                    origin_x + (gx as f32) * step,
                    h,
                    origin_z - (gz as f32) * step,
                ],
                normal,
                color: [1.0, 1.0, 1.0, 1.0],
                uv: [u, v],
            });
        }
    }

    let mut indices = Vec::with_capacity((grid as usize) * (grid as usize) * 6);
    for gz in 0..(grid as usize) {
        for gx in 0..(grid as usize) {
            let i0 = (gz * side + gx) as u32;
            let i1 = i0 + 1;
            let i2 = ((gz + 1) * side + gx) as u32;
            let i3 = i2 + 1;
            indices.extend_from_slice(&[i0, i1, i2, i1, i3, i2]);
        }
    }
    (vertices, indices)
}

/// Build a subdivided water plane mesh with `edge_factor` fading to 0 at shoreline borders.
pub fn build_water_plane_mesh(plane: &WaterPlane) -> (Vec<WaterVertex>, Vec<u16>) {
    let segs = 8usize;
    let side = segs + 1;
    let mut verts = Vec::with_capacity(side * side);
    for iz in 0..side {
        let vz = iz as f32 / segs as f32;
        let wz = plane.center_z + (vz * 2.0 - 1.0) * plane.half_z;
        let ez = (vz.min(1.0 - vz) * 4.0).clamp(0.0, 1.0);
        for ix in 0..side {
            let vx = ix as f32 / segs as f32;
            let wx = plane.center_x + (vx * 2.0 - 1.0) * plane.half_x;
            let ex = (vx.min(1.0 - vx) * 4.0).clamp(0.0, 1.0);
            verts.push(WaterVertex {
                position: [wx, plane.height_y, wz],
                uv: [vx, vz],
                edge_factor: ex.min(ez),
            });
        }
    }
    let mut idxs = Vec::with_capacity(segs * segs * 6);
    for iz in 0..segs {
        for ix in 0..segs {
            let i0 = (iz * side + ix) as u16;
            let i1 = i0 + 1;
            let i2 = ((iz + 1) * side + ix) as u16;
            let i3 = i2 + 1;
            idxs.extend_from_slice(&[i0, i2, i1, i1, i2, i3]);
        }
    }
    (verts, idxs)
}

/// Generate a procedural 64x64 tileable tangent-space water normal map (RGBA8) as fallback
/// when client water normal textures are not loaded.
pub fn generate_procedural_water_normal_map() -> DecodedTexture {
    let size = 64u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    let tau = std::f32::consts::TAU;
    for y in 0..size {
        let v = y as f32 / size as f32;
        for x in 0..size {
            let u = x as f32 / size as f32;
            let dx = (u * tau * 2.0).cos() * 0.25 + ((u + v) * tau * 3.0).sin() * 0.15;
            let dy = (v * tau * 2.0).sin() * 0.25 + ((u - v) * tau * 3.0).cos() * 0.15;
            let n = Vec3::new(dx, dy, 1.0).normalize();
            rgba.push(((n.x * 0.5 + 0.5) * 255.0) as u8);
            rgba.push(((n.y * 0.5 + 0.5) * 255.0) as u8);
            rgba.push(((n.z * 0.5 + 0.5) * 255.0) as u8);
            rgba.push(255);
        }
    }
    DecodedTexture {
        width: size,
        height: size,
        rgba,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OffscreenSmokeMetrics {
    pub width: u32,
    pub height: u32,
    pub scaled_width: u32,
    pub scaled_height: u32,
    pub streamed_blocks: usize,
    pub total_pixels: usize,
    pub sky_pixels: usize,
    pub non_clear_pixels: usize,
    pub scenery_pixels: usize,
    pub tree_pixels: usize,
    pub grass_pixels: usize,
    pub translucent_pixels: usize,
    pub water_pixels: usize,
    pub water_differs_from_non_water: bool,
    pub distinct_colors: usize,
}

fn create_cam_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("camera-bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

fn create_terrain_lm_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("terrain-lm-bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn create_scenery_mat_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("scenery-mat-bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}

fn create_water_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("water-bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}

fn create_sky_pipeline(
    device: &wgpu::Device,
    cam_bgl: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("sky-shader"),
        source: wgpu::ShaderSource::Wgsl(SKY_WGSL.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("sky-pipeline-layout"),
        bind_group_layouts: &[cam_bgl],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sky-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: false,
            depth_compare: wgpu::CompareFunction::LessEqual,
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        multiview: None,
        cache: None,
    })
}

fn create_scenery_pipeline(
    device: &wgpu::Device,
    cam_bgl: &wgpu::BindGroupLayout,
    mat_bgl: &wgpu::BindGroupLayout,
    transparent: bool,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(if transparent {
            "scenery-transparent-shader"
        } else {
            "scenery-opaque-shader"
        }),
        source: wgpu::ShaderSource::Wgsl(SCENERY_WGSL.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("scenery-pipeline-layout"),
        bind_group_layouts: &[cam_bgl, mat_bgl],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(if transparent {
            "scenery-transparent-pipeline"
        } else {
            "scenery-opaque-pipeline"
        }),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[
                wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<SceneryVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 12,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 24,
                            shader_location: 2,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 40,
                            shader_location: 3,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 56,
                            shader_location: 4,
                        },
                    ],
                },
                wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<SceneryInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 0,
                            shader_location: 5,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 16,
                            shader_location: 6,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 32,
                            shader_location: 7,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 48,
                            shader_location: 8,
                        },
                    ],
                },
            ],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: if transparent {
                    Some(wgpu::BlendState::ALPHA_BLENDING)
                } else {
                    None
                },
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: !transparent,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        multiview: None,
        cache: None,
    })
}

fn create_vegetation_pipeline(
    device: &wgpu::Device,
    cam_bgl: &wgpu::BindGroupLayout,
    mat_bgl: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("vegetation-shader"),
        source: wgpu::ShaderSource::Wgsl(VEGETATION_WGSL.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("vegetation-pipeline-layout"),
        bind_group_layouts: &[cam_bgl, mat_bgl],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("vegetation-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[
                wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<VegVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 12,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 24,
                            shader_location: 2,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32,
                            offset: 32,
                            shader_location: 3,
                        },
                    ],
                },
                wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<SceneryInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 0,
                            shader_location: 4,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 16,
                            shader_location: 5,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 32,
                            shader_location: 6,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 48,
                            shader_location: 7,
                        },
                    ],
                },
            ],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        multiview: None,
        cache: None,
    })
}

fn create_water_pipeline(
    device: &wgpu::Device,
    cam_bgl: &wgpu::BindGroupLayout,
    water_bgl: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("water-shader"),
        source: wgpu::ShaderSource::Wgsl(WATER_WGSL.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("water-pipeline-layout"),
        bind_group_layouts: &[cam_bgl, water_bgl],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("water-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<WaterVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 12,
                        shader_location: 1,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32,
                        offset: 20,
                        shader_location: 2,
                    },
                ],
            }],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: false,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        multiview: None,
        cache: None,
    })
}

/// Offscreen smoke test (for Mesa `lavapipe` CI): renders sky, streamed terrain (day+night
/// lightmap blend), opaque `.bmd` building, translucent test quad, and an animated water
/// plane with scissored reflection/refraction passes.
///
/// Proves pipelines compile and execute on CPU; does **not** prove visual parity.
pub fn render_offscreen_smoke(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    scale_pct: u16,
) -> Result<OffscreenSmokeMetrics, String> {
    let (scaled_w, scaled_h) = scaled_render_target(width, height, scale_pct);

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let cache_root =
        env::temp_dir().join(format!("fw-terrain-smoke-{}-{nanos}", std::process::id()));
    let mut store = TerrainStore::open(&cache_root, DEFAULT_HOT_RAM_BYTES)?;

    let grid = 16u32;
    let block_size = 64.0f32;
    let t2bk_bytes = terrain_block::synthetic::build_t2bk_v6(grid, 4);
    let t2bk = T2bkFile::parse_header(&t2bk_bytes)?;

    // Fixed camera showing sky in upper band, terrain across mid/lower, building on left-center,
    // translucent quad in upper-right-center, and water lake in foreground-right.
    let camera = FreeCamera {
        position: Vec3::new(10.0, 140.0, 10.0),
        yaw: 0.78,
        pitch: -0.22,
        ..Default::default()
    };
    let visible = streaming_blocks(camera.position, block_size, 2, 2, 2);

    let mut all_verts = Vec::new();
    let mut all_indices = Vec::new();
    for &(row, col) in &visible {
        let block_idx = (row * 2 + col) as usize;
        let key = TerrainStore::content_key(&format!("smoke-block-{block_idx}").into_bytes());
        let page = store.get_or_insert_with(&key, || {
            let decoded = t2bk.decode_block(&t2bk_bytes, block_idx, grid)?;
            Ok(TerrainPage {
                heights: decoded.heights,
                normals: decoded.normals,
            })
        })?;
        let base_vert = all_verts.len() as u32;
        let (verts, idxs) = build_block_mesh(&page, None, row, col, block_size, grid);
        all_verts.extend(verts);
        all_indices.extend(idxs.into_iter().map(|i| i + base_vert));
    }
    let _ = fs::remove_dir_all(&cache_root);

    let vp: Mat4 = camera.view_proj(width as f32 / height as f32);
    let uniform = CameraUniform::build(vp, camera.position, 13.0, 1.5, true, 1.0, None, None);

    // Opaque .bmd building (vivid crimson) + translucent test quad (vivid magenta-violet, alpha 0.65)
    let raw_bmd = bmd::synthetic::build_bmd_v5_box("textures/smoke_tower.dds", 12.0, 42.0);
    let bmd_model = BmdModel::parse(&raw_bmd).map_err(|e| e.to_string())?;
    let tower_mat = placement_transform_matrix(
        [52.0, 95.0, 74.0],
        [0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 1.0],
    );
    let (mut opaque_batches, _, _, _) =
        build_scenery_batches_for_model(&bmd_model, &[tower_mat], true, 1, |_| None);
    for b in &mut opaque_batches {
        b.diffuse_rgba = [0.95, 0.14, 0.10, 1.0];
    }

    let trans_quad = SceneryDrawBatch {
        vertices: vec![
            SceneryVertex {
                position: [-10.0, 0.0, 0.0],
                normal: [0.0, 0.0, -1.0],
                day_color: [1.0, 1.0, 1.0, 1.0],
                night_color: [0.5, 0.5, 0.7, 1.0],
                uv: [0.0, 1.0],
            },
            SceneryVertex {
                position: [10.0, 0.0, 0.0],
                normal: [0.0, 0.0, -1.0],
                day_color: [1.0, 1.0, 1.0, 1.0],
                night_color: [0.5, 0.5, 0.7, 1.0],
                uv: [1.0, 1.0],
            },
            SceneryVertex {
                position: [10.0, 26.0, 0.0],
                normal: [0.0, 0.0, -1.0],
                day_color: [1.0, 1.0, 1.0, 1.0],
                night_color: [0.5, 0.5, 0.7, 1.0],
                uv: [1.0, 0.0],
            },
            SceneryVertex {
                position: [-10.0, 26.0, 0.0],
                normal: [0.0, 0.0, -1.0],
                day_color: [1.0, 1.0, 1.0, 1.0],
                night_color: [0.5, 0.5, 0.7, 1.0],
                uv: [0.0, 0.0],
            },
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
        instances: vec![SceneryInstance {
            model_cols: placement_transform_matrix(
                [76.0, 118.0, 56.0],
                [-0.7, 0.0, -0.7],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 1.0],
            )
            .to_cols_array_2d(),
        }],
        diffuse_rgba: [0.88, 0.15, 0.92, 0.65],
        alpha_cutoff: 0.0,
        is_transparent: true,
        has_vertex_lighting: true,
        sort_center: Vec3::new(76.0, 130.0, 56.0),
        sort_id: 99,
        texture: None,
    };

    // Synthetic water plane sitting above lower terrain in the foreground-right
    let water_plane = WaterPlane {
        id: 1,
        center_x: 78.0,
        center_z: 78.0,
        half_x: 42.0,
        half_z: 42.0,
        height_y: 112.0,
        color_rgba: [0.05, 0.58, 0.88, 0.88],
    };

    let mut renderer = TerrainViewportRenderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm);

    let dxt1_block = [0xef, 0xf7, 0x20, 0x84, 0xe4, 0x1b, 0xe4, 0x1b];
    let dds = dxt::synthetic::build_dds(4, 4, b"DXT1", &dxt1_block);
    let surf = dxt::parse_dds(&dds)?;
    let lm_rgba = dxt::decode_rgba8(&surf);
    let day_lm = TerrainTextureSnapshot {
        width: surf.width,
        height: surf.height,
        rgba: lm_rgba.clone(),
        revision: 1,
    };
    let night_lm = TerrainTextureSnapshot {
        width: surf.width,
        height: surf.height,
        rgba: lm_rgba.iter().map(|&b| b / 2).collect(),
        revision: 1,
    };

    let mesh_snap = TerrainMeshSnapshot {
        vertices: all_verts,
        indices: all_indices,
        wireframe_indices: Vec::new(),
        revision: 1,
        streamed_blocks: visible.len(),
        hot_ram_bytes: 0,
    };
    let scenery_snap = SceneryDrawSnapshot {
        opaque_batches,
        transparent_batches: vec![trans_quad],
        revision: 1,
    };
    let water_snap = WaterRenderSnapshot {
        planes: vec![water_plane.clone()],
        reflection_enabled: true,
        refraction_enabled: true,
        quality: WaterQuality::Medium,
    };

    // Synthetic SpeedTree mesh (round-tripped through FWST v1) + alpha-tested leaf texture
    // and synthetic crossed-blade grass patch to prove vegetation.wgsl executes on lavapipe.
    let mut alpha_tex_rgba = Vec::with_capacity(8 * 8 * 4);
    for ty in 0..8u32 {
        for tx in 0..8u32 {
            let opaque = (1..=6).contains(&tx) && (1..=6).contains(&ty);
            alpha_tex_rgba.extend_from_slice(&[255, 255, 255, if opaque { 255 } else { 0 }]);
        }
    }
    let alpha_tex = Arc::new(DecodedTexture {
        width: 8,
        height: 8,
        rgba: alpha_tex_rgba,
    });
    let tree_bytes = vegetation::synthetic::build_synthetic_tree_fwst(0, 0).encode();
    let tree_mesh = FwstTreeMesh::decode(&tree_bytes)?;
    let tree_mat = Mat4::from_translation(Vec3::new(34.0, 102.0, 56.0));
    let mut veg_batches = build_tree_batches(
        &tree_mesh,
        &[tree_mat],
        [0.42, 0.25, 0.10, 1.0],
        [0.06, 0.94, 0.18, 1.0],
        Some(Arc::clone(&alpha_tex)),
    );
    let (grass_verts, grass_idxs) = vegetation::synthetic::build_grass_tuft_mesh(9.0);
    let mut grass_instances = Vec::new();
    for gx in 0..3 {
        for gz in 0..3 {
            let pos = Vec3::new(34.0 + (gx as f32) * 7.0, 106.0, 26.0 + (gz as f32) * 7.0);
            grass_instances.push(SceneryInstance {
                model_cols: Mat4::from_translation(pos).to_cols_array_2d(),
            });
        }
    }
    veg_batches.push(VegetationDrawBatch {
        vertices: grass_verts,
        indices: grass_idxs,
        instances: grass_instances,
        base_color: [0.92, 0.95, 0.08, 1.0],
        alpha_cutoff: 0.5,
        wind_amplitude: 0.25,
        texture: Some(alpha_tex),
    });
    let veg_snap = VegetationDrawSnapshot {
        batches: veg_batches,
        revision: 1,
    };

    let world_cmd = renderer.prepare_frame(
        device,
        queue,
        width,
        height,
        scale_pct,
        &uniform,
        &mesh_snap,
        &day_lm,
        &night_lm,
        &scenery_snap,
        &veg_snap,
        &water_snap,
        true,
        true,
        false,
    );

    let final_color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("smoke-final-color"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let final_view = final_color.create_view(&wgpu::TextureViewDescriptor::default());

    let mut blit_enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("smoke-blit-encoder"),
    });
    {
        let mut bpass = blit_enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("smoke-blit-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &final_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
        renderer.paint_blit(&mut bpass);
    }

    let unpadded_bpr = width * 4;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_bpr = unpadded_bpr.div_ceil(align) * align;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("smoke-readback"),
        size: (padded_bpr as u64) * (height as u64),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    blit_enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &final_color,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bpr),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([world_cmd, blit_enc.finish()]);

    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        let _ = tx.send(res);
    });
    device.poll(wgpu::Maintain::Wait);
    rx.recv()
        .map_err(|e| format!("readback channel error: {e}"))?
        .map_err(|e| format!("readback map error: {e:?}"))?;

    let data = slice.get_mapped_range();
    let water_rect =
        atmosphere::compute_water_scissor(&water_plane, vp, width, height).unwrap_or(ScissorRect {
            x: 0,
            y: height / 2,
            width,
            height: height / 2,
        });

    let mut sky_pixels = 0usize;
    let mut non_clear_pixels = 0usize;
    let mut scenery_pixels = 0usize;
    let mut tree_pixels = 0usize;
    let mut grass_pixels = 0usize;
    let mut translucent_pixels = 0usize;
    let mut water_pixels = 0usize;
    let mut water_sum = [0u64; 3];
    let mut water_cnt = 0u64;
    let mut non_water_sum = [0u64; 3];
    let mut non_water_cnt = 0u64;
    let mut distinct = std::collections::HashSet::new();

    for y in 0..height as usize {
        let row = &data[y * padded_bpr as usize..y * padded_bpr as usize + unpadded_bpr as usize];
        for (x, px) in row.as_chunks::<4>().0.iter().enumerate() {
            let (r, g, b) = (px[0] as i32, px[1] as i32, px[2] as i32);
            if r > 5 || g > 5 || b > 5 {
                non_clear_pixels += 1;
            }
            if y < (height as usize) / 4 && b > 80 {
                sky_pixels += 1;
            }
            // Opaque crimson .bmd building
            if r > 90 && r > g + 35 && r > b + 35 {
                scenery_pixels += 1;
            }
            // Synthetic emerald-green tree canopy (vegetation.wgsl)
            if g > 90 && g > r + 40 && g > b + 35 {
                tree_pixels += 1;
            }
            // Synthetic lime-yellow grass patch (vegetation.wgsl)
            if r > 95 && g > 105 && g > b + 50 && r > b + 45 {
                grass_pixels += 1;
            }
            // Translucent magenta-violet test quad
            if r > 80 && b > 80 && r > g + 25 && b > g + 25 {
                translucent_pixels += 1;
            }
            let in_water_box = (x as u32) >= water_rect.x
                && (x as u32) < water_rect.x + water_rect.width
                && (y as u32) >= water_rect.y
                && (y as u32) < water_rect.y + water_rect.height;
            if in_water_box {
                water_sum[0] += px[0] as u64;
                water_sum[1] += px[1] as u64;
                water_sum[2] += px[2] as u64;
                water_cnt += 1;
                if b > r + 15 && g > r + 5 {
                    water_pixels += 1;
                }
            } else if y >= (height as usize) / 2 {
                non_water_sum[0] += px[0] as u64;
                non_water_sum[1] += px[1] as u64;
                non_water_sum[2] += px[2] as u64;
                non_water_cnt += 1;
            }
            distinct.insert([px[0], px[1], px[2]]);
        }
    }
    drop(data);
    readback.unmap();

    let water_differs_from_non_water = if water_cnt > 0 && non_water_cnt > 0 {
        let wr = water_sum[0] as f64 / water_cnt as f64;
        let wg = water_sum[1] as f64 / water_cnt as f64;
        let wb = water_sum[2] as f64 / water_cnt as f64;
        let nr = non_water_sum[0] as f64 / non_water_cnt as f64;
        let ng = non_water_sum[1] as f64 / non_water_cnt as f64;
        let nb = non_water_sum[2] as f64 / non_water_cnt as f64;
        ((wr - nr).abs() + (wg - ng).abs() + (wb - nb).abs()) > 8.0
    } else {
        water_pixels > 50
    };

    Ok(OffscreenSmokeMetrics {
        width,
        height,
        scaled_width: scaled_w,
        scaled_height: scaled_h,
        streamed_blocks: visible.len(),
        total_pixels: (width as usize) * (height as usize),
        sky_pixels,
        non_clear_pixels,
        scenery_pixels,
        tree_pixels,
        grass_pixels,
        translucent_pixels,
        water_pixels,
        water_differs_from_non_water,
        distinct_colors: distinct.len(),
    })
}

/// CPU-side streamed terrain mesh ready for upload to the live viewport renderer.
#[derive(Debug, Clone, Default)]
pub struct TerrainMeshSnapshot {
    pub vertices: Vec<TerrainVertex>,
    pub indices: Vec<u32>,
    pub wireframe_indices: Vec<u32>,
    pub revision: u64,
    pub streamed_blocks: usize,
    pub hot_ram_bytes: usize,
}

/// CPU-side decoded RGBA8 lightmap texture ready for upload to the live viewport renderer.
#[derive(Debug, Clone)]
pub struct TerrainTextureSnapshot {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub revision: u64,
}

impl Default for TerrainTextureSnapshot {
    fn default() -> Self {
        Self {
            width: 1,
            height: 1,
            rgba: vec![255, 255, 255, 255],
            revision: 0,
        }
    }
}

/// Camera-centered streaming session backed by `TerrainStore` (`cache/terrain-v1`).
/// Loads both day (`t2lm`) and night (`t2lm1`) lightmaps when present.
pub struct TerrainSession {
    pub zone_name: String,
    pub block_size: f32,
    pub block_grid: u32,
    pub block_rows: u32,
    pub block_cols: u32,
    t2bk_bytes: Vec<u8>,
    t2bk: T2bkFile,
    store: TerrainStore,
    last_center: Option<(i32, i32, i32)>,
    pub is_synthetic: bool,
    pub terrain_layers_loaded: usize,
    pub terrain_layers_failed: usize,
    pub terrain_tex_bytes: usize,
    pub load_notes: Vec<String>,
    pub mesh: TerrainMeshSnapshot,
    pub day_lightmap: TerrainTextureSnapshot,
    pub night_lightmap: TerrainTextureSnapshot,
}

/// Stitch all mask-area lightmaps from `<zone>_1.t2lm` (or `t2lm1`) into one global
/// `target_side x target_side` RGBA8 texture (4x4 grid of mask areas for `s0`), and optionally
/// multiply by real `.trn2` layer DDS textures blended via `<zone>_1.t2mk` 8-bit masks.
fn build_real_zone_terrain_textures(
    client_root: &std::path::Path,
    zone_dir: &std::path::Path,
    zone: &str,
    trn2: &Trn2Header,
) -> (
    TerrainTextureSnapshot,
    TerrainTextureSnapshot,
    usize,
    usize,
    usize,
    Vec<String>,
) {
    let mut notes = Vec::new();
    let num_ma = trn2.mask_areas.max(1) as usize;
    let ma_grid = (num_ma as f64).sqrt().round().max(1.0) as usize;
    let tile_px = 128usize;
    let full_side = (ma_grid * tile_px) as u32;

    let (mut archives, open_errors) = open_all_resource_pcks(client_root);
    for (name, err) in &open_errors {
        notes.push(format!("PCK open error for {name}: {err}"));
    }

    let mut layer_textures: Vec<Option<DecodedTexture>> = vec![None; trn2.texture_paths.len()];
    let mut loaded_cnt = 0usize;
    let mut failed_cnt = 0usize;
    let mut decoded_bytes = 0usize;

    for (i, tp) in trn2.texture_paths.iter().enumerate() {
        let mut found: Option<DecodedTexture> = None;
        for (_, mount, arch) in archives.iter_mut() {
            if !matches!(
                mount.as_str(),
                "textures" | "litmodels" | "building" | "surfaces"
            ) {
                continue;
            }
            for cand in texture_lookup_candidates_for_archive(tp, Some(mount)) {
                if let Ok(bytes) = arch.read_entry(&cand) {
                    if let Ok(surf) = dxt::parse_dds(&bytes) {
                        let rgba = dxt::decode_rgba8(&surf);
                        decoded_bytes += rgba.len();
                        found = Some(DecodedTexture {
                            width: surf.width,
                            height: surf.height,
                            rgba,
                        });
                        break;
                    }
                }
            }
            if found.is_some() {
                break;
            }
        }
        if found.is_some() {
            loaded_cnt += 1;
        } else {
            failed_cnt += 1;
            notes.push(format!("Missing terrain layer[{i}] texture: `{tp}`"));
        }
        layer_textures[i] = found;
    }
    notes.push(format!(
        "Terrain layer textures: loaded={loaded_cnt}, failed={failed_cnt}, bytes={decoded_bytes}"
    ));

    // No grey fallback: if 0 real layer textures resolved, do NOT synthesize a fallback diffuse.
    if loaded_cnt == 0 {
        let empty = TerrainTextureSnapshot {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 0],
            revision: next_terrain_tex_revision(),
        };
        return (
            empty.clone(),
            empty,
            loaded_cnt,
            failed_cnt,
            decoded_bytes,
            notes,
        );
    }

    let mut diffuse = vec![0u8; (full_side as usize) * (full_side as usize) * 4];
    let t2mk_path = zone_dir.join(format!("{zone}_1.t2mk"));
    if let Ok(mk_bytes) = fs::read(&t2mk_path) {
        if let Ok(mk) = T2mkHeader::parse(&mk_bytes) {
            for ma in 0..num_ma.min(mk.num_areas as usize) {
                let ma_r = ma / ma_grid;
                let ma_c = ma % ma_grid;
                let Ok(layers) = mk.area_layers(&mk_bytes, ma) else {
                    continue;
                };
                let mut tile_rgb = vec![[0.0f32; 3]; tile_px * tile_px];
                let mut tile_has_base = false;
                for (li, lyr) in layers.iter().enumerate() {
                    // Missing layer texture is skipped (never painted with a fallback color)
                    let Some(Some(tex)) = layer_textures.get(lyr.layer_index as usize) else {
                        continue;
                    };
                    let Ok((m_side, m_bytes)) = mk.decode_layer_mask(&mk_bytes, lyr) else {
                        continue;
                    };
                    let tw = tex.width.max(1) as usize;
                    let th = tex.height.max(1) as usize;
                    for ty in 0..tile_px {
                        for tx in 0..tile_px {
                            let sx = ((tx * tw * 8) / tile_px) % tw;
                            let sy = ((ty * th * 8) / tile_px) % th;
                            let tidx = (sy * tw + sx) * 4;
                            let tr = tex.rgba[tidx] as f32;
                            let tg = tex.rgba[tidx + 1] as f32;
                            let tb = tex.rgba[tidx + 2] as f32;

                            let alpha = if !tile_has_base || li == 0 || m_side <= 1 {
                                1.0f32
                            } else {
                                let mx = (tx * (m_side as usize)) / tile_px;
                                let my = (ty * (m_side as usize)) / tile_px;
                                m_bytes
                                    .get(my * (m_side as usize) + mx)
                                    .copied()
                                    .unwrap_or(0) as f32
                                    / 255.0
                            };
                            let dst = &mut tile_rgb[ty * tile_px + tx];
                            if !tile_has_base {
                                *dst = [tr, tg, tb];
                            } else {
                                dst[0] = dst[0] * (1.0 - alpha) + tr * alpha;
                                dst[1] = dst[1] * (1.0 - alpha) + tg * alpha;
                                dst[2] = dst[2] * (1.0 - alpha) + tb * alpha;
                            }
                        }
                    }
                    tile_has_base = true;
                }
                for ty in 0..tile_px {
                    for tx in 0..tile_px {
                        let gx = ma_c * tile_px + tx;
                        let gy = ma_r * tile_px + ty;
                        let o = (gy * (full_side as usize) + gx) * 4;
                        let rgb = tile_rgb[ty * tile_px + tx];
                        diffuse[o] = rgb[0].clamp(0.0, 255.0) as u8;
                        diffuse[o + 1] = rgb[1].clamp(0.0, 255.0) as u8;
                        diffuse[o + 2] = rgb[2].clamp(0.0, 255.0) as u8;
                        diffuse[o + 3] = 255;
                    }
                }
            }
            notes.push(format!(
                "Composited {full_side}x{full_side} real terrain diffuse from {zone}_1.t2mk ({} mask areas)",
                mk.num_areas
            ));
        }
    }

    let stitch_lm = |suffix: &str, notes: &mut Vec<String>| -> TerrainTextureSnapshot {
        let mut rgba = diffuse.clone();
        let lm_path = zone_dir.join(format!("{zone}_1.{suffix}"));
        if let Ok(lm_bytes) = fs::read(&lm_path) {
            if let Ok(lm) = T2lmHeader::parse(&lm_bytes) {
                let mut ok_areas = 0usize;
                for ma in 0..num_ma.min(lm.num_areas as usize) {
                    let Ok(dds_bytes) = lm.area_bytes(&lm_bytes, ma) else {
                        continue;
                    };
                    let Ok(surf) = dxt::parse_dds(dds_bytes) else {
                        continue;
                    };
                    let dec = dxt::decode_rgba8(&surf);
                    let sw = surf.width.max(1) as usize;
                    let sh = surf.height.max(1) as usize;
                    let ma_r = ma / ma_grid;
                    let ma_c = ma % ma_grid;
                    for ty in 0..tile_px {
                        for tx in 0..tile_px {
                            let sx = (tx * sw) / tile_px;
                            let sy = (ty * sh) / tile_px;
                            let sidx = (sy * sw + sx) * 4;
                            let gx = ma_c * tile_px + tx;
                            let gy = ma_r * tile_px + ty;
                            let o = (gy * (full_side as usize) + gx) * 4;
                            rgba[o] = ((rgba[o] as u16 * dec[sidx] as u16) / 255) as u8;
                            rgba[o + 1] = ((rgba[o + 1] as u16 * dec[sidx + 1] as u16) / 255) as u8;
                            rgba[o + 2] = ((rgba[o + 2] as u16 * dec[sidx + 2] as u16) / 255) as u8;
                        }
                    }
                    ok_areas += 1;
                }
                notes.push(format!(
                    "Stitched {ok_areas}/{} lightmap areas from {zone}_1.{suffix}",
                    lm.num_areas
                ));
            }
        } else {
            notes.push(format!("Missing lightmap file {}", lm_path.display()));
        }
        TerrainTextureSnapshot {
            width: full_side,
            height: full_side,
            rgba,
            revision: next_terrain_tex_revision(),
        }
    };

    let day = stitch_lm("t2lm", &mut notes);
    let night = stitch_lm("t2lm1", &mut notes);
    (day, night, loaded_cnt, failed_cnt, decoded_bytes, notes)
}

#[allow(dead_code)]
fn load_first_t2lm_dds(path: &std::path::Path, rev: u64) -> Option<TerrainTextureSnapshot> {
    let lm_bytes = fs::read(path).ok()?;
    let lm = terrain_block::T2lmHeader::parse(&lm_bytes).ok()?;
    let &first_off = lm.offsets.first()?;
    let start = first_off as usize;
    let end = start + lm.data_size as usize;
    let dds_slice = lm_bytes.get(start..end)?;
    let surf = dxt::parse_dds(dds_slice).ok()?;
    Some(TerrainTextureSnapshot {
        width: surf.width,
        height: surf.height,
        rgba: dxt::decode_rgba8(&surf),
        revision: rev,
    })
}

impl TerrainSession {
    pub fn open_zone(
        client_root: &std::path::Path,
        zone: &str,
        cache_root: &std::path::Path,
        hot_ram_bytes: usize,
    ) -> Result<Self, String> {
        let zone_dir = client_root.join("maps").join(zone);
        let trn2_path = zone_dir.join(format!("{zone}.trn2"));
        let trn2_bytes = fs::read(&trn2_path)
            .map_err(|e| format!("failed to read {}: {e}", trn2_path.display()))?;
        let trn2 = terrain_block::Trn2Header::parse(&trn2_bytes)?;

        let t2bk_path = zone_dir.join(format!("{zone}_1.t2bk"));
        let t2bk_bytes = fs::read(&t2bk_path)
            .map_err(|e| format!("failed to read {}: {e}", t2bk_path.display()))?;
        let t2bk = T2bkFile::parse_header(&t2bk_bytes)?;

        let store = TerrainStore::open(cache_root, hot_ram_bytes)?;

        let (
            day_lightmap,
            night_lightmap,
            terrain_layers_loaded,
            terrain_layers_failed,
            terrain_tex_bytes,
            load_notes,
        ) = build_real_zone_terrain_textures(client_root, &zone_dir, zone, &trn2);

        Ok(Self {
            zone_name: zone.to_string(),
            block_size: trn2.block_size,
            block_grid: trn2.block_grid,
            block_rows: trn2.block_rows,
            block_cols: trn2.block_cols,
            t2bk_bytes,
            t2bk,
            store,
            last_center: None,
            is_synthetic: false,
            terrain_layers_loaded,
            terrain_layers_failed,
            terrain_tex_bytes,
            load_notes,
            mesh: TerrainMeshSnapshot::default(),
            day_lightmap,
            night_lightmap,
        })
    }

    pub fn open_synthetic(
        cache_root: &std::path::Path,
        hot_ram_bytes: usize,
    ) -> Result<Self, String> {
        let grid = 16u32;
        let block_size = 64.0f32;
        let rows = 4u32;
        let cols = 4u32;
        let t2bk_bytes = terrain_block::synthetic::build_t2bk_v6(grid, (rows * cols) as usize);
        let t2bk = T2bkFile::parse_header(&t2bk_bytes)?;
        let store = TerrainStore::open(cache_root, hot_ram_bytes)?;

        let dxt1_block = [0xef, 0xf7, 0x20, 0x84, 0xe4, 0x1b, 0xe4, 0x1b];
        let dds = dxt::synthetic::build_dds(4, 4, b"DXT1", &dxt1_block);
        let surf = dxt::parse_dds(&dds)?;
        let rgba = dxt::decode_rgba8(&surf);
        let day_lightmap = TerrainTextureSnapshot {
            width: surf.width,
            height: surf.height,
            rgba: rgba.clone(),
            revision: next_terrain_tex_revision(),
        };
        let night_lightmap = TerrainTextureSnapshot {
            width: surf.width,
            height: surf.height,
            rgba: rgba.iter().map(|&b| b / 2).collect(),
            revision: next_terrain_tex_revision(),
        };

        Ok(Self {
            zone_name: "synthetic-preview".to_string(),
            block_size,
            block_grid: grid,
            block_rows: rows,
            block_cols: cols,
            t2bk_bytes,
            t2bk,
            store,
            last_center: None,
            is_synthetic: true,
            terrain_layers_loaded: 1,
            terrain_layers_failed: 0,
            terrain_tex_bytes: 64,
            load_notes: vec!["Synthetic developer preview (not a game world)".into()],
            mesh: TerrainMeshSnapshot::default(),
            day_lightmap,
            night_lightmap,
        })
    }

    /// Sample terrain height at Angelica world coordinates `(x, z)` from the real `.t2bk`.
    pub fn sample_height(&mut self, x: f32, z: f32) -> Option<f32> {
        let half_w = (self.block_cols as f32) * self.block_size * 0.5;
        let half_l = (self.block_rows as f32) * self.block_size * 0.5;
        let fx = (x + half_w) / self.block_size;
        let fz = (half_l - z) / self.block_size;
        let col = (fx.floor() as i32).clamp(0, self.block_cols as i32 - 1) as u32;
        let row = (fz.floor() as i32).clamp(0, self.block_rows as i32 - 1) as u32;
        let block_idx = (row * self.block_cols + col) as usize;
        let decoded = self
            .t2bk
            .decode_block(&self.t2bk_bytes, block_idx, self.block_grid)
            .ok()?;
        let side = (self.block_grid as usize) + 1;
        let gx = ((fx - col as f32) * (self.block_grid as f32))
            .round()
            .clamp(0.0, self.block_grid as f32) as usize;
        let gz = ((fz - row as f32) * (self.block_grid as f32))
            .round()
            .clamp(0.0, self.block_grid as f32) as usize;
        decoded.heights.get(gz * side + gx).copied()
    }

    pub fn update_for_camera(
        &mut self,
        camera_pos: Vec3,
        radius_blocks: i32,
    ) -> Result<bool, String> {
        let (cam_col, cam_row) = if self.is_synthetic {
            (
                (camera_pos.x / self.block_size).floor() as i32,
                (camera_pos.z / self.block_size).floor() as i32,
            )
        } else {
            let half_w = (self.block_cols as f32) * self.block_size * 0.5;
            let half_l = (self.block_rows as f32) * self.block_size * 0.5;
            (
                ((camera_pos.x + half_w) / self.block_size).floor() as i32,
                ((half_l - camera_pos.z) / self.block_size).floor() as i32,
            )
        };
        let key = (cam_row, cam_col, radius_blocks);
        if self.last_center == Some(key) {
            return Ok(false);
        }
        self.last_center = Some(key);

        let visible = if self.is_synthetic {
            streaming_blocks(
                camera_pos,
                self.block_size,
                self.block_rows,
                self.block_cols,
                radius_blocks,
            )
        } else {
            streaming_blocks_angelica(
                camera_pos,
                self.block_size,
                self.block_rows,
                self.block_cols,
                radius_blocks,
            )
        };

        let mut all_verts = Vec::new();
        let mut all_indices = Vec::new();
        let mut loaded = 0usize;
        for &(row, col) in &visible {
            let block_idx = (row * self.block_cols + col) as usize;
            if block_idx >= self.t2bk.offsets.len() {
                continue;
            }
            let cache_id = format!("{}-b{block_idx}-g{}", self.zone_name, self.block_grid);
            let content_key = TerrainStore::content_key(cache_id.as_bytes());
            let page = self.store.get_or_insert_with(&content_key, || {
                let decoded = self
                    .t2bk
                    .decode_block(&self.t2bk_bytes, block_idx, self.block_grid)
                    .map_err(|e| format!("{}_1.t2bk: {e}", self.zone_name))?;
                Ok(TerrainPage {
                    heights: decoded.heights,
                    normals: decoded.normals,
                })
            })?;
            let base_vert = all_verts.len() as u32;
            let (verts, idxs) = if self.is_synthetic {
                build_block_mesh(&page, None, row, col, self.block_size, self.block_grid)
            } else {
                build_block_mesh_angelica(
                    &page,
                    row,
                    col,
                    self.block_rows,
                    self.block_cols,
                    self.block_size,
                    self.block_grid,
                )
            };
            all_verts.extend(verts);
            all_indices.extend(idxs.into_iter().map(|i| i + base_vert));
            loaded += 1;
        }

        let mut wireframe_indices = Vec::with_capacity(all_indices.len() * 2);
        for tri in all_indices.as_chunks::<3>().0 {
            wireframe_indices.extend_from_slice(&[tri[0], tri[1], tri[1], tri[2], tri[2], tri[0]]);
        }
        self.mesh = TerrainMeshSnapshot {
            vertices: all_verts,
            indices: all_indices,
            wireframe_indices,
            revision: self.mesh.revision.wrapping_add(1),
            streamed_blocks: loaded,
            hot_ram_bytes: self.store.stats().owned_ram_bytes,
        };
        Ok(true)
    }
}

struct GpuSceneryBatch {
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    inst_buf: wgpu::Buffer,
    mat_bg: wgpu::BindGroup,
    index_count: u32,
    instance_count: u32,
}

/// Live `wgpu` viewport renderer: Sky -> Terrain -> Opaque Scenery -> Transparent Scenery -> Water.
pub struct TerrainViewportRenderer {
    sky_pipeline: wgpu::RenderPipeline,
    terrain_pipeline: wgpu::RenderPipeline,
    terrain_wireframe_pipeline: wgpu::RenderPipeline,
    scenery_opaque_pipeline: wgpu::RenderPipeline,
    scenery_transparent_pipeline: wgpu::RenderPipeline,
    vegetation_pipeline: wgpu::RenderPipeline,
    water_pipeline: wgpu::RenderPipeline,
    blit_pipeline: wgpu::RenderPipeline,
    tex_bgl: wgpu::BindGroupLayout,
    terrain_lm_bgl: wgpu::BindGroupLayout,
    scenery_mat_bgl: wgpu::BindGroupLayout,
    water_bgl: wgpu::BindGroupLayout,
    cam_buf: wgpu::Buffer,
    cam_bg: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    white_view: wgpu::TextureView,
    water_normal_view: wgpu::TextureView,
    lm_bg: wgpu::BindGroup,
    lm_revision: (u64, u64),
    scene_color: wgpu::Texture,
    scene_view: wgpu::TextureView,
    scene_depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    refl_color: wgpu::Texture,
    refl_view: wgpu::TextureView,
    refl_depth: wgpu::Texture,
    refl_depth_view: wgpu::TextureView,
    refr_color: wgpu::Texture,
    refr_view: wgpu::TextureView,
    refr_depth: wgpu::Texture,
    refr_depth_view: wgpu::TextureView,
    water_pass_size: (u32, u32),
    blit_bg: wgpu::BindGroup,
    scaled_size: (u32, u32),
    vbuf: Option<wgpu::Buffer>,
    ibuf: Option<wgpu::Buffer>,
    wibuf: Option<wgpu::Buffer>,
    index_count: u32,
    wireframe_index_count: u32,
    mesh_revision: u64,
    opaque_batches: Vec<GpuSceneryBatch>,
    transparent_batches: Vec<GpuSceneryBatch>,
    scenery_revision: u64,
    veg_batches: Vec<GpuSceneryBatch>,
    veg_revision: u64,
    pub last_world_render_passes: usize,
    pub timestamp_queries_supported: bool,
}

impl TerrainViewportRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let timestamp_queries_supported =
            device.features().contains(wgpu::Features::TIMESTAMP_QUERY);

        let initial_uniform =
            CameraUniform::build(Mat4::IDENTITY, Vec3::ZERO, 12.0, 0.0, true, 1.0, None, None);
        let cam_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("live-camera-uniform"),
            contents: bytemuck::bytes_of(&initial_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let cam_bgl = create_cam_bgl(device);
        let cam_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live-camera-bg"),
            layout: &cam_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: cam_buf.as_entire_binding(),
            }],
        });

        let tex_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("live-tex-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let terrain_lm_bgl = create_terrain_lm_bgl(device);
        let scenery_mat_bgl = create_scenery_mat_bgl(device);
        let water_bgl = create_water_bgl(device);

        let sky_pipeline = create_sky_pipeline(device, &cam_bgl);
        let scenery_opaque_pipeline =
            create_scenery_pipeline(device, &cam_bgl, &scenery_mat_bgl, false);
        let scenery_transparent_pipeline =
            create_scenery_pipeline(device, &cam_bgl, &scenery_mat_bgl, true);
        let vegetation_pipeline = create_vegetation_pipeline(device, &cam_bgl, &scenery_mat_bgl);
        let water_pipeline = create_water_pipeline(device, &cam_bgl, &water_bgl);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("live-linear-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let white_view = Self::upload_rgba_view(device, queue, 1, 1, &[255, 255, 255, 255]);
        let proc_nrm = generate_procedural_water_normal_map();
        let water_normal_view = Self::upload_rgba_view(
            device,
            queue,
            proc_nrm.width,
            proc_nrm.height,
            &proc_nrm.rgba,
        );

        let terrain_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("live-terrain-shader"),
            source: wgpu::ShaderSource::Wgsl(TERRAIN_WGSL.into()),
        });
        let terrain_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("live-terrain-layout"),
            bind_group_layouts: &[&cam_bgl, &terrain_lm_bgl],
            push_constant_ranges: &[],
        });
        let terrain_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("live-terrain-pipeline"),
            layout: Some(&terrain_layout),
            vertex: wgpu::VertexState {
                module: &terrain_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<TerrainVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 12,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 24,
                            shader_location: 2,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 40,
                            shader_location: 3,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &terrain_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let terrain_wireframe_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("live-terrain-wireframe-pipeline"),
                layout: Some(&terrain_layout),
                vertex: wgpu::VertexState {
                    module: &terrain_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<TerrainVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x3,
                                offset: 0,
                                shader_location: 0,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x3,
                                offset: 12,
                                shader_location: 1,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x4,
                                offset: 24,
                                shader_location: 2,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x2,
                                offset: 40,
                                shader_location: 3,
                            },
                        ],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &terrain_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::LineList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::Less,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview: None,
                cache: None,
            });

        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("live-blit-shader"),
            source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
        });
        let blit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("live-blit-layout"),
            bind_group_layouts: &[&tex_bgl],
            push_constant_ranges: &[],
        });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("live-blit-pipeline"),
            layout: Some(&blit_layout),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let default_lm = TerrainTextureSnapshot::default();
        let lm_bg = Self::upload_dual_lightmap_bg(
            device,
            queue,
            &terrain_lm_bgl,
            &sampler,
            &default_lm,
            &default_lm,
        );
        let (scene_color, scene_view, scene_depth, depth_view, blit_bg) =
            Self::create_scaled_targets(device, &tex_bgl, &sampler, 64, 64);
        let (refl_color, refl_view, refl_depth, refl_depth_view) =
            Self::create_aux_target(device, 32, 32, "water-refl");
        let (refr_color, refr_view, refr_depth, refr_depth_view) =
            Self::create_aux_target(device, 32, 32, "water-refr");

        Self {
            sky_pipeline,
            terrain_pipeline,
            terrain_wireframe_pipeline,
            scenery_opaque_pipeline,
            scenery_transparent_pipeline,
            vegetation_pipeline,
            water_pipeline,
            blit_pipeline,
            tex_bgl,
            terrain_lm_bgl,
            scenery_mat_bgl,
            water_bgl,
            cam_buf,
            cam_bg,
            sampler,
            white_view,
            water_normal_view,
            lm_bg,
            lm_revision: (0, 0),
            scene_color,
            scene_view,
            scene_depth,
            depth_view,
            refl_color,
            refl_view,
            refl_depth,
            refl_depth_view,
            refr_color,
            refr_view,
            refr_depth,
            refr_depth_view,
            water_pass_size: (32, 32),
            blit_bg,
            scaled_size: (64, 64),
            vbuf: None,
            ibuf: None,
            wibuf: None,
            index_count: 0,
            wireframe_index_count: 0,
            mesh_revision: 0,
            opaque_batches: Vec::new(),
            transparent_batches: Vec::new(),
            scenery_revision: 0,
            veg_batches: Vec::new(),
            veg_revision: 0,
            last_world_render_passes: 1,
            timestamp_queries_supported,
        }
    }

    fn upload_rgba_view(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        w: u32,
        h: u32,
        rgba: &[u8],
    ) -> wgpu::TextureView {
        let w = w.max(1);
        let h = h.max(1);
        let gpu_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("live-rgba-tex"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Must be Rgba8Unorm (not Srgb) because scene_color is Rgba8Unorm and blit.wgsl
            // copies it verbatim without linear->sRGB encoding; Srgb here caused pow(x, 2.2) crushing.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &gpu_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        gpu_tex.create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn upload_dual_lightmap_bg(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bgl: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        day: &TerrainTextureSnapshot,
        night: &TerrainTextureSnapshot,
    ) -> wgpu::BindGroup {
        let day_view = Self::upload_rgba_view(device, queue, day.width, day.height, &day.rgba);
        let night_view =
            Self::upload_rgba_view(device, queue, night.width, night.height, &night.rgba);
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live-dual-lm-bg"),
            layout: bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&day_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&night_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    fn create_aux_target(
        device: &wgpu::Device,
        w: u32,
        h: u32,
        label: &str,
    ) -> (
        wgpu::Texture,
        wgpu::TextureView,
        wgpu::Texture,
        wgpu::TextureView,
    ) {
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: w.max(1),
                height: h.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("aux-depth"),
            size: wgpu::Extent3d {
                width: w.max(1),
                height: h.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let cv = color.create_view(&wgpu::TextureViewDescriptor::default());
        let dv = depth.create_view(&wgpu::TextureViewDescriptor::default());
        (color, cv, depth, dv)
    }

    fn create_scaled_targets(
        device: &wgpu::Device,
        tex_bgl: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        w: u32,
        h: u32,
    ) -> (
        wgpu::Texture,
        wgpu::TextureView,
        wgpu::Texture,
        wgpu::TextureView,
        wgpu::BindGroup,
    ) {
        let (color, color_view, depth, depth_view) =
            Self::create_aux_target(device, w, h, "live-scaled-color");
        let blit_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live-blit-bg"),
            layout: tex_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        (color, color_view, depth, depth_view, blit_bg)
    }

    fn upload_scenery_batches(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        src: &[SceneryDrawBatch],
    ) -> Vec<GpuSceneryBatch> {
        let mut out = Vec::with_capacity(src.len());
        for b in src {
            if b.vertices.is_empty() || b.indices.is_empty() || b.instances.is_empty() {
                continue;
            }
            let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("live-scenery-vbuf"),
                contents: bytemuck::cast_slice(&b.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("live-scenery-ibuf"),
                contents: bytemuck::cast_slice(&b.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
            let inst_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("live-scenery-inst"),
                contents: bytemuck::cast_slice(&b.instances),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let mat_u = SceneryMaterialUniform {
                diffuse_rgba: b.diffuse_rgba,
                params: [
                    b.alpha_cutoff,
                    if b.has_vertex_lighting { 1.0 } else { 0.0 },
                    0.0,
                    0.0,
                ],
            };
            let mat_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("live-scenery-mat"),
                contents: bytemuck::bytes_of(&mat_u),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let owned_view = b
                .texture
                .as_ref()
                .map(|t| Self::upload_rgba_view(device, queue, t.width, t.height, &t.rgba));
            let tex_view_ref = owned_view.as_ref().unwrap_or(&self.white_view);
            let mat_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("live-scenery-mat-bg"),
                layout: &self.scenery_mat_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(tex_view_ref),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: mat_buf.as_entire_binding(),
                    },
                ],
            });
            out.push(GpuSceneryBatch {
                vbuf,
                ibuf,
                inst_buf,
                mat_bg,
                index_count: b.indices.len() as u32,
                instance_count: b.instances.len() as u32,
            });
        }
        out
    }

    fn upload_vegetation_batches(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        src: &[VegetationDrawBatch],
    ) -> Vec<GpuSceneryBatch> {
        let mut out = Vec::with_capacity(src.len());
        for b in src {
            if b.vertices.is_empty() || b.indices.is_empty() || b.instances.is_empty() {
                continue;
            }
            let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("live-veg-vbuf"),
                contents: bytemuck::cast_slice(&b.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("live-veg-ibuf"),
                contents: bytemuck::cast_slice(&b.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
            let inst_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("live-veg-inst"),
                contents: bytemuck::cast_slice(&b.instances),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let mat_u = VegMaterialUniform {
                base_color: b.base_color,
                params: [b.alpha_cutoff, b.wind_amplitude, 0.0, 0.0],
            };
            let mat_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("live-veg-mat"),
                contents: bytemuck::bytes_of(&mat_u),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let owned_view = b
                .texture
                .as_ref()
                .map(|t| Self::upload_rgba_view(device, queue, t.width, t.height, &t.rgba));
            let tex_view_ref = owned_view.as_ref().unwrap_or(&self.white_view);
            let mat_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("live-veg-mat-bg"),
                layout: &self.scenery_mat_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(tex_view_ref),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: mat_buf.as_entire_binding(),
                    },
                ],
            });
            out.push(GpuSceneryBatch {
                vbuf,
                ibuf,
                inst_buf,
                mat_bg,
                index_count: b.indices.len() as u32,
                instance_count: b.instances.len() as u32,
            });
        }
        out
    }

    fn draw_world_geometry<'a>(
        &'a self,
        rpass: &mut wgpu::RenderPass<'a>,
        include_transparent: bool,
        draw_sky_gradient: bool,
        terrain_has_real_textures: bool,
        terrain_wireframe_debug: bool,
    ) {
        // 1. Sky dome/gradient (disabled in real-world mode until surfaces.pck skyboxes are decoded)
        if draw_sky_gradient {
            rpass.set_pipeline(&self.sky_pipeline);
            rpass.set_bind_group(0, &self.cam_bg, &[]);
            rpass.draw(0..3, 0..1);
        }

        // 2. Terrain: draw textured solid ONLY when real layer textures resolved;
        //    otherwise draw nothing unless explicit wireframe debug mode is enabled.
        if terrain_has_real_textures {
            if let (Some(vbuf), Some(ibuf)) = (&self.vbuf, &self.ibuf) {
                if self.index_count > 0 {
                    rpass.set_pipeline(&self.terrain_pipeline);
                    rpass.set_bind_group(0, &self.cam_bg, &[]);
                    rpass.set_bind_group(1, &self.lm_bg, &[]);
                    rpass.set_vertex_buffer(0, vbuf.slice(..));
                    rpass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
                    rpass.draw_indexed(0..self.index_count, 0, 0..1);
                }
            }
        } else if terrain_wireframe_debug {
            if let (Some(vbuf), Some(wibuf)) = (&self.vbuf, &self.wibuf) {
                if self.wireframe_index_count > 0 {
                    rpass.set_pipeline(&self.terrain_wireframe_pipeline);
                    rpass.set_bind_group(0, &self.cam_bg, &[]);
                    rpass.set_bind_group(1, &self.lm_bg, &[]);
                    rpass.set_vertex_buffer(0, vbuf.slice(..));
                    rpass.set_index_buffer(wibuf.slice(..), wgpu::IndexFormat::Uint32);
                    rpass.draw_indexed(0..self.wireframe_index_count, 0, 0..1);
                }
            }
        }

        // 3. Opaque & alpha-tested static scenery
        if !self.opaque_batches.is_empty() {
            rpass.set_pipeline(&self.scenery_opaque_pipeline);
            rpass.set_bind_group(0, &self.cam_bg, &[]);
            for b in &self.opaque_batches {
                rpass.set_bind_group(1, &b.mat_bg, &[]);
                rpass.set_vertex_buffer(0, b.vbuf.slice(..));
                rpass.set_vertex_buffer(1, b.inst_buf.slice(..));
                rpass.set_index_buffer(b.ibuf.slice(..), wgpu::IndexFormat::Uint16);
                rpass.draw_indexed(0..b.index_count, 0, 0..b.instance_count);
            }
        }

        // 3b. Instanced vegetation (SpeedTree trunks/leaves + grass, alpha-tested)
        if !self.veg_batches.is_empty() {
            rpass.set_pipeline(&self.vegetation_pipeline);
            rpass.set_bind_group(0, &self.cam_bg, &[]);
            for b in &self.veg_batches {
                rpass.set_bind_group(1, &b.mat_bg, &[]);
                rpass.set_vertex_buffer(0, b.vbuf.slice(..));
                rpass.set_vertex_buffer(1, b.inst_buf.slice(..));
                rpass.set_index_buffer(b.ibuf.slice(..), wgpu::IndexFormat::Uint16);
                rpass.draw_indexed(0..b.index_count, 0, 0..b.instance_count);
            }
        }

        // 4. Transparent static scenery (sorted back-to-front, blend ON, depth write OFF)
        if include_transparent && !self.transparent_batches.is_empty() {
            rpass.set_pipeline(&self.scenery_transparent_pipeline);
            rpass.set_bind_group(0, &self.cam_bg, &[]);
            for b in &self.transparent_batches {
                rpass.set_bind_group(1, &b.mat_bg, &[]);
                rpass.set_vertex_buffer(0, b.vbuf.slice(..));
                rpass.set_vertex_buffer(1, b.inst_buf.slice(..));
                rpass.set_index_buffer(b.ibuf.slice(..), wgpu::IndexFormat::Uint16);
                rpass.draw_indexed(0..b.index_count, 0, 0..b.instance_count);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport_w: u32,
        viewport_h: u32,
        scale_pct: u16,
        uniform: &CameraUniform,
        mesh: &TerrainMeshSnapshot,
        day_lightmap: &TerrainTextureSnapshot,
        night_lightmap: &TerrainTextureSnapshot,
        scenery: &SceneryDrawSnapshot,
        vegetation: &VegetationDrawSnapshot,
        water: &WaterRenderSnapshot,
        draw_sky_gradient: bool,
        terrain_has_real_textures: bool,
        terrain_wireframe_debug: bool,
    ) -> wgpu::CommandBuffer {
        let desired = scaled_render_target(viewport_w, viewport_h, scale_pct);
        if self.scaled_size != desired {
            let (c, cv, d, dv, bg) = Self::create_scaled_targets(
                device,
                &self.tex_bgl,
                &self.sampler,
                desired.0,
                desired.1,
            );
            self.scene_color = c;
            self.scene_view = cv;
            self.scene_depth = d;
            self.depth_view = dv;
            self.blit_bg = bg;
            self.scaled_size = desired;
        }

        let water_pct = water.quality.pass_scale_pct();
        let desired_water_sz = scaled_render_target(desired.0, desired.1, water_pct);
        if self.water_pass_size != desired_water_sz {
            let (rc, rcv, rd, rdv) = Self::create_aux_target(
                device,
                desired_water_sz.0,
                desired_water_sz.1,
                "water-refl",
            );
            let (fc, fcv, fd, fdv) = Self::create_aux_target(
                device,
                desired_water_sz.0,
                desired_water_sz.1,
                "water-refr",
            );
            self.refl_color = rc;
            self.refl_view = rcv;
            self.refl_depth = rd;
            self.refl_depth_view = rdv;
            self.refr_color = fc;
            self.refr_view = fcv;
            self.refr_depth = fd;
            self.refr_depth_view = fdv;
            self.water_pass_size = desired_water_sz;
        }

        queue.write_buffer(&self.cam_buf, 0, bytemuck::bytes_of(uniform));

        let lm_rev = (day_lightmap.revision, night_lightmap.revision);
        if self.lm_revision != lm_rev {
            self.lm_bg = Self::upload_dual_lightmap_bg(
                device,
                queue,
                &self.terrain_lm_bgl,
                &self.sampler,
                day_lightmap,
                night_lightmap,
            );
            self.lm_revision = lm_rev;
        }

        if self.mesh_revision != mesh.revision {
            if mesh.vertices.is_empty() || mesh.indices.is_empty() {
                self.vbuf = None;
                self.ibuf = None;
                self.wibuf = None;
                self.index_count = 0;
                self.wireframe_index_count = 0;
            } else {
                self.vbuf = Some(
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("live-terrain-vbuf"),
                        contents: bytemuck::cast_slice(&mesh.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                );
                self.ibuf = Some(
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("live-terrain-ibuf"),
                        contents: bytemuck::cast_slice(&mesh.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                );
                self.index_count = mesh.indices.len() as u32;
                if mesh.wireframe_indices.is_empty() {
                    self.wibuf = None;
                    self.wireframe_index_count = 0;
                } else {
                    self.wibuf = Some(device.create_buffer_init(
                        &wgpu::util::BufferInitDescriptor {
                            label: Some("live-terrain-wibuf"),
                            contents: bytemuck::cast_slice(&mesh.wireframe_indices),
                            usage: wgpu::BufferUsages::INDEX,
                        },
                    ));
                    self.wireframe_index_count = mesh.wireframe_indices.len() as u32;
                }
            }
            self.mesh_revision = mesh.revision;
        }

        if self.scenery_revision != scenery.revision {
            self.opaque_batches =
                self.upload_scenery_batches(device, queue, &scenery.opaque_batches);
            self.transparent_batches =
                self.upload_scenery_batches(device, queue, &scenery.transparent_batches);
            self.scenery_revision = scenery.revision;
        }

        if self.veg_revision != vegetation.revision {
            self.veg_batches = self.upload_vegetation_batches(device, queue, &vegetation.batches);
            self.veg_revision = vegetation.revision;
        }

        let vp = Mat4::from_cols_array_2d(&uniform.view_proj);
        let vis_water = atmosphere::visible_water_planes(
            &water.planes,
            vp,
            desired_water_sz.0,
            desired_water_sz.1,
        );

        let mut world_passes = 0usize;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("live-world-encoder"),
        });

        // Optional scissored water reflection & refraction passes ONLY when water is visible
        if !vis_water.is_empty() {
            let union_scissor = vis_water.iter().fold(vis_water[0].1, |acc, &(_, r)| {
                let x0 = acc.x.min(r.x);
                let y0 = acc.y.min(r.y);
                let x1 = (acc.x + acc.width).max(r.x + r.width);
                let y1 = (acc.y + acc.height).max(r.y + r.height);
                ScissorRect {
                    x: x0,
                    y: y0,
                    width: x1 - x0,
                    height: y1 - y0,
                }
            });

            if water.reflection_enabled {
                world_passes += 1;
                let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("water-reflection-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.refl_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.refl_depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                rpass.set_scissor_rect(
                    union_scissor.x,
                    union_scissor.y,
                    union_scissor.width.max(1),
                    union_scissor.height.max(1),
                );
                self.draw_world_geometry(
                    &mut rpass,
                    false,
                    draw_sky_gradient,
                    terrain_has_real_textures,
                    terrain_wireframe_debug,
                );
            }

            if water.refraction_enabled {
                world_passes += 1;
                let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("water-refraction-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.refr_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.refr_depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                rpass.set_scissor_rect(
                    union_scissor.x,
                    union_scissor.y,
                    union_scissor.width.max(1),
                    union_scissor.height.max(1),
                );
                self.draw_world_geometry(
                    &mut rpass,
                    false,
                    draw_sky_gradient,
                    terrain_has_real_textures,
                    terrain_wireframe_debug,
                );
            }
        }

        // Main world pass: Sky -> Terrain -> Opaque Scenery -> Transparent Scenery -> Water
        world_passes += 1;
        let mut water_draws = Vec::new();
        for &(idx, _) in &vis_water {
            let plane = &water.planes[idx];
            let (wv, wi) = build_water_plane_mesh(plane);
            let wvb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("water-vbuf"),
                contents: bytemuck::cast_slice(&wv),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let wib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("water-ibuf"),
                contents: bytemuck::cast_slice(&wi),
                usage: wgpu::BufferUsages::INDEX,
            });
            let wu = WaterUniform {
                water_color: plane.color_rgba,
                flags: [
                    if water.reflection_enabled { 1.0 } else { 0.0 },
                    if water.refraction_enabled { 1.0 } else { 0.0 },
                    1.0,
                    1.0,
                ],
            };
            let wub = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("water-uniform"),
                contents: bytemuck::bytes_of(&wu),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let wbg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("water-bg"),
                layout: &self.water_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&self.water_normal_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&self.refl_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&self.refr_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wub.as_entire_binding(),
                    },
                ],
            });
            water_draws.push((wvb, wib, wbg, wi.len() as u32));
        }

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("live-world-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.scene_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            self.draw_world_geometry(
                &mut rpass,
                true,
                draw_sky_gradient,
                terrain_has_real_textures,
                terrain_wireframe_debug,
            );

            // 5. Water planes (after opaque & transparent scenery, depth-tested against terrain)
            if !water_draws.is_empty() {
                rpass.set_pipeline(&self.water_pipeline);
                rpass.set_bind_group(0, &self.cam_bg, &[]);
                for (wvb, wib, wbg, icnt) in &water_draws {
                    rpass.set_bind_group(1, wbg, &[]);
                    rpass.set_vertex_buffer(0, wvb.slice(..));
                    rpass.set_index_buffer(wib.slice(..), wgpu::IndexFormat::Uint16);
                    rpass.draw_indexed(0..*icnt, 0, 0..1);
                }
            }
        }

        self.last_world_render_passes = world_passes;
        encoder.finish()
    }

    pub fn paint_blit(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        render_pass.set_pipeline(&self.blit_pipeline);
        render_pass.set_bind_group(0, &self.blit_bg, &[]);
        render_pass.draw(0..3, 0..1);
    }
}
