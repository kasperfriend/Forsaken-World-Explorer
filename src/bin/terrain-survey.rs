//! Local terrain and world-placement survey tool for Forsaken World clients.
//!
//! Decodes every `.trn2`, `.t2bk`, `.t2lm`, `.t2lm1`, `.t2mk`, `.ecwld`, and `.ecbsd`
//! file across zones and writes a Markdown report with zero proprietary client data.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use forsaken_world_explorer::{
    client_detect, dxt,
    settings::ENV_CLIENT_DIR,
    terrain_block::{T2bkFile, T2lmHeader, T2mkHeader, Trn2Header},
    world_objects::{scan_ecbsd_bmd_placements, EcwldHeader},
};

fn print_usage() {
    println!(
        "terrain-survey — decode and verify terrain blocks, lightmaps, masks, and placements\n\
         \n\
         USAGE:\n\
         \tterrain-survey [CLIENT_DIR] [--zone ZONE] [--out REPORT.md]\n\
         \n\
         If CLIENT_DIR is omitted, FW_CLIENT_DIR is checked. If unset, exits 0 with 'skipped: no client'."
    );
}

fn collect_ext_files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case(ext))
                    .unwrap_or(false)
        })
        .collect();
    files.sort();
    files
}

fn main() -> ExitCode {
    let mut client_arg: Option<PathBuf> = None;
    let mut zone_filter: Option<String> = None;
    let mut out_path: Option<PathBuf> = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                return ExitCode::SUCCESS;
            }
            "--zone" => {
                let Some(z) = args.next() else {
                    eprintln!("error: --zone requires a zone name (e.g. s0)");
                    return ExitCode::FAILURE;
                };
                zone_filter = Some(z);
            }
            "--out" => {
                let Some(p) = args.next() else {
                    eprintln!("error: --out requires a file path");
                    return ExitCode::FAILURE;
                };
                out_path = Some(PathBuf::from(p));
            }
            other if other.starts_with('-') => {
                eprintln!("error: unknown option '{other}'");
                return ExitCode::FAILURE;
            }
            path => client_arg = Some(PathBuf::from(path)),
        }
    }

    let client_path = match client_arg.or_else(|| {
        env::var_os(ENV_CLIENT_DIR)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    }) {
        Some(path) => path,
        None => {
            println!("skipped: no client (pass a client path or set {ENV_CLIENT_DIR})");
            return ExitCode::SUCCESS;
        }
    };

    let client = match client_detect::detect(&client_path) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };

    let zones: Vec<String> = client
        .zones
        .iter()
        .filter(|z| {
            zone_filter
                .as_ref()
                .map(|f| f.eq_ignore_ascii_case(z))
                .unwrap_or(true)
        })
        .cloned()
        .collect();

    let mut report = String::new();
    report.push_str("# Forsaken World Terrain & Placement Survey\n\n");
    report.push_str("| Zone | Grid | T2BK Blocks (ok/total) | Day/Night Lightmaps | Mask Areas | ECWLD Ver | ECBSD `.bmd` Refs | Status |\n");
    report.push_str("| --- | ---: | --- | --- | ---: | --- | ---: | --- |\n");

    let mut failures = 0usize;

    for zone in &zones {
        let dir = client.maps_dir.join(zone);
        let trn2_path = dir.join(format!("{zone}.trn2"));
        let trn2 = match fs::read(&trn2_path)
            .and_then(|b| Trn2Header::parse(&b).map_err(std::io::Error::other))
        {
            Ok(h) => h,
            Err(e) => {
                failures += 1;
                report.push_str(&format!(
                    "| `{zone}` | — | — | — | — | — | — | trn2 error: {e} |\n"
                ));
                continue;
            }
        };

        let mut blocks_ok = 0usize;
        let mut blocks_total = 0usize;
        let mut zone_errors: Vec<String> = Vec::new();

        for bk_path in collect_ext_files(&dir, "t2bk") {
            match fs::read(&bk_path) {
                Ok(bytes) => match T2bkFile::parse_header(&bytes) {
                    Ok(t2bk) => {
                        blocks_total += t2bk.offsets.len();
                        for idx in 0..t2bk.offsets.len() {
                            match t2bk.decode_block(&bytes, idx, trn2.block_grid) {
                                Ok(_) => blocks_ok += 1,
                                Err(e) => {
                                    zone_errors.push(format!("{}[{idx}]: {e}", bk_path.display()));
                                }
                            }
                        }
                    }
                    Err(e) => zone_errors.push(format!("{}: {e}", bk_path.display())),
                },
                Err(e) => zone_errors.push(format!("{}: {e}", bk_path.display())),
            }
        }

        let mut lm_day_ok = 0usize;
        for lm_path in collect_ext_files(&dir, "t2lm") {
            if let Ok(bytes) = fs::read(&lm_path) {
                match T2lmHeader::parse(&bytes) {
                    Ok(hdr) => {
                        for a in 0..hdr.num_areas as usize {
                            match hdr.area_bytes(&bytes, a).and_then(dxt::parse_dds) {
                                Ok(_) => lm_day_ok += 1,
                                Err(e) => zone_errors.push(format!("t2lm[{a}]: {e}")),
                            }
                        }
                    }
                    Err(e) => zone_errors.push(format!("t2lm: {e}")),
                }
            }
        }

        let mut lm_night_ok = 0usize;
        for lm_path in collect_ext_files(&dir, "t2lm1") {
            if let Ok(bytes) = fs::read(&lm_path) {
                match T2lmHeader::parse(&bytes) {
                    Ok(hdr) => {
                        for a in 0..hdr.num_areas as usize {
                            match hdr.area_bytes(&bytes, a).and_then(dxt::parse_dds) {
                                Ok(_) => lm_night_ok += 1,
                                Err(e) => zone_errors.push(format!("t2lm1[{a}]: {e}")),
                            }
                        }
                    }
                    Err(e) => zone_errors.push(format!("t2lm1: {e}")),
                }
            }
        }

        let mut mask_areas = 0usize;
        for mk_path in collect_ext_files(&dir, "t2mk") {
            if let Ok(bytes) = fs::read(&mk_path) {
                match T2mkHeader::parse(&bytes) {
                    Ok(hdr) => mask_areas += hdr.num_areas as usize,
                    Err(e) => zone_errors.push(format!("t2mk: {e}")),
                }
            }
        }

        let ecwld_ver = fs::read(dir.join(format!("{zone}.ecwld")))
            .ok()
            .and_then(|b| EcwldHeader::parse(&b).ok())
            .map(|h| format!("v{}", h.version))
            .unwrap_or_else(|| "err".into());

        let bmd_refs = fs::read(dir.join(format!("{zone}.ecbsd")))
            .ok()
            .and_then(|b| scan_ecbsd_bmd_placements(&b).ok())
            .map(|(_, items)| items.len())
            .unwrap_or(0);

        let status = if zone_errors.is_empty() {
            "ok".to_string()
        } else {
            failures += 1;
            format!("{} error(s): {}", zone_errors.len(), zone_errors[0])
        };

        report.push_str(&format!(
            "| `{zone}` | {} | {blocks_ok}/{blocks_total} | {lm_day_ok}/{lm_night_ok} | {mask_areas} | {ecwld_ver} | {bmd_refs} | {status} |\n",
            trn2.block_grid
        ));
    }

    if let Some(out) = out_path {
        if let Err(e) = fs::write(&out, &report) {
            eprintln!("error writing {}: {e}", out.display());
            return ExitCode::FAILURE;
        }
        println!("Wrote terrain survey report to {}", out.display());
    } else {
        print!("{report}");
    }

    if failures > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
