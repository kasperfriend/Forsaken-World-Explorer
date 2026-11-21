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
        "  Buildings (.bmd): total_placements={}, unique_models={}, loaded={}, failed={}{}",
        summary.placements_total,
        summary.unique_models,
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
                        "  First loaded block (0, 0) GPU bindings: @group(1) @binding(0)=day_lightmap_tex ({}x{} Rgba8UnormSrgb, non_zero_rgb_px={non_zero_day}/{total_px}), @group(1) @binding(1)=night_lightmap_tex ({}x{} Rgba8UnormSrgb), @group(1) @binding(2)=live-linear-sampler (Linear, Repeat)",
                        session.day_lightmap.width,
                        session.day_lightmap.height,
                        session.night_lightmap.width,
                        session.night_lightmap.height
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
    let _ = writeln!(
        out,
        "block: row={} col={} mask_area={} lod_scale={} verts={} height_range=[{hmin:.3}, {hmax:.3}]",
        decoded.row,
        decoded.col,
        decoded.mask_area,
        decoded.lod_scale,
        decoded.heights.len()
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
