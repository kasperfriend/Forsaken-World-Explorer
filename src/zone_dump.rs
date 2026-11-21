//! Real-zone diagnostic dumper (`--dump-zone <zone>`) and shared client archive
//! path resolution helpers.
//!
//! Reads a real Forsaken World client directory stage-by-stage using the exact same
//! readers the viewer uses, and emits verifiable per-stage counts, terrain height
//! samples, texture dimensions, placement counts, and `.bmd` submesh statistics.

use std::{collections::BTreeSet, fmt::Write as FmtWrite, fs, path::Path};

use crate::{
    atmosphere,
    bmd::BmdModel,
    dxt,
    pck::PckArchive,
    terrain_block::{T2bkFile, T2lmHeader, T2mkHeader, Trn2Header},
    vegetation, world_objects,
};

/// Normalize a client texture path for lookup inside `textures.pck`, `litmodels.pck`,
/// or `building.pck`. Real Forsaken World `.pck` archives omit their mount prefix
/// (e.g. `Textures\maps\...` is stored inside `textures.pck` as `maps/...`).
pub fn texture_lookup_candidates(raw_path: &str) -> Vec<String> {
    let norm = raw_path.replace('\\', "/");
    let trimmed = norm.trim_start_matches("./").trim_start_matches('/');
    let lower = trimmed.to_ascii_lowercase();
    let mut out = Vec::new();
    for prefix in ["textures/", "litmodels/", "building/", "surfaces/"] {
        if lower.starts_with(prefix) {
            out.push(trimmed[prefix.len()..].to_string());
        }
    }
    out.push(trimmed.to_string());
    out.push(format!("textures/{trimmed}"));
    out.push(format!("litmodels/{trimmed}"));
    out
}

/// Build lookup candidates for a static `.bmd` model referenced by `<zone>.ecbsd`
/// (e.g. `1\litmodel_37.bmd`). Real `litmodels.pck` omits the `litmodels\` mount prefix,
/// so `<zone>/<model_key>` (`s0/1/litmodel_37.bmd`) is checked first.
pub fn bmd_lookup_candidates(zone: &str, raw_path: &str) -> Vec<String> {
    let norm = raw_path.replace('\\', "/");
    let trimmed = norm.trim_start_matches("./").trim_start_matches('/');
    let lower = trimmed.to_ascii_lowercase();
    let mut out = vec![
        format!("{zone}/{trimmed}"),
        trimmed.to_string(),
        format!("litmodels/{zone}/{trimmed}"),
        format!("litmodels/{trimmed}"),
        format!("building/{trimmed}"),
    ];
    for prefix in ["litmodels/", "building/"] {
        if lower.starts_with(prefix) {
            out.push(trimmed[prefix.len()..].to_string());
        }
    }
    out
}

/// Sample real terrain height at Angelica world coordinates `(x, z)` from decoded blocks.
#[allow(clippy::too_many_arguments)]
pub fn sample_angelica_height(
    blocks: &[(Vec<f32>, u32)],
    world_width: f32,
    world_length: f32,
    block_size: f32,
    block_rows: u32,
    block_cols: u32,
    x: f32,
    z: f32,
) -> Option<f32> {
    if block_rows == 0 || block_cols == 0 || block_size <= 0.0 {
        return None;
    }
    let fx = (x + world_width * 0.5) / block_size;
    let fz = (world_length * 0.5 - z) / block_size;
    let col = (fx.floor() as i32).clamp(0, block_cols as i32 - 1) as u32;
    let row = (fz.floor() as i32).clamp(0, block_rows as i32 - 1) as u32;
    let idx = (row * block_cols + col) as usize;
    let (heights, grid) = blocks.get(idx)?;
    let side = (*grid as usize) + 1;
    let gx = ((fx - col as f32) * (*grid as f32))
        .round()
        .clamp(0.0, *grid as f32) as usize;
    let gz = ((fz - row as f32) * (*grid as f32))
        .round()
        .clamp(0.0, *grid as f32) as usize;
    heights.get(gz * side + gx).copied()
}

#[derive(Debug, Clone, Default)]
pub struct ZoneDumpSummary {
    pub zone: String,
    pub trn2_ok: bool,
    pub block_rows: u32,
    pub block_cols: u32,
    pub decoded_blocks: usize,
    pub height_min: f32,
    pub height_max: f32,
    pub height_at_origin: Option<f32>,
    pub lightmap_areas: usize,
    pub lightmap_size: u32,
    pub mask_areas: usize,
    pub mask_layers: usize,
    pub trn2_textures_total: usize,
    pub trn2_textures_decoded: usize,
    pub placements_total: usize,
    pub unique_models: usize,
    pub bmd_resolved: usize,
    pub bmd_failed: usize,
    pub bmd_sample_submeshes: usize,
    pub trees_total: usize,
    pub grass_types: usize,
    pub grass_placements: usize,
    pub box_areas: usize,
    pub water_planes: usize,
    pub report_text: String,
}

/// Run the full stage-by-stage real-zone decoder dump against `client_root/maps/<zone>/`
/// and `client_root/resources/*.pck`. Never panics; records every missing file and error.
pub fn dump_zone(client_root: &Path, zone: &str) -> ZoneDumpSummary {
    let mut out = String::new();
    let mut summary = ZoneDumpSummary {
        zone: zone.to_string(),
        height_min: f32::INFINITY,
        height_max: f32::NEG_INFINITY,
        ..Default::default()
    };

    let _ = writeln!(out, "=== Forsaken World Explorer Zone Dump: {zone} ===");
    let _ = writeln!(out, "Client root: {}", client_root.display());

    let zone_dir = client_root.join("maps").join(zone);
    let res_dir = client_root.join("resources");

    // Open optional PCK archives
    let mut tex_pck = PckArchive::open(&res_dir.join("textures.pck")).ok();
    let mut bmd_pcks: Vec<(&str, PckArchive)> = Vec::new();
    for name in ["litmodels.pck", "building.pck"] {
        if let Ok(a) = PckArchive::open(&res_dir.join(name)) {
            bmd_pcks.push((name, a));
        }
    }

    // ---------------- Stage 1: Terrain ----------------
    let _ = writeln!(out, "\n[Stage 1] Terrain & Lightmaps");
    let trn2_path = zone_dir.join(format!("{zone}.trn2"));
    let t2bk_path = zone_dir.join(format!("{zone}_1.t2bk"));
    let mut decoded_blocks: Vec<(Vec<f32>, u32)> = Vec::new();
    let mut trn2_dims: Option<(f32, f32, f32, u32, u32, u32)> = None;

    match fs::read(&trn2_path).and_then(|b| {
        Trn2Header::parse(&b).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }) {
        Ok(trn2) => {
            summary.trn2_ok = true;
            summary.block_rows = trn2.block_rows;
            summary.block_cols = trn2.block_cols;
            summary.trn2_textures_total = trn2.texture_paths.len();
            trn2_dims = Some((
                trn2.width,
                trn2.length,
                trn2.block_size,
                trn2.block_grid,
                trn2.block_rows,
                trn2.block_cols,
            ));
            let _ = writeln!(
                out,
                "  {zone}.trn2: ver={} size={}x{}m block_size={}m grid={} blocks={}x{} ({}) mask_areas={} textures={}",
                trn2.version,
                trn2.width,
                trn2.length,
                trn2.block_size,
                trn2.block_grid,
                trn2.block_rows,
                trn2.block_cols,
                trn2.num_blocks,
                trn2.mask_areas,
                trn2.texture_paths.len()
            );
            for (i, tp) in trn2.texture_paths.iter().enumerate() {
                let mut status = "textures.pck not found".to_string();
                if let Some(arch) = tex_pck.as_mut() {
                    let mut found = None;
                    for cand in texture_lookup_candidates(tp) {
                        if let Ok(bytes) = arch.read_entry(&cand) {
                            if let Ok(dds) = dxt::parse_dds(&bytes) {
                                found = Some((cand, dds.width, dds.height, dds.format));
                                break;
                            }
                        }
                    }
                    if let Some((cand, w, h, fmt)) = found {
                        summary.trn2_textures_decoded += 1;
                        status = format!("OK via '{cand}' ({w}x{h} {fmt:?})");
                    } else {
                        status = "MISSING in textures.pck".to_string();
                    }
                }
                let _ = writeln!(out, "    layer[{i}]: {tp} -> {status}");
            }
        }
        Err(e) => {
            let _ = writeln!(out, "  ERROR reading {}: {e}", trn2_path.display());
        }
    }

    if let Some((w, l, bs, grid, rows, cols)) = trn2_dims {
        match fs::read(&t2bk_path) {
            Ok(bytes) => match T2bkFile::parse_header(&bytes) {
                Ok(t2bk) => {
                    let mut err_count = 0usize;
                    for idx in 0..t2bk.offsets.len() {
                        match t2bk.decode_block(&bytes, idx, grid) {
                            Ok(blk) => {
                                for &h in &blk.heights {
                                    summary.height_min = summary.height_min.min(h);
                                    summary.height_max = summary.height_max.max(h);
                                }
                                decoded_blocks.push((blk.heights, grid));
                            }
                            Err(_) => {
                                err_count += 1;
                                decoded_blocks.push((Vec::new(), grid));
                            }
                        }
                    }
                    summary.decoded_blocks = decoded_blocks.len().saturating_sub(err_count);
                    let _ = writeln!(
                        out,
                        "  {zone}_1.t2bk: ver={} flags=0x{:02x} decoded={}/{} blocks height_range=[{:.2}, {:.2}]m",
                        t2bk.version,
                        t2bk.flags,
                        summary.decoded_blocks,
                        t2bk.offsets.len(),
                        summary.height_min,
                        summary.height_max
                    );
                    let probes = [
                        (0.0f32, 0.0f32),
                        (-128.0, 128.0),
                        (128.0, -128.0),
                        (-306.17, 252.98),
                    ];
                    for (px, pz) in probes {
                        if let Some(h) =
                            sample_angelica_height(&decoded_blocks, w, l, bs, rows, cols, px, pz)
                        {
                            if px == 0.0 && pz == 0.0 {
                                summary.height_at_origin = Some(h);
                            }
                            let _ = writeln!(out, "    height({px:.2}, {pz:.2}) = {h:.2} m");
                        }
                    }
                }
                Err(e) => {
                    let _ = writeln!(out, "  ERROR parsing {}: {e}", t2bk_path.display());
                }
            },
            Err(e) => {
                let _ = writeln!(out, "  ERROR reading {}: {e}", t2bk_path.display());
            }
        }
    }

    // Lightmaps
    for suffix in ["t2lm", "t2lm1"] {
        let p = zone_dir.join(format!("{zone}_1.{suffix}"));
        match fs::read(&p) {
            Ok(bytes) => match T2lmHeader::parse(&bytes) {
                Ok(lm) => {
                    if suffix == "t2lm" {
                        summary.lightmap_areas = lm.num_areas as usize;
                        summary.lightmap_size = lm.lm_size;
                    }
                    let dds_info = lm
                        .area_bytes(&bytes, 0)
                        .and_then(dxt::parse_dds)
                        .map(|d| format!("{}x{} {:?}", d.width, d.height, d.format))
                        .unwrap_or_else(|e| format!("DDS error: {e}"));
                    let _ = writeln!(
                        out,
                        "  {zone}_1.{suffix}: ver={} areas={} lm_size={} area[0]={dds_info}",
                        lm.version, lm.num_areas, lm.lm_size
                    );
                }
                Err(e) => {
                    let _ = writeln!(out, "  ERROR parsing {}: {e}", p.display());
                }
            },
            Err(e) => {
                let _ = writeln!(out, "  missing {}: {e}", p.display());
            }
        }
    }

    // Mask areas
    let t2mk_path = zone_dir.join(format!("{zone}_1.t2mk"));
    match fs::read(&t2mk_path) {
        Ok(bytes) => match T2mkHeader::parse(&bytes) {
            Ok(mk) => {
                summary.mask_areas = mk.num_areas as usize;
                let mut total_layers = 0usize;
                let mut sample_mask_dim = 0u32;
                for mi in 0..mk.num_areas as usize {
                    if let Ok(layers) = mk.area_layers(&bytes, mi) {
                        total_layers += layers.len();
                        if sample_mask_dim == 0 {
                            for lyr in &layers {
                                if lyr.data_len > 0 {
                                    if let Ok((side, _)) = mk.decode_layer_mask(&bytes, lyr) {
                                        sample_mask_dim = side;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                summary.mask_layers = total_layers;
                let _ = writeln!(
                    out,
                    "  {zone}_1.t2mk: ver={} mask_areas={} total_layers={} sample_mask={}x{}",
                    mk.version, mk.num_areas, total_layers, sample_mask_dim, sample_mask_dim
                );
            }
            Err(e) => {
                let _ = writeln!(out, "  ERROR parsing {}: {e}", t2mk_path.display());
            }
        },
        Err(e) => {
            let _ = writeln!(out, "  missing {}: {e}", t2mk_path.display());
        }
    }

    // ---------------- Stage 2: Placements & Buildings ----------------
    let _ = writeln!(out, "\n[Stage 2] World Placements & Static .bmd Buildings");
    let wld_path = zone_dir.join(format!("{zone}.ecwld"));
    let bsd_path = zone_dir.join(format!("{zone}.ecbsd"));
    let wld_bytes = fs::read(&wld_path).ok();
    let bsd_bytes = fs::read(&bsd_path).ok();

    if let (Some(wld), Some(bsd)) = (wld_bytes.as_deref(), bsd_bytes.as_deref()) {
        match world_objects::parse_ecwld_ecbsd_ornaments(wld, bsd) {
            Ok((wh, bh, placements)) => {
                summary.placements_total = placements.len();
                let unique: BTreeSet<String> =
                    placements.iter().map(|p| p.asset_path.clone()).collect();
                summary.unique_models = unique.len();
                let _ = writeln!(
                    out,
                    "  {zone}.ecwld (v{}) + {zone}.ecbsd (v{}): {} unique placements, {} unique .bmd models",
                    wh.version,
                    bh.version,
                    placements.len(),
                    unique.len()
                );
                for p in placements.iter().take(5) {
                    let h_note = trn2_dims
                        .and_then(|(w, l, bs, _, rows, cols)| {
                            sample_angelica_height(
                                &decoded_blocks,
                                w,
                                l,
                                bs,
                                rows,
                                cols,
                                p.position[0],
                                p.position[2],
                            )
                        })
                        .map(|h| format!(" (terrain_h={h:.2}m, dy={:+.2}m)", p.position[1] - h))
                        .unwrap_or_default();
                    let _ = writeln!(
                        out,
                        "    placement id={} pos=({:.2}, {:.2}, {:.2}){} path={}",
                        p.export_id,
                        p.position[0],
                        p.position[1],
                        p.position[2],
                        h_note,
                        p.asset_path
                    );
                }
                if bmd_pcks.is_empty() {
                    let _ = writeln!(
                        out,
                        "  NOTE: resources/litmodels.pck and resources/building.pck not present; skipping .bmd mesh decode."
                    );
                } else {
                    for model_key in unique.iter().take(25) {
                        let mut raw_found = None;
                        for (arch_name, arch) in bmd_pcks.iter_mut() {
                            for cand in bmd_lookup_candidates(zone, model_key) {
                                if let Ok(bytes) = arch.read_entry(&cand) {
                                    raw_found = Some((*arch_name, cand, bytes));
                                    break;
                                }
                            }
                            if raw_found.is_some() {
                                break;
                            }
                        }
                        if let Some((arch_name, cand, bytes)) = raw_found {
                            match BmdModel::parse(&bytes) {
                                Ok(model) => {
                                    summary.bmd_resolved += 1;
                                    if summary.bmd_resolved <= 5 {
                                        summary.bmd_sample_submeshes += model.submeshes.len();
                                        let verts: usize =
                                            model.submeshes.iter().map(|s| s.vertices.len()).sum();
                                        let tris: usize = model
                                            .submeshes
                                            .iter()
                                            .map(|s| s.indices.len() / 3)
                                            .sum();
                                        let _ = writeln!(
                                            out,
                                            "    bmd OK [{arch_name}:{cand}]: submeshes={} verts={} tris={}",
                                            model.submeshes.len(),
                                            verts,
                                            tris
                                        );
                                    }
                                }
                                Err(e) => {
                                    summary.bmd_failed += 1;
                                    let _ = writeln!(out, "    bmd DECODE ERROR [{cand}]: {e}");
                                }
                            }
                        } else {
                            summary.bmd_failed += 1;
                        }
                    }
                    let _ = writeln!(
                        out,
                        "  Sampled first 25 unique models: resolved={}, missing/failed={}",
                        summary.bmd_resolved, summary.bmd_failed
                    );
                }
            }
            Err(e) => {
                let _ = writeln!(out, "  ERROR parsing placements: {e}");
            }
        }

        // ---------------- Stage 3: Trees & Grass ----------------
        let _ = writeln!(out, "\n[Stage 3] Vegetation (Trees & Grass)");
        match vegetation::parse_ecwld_ecbsd_vegetation(wld, bsd) {
            Ok((_, _, tree_types, trees, grass_types, grasses)) => {
                summary.trees_total = trees.len();
                summary.grass_types = grass_types.len();
                summary.grass_placements = grasses.len();
                let dll_status = match vegetation::verify_client_speedtree_dll(client_root) {
                    Ok(sha) => format!("verified ({})", &sha[..16]),
                    Err(e) => format!("missing/untrusted ({e})"),
                };
                let _ = writeln!(
                    out,
                    "  Trees: {} types, {} placements | speedtreert.dll: {dll_status}",
                    tree_types.len(),
                    trees.len()
                );
                let _ = writeln!(
                    out,
                    "  Grass: {} types, {} placements",
                    grass_types.len(),
                    grasses.len()
                );
            }
            Err(e) => {
                let _ = writeln!(out, "  ERROR parsing vegetation: {e}");
            }
        }

        // ---------------- Stage 4: Atmosphere, Water, Effects & Sky ----------------
        let _ = writeln!(out, "\n[Stage 4] Atmosphere, Water, Effects & Sky");
        match atmosphere::parse_ecwld_ecbsd_atmosphere_and_water(wld, bsd) {
            Ok((_, _, box_areas, water_refs)) => {
                summary.box_areas = box_areas.len();
                summary.water_planes = water_refs.len();
                let _ = writeln!(
                    out,
                    "  Atmosphere box areas: {} | Water refs: {}",
                    box_areas.len(),
                    water_refs.len()
                );
            }
            Err(e) => {
                let _ = writeln!(out, "  ERROR parsing atmosphere/water: {e}");
            }
        }
        let _ = writeln!(
            out,
            "  Sky: missing (surfaces.pck skybox textures not decoded yet — Issue #15)"
        );
        let _ = writeln!(
            out,
            "  Effects: missing (ECWLD effects / gfx.pck not decoded yet)"
        );
    } else {
        let _ = writeln!(
            out,
            "  ERROR: missing {} or {}",
            wld_path.display(),
            bsd_path.display()
        );
    }

    summary.report_text = out;
    summary
}
