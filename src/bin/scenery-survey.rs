//! Local client scenery survey tool.
//!
//! Reads `<client_root>/maps/<zone>/<zone>.ecwld` + `<zone>.ecbsd`, decodes every
//! unique referenced `.bmd` from `resources/litmodels.pck` and `resources/building.pck`,
//! and writes a Markdown summary report containing counts, decode failure tallies,
//! unique texture counts, and estimated GPU memory (never raw client data).
//!
//! Usage:
//!   cargo run --locked --bin scenery-survey -- <client-root> <zone> --out <report.md>

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use forsaken_world_explorer::{
    bmd::{AlphaMode, BmdModel},
    pck::PckArchive,
    world_objects,
};

fn try_extract_bmd(
    archives: &mut [PckArchive],
    zone: &str,
    rel_bmd: &str,
) -> Option<Result<Vec<u8>, String>> {
    let candidates = [
        rel_bmd.to_string(),
        format!("litmodels/{zone}/{rel_bmd}"),
        format!("litmodels/{rel_bmd}"),
        format!("building/{rel_bmd}"),
    ];
    for arch in archives.iter_mut() {
        for cand in &candidates {
            if let Ok(bytes) = arch.read_entry(cand) {
                return Some(Ok(bytes));
            }
        }
    }
    None
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 4 || args[2] != "--out" {
        eprintln!("Usage: scenery-survey <client-root> <zone> --out <report.md>");
        return ExitCode::FAILURE;
    }
    let client_root = PathBuf::from(&args[0]);
    let zone = &args[1];
    let out_path = PathBuf::from(&args[3]);

    let zone_dir = client_root.join("maps").join(zone);
    let wld_path = zone_dir.join(format!("{zone}.ecwld"));
    let bsd_path = zone_dir.join(format!("{zone}.ecbsd"));
    let wld_bytes = match fs::read(&wld_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: failed to read {}: {e}", wld_path.display());
            return ExitCode::FAILURE;
        }
    };
    let bsd_bytes = match fs::read(&bsd_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: failed to read {}: {e}", bsd_path.display());
            return ExitCode::FAILURE;
        }
    };

    let (wld_hdr, bsd_hdr, placements) =
        match world_objects::parse_ecwld_ecbsd_ornaments(&wld_bytes, &bsd_bytes) {
            Ok(parsed) => parsed,
            Err(e) => {
                eprintln!("error: failed to parse placements for zone '{zone}': {e}");
                return ExitCode::FAILURE;
            }
        };

    // Open litmodels.pck and building.pck if present
    let res_dir = client_root.join("resources");
    let mut archives = Vec::new();
    for name in ["litmodels.pck", "building.pck"] {
        let p = res_dir.join(name);
        if p.is_file() {
            if let Ok(a) = PckArchive::open(&p) {
                archives.push(a);
            }
        }
    }

    let mut counts_by_model: BTreeMap<String, usize> = BTreeMap::new();
    for p in &placements {
        *counts_by_model
            .entry(p.normalized_bmd_path().to_ascii_lowercase())
            .or_insert(0) += 1;
    }

    let mut decoded_models = 0usize;
    let mut missing_models = 0usize;
    let mut decode_failures: BTreeMap<String, usize> = BTreeMap::new();
    let mut mesh_versions: BTreeMap<u32, usize> = BTreeMap::new();
    let mut unique_textures: BTreeSet<String> = BTreeSet::new();
    let mut opaque_submeshes = 0usize;
    let mut alpha_tested_submeshes = 0usize;
    let mut transparent_deferred_submeshes = 0usize;
    let mut est_vertex_bytes = 0u64;
    let mut est_index_bytes = 0u64;

    for model_key in counts_by_model.keys() {
        match try_extract_bmd(&mut archives, zone, model_key) {
            None => missing_models += 1,
            Some(Err(e)) => {
                *decode_failures.entry(e).or_insert(0) += 1;
            }
            Some(Ok(bytes)) => match BmdModel::parse(&bytes) {
                Ok(model) => {
                    decoded_models += 1;
                    for sm in &model.submeshes {
                        *mesh_versions.entry(sm.version).or_insert(0) += 1;
                        if !sm.texture.is_empty() {
                            unique_textures.insert(sm.texture.to_ascii_lowercase());
                        }
                        match sm.alpha_mode {
                            AlphaMode::Opaque => opaque_submeshes += 1,
                            AlphaMode::AlphaTested => alpha_tested_submeshes += 1,
                            AlphaMode::TransparentDeferred => transparent_deferred_submeshes += 1,
                        }
                        // SceneryVertex is 48 bytes; u16 index is 2 bytes
                        est_vertex_bytes += (sm.vertices.len() as u64) * 48;
                        est_index_bytes += (sm.indices.len() as u64) * 2;
                    }
                }
                Err(err) => {
                    *decode_failures.entry(err.to_string()).or_insert(0) += 1;
                }
            },
        }
    }

    let est_instance_bytes = (placements.len() as u64) * 64;
    let est_total_gpu_mib =
        (est_vertex_bytes + est_index_bytes + est_instance_bytes) as f64 / (1024.0 * 1024.0);

    let mut md = String::new();
    md.push_str(&format!("# Scenery Survey — Zone `{zone}`\n\n"));
    md.push_str("## Summary\n\n");
    md.push_str(&format!(
        "- `.ecwld` version: `{}` ({} blocks)\n",
        wld_hdr.version, wld_hdr.num_blocks
    ));
    md.push_str(&format!("- `.ecbsd` version: `{}`\n", bsd_hdr.version));
    md.push_str(&format!(
        "- Total unique placements (`dwExportID`): **{}**\n",
        placements.len()
    ));
    md.push_str(&format!(
        "- Unique `.bmd` model references: **{}**\n",
        counts_by_model.len()
    ));
    md.push_str(&format!("- Decoded `.bmd` models: **{decoded_models}**\n"));
    md.push_str(&format!(
        "- Missing from opened archives: **{missing_models}**\n"
    ));
    md.push_str(&format!(
        "- Decode failure count: **{}**\n",
        decode_failures.values().sum::<usize>()
    ));
    md.push_str(&format!(
        "- Unique texture references: **{}**\n",
        unique_textures.len()
    ));
    md.push_str(&format!(
        "- Submeshes: opaque={opaque_submeshes}, alpha-tested={alpha_tested_submeshes}, transparent-deferred={transparent_deferred_submeshes}\n"
    ));
    md.push_str(&format!(
        "- Estimated GPU geometry + instance buffers: **{est_total_gpu_mib:.2} MiB** (vertices: {} B, indices: {} B, instances: {} B)\n\n",
        est_vertex_bytes, est_index_bytes, est_instance_bytes
    ));

    md.push_str("## Observed `A3DLitMesh` Versions\n\n");
    if mesh_versions.is_empty() {
        md.push_str("- None decoded.\n\n");
    } else {
        for (ver, cnt) in &mesh_versions {
            md.push_str(&format!("- `0x{ver:08x}`: {cnt} submeshes\n"));
        }
        md.push('\n');
    }

    md.push_str("## Decode Failure Categories\n\n");
    if decode_failures.is_empty() {
        md.push_str("- None.\n\n");
    } else {
        for (reason, cnt) in &decode_failures {
            md.push_str(&format!("- {cnt}x: `{reason}`\n"));
        }
        md.push('\n');
    }

    md.push_str("## Top 20 Repeated Models (Instance Reuse)\n\n");
    md.push_str("| Model index | Placement count |\n| --- | --- |\n");
    let mut top: Vec<(&String, &usize)> = counts_by_model.iter().collect();
    top.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    for (idx, (_, cnt)) in top.into_iter().take(20).enumerate() {
        md.push_str(&format!("| Model #{} | {cnt} |\n", idx + 1));
    }

    if let Some(parent) = Path::new(&out_path).parent() {
        if !parent.as_os_str().is_empty() {
            let _ = fs::create_dir_all(parent);
        }
    }
    if let Err(e) = fs::write(&out_path, md) {
        eprintln!("error: failed to write {}: {e}", out_path.display());
        return ExitCode::FAILURE;
    }
    println!(
        "Wrote scenery survey for zone '{zone}' ({} placements, {} unique models, {} decoded) to {}",
        placements.len(),
        counts_by_model.len(),
        decoded_models,
        out_path.display()
    );
    ExitCode::SUCCESS
}
