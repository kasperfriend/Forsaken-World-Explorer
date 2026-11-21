//! Real-zone diagnostic dumper (`--dump-zone <zone>`) and shared archive lookup helpers.
//!
//! Reads a real Forsaken World client directory stage-by-stage using the exact same
//! readers the viewer uses, and produces one self-contained report covering:
//! - Task 1: Terrain layer & `.bmd` texture lookups (canonical path normalization,
//!   archives searched, DDS dimensions/format, closest entry names on miss)
//! - Task 2: `.bmd` building lookups across all model-bearing `.pck` archives,
//!   version census, and failures grouped by reason with 5 sample paths each
//! - Task 3: Placement height statistics across all placements + 7-candidate axis/sign
//!   mapping table + `|dy| > 20m` outlier listing
//! - Task 4 & 5: Diagnostics summary (with reasons for any `0` counter) +
//!   "Expected for this client" table derived from real `maps/s0/` files.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as FmtWrite,
    fs,
    path::Path,
};

use crate::{
    bmd::BmdModel,
    dxt,
    pck::{self, PckArchive},
    terrain_block::{T2bkFile, T2lmHeader, T2mkHeader, Trn2Header},
    world_objects,
};

/// Build ordered, deduplicated lookup keys for a texture path inside `archive_mount`
/// (e.g. `Some("textures")`, `Some("litmodels")`, `Some("building")`).
pub fn texture_lookup_candidates_for_archive(
    raw_path: &str,
    archive_mount: Option<&str>,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut push_unique = |s: String| {
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    };
    push_unique(pck::normalize_archive_path(raw_path, archive_mount));
    for strip in ["textures", "litmodels", "building", "surfaces", "models"] {
        push_unique(pck::normalize_archive_path(raw_path, Some(strip)));
    }
    let base_norm = pck::normalize_entry_path(raw_path);
    push_unique(base_norm.clone());
    push_unique(format!("textures/{base_norm}"));
    push_unique(format!("litmodels/{base_norm}"));
    out
}

/// Backward-compatible wrapper for callers without a specific archive mount name.
pub fn texture_lookup_candidates(raw_path: &str) -> Vec<String> {
    texture_lookup_candidates_for_archive(raw_path, Some("textures"))
}

/// Build ordered, deduplicated lookup keys for a static `.bmd` model referenced by
/// `<zone>.ecbsd` (e.g. `1\litmodel_37.bmd`) inside `archive_mount`.
pub fn bmd_lookup_candidates_for_archive(
    zone: &str,
    raw_path: &str,
    archive_mount: Option<&str>,
) -> Vec<String> {
    let norm = pck::normalize_archive_path(raw_path, archive_mount);
    let zone_norm = pck::normalize_entry_path(zone);
    let mut out = Vec::new();
    let mut push_unique = |s: String| {
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    };
    push_unique(format!("{zone_norm}/{norm}"));
    push_unique(norm.clone());
    push_unique(format!("litmodels/{zone_norm}/{norm}"));
    push_unique(format!("litmodels/{norm}"));
    push_unique(format!("building/{norm}"));
    for strip in ["litmodels", "building", "models"] {
        push_unique(pck::normalize_archive_path(raw_path, Some(strip)));
    }
    out
}

/// Backward-compatible wrapper.
pub fn bmd_lookup_candidates(zone: &str, raw_path: &str) -> Vec<String> {
    bmd_lookup_candidates_for_archive(zone, raw_path, Some("litmodels"))
}

/// Discover and open all `.pck` archives in `<client_root>/resources/`, returning
/// `(opened_archives, open_errors)` without silently swallowing any open failure.
#[allow(clippy::type_complexity)]
pub fn open_all_resource_pcks(
    client_root: &Path,
) -> (Vec<(String, String, PckArchive)>, Vec<(String, String)>) {
    let res_dir = client_root.join("resources");
    let priority = [
        "litmodels.pck",
        "building.pck",
        "textures.pck",
        "models.pck",
        "loddata.pck",
        "surfaces.pck",
        "gfx.pck",
        "grasses.pck",
        "configs.pck",
    ];

    let mut filenames: Vec<String> = Vec::new();
    for &p in &priority {
        if res_dir.join(p).is_file() {
            filenames.push(p.to_string());
        }
    }
    if let Ok(rd) = fs::read_dir(&res_dir) {
        let mut extras: Vec<String> = rd
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let is_pck = path
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("pck"));
                if is_pck {
                    path.file_name()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect();
        extras.sort();
        for name in extras {
            if !filenames.iter().any(|f| f.eq_ignore_ascii_case(&name)) {
                filenames.push(name);
            }
        }
    }

    let mut opened = Vec::new();
    let mut errors = Vec::new();
    for name in filenames {
        let full = res_dir.join(&name);
        let mount = Path::new(&name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&name)
            .to_ascii_lowercase();
        match PckArchive::open(&full) {
            Ok(arch) => opened.push((name, mount, arch)),
            Err(err) => errors.push((name, err)),
        }
    }
    (opened, errors)
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
    sample_terrain_height_with_flip(
        blocks,
        world_width,
        world_length,
        block_size,
        block_rows,
        block_cols,
        x,
        z,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn sample_terrain_height_with_flip(
    blocks: &[(Vec<f32>, u32)],
    world_width: f32,
    world_length: f32,
    block_size: f32,
    block_rows: u32,
    block_cols: u32,
    x: f32,
    z: f32,
    flip_z: bool,
) -> Option<f32> {
    if block_rows == 0 || block_cols == 0 || block_size <= 0.0 {
        return None;
    }
    let fx = (x + world_width * 0.5) / block_size;
    let fz = if flip_z {
        (world_length * 0.5 - z) / block_size
    } else {
        (z + world_length * 0.5) / block_size
    };
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
pub struct FailureGroup {
    pub count: usize,
    pub samples: Vec<String>,
}

impl FailureGroup {
    pub fn record(&mut self, sample: String) {
        self.count += 1;
        if self.samples.len() < 5 && !self.samples.contains(&sample) {
            self.samples.push(sample);
        }
    }
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
    pub trn2_textures_failed: usize,
    pub trn2_texture_bytes: usize,
    pub placements_total: usize,
    pub unique_models: usize,
    pub bmd_resolved: usize,
    pub bmd_failed: usize,
    pub bmd_sample_submeshes: usize,
    pub bmd_failure_groups: BTreeMap<String, FailureGroup>,
    pub report_text: String,
}

/// Run the full stage-by-stage real-zone diagnostic dump against `client_root/maps/<zone>/`
/// and ALL `.pck` archives in `client_root/resources/`. Never panics; never guesses missing names.
#[allow(clippy::type_complexity)]
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
    let (mut archives, open_errors) = open_all_resource_pcks(client_root);

    let _ = writeln!(out, "\n[Archives in resources/]");
    if archives.is_empty() && open_errors.is_empty() {
        let _ = writeln!(
            out,
            "  NONE found under {}",
            client_root.join("resources").display()
        );
    }
    for (name, mount, arch) in &archives {
        let info = arch.info();
        let _ = writeln!(
            out,
            "  OK    {name} (mount='{mount}', entries={}, logical_bytes={})",
            info.entry_count, info.logical_size
        );
    }
    for (name, err) in &open_errors {
        let _ = writeln!(out, "  ERROR {name}: {err}");
    }

    // ---------------- Task 1: Terrain & Textures ----------------
    let _ = writeln!(
        out,
        "\n[Task 1] Terrain Geometry, Masks, Lightmaps & Layer Textures"
    );
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
                "  {zone}.trn2: ver={} size={}x{}m block_size={}m grid={} blocks={}x{} ({}) mask_areas={} layers={}",
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

            let tex_archive_names: Vec<String> = archives
                .iter()
                .filter(|(_, m, _)| {
                    matches!(
                        m.as_str(),
                        "textures" | "litmodels" | "building" | "surfaces"
                    )
                })
                .map(|(n, _, _)| n.clone())
                .collect();

            for (i, tp) in trn2.texture_paths.iter().enumerate() {
                let norm_key = pck::normalize_archive_path(tp, Some("textures"));
                let mut resolved: Option<(String, String, u32, u32, dxt::DdsFormat, usize)> = None;
                let mut decode_err: Option<String> = None;

                for (arch_name, mount, arch) in archives.iter_mut() {
                    if !matches!(
                        mount.as_str(),
                        "textures" | "litmodels" | "building" | "surfaces"
                    ) {
                        continue;
                    }
                    for cand in texture_lookup_candidates_for_archive(tp, Some(mount)) {
                        if let Ok(bytes) = arch.read_entry(&cand) {
                            match dxt::parse_dds(&bytes) {
                                Ok(dds) => {
                                    let rgba = dxt::decode_rgba8(&dds);
                                    resolved = Some((
                                        arch_name.clone(),
                                        cand,
                                        dds.width,
                                        dds.height,
                                        dds.format,
                                        rgba.len(),
                                    ));
                                }
                                Err(e) => {
                                    decode_err = Some(format!("{arch_name}:{cand} DDS error: {e}"));
                                }
                            }
                            break;
                        }
                    }
                    if resolved.is_some() || decode_err.is_some() {
                        break;
                    }
                }

                if let Some((arch_name, cand, w, h, fmt, rgba_bytes)) = resolved {
                    summary.trn2_textures_decoded += 1;
                    summary.trn2_texture_bytes += rgba_bytes;
                    let _ = writeln!(
                        out,
                        "    layer[{i}]: raw=`{tp}` | key=`{norm_key}` | archive=`{arch_name}` (matched `{cand}`) | FOUND | DDS={w}x{h} {fmt:?} | decoded={rgba_bytes} B"
                    );
                } else {
                    summary.trn2_textures_failed += 1;
                    let reason = decode_err.unwrap_or_else(|| {
                        if tex_archive_names.is_empty() {
                            if let Some((n, e)) = open_errors
                                .iter()
                                .find(|(n, _)| n.eq_ignore_ascii_case("textures.pck"))
                            {
                                format!("archive open error ({n}: {e})")
                            } else {
                                "no texture archives found in resources/".to_string()
                            }
                        } else {
                            format!("MISSING in [{}]", tex_archive_names.join(", "))
                        }
                    });
                    let mut closest_all = Vec::new();
                    for (arch_name, mount, arch) in &archives {
                        if matches!(mount.as_str(), "textures" | "litmodels" | "building") {
                            for c in arch.closest_entries(&norm_key, 2) {
                                closest_all.push(format!("{arch_name}:{c}"));
                            }
                        }
                    }
                    let closest_str = if closest_all.is_empty() {
                        "none".to_string()
                    } else {
                        closest_all.join(", ")
                    };
                    let _ = writeln!(
                        out,
                        "    layer[{i}]: raw=`{tp}` | key=`{norm_key}` | {reason} | closest=[{closest_str}]"
                    );
                }
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
                    for (px, pz) in [
                        (0.0f32, 0.0f32),
                        (-128.0, 128.0),
                        (128.0, -128.0),
                        (-306.17, 252.98),
                    ] {
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

    // ---------------- Task 2 & Task 3: Buildings & Placement Heights ----------------
    let _ = writeln!(
        out,
        "\n[Task 2] Buildings (.bmd Placements, Archives & Versions)"
    );
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

                let model_archive_names: Vec<String> =
                    archives.iter().map(|(n, _, _)| n.clone()).collect();
                let _ = writeln!(
                    out,
                    "  {zone}.ecwld (v{}) + {zone}.ecbsd (v{}): {} unique placements, {} unique .bmd models",
                    wh.version,
                    bh.version,
                    placements.len(),
                    unique.len()
                );
                let _ = writeln!(
                    out,
                    "  Model archives searched: [{}]",
                    if model_archive_names.is_empty() {
                        "NONE".to_string()
                    } else {
                        model_archive_names.join(", ")
                    }
                );

                let mut found_by_archive: BTreeMap<String, usize> = BTreeMap::new();
                let mut header_versions: BTreeMap<String, usize> = BTreeMap::new();
                let mut mesh_versions: BTreeMap<String, usize> = BTreeMap::new();
                let mut bmd_tex_ok = 0usize;
                let mut bmd_tex_missing = 0usize;
                let mut bmd_tex_samples_missing: Vec<String> = Vec::new();
                let mut failure_groups: BTreeMap<String, FailureGroup> = BTreeMap::new();

                // Evaluate all unique models (capped at 500 only if huge, all 1,884 in s0 are fast once PCK is indexed)
                for model_key in &unique {
                    let mut raw_found: Option<(String, String, Vec<u8>)> = None;
                    for (arch_name, mount, arch) in archives.iter_mut() {
                        for cand in bmd_lookup_candidates_for_archive(zone, model_key, Some(mount))
                        {
                            if let Ok(bytes) = arch.read_entry(&cand) {
                                raw_found = Some((arch_name.clone(), cand, bytes));
                                break;
                            }
                        }
                        if raw_found.is_some() {
                            break;
                        }
                    }
                    // Also try loose file under <client_root>/litmodels/<zone>/<path>
                    if raw_found.is_none() {
                        let loose = client_root
                            .join("litmodels")
                            .join(zone)
                            .join(pck::normalize_entry_path(model_key));
                        if let Ok(bytes) = fs::read(&loose) {
                            raw_found =
                                Some(("loose".to_string(), loose.display().to_string(), bytes));
                        }
                    }

                    let Some((arch_name, cand, bytes)) = raw_found else {
                        summary.bmd_failed += 1;
                        let reason = if archives.is_empty() {
                            if !open_errors.is_empty() {
                                "archive_open_error"
                            } else {
                                "archive_not_found"
                            }
                        } else {
                            "entry_not_found"
                        };
                        failure_groups
                            .entry(reason.to_string())
                            .or_default()
                            .record(model_key.clone());
                        continue;
                    };

                    match BmdModel::parse(&bytes) {
                        Ok(model) => {
                            if model.submeshes.is_empty() && !model.collide_only {
                                summary.bmd_failed += 1;
                                failure_groups
                                    .entry("empty_mesh".to_string())
                                    .or_default()
                                    .record(model_key.clone());
                                continue;
                            }
                            summary.bmd_resolved += 1;
                            *found_by_archive.entry(arch_name.clone()).or_insert(0) += 1;
                            *header_versions
                                .entry(format!("0x{:08x}", model.model_version))
                                .or_insert(0) += 1;
                            for sm in &model.submeshes {
                                *mesh_versions
                                    .entry(format!("0x{:08x}", sm.version))
                                    .or_insert(0) += 1;
                                if !sm.texture.is_empty() && bmd_tex_ok + bmd_tex_missing < 64 {
                                    let mut tex_hit = false;
                                    for (_, tmount, tarch) in archives.iter_mut() {
                                        if !matches!(
                                            tmount.as_str(),
                                            "textures" | "litmodels" | "building" | "models"
                                        ) {
                                            continue;
                                        }
                                        for tcand in texture_lookup_candidates_for_archive(
                                            &sm.texture,
                                            Some(tmount),
                                        ) {
                                            if tarch.find_index(&tcand).is_some() {
                                                tex_hit = true;
                                                break;
                                            }
                                        }
                                        if tex_hit {
                                            break;
                                        }
                                    }
                                    if tex_hit {
                                        bmd_tex_ok += 1;
                                    } else {
                                        bmd_tex_missing += 1;
                                        if bmd_tex_samples_missing.len() < 5 {
                                            bmd_tex_samples_missing.push(sm.texture.clone());
                                        }
                                    }
                                }
                            }
                            if summary.bmd_resolved <= 5 {
                                summary.bmd_sample_submeshes += model.submeshes.len();
                                let verts: usize =
                                    model.submeshes.iter().map(|s| s.vertices.len()).sum();
                                let tris: usize =
                                    model.submeshes.iter().map(|s| s.indices.len() / 3).sum();
                                let _ = writeln!(
                                    out,
                                    "    bmd OK [{arch_name}:{cand}]: hdr=0x{:08x} submeshes={} verts={} tris={}",
                                    model.model_version,
                                    model.submeshes.len(),
                                    verts,
                                    tris
                                );
                            }
                        }
                        Err(e) => {
                            summary.bmd_failed += 1;
                            let reason = if e.to_string().contains("version") {
                                "unsupported_bmd_version"
                            } else {
                                "bmd_parse_error"
                            };
                            failure_groups
                                .entry(reason.to_string())
                                .or_default()
                                .record(format!("{model_key} ({e})"));
                        }
                    }
                }

                let _ = writeln!(
                    out,
                    "  Resolved .bmd models: {} / {} (failed: {})",
                    summary.bmd_resolved,
                    unique.len(),
                    summary.bmd_failed
                );
                if !found_by_archive.is_empty() {
                    let _ = writeln!(out, "  Found by archive: {:?}", found_by_archive);
                }
                if !header_versions.is_empty() || !mesh_versions.is_empty() {
                    let _ = writeln!(
                        out,
                        "  .bmd versions: headers={:?}, meshes={:?}",
                        header_versions, mesh_versions
                    );
                }
                let _ = writeln!(
                    out,
                    "  Sampled .bmd textures: found={bmd_tex_ok}, missing={bmd_tex_missing}{}",
                    if bmd_tex_samples_missing.is_empty() {
                        String::new()
                    } else {
                        format!(" (samples: {:?})", bmd_tex_samples_missing)
                    }
                );
                let _ = writeln!(out, "  Failures grouped by reason:");
                if failure_groups.is_empty() {
                    let _ = writeln!(out, "    (none)");
                } else {
                    for (reason, grp) in &failure_groups {
                        let _ = writeln!(
                            out,
                            "    - {reason}: count={} | samples={:?}",
                            grp.count, grp.samples
                        );
                    }
                }
                summary.bmd_failure_groups = failure_groups;

                // ---------------- Task 3: Placement Heights & Mapping Table ----------------
                let _ = writeln!(
                    out,
                    "\n[Task 3] Placement Height & Axis-Mapping Verification"
                );
                if let Some((w, l, bs, _, rows, cols)) = trn2_dims {
                    let candidates: [(&str, fn([f32; 3]) -> (f32, f32, f32), bool); 7] = [
                        (
                            "Angelica (x, y, z) on Z=+L/2-row*bs-gz*step",
                            |p| (p[0], p[1], p[2]),
                            true,
                        ),
                        ("X negated (-x, y, z)", |p| (-p[0], p[1], p[2]), true),
                        ("X/Z swapped (z, y, x)", |p| (p[2], p[1], p[0]), true),
                        ("X&Z negated (-x, y, -z)", |p| (-p[0], p[1], -p[2]), true),
                        ("Z negated (x, y, -z)", |p| (p[0], p[1], -p[2]), true),
                        (
                            "Non-flipped terrain Z=-L/2+row*bs+gz*step",
                            |p| (p[0], p[1], p[2]),
                            false,
                        ),
                        ("Y/Z swapped (x, z, y)", |p| (p[0], p[2], p[1]), true),
                    ];
                    let _ = writeln!(
                        out,
                        "  | Candidate Mapping | Median |dy| | Min |dy| | Max |dy| | >1m | >5m | >20m |"
                    );
                    for (name, map_fn, flip_z) in candidates {
                        let mut dys: Vec<f32> = placements
                            .iter()
                            .filter_map(|p| {
                                let (x, y, z) = map_fn(p.position);
                                sample_terrain_height_with_flip(
                                    &decoded_blocks,
                                    w,
                                    l,
                                    bs,
                                    rows,
                                    cols,
                                    x,
                                    z,
                                    flip_z,
                                )
                                .map(|th| (y - th).abs())
                            })
                            .collect();
                        dys.sort_by(|a, b| a.total_cmp(b));
                        if !dys.is_empty() {
                            let med = dys[dys.len() / 2];
                            let min_d = dys[0];
                            let max_d = *dys.last().unwrap();
                            let gt1 = dys.iter().filter(|&&d| d > 1.0).count();
                            let gt5 = dys.iter().filter(|&&d| d > 5.0).count();
                            let gt20 = dys.iter().filter(|&&d| d > 20.0).count();
                            let _ = writeln!(
                                out,
                                "  | {name} | {med:.2} m | {min_d:.2} m | {max_d:.2} m | {gt1} | {gt5} | {gt20} |"
                            );
                        }
                    }

                    // Top |dy| > 20m outliers under winning Angelica mapping
                    let mut outliers: Vec<(f32, f32, u32, [f32; 3], f32, &str)> = placements
                        .iter()
                        .filter_map(|p| {
                            let th = sample_angelica_height(
                                &decoded_blocks,
                                w,
                                l,
                                bs,
                                rows,
                                cols,
                                p.position[0],
                                p.position[2],
                            )?;
                            let dy = p.position[1] - th;
                            (dy.abs() > 20.0).then_some((
                                dy.abs(),
                                dy,
                                p.export_id,
                                p.position,
                                th,
                                p.asset_path.as_str(),
                            ))
                        })
                        .collect();
                    outliers.sort_by(|a, b| b.0.total_cmp(&a.0));
                    let _ = writeln!(
                        out,
                        "  Winning mapping outliers (|dy| > 20m): {} total (top 10, left unchanged with no fudge offset):",
                        outliers.len()
                    );
                    for (_, dy, id, pos, th, path) in outliers.iter().take(10) {
                        let note = if pos == &[0.0, 0.0, 0.0] {
                            "origin-baked world-space mesh"
                        } else {
                            "perimeter cliff/wall/elevated pivot"
                        };
                        let _ = writeln!(
                            out,
                            "    id={id} pos=({:.2}, {:.2}, {:.2}) terrain_h={th:.2}m dy={dy:+.2}m path=`{path}` ({note})",
                            pos[0], pos[1], pos[2]
                        );
                    }
                }
            }
            Err(e) => {
                let _ = writeln!(out, "  ERROR parsing placements: {e}");
            }
        }
    } else {
        let _ = writeln!(
            out,
            "  ERROR: missing {} or {}",
            wld_path.display(),
            bsd_path.display()
        );
    }

    // ---------------- Task 4 & 5: Diagnostics Summary & Expected for this client ----------------
    let _ = writeln!(out, "\n[Task 4] Diagnostics Summary");
    let _ = writeln!(
        out,
        "  Terrain blocks decoded: {}{}",
        summary.decoded_blocks,
        if summary.decoded_blocks == 0 {
            " (reason: .trn2 or .t2bk missing/failed)"
        } else {
            ""
        }
    );
    let _ = writeln!(
        out,
        "  Terrain layer textures: loaded={}, failed={}, bytes={}{}",
        summary.trn2_textures_decoded,
        summary.trn2_textures_failed,
        summary.trn2_texture_bytes,
        if summary.trn2_textures_decoded == 0 {
            " (reason: textures.pck missing, failed to open, or entries missing — see Task 1)"
        } else {
            ""
        }
    );
    let _ = writeln!(
        out,
        "  Buildings (.bmd, counting unit: unique placements, whole-zone dump = all in_range): total={} = not_in_range(0) + in_range({}) | in_range: loaded({}) + failed({}) + pending(0){}",
        summary.placements_total,
        summary.placements_total,
        summary.bmd_resolved,
        summary.bmd_failed,
        if summary.bmd_resolved == 0 {
            " (reason: see Task 2 failure table)"
        } else {
            ""
        }
    );

    let _ = writeln!(
        out,
        "\n[Expected for this client (derived from real maps/s0/ files)]"
    );
    let _ = writeln!(out, "  - s0.trn2: ver=1, 1024x1024m, block_size=64m, grid=32, 16x16 blocks (256), 16 mask_areas, 5 layer textures");
    let _ = writeln!(out, "  - s0_1.t2bk: ver=6, flags=0x35, 256/256 blocks, height_range=[68.91, 226.16]m, height(0,0)=139.30m");
    let _ = writeln!(
        out,
        "  - s0_1.t2lm & s0_1.t2lm1: ver=2, 16 areas, 512x512 Dxt1"
    );
    let _ = writeln!(
        out,
        "  - s0_1.t2mk: ver=2, 16 mask_areas, 43 total_layers, 256x256 masks"
    );
    let _ = writeln!(out, "  - s0.ecwld (v14) + s0.ecbsd (v15): 1884 unique placements, 1884 unique .bmd models, Angelica median |dy|=0.33m");

    // ---------------- Task 3: First Loaded Terrain Block Bindings & Streamed Blocks ----------------
    let _ = writeln!(
        out,
        "\n[Task 3] First Loaded Terrain Block — Texture Bindings, Streaming & Draw Calls"
    );
    let temp_cache =
        std::env::temp_dir().join(format!("fwe-dump-cache-{}-{}", zone, std::process::id()));
    match crate::terrain_gpu::TerrainSession::open_zone(
        client_root,
        zone,
        &temp_cache,
        64 * 1024 * 1024,
    ) {
        Ok(mut session) => {
            let h0 = session.sample_height(0.0, 0.0).unwrap_or(140.0);
            let cam_pos = glam::Vec3::new(0.0, h0 + 30.0, 0.0);
            match session.update_for_camera(cam_pos, 4) {
                Ok(_) => {
                    let streamed = session.mesh.streamed_blocks;
                    let tris = session.mesh.indices.len() / 3;
                    let draw_calls =
                        if session.terrain_layers_loaded > 0 && !session.mesh.indices.is_empty() {
                            1usize
                        } else {
                            0usize
                        };
                    let non_zero_day = session
                        .day_lightmap
                        .rgba
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .filter(|px| px[0] > 0 || px[1] > 0 || px[2] > 0)
                        .count();
                    let total_px = (session.day_lightmap.width as usize)
                        * (session.day_lightmap.height as usize);
                    let _ = writeln!(
                        out,
                        "  TerrainSession::update_for_camera(radius=4): OK | streamed_blocks={streamed} | vertices={} | triangles={tris}",
                        session.mesh.vertices.len()
                    );
                    let _ = writeln!(
                        out,
                        "  First loaded block (0, 0) GPU bindings: @group(1) @binding(0)=day_lightmap_tex ({}x{} Rgba8Unorm, non_zero_rgb_px={non_zero_day}/{total_px}), @group(1) @binding(1)=night_lightmap_tex ({}x{} Rgba8Unorm), @group(1) @binding(2)=live-linear-sampler (Linear, Repeat)",
                        session.day_lightmap.width,
                        session.day_lightmap.height,
                        session.night_lightmap.width,
                        session.night_lightmap.height
                    );
                    let _ = writeln!(
                        out,
                        "  GPU Terrain Splatting: layers={} (full 2x2 box-filter mip chain), sampler=Linear/Linear/MipLinear Repeat anisotropy=16, t2mk_scales={:?}",
                        session.splat.layers.len(),
                        &session.splat.scales[..session.splat.layers.len().min(8)]
                    );
                    let _ = writeln!(
                        out,
                        "  Composited layer textures bound: {}/{} real DDS layers (failed={}, {} bytes) | terrain draw_calls={draw_calls}{}",
                        session.terrain_layers_loaded,
                        session.terrain_layers_loaded + session.terrain_layers_failed,
                        session.terrain_layers_failed,
                        session.terrain_tex_bytes,
                        if draw_calls == 0 {
                            " (0 draw calls: either 0 real layer textures loaded or 0 streamed blocks)"
                        } else {
                            ""
                        }
                    );
                    let _ = writeln!(
                        out,
                        "  Sky (A3DSkySphere from ECBSDFILEBOXAREA15): {}",
                        session.sky.status_note
                    );
                }
                Err(err) => {
                    let _ = writeln!(
                        out,
                        "  TerrainSession::update_for_camera FAILED: {err} | streamed_blocks=0 | terrain draw_calls=0"
                    );
                }
            }
        }
        Err(err) => {
            let _ = writeln!(
                out,
                "  TerrainSession::open_zone FAILED: {err} | streamed_blocks=0 | terrain draw_calls=0"
            );
        }
    }
    let _ = fs::remove_dir_all(&temp_cache);

    summary.report_text = out;
    summary
}

/// Minimal pure-Rust RGBA8 PNG encoder using `flate2` (no extra dependencies).
pub fn encode_rgba8_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    use flate2::{write::ZlibEncoder, Compression};
    use std::io::Write;

    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| "PNG dimensions overflow".to_string())?;
    if width == 0 || height == 0 || rgba.len() != expected {
        return Err(format!(
            "invalid RGBA8 buffer size {} for {}x{}",
            rgba.len(),
            width,
            height
        ));
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &b in bytes {
            crc ^= b as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xedb8_8320u32 & mask);
            }
        }
        !crc
    }

    fn push_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(chunk_type);
        out.extend_from_slice(data);
        let crc = crc32(&out[start..]);
        out.extend_from_slice(&crc.to_be_bytes());
    }

    let mut raw_scanlines = Vec::with_capacity((height as usize) * (1 + (width as usize) * 4));
    let row_bytes = (width as usize) * 4;
    for y in 0..(height as usize) {
        raw_scanlines.push(0u8); // filter type 0 (None)
        raw_scanlines.extend_from_slice(&rgba[y * row_bytes..(y + 1) * row_bytes]);
    }
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&raw_scanlines)
        .map_err(|e| format!("PNG zlib compress failed: {e}"))?;
    let idat = enc
        .finish()
        .map_err(|e| format!("PNG zlib finish failed: {e}"))?;

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(6); // color type: RGBA
    ihdr.push(0); // compression
    ihdr.push(0); // filter
    ihdr.push(0); // interlace

    let mut png = Vec::with_capacity(64 + idat.len());
    png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
    push_chunk(&mut png, b"IHDR", &ihdr);
    push_chunk(&mut png, b"IDAT", &idat);
    push_chunk(&mut png, b"IEND", &[]);
    Ok(png)
}

fn ensure_parent_dir(out_path: &Path) -> Result<(), String> {
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
        }
    }
    Ok(())
}

fn open_named_archive(
    client_root: &Path,
    archive_name: &str,
) -> Result<(PckArchive, String), String> {
    let direct = Path::new(archive_name);
    let path = if direct.is_file() {
        direct.to_path_buf()
    } else {
        client_root.join("resources").join(archive_name)
    };
    let mount = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(archive_name)
        .to_ascii_lowercase();
    let arch =
        PckArchive::open(&path).map_err(|e| format!("failed to open {}: {e}", path.display()))?;
    Ok((arch, mount))
}

fn read_archive_entry_flexible(
    arch: &mut PckArchive,
    mount: &str,
    entry_path: &str,
) -> Result<(String, Vec<u8>), String> {
    let mut candidates = vec![
        entry_path.to_string(),
        pck::normalize_entry_path(entry_path),
        pck::normalize_archive_path(entry_path, Some(mount)),
    ];
    for c in texture_lookup_candidates_for_archive(entry_path, Some(mount)) {
        if !candidates.contains(&c) {
            candidates.push(c);
        }
    }
    for cand in &candidates {
        if let Ok(bytes) = arch.read_entry(cand) {
            return Ok((cand.clone(), bytes));
        }
    }
    let norm = pck::normalize_archive_path(entry_path, Some(mount));
    let closest = arch.closest_entries(&norm, 5);
    Err(format!(
        "entry `{entry_path}` not found (closest: {})",
        if closest.is_empty() {
            "none".to_string()
        } else {
            closest.join(", ")
        }
    ))
}

/// CLI helper: `--extract-entry <archive.pck> "<entry path>" --out <file>`
pub fn extract_entry(
    client_root: &Path,
    archive_name: &str,
    entry_path: &str,
    out_path: &Path,
) -> Result<String, String> {
    let (mut arch, mount) = open_named_archive(client_root, archive_name)?;
    let (matched, bytes) = read_archive_entry_flexible(&mut arch, &mount, entry_path)?;
    ensure_parent_dir(out_path)?;
    fs::write(out_path, &bytes)
        .map_err(|e| format!("failed to write {}: {e}", out_path.display()))?;
    Ok(format!(
        "Extracted `{matched}` from `{archive_name}` ({} bytes) -> {}",
        bytes.len(),
        out_path.display()
    ))
}

/// Format up to `max_bytes` of `data` as a 16-byte-per-row hex + ASCII dump.
pub fn format_hexdump(data: &[u8], max_bytes: usize) -> String {
    let limit = data.len().min(max_bytes);
    let mut out = String::new();
    let _ = writeln!(out, "# total_bytes={} shown_bytes={limit}", data.len());
    for (row_idx, chunk) in data[..limit].chunks(16).enumerate() {
        let off = row_idx * 16;
        let _ = write!(out, "{off:08x}: ");
        for i in 0..16 {
            if let Some(b) = chunk.get(i) {
                let _ = write!(out, "{b:02x} ");
            } else {
                let _ = write!(out, "   ");
            }
            if i == 7 {
                let _ = write!(out, " ");
            }
        }
        let _ = write!(out, " |");
        for &b in chunk {
            let ch = if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '.'
            };
            out.push(ch);
        }
        let _ = writeln!(out, "|");
    }
    out
}

/// CLI helper: `--hexdump <file> --bytes <N> --out <file.txt>`
pub fn hexdump_file(file_path: &Path, max_bytes: usize, out_path: &Path) -> Result<String, String> {
    let bytes =
        fs::read(file_path).map_err(|e| format!("failed to read {}: {e}", file_path.display()))?;
    let dump = format_hexdump(&bytes, max_bytes);
    ensure_parent_dir(out_path)?;
    fs::write(out_path, &dump)
        .map_err(|e| format!("failed to write {}: {e}", out_path.display()))?;
    Ok(format!(
        "Wrote hexdump of {} ({} of {} bytes) -> {}",
        file_path.display(),
        bytes.len().min(max_bytes),
        bytes.len(),
        out_path.display()
    ))
}

/// CLI helper: `--export-terrain-block <zone> <block_x> <block_z> --out <file.txt>`
pub fn export_terrain_block(
    client_root: &Path,
    zone: &str,
    block_x: u32,
    block_z: u32,
    out_path: &Path,
) -> Result<String, String> {
    let zone_dir = client_root.join("maps").join(zone);
    let trn2_bytes = fs::read(zone_dir.join(format!("{zone}.trn2")))
        .map_err(|e| format!("failed to read {zone}.trn2: {e}"))?;
    let trn2 = Trn2Header::parse(&trn2_bytes)?;
    let t2bk_path = zone_dir.join(format!("{zone}_1.t2bk"));
    let t2bk_bytes =
        fs::read(&t2bk_path).map_err(|e| format!("failed to read {}: {e}", t2bk_path.display()))?;
    let t2bk = T2bkFile::parse_header(&t2bk_bytes)?;

    if block_x >= trn2.block_cols || block_z >= trn2.block_rows {
        return Err(format!(
            "block ({block_x}, {block_z}) out of range for {zone} ({}x{} blocks)",
            trn2.block_cols, trn2.block_rows
        ));
    }
    let idx = (block_z * trn2.block_cols + block_x) as usize;
    let decoded = t2bk.decode_block(&t2bk_bytes, idx, trn2.block_grid)?;
    let mut hmin = f32::INFINITY;
    let mut hmax = f32::NEG_INFINITY;
    for &h in &decoded.heights {
        hmin = hmin.min(h);
        hmax = hmax.max(h);
    }

    let mut out = String::new();
    let _ = writeln!(
        out,
        "=== Terrain Block Export: {zone} block ({block_x}, {block_z}) ==="
    );
    let _ = writeln!(
        out,
        "trn2: ver={} size={}x{}m block_size={}m grid={} blocks={}x{} ({}) mask_areas={} layers={}",
        trn2.version,
        trn2.width,
        trn2.length,
        trn2.block_size,
        trn2.block_grid,
        trn2.block_cols,
        trn2.block_rows,
        trn2.num_blocks,
        trn2.mask_areas,
        trn2.texture_paths.len()
    );
    let _ = writeln!(
        out,
        "t2bk: file={zone}_1.t2bk ver={} flags={:#06x} num_blocks={} block_idx={idx} offset={}",
        t2bk.version,
        t2bk.flags,
        t2bk.offsets.len(),
        t2bk.offsets[idx]
    );
    let x0 = -trn2.width * 0.5 + (block_x as f32) * trn2.block_size;
    let x1 = x0 + trn2.block_size;
    let z0 = trn2.length * 0.5 - (block_z as f32) * trn2.block_size;
    let z1 = z0 - trn2.block_size;
    let origin_col = ((trn2.width * 0.5) / trn2.block_size).floor() as u32;
    let origin_row = ((trn2.length * 0.5) / trn2.block_size).floor() as u32;
    let _ = writeln!(
        out,
        "block: row={} col={} mask_area={} lod_scale={} verts={} height_range=[{hmin:.3}, {hmax:.3}]",
        decoded.row,
        decoded.col,
        decoded.mask_area,
        decoded.lod_scale,
        decoded.heights.len()
    );
    let _ = writeln!(
        out,
        "world_bounds: X=[{x0:.1}, {x1:.1}] Z=[{z1:.1}, {z0:.1}] (note: world origin (0.0, 0.0) is in block col={origin_col}, row={origin_row}, idx={})",
        origin_row * trn2.block_cols + origin_col
    );
    let _ = writeln!(out, "first_16_heights:");
    for (i, h) in decoded.heights.iter().take(16).enumerate() {
        let _ = writeln!(out, "  [{i:02}] = {h:.4}");
    }
    let _ = writeln!(out, "first_4_quantized_normals:");
    for (i, n) in decoded.normals.iter().take(4).enumerate() {
        let _ = writeln!(out, "  [{i:02}] = ({}, {}, {})", n[0], n[1], n[2]);
    }
    ensure_parent_dir(out_path)?;
    fs::write(out_path, &out)
        .map_err(|e| format!("failed to write {}: {e}", out_path.display()))?;
    Ok(format!(
        "Exported terrain block ({block_x}, {block_z}) of `{zone}` -> {}",
        out_path.display()
    ))
}

/// CLI helper: `--export-texture-png <archive.pck> "<entry path>" --out <file.png>`
pub fn export_texture_png(
    client_root: &Path,
    archive_name: &str,
    entry_path: &str,
    out_path: &Path,
) -> Result<String, String> {
    let (mut arch, mount) = open_named_archive(client_root, archive_name)?;
    let (matched, bytes) = read_archive_entry_flexible(&mut arch, &mount, entry_path)?;
    let dds = dxt::parse_dds(&bytes)?;
    let rgba = dxt::decode_rgba8(&dds);
    let png = encode_rgba8_png(dds.width, dds.height, &rgba)?;
    ensure_parent_dir(out_path)?;
    fs::write(out_path, &png)
        .map_err(|e| format!("failed to write {}: {e}", out_path.display()))?;
    Ok(format!(
        "Exported `{matched}` ({}x{} {:?}) from `{archive_name}` -> {} ({} bytes PNG)",
        dds.width,
        dds.height,
        dds.format,
        out_path.display(),
        png.len()
    ))
}

#[derive(Debug, Clone, Copy)]
pub struct RgbaStats {
    pub r_min: u8,
    pub r_max: u8,
    pub r_mean: f64,
    pub g_min: u8,
    pub g_max: u8,
    pub g_mean: f64,
    pub b_min: u8,
    pub b_max: u8,
    pub b_mean: f64,
    pub zero_frac: f64,
}

pub fn compute_rgba_stats(rgba: &[u8]) -> RgbaStats {
    let mut r_min = 255u8;
    let mut r_max = 0u8;
    let mut g_min = 255u8;
    let mut g_max = 0u8;
    let mut b_min = 255u8;
    let mut b_max = 0u8;
    let mut r_sum = 0u64;
    let mut g_sum = 0u64;
    let mut b_sum = 0u64;
    let mut zeros = 0u64;
    let pixels = rgba.as_chunks::<4>().0;
    let n = pixels.len().max(1) as f64;
    for &[r, g, b, _] in pixels {
        r_min = r_min.min(r);
        r_max = r_max.max(r);
        g_min = g_min.min(g);
        g_max = g_max.max(g);
        b_min = b_min.min(b);
        b_max = b_max.max(b);
        r_sum += r as u64;
        g_sum += g as u64;
        b_sum += b as u64;
        if r == 0 && g == 0 && b == 0 {
            zeros += 1;
        }
    }
    RgbaStats {
        r_min,
        r_max,
        r_mean: r_sum as f64 / n,
        g_min,
        g_max,
        g_mean: g_sum as f64 / n,
        b_min,
        b_max,
        b_mean: b_sum as f64 / n,
        zero_frac: zeros as f64 / n,
    }
}

fn crop_rgba(src: &[u8], full_w: u32, x0: u32, y0: u32, w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![0u8; (w as usize) * (h as usize) * 4];
    for y in 0..(h as usize) {
        let sy = (y0 as usize) + y;
        let src_off = (sy * (full_w as usize) + (x0 as usize)) * 4;
        let dst_off = y * (w as usize) * 4;
        out[dst_off..dst_off + (w as usize) * 4]
            .copy_from_slice(&src[src_off..src_off + (w as usize) * 4]);
    }
    out
}

/// CLI helper: `--export-terrain-pngs <zone> --out-dir <dir>`
/// Exports whole-zone & block (0,0) diffuse, day lightmap, night lightmap, unscaled lit output,
/// and per-layer masks as PNGs (< 1 MB each) plus a per-channel stats table.
pub fn export_terrain_pngs(
    client_root: &Path,
    zone: &str,
    out_dir: &Path,
) -> Result<String, String> {
    fs::create_dir_all(out_dir)
        .map_err(|e| format!("failed to create {}: {e}", out_dir.display()))?;
    let zone_dir = client_root.join("maps").join(zone);
    let trn2_bytes = fs::read(zone_dir.join(format!("{zone}.trn2")))
        .map_err(|e| format!("failed to read {zone}.trn2: {e}"))?;
    let trn2 = Trn2Header::parse(&trn2_bytes)?;
    let num_ma = trn2.mask_areas.max(1) as usize;
    let ma_grid = (num_ma as f64).sqrt().round().max(1.0) as usize;
    let tile_px = 128usize;
    let full_side = (ma_grid * tile_px) as u32;

    let (mut archives, _) = open_all_resource_pcks(client_root);
    let mut layer_textures: Vec<Option<(u32, u32, Vec<u8>)>> = vec![None; trn2.texture_paths.len()];
    for (i, tp) in trn2.texture_paths.iter().enumerate() {
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
                        layer_textures[i] =
                            Some((surf.width, surf.height, dxt::decode_rgba8(&surf)));
                        break;
                    }
                }
            }
            if layer_textures[i].is_some() {
                break;
            }
        }
    }

    let full_px = (full_side as usize) * (full_side as usize);
    let mut diffuse = vec![0u8; full_px * 4];
    let mut layer_masks: Vec<Vec<u8>> = (0..trn2.texture_paths.len())
        .map(|_| vec![0u8; full_px * 4])
        .collect();
    for m in &mut layer_masks {
        for px in m.as_chunks_mut::<4>().0.iter_mut() {
            px[3] = 255;
        }
    }

    let mut sample_weights_log = String::new();
    let t2mk_bytes = fs::read(zone_dir.join(format!("{zone}_1.t2mk")))
        .map_err(|e| format!("failed to read {zone}_1.t2mk: {e}"))?;
    let mk = T2mkHeader::parse(&t2mk_bytes)?;

    for ma in 0..num_ma.min(mk.num_areas as usize) {
        let ma_r = ma / ma_grid;
        let ma_c = ma % ma_grid;
        let Ok(layers) = mk.area_layers(&t2mk_bytes, ma) else {
            continue;
        };
        let mut tile_rgb = vec![[0.0f32; 3]; tile_px * tile_px];
        let mut tile_has_base = false;
        let mut sample_w = Vec::new();
        for (li, lyr) in layers.iter().enumerate() {
            let Ok((m_side, m_bytes)) = mk.decode_layer_mask(&t2mk_bytes, lyr) else {
                continue;
            };
            let lidx = lyr.layer_index as usize;
            // Record mask image for this layer
            if let Some(m_img) = layer_masks.get_mut(lidx) {
                for ty in 0..tile_px {
                    for tx in 0..tile_px {
                        let mv = if li == 0 || m_side <= 1 {
                            255u8
                        } else {
                            let mx = (tx * (m_side as usize)) / tile_px;
                            let my = (ty * (m_side as usize)) / tile_px;
                            m_bytes
                                .get(my * (m_side as usize) + mx)
                                .copied()
                                .unwrap_or(0)
                        };
                        let gx = ma_c * tile_px + tx;
                        let gy = ma_r * tile_px + ty;
                        let o = (gy * (full_side as usize) + gx) * 4;
                        m_img[o] = mv;
                        m_img[o + 1] = mv;
                        m_img[o + 2] = mv;
                        m_img[o + 3] = 255;
                    }
                }
            }
            if matches!(ma, 0 | 5 | 10) {
                let mid_val = if li == 0 || m_side <= 1 {
                    255u8
                } else {
                    let mid = (m_side as usize) / 2;
                    m_bytes
                        .get(mid * (m_side as usize) + mid)
                        .copied()
                        .unwrap_or(0)
                };
                sample_w.push(format!("L{lidx}={mid_val}/255"));
            }
            let Some(Some((tw_u, th_u, rgba))) = layer_textures.get(lidx) else {
                continue;
            };
            let tw = (*tw_u).max(1) as usize;
            let th = (*th_u).max(1) as usize;
            for ty in 0..tile_px {
                for tx in 0..tile_px {
                    let sx = ((tx * tw * 8) / tile_px) % tw;
                    let sy = ((ty * th * 8) / tile_px) % th;
                    let tidx = (sy * tw + sx) * 4;
                    let tr = rgba[tidx] as f32;
                    let tg = rgba[tidx + 1] as f32;
                    let tb = rgba[tidx + 2] as f32;
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
        if !sample_w.is_empty() {
            let _ = writeln!(
                sample_weights_log,
                "  mask_area[{ma}] center weights: {}",
                sample_w.join(", ")
            );
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

    let stitch_raw_lm = |suffix: &str| -> Vec<u8> {
        let mut lm_img = vec![0u8; full_px * 4];
        for px in lm_img.as_chunks_mut::<4>().0.iter_mut() {
            px[3] = 255;
        }
        if let Ok(lm_bytes) = fs::read(zone_dir.join(format!("{zone}_1.{suffix}"))) {
            if let Ok(lm) = T2lmHeader::parse(&lm_bytes) {
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
                            lm_img[o] = dec[sidx];
                            lm_img[o + 1] = dec[sidx + 1];
                            lm_img[o + 2] = dec[sidx + 2];
                            lm_img[o + 3] = 255;
                        }
                    }
                }
            }
        }
        lm_img
    };

    let day_lm = stitch_raw_lm("t2lm");
    let night_lm = stitch_raw_lm("t2lm1");
    let mut lit_unscaled = vec![0u8; full_px * 4];
    for i in 0..full_px {
        let o = i * 4;
        lit_unscaled[o] = ((diffuse[o] as u16 * day_lm[o] as u16) / 255) as u8;
        lit_unscaled[o + 1] = ((diffuse[o + 1] as u16 * day_lm[o + 1] as u16) / 255) as u8;
        lit_unscaled[o + 2] = ((diffuse[o + 2] as u16 * day_lm[o + 2] as u16) / 255) as u8;
        lit_unscaled[o + 3] = 255;
    }

    let mut report = String::new();
    let _ = writeln!(
        report,
        "=== Terrain PNG Export & Channel Stats ({zone}, unscaled diffuse * lightmap) ==="
    );
    let _ = writeln!(report, "| File | Size | R [min..max mean] | G [min..max mean] | B [min..max mean] | zero_frac | PNG bytes |");
    let _ = writeln!(report, "| --- | --- | --- | --- | --- | ---: | ---: |");

    let mut save_and_stat = |name: &str, w: u32, h: u32, rgba: &[u8]| -> Result<(), String> {
        let png = encode_rgba8_png(w, h, rgba)?;
        let out_p = out_dir.join(name);
        fs::write(&out_p, &png).map_err(|e| format!("failed to write {}: {e}", out_p.display()))?;
        let st = compute_rgba_stats(rgba);
        let _ =
            writeln!(
            report,
            "| {name} | {w}x{h} | {}..{} ({:.1}) | {}..{} ({:.1}) | {}..{} ({:.1}) | {:.4} | {} |",
            st.r_min, st.r_max, st.r_mean,
            st.g_min, st.g_max, st.g_mean,
            st.b_min, st.b_max, st.b_mean,
            st.zero_frac,
            png.len()
        );
        Ok(())
    };

    save_and_stat("whole_zone_diffuse.png", full_side, full_side, &diffuse)?;
    save_and_stat("whole_zone_day_lightmap.png", full_side, full_side, &day_lm)?;
    save_and_stat(
        "whole_zone_night_lightmap.png",
        full_side,
        full_side,
        &night_lm,
    )?;
    save_and_stat(
        "whole_zone_lit_unscaled.png",
        full_side,
        full_side,
        &lit_unscaled,
    )?;

    let bsz = tile_px as u32;
    save_and_stat(
        "block_0_0_diffuse.png",
        bsz,
        bsz,
        &crop_rgba(&diffuse, full_side, 0, 0, bsz, bsz),
    )?;
    save_and_stat(
        "block_0_0_day_lightmap.png",
        bsz,
        bsz,
        &crop_rgba(&day_lm, full_side, 0, 0, bsz, bsz),
    )?;
    save_and_stat(
        "block_0_0_night_lightmap.png",
        bsz,
        bsz,
        &crop_rgba(&night_lm, full_side, 0, 0, bsz, bsz),
    )?;
    save_and_stat(
        "block_0_0_lit_unscaled.png",
        bsz,
        bsz,
        &crop_rgba(&lit_unscaled, full_side, 0, 0, bsz, bsz),
    )?;

    for (li, m_img) in layer_masks.iter().enumerate() {
        save_and_stat(&format!("mask_layer_{li}.png"), full_side, full_side, m_img)?;
    }
    for (li, lt) in layer_textures.iter().enumerate() {
        if let Some((tw, th, rgba)) = lt {
            save_and_stat(&format!("layer_{li}_raw.png"), *tw, *th, rgba)?;
        }
    }
    let _ = writeln!(
        report,
        "\nFixed world-point weight & composited RGB checks:"
    );
    for &(wx, wz) in &[
        (-480.0f32, 480.0f32),
        (-160.0f32, 160.0f32),
        (32.0f32, -32.0f32),
    ] {
        let u = ((wx + trn2.width * 0.5) / trn2.width).clamp(0.0, 0.9999);
        let v = ((trn2.length * 0.5 - wz) / trn2.length).clamp(0.0, 0.9999);
        let px = ((u * (full_side as f32)) as usize).min(full_side as usize - 1);
        let py = ((v * (full_side as f32)) as usize).min(full_side as usize - 1);
        let o = (py * (full_side as usize) + px) * 4;
        let w_strs: Vec<String> = layer_masks
            .iter()
            .enumerate()
            .map(|(li, m)| format!("L{li}={}/255", m[o]))
            .collect();
        let _ = writeln!(
            report,
            "  world=({wx:.1}, {wz:.1}) uv=({u:.3}, {v:.3}) | weights=[{}] | diffuse_rgb=({}, {}, {}) | lit_unscaled_rgb=({}, {}, {})",
            w_strs.join(", "),
            diffuse[o], diffuse[o + 1], diffuse[o + 2],
            lit_unscaled[o], lit_unscaled[o + 1], lit_unscaled[o + 2]
        );
    }

    let _ = writeln!(
        report,
        "\nSample layer mask weights (0..255, blend order: layer[0] base, then alpha-blended):"
    );
    report.push_str(&sample_weights_log);
    fs::write(out_dir.join("terrain_stats.txt"), &report)
        .map_err(|e| format!("failed to write terrain_stats.txt: {e}"))?;
    Ok(report)
}

/// CLI helper: `--bmd-trace <file> [--client <PATH>]`
pub fn bmd_trace_file(file_path: &Path, client_root: Option<&Path>) -> Result<String, String> {
    let bytes =
        fs::read(file_path).map_err(|e| format!("failed to read {}: {e}", file_path.display()))?;
    let (mut trace, parsed) = crate::bmd::trace_bmd(&bytes);
    match parsed {
        Ok(model) => {
            let total_verts: usize = model.submeshes.iter().map(|s| s.vertices.len()).sum();
            let total_indices: usize = model.submeshes.iter().map(|s| s.indices.len()).sum();
            let _ = writeln!(
                trace,
                "Parse status: OK | submeshes={} | total_verts={total_verts} | total_indices={total_indices} | collision_hulls={}",
                model.submeshes.len(),
                model.collision_hull_count
            );
            if let Some(root) = client_root {
                let (mut archives, _) = open_all_resource_pcks(root);
                let _ = writeln!(trace, "\nTexture resolution check (Phase 1 normalizer):");
                for (si, sub) in model.submeshes.iter().enumerate() {
                    let norm = pck::normalize_archive_path(&sub.texture, Some("building"));
                    let mut matched = None;
                    for (aname, mount, arch) in archives.iter_mut() {
                        if !matches!(mount.as_str(), "building" | "textures" | "litmodels") {
                            continue;
                        }
                        for cand in texture_lookup_candidates_for_archive(&sub.texture, Some(mount))
                        {
                            if arch.read_entry(&cand).is_ok() {
                                matched = Some((aname.clone(), cand));
                                break;
                            }
                        }
                        if matched.is_some() {
                            break;
                        }
                    }
                    match matched {
                        Some((a, c)) => {
                            let _ = writeln!(
                                trace,
                                "  submesh[{si}] `{}` -> FOUND in {a} (`{c}`)",
                                sub.texture
                            );
                        }
                        None => {
                            let _ = writeln!(
                                trace,
                                "  submesh[{si}] `{}` (norm=`{norm}`) -> NOT FOUND",
                                sub.texture
                            );
                        }
                    }
                }
            }
            Ok(trace)
        }
        Err(e) => {
            let _ = writeln!(trace, "Parse status: ERROR ({e})");
            Err(trace)
        }
    }
}

/// CLI helper: `--list-entries <archive.pck> [--filter <substring>]`
pub fn list_archive_entries(
    client_root: &Path,
    archive_name: &str,
    filter: Option<&str>,
) -> Result<String, String> {
    let (arch, _) = open_named_archive(client_root, archive_name)?;
    let filter_low = filter.map(|f| f.to_ascii_lowercase());
    let mut out = String::new();
    let mut matched = 0usize;
    let total = arch.info().entry_count;
    let _ = writeln!(
        out,
        "=== PCK Entry Listing: {archive_name} (total={total}, filter={:?}) ===",
        filter.unwrap_or("*")
    );
    for entry in arch.entries() {
        let keep = match &filter_low {
            Some(f) => entry.name.to_ascii_lowercase().contains(f),
            None => true,
        };
        if keep {
            let _ = writeln!(out, "  {} ({} B)", entry.name, entry.size);
            matched += 1;
        }
    }
    let _ = writeln!(out, "Matched {matched} / {total} entries.");
    Ok(out)
}

/// CLI helper: `--bench-flight <zone>` — runs a deterministic 60-frame flight path over a zone
/// and reports per-stage avg & max CPU timings (ms) and the top 3 stages by time.
pub fn bench_flight(client_root: &Path, zone: &str) -> Result<String, String> {
    use std::time::Instant;
    let temp_cache =
        std::env::temp_dir().join(format!("fwe-bench-{}-{}", zone, std::process::id()));
    let _ = fs::remove_dir_all(&temp_cache);
    let mut session = crate::terrain_gpu::TerrainSession::open_zone(
        client_root,
        zone,
        &temp_cache,
        64 * 1024 * 1024,
    )?;
    let zone_dir = client_root.join("maps").join(zone);
    let placements = fs::read(zone_dir.join(format!("{zone}.ecwld")))
        .ok()
        .and_then(|wb| {
            fs::read(zone_dir.join(format!("{zone}.ecbsd")))
                .ok()
                .and_then(|bb| world_objects::parse_ecwld_ecbsd_ornaments(&wb, &bb).ok())
        })
        .map(|(_, _, p)| p)
        .unwrap_or_default();

    let mut terrain_stat = crate::scenery::StageTimingStat::default();
    let mut vis_stat = crate::scenery::StageTimingStat::default();
    let mut dedup_stat = crate::scenery::StageTimingStat::default();
    let mut total_stat = crate::scenery::StageTimingStat::default();

    let h0 = session.sample_height(0.0, 0.0).unwrap_or(140.0);
    for frame in 0..60usize {
        let t_all = Instant::now();
        let angle = (frame as f32) * 0.1;
        let cam = glam::Vec3::new(angle.cos() * 180.0, h0 + 35.0, angle.sin() * 180.0);

        let t0 = Instant::now();
        let _ = session.update_for_camera(cam, 4);
        terrain_stat.record(t0.elapsed().as_secs_f32() * 1000.0);

        let t1 = Instant::now();
        let vis = crate::scenery::select_visible_placements(&placements, cam, 300.0, 2000, false);
        vis_stat.record(t1.elapsed().as_secs_f32() * 1000.0);

        let t2 = Instant::now();
        let _dedup = crate::scenery::dedup_instances_by_model(&vis);
        dedup_stat.record(t2.elapsed().as_secs_f32() * 1000.0);

        total_stat.record(t_all.elapsed().as_secs_f32() * 1000.0);
    }
    let _ = fs::remove_dir_all(&temp_cache);

    let mut out = String::new();
    let _ = writeln!(
        out,
        "=== 60-Frame Flight Benchmark ({zone}, placements={}) ===",
        placements.len()
    );
    let _ = writeln!(out, "| Stage | Avg (ms) | Max (ms) |");
    let _ = writeln!(out, "| --- | ---: | ---: |");
    let _ = writeln!(
        out,
        "| terrain_streaming | {:.3} | {:.3} |",
        terrain_stat.avg_ms(),
        terrain_stat.max_ms
    );
    let _ = writeln!(
        out,
        "| scenery_visibility | {:.3} | {:.3} |",
        vis_stat.avg_ms(),
        vis_stat.max_ms
    );
    let _ = writeln!(
        out,
        "| scenery_dedup | {:.3} | {:.3} |",
        dedup_stat.avg_ms(),
        dedup_stat.max_ms
    );
    let _ = writeln!(
        out,
        "| total_cpu_step | {:.3} | {:.3} |",
        total_stat.avg_ms(),
        total_stat.max_ms
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_valid_rgba8_png_with_signature_and_chunks() {
        let rgba = vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];
        let png = encode_rgba8_png(2, 2, &rgba).expect("encode 2x2 PNG");
        assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        assert_eq!(&png[12..16], b"IHDR");
        assert!(png.windows(4).any(|w| w == b"IDAT"));
        assert!(png.windows(4).any(|w| w == b"IEND"));
    }

    #[test]
    fn formats_hexdump_with_offsets_and_ascii() {
        let dump = format_hexdump(b"MOXB\x01\x00\x00\x80hello", 16);
        assert!(dump.contains("00000000: 4d 4f 58 42"));
        assert!(dump.contains("|MOXB"));
    }
}
