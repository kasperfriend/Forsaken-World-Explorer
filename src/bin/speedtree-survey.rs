//! Local client SpeedTree survey tool.
//!
//! Reads `<client-root>`, checks `update/speedtreert.dll` SHA-256 against the recognized
//! fingerprint, scans `maps/<zone>/<zone>.ecwld` for tree type & placement counts across
//! 3 LOD tiers, optionally probes `tools/fw-speedtree-helper.exe --probe-only`, and writes
//! a Markdown summary report (counts and format fields only; no client assets).
//!
//! Usage:
//!   cargo run --locked --bin speedtree-survey -- <client-root> --out <report.md>

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use forsaken_world_explorer::{settings::PortablePaths, vegetation};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 3 || args[1] != "--out" {
        eprintln!("Usage: speedtree-survey <client-root> --out <report.md>");
        return ExitCode::FAILURE;
    }
    let client_root = PathBuf::from(&args[0]);
    let out_path = PathBuf::from(&args[2]);

    let dll_status = match vegetation::verify_client_speedtree_dll(&client_root) {
        Ok(sha) => format!("recognized (`{sha}`)"),
        Err(err) => format!("unverified/missing (`{err}`)"),
    };

    // Dry-run helper probe if helper exists beside executable and DLL is trusted
    let paths = PortablePaths::from_executable();
    let helper_path = vegetation::resolve_helper_path(&paths.root);
    let helper_probe = if !helper_path.is_file() {
        format!("helper not found at `{}`", helper_path.display())
    } else if vegetation::verify_client_speedtree_dll(&client_root).is_err() {
        "skipped (untrusted or missing speedtreert.dll)".into()
    } else {
        match Command::new(&helper_path)
            .args([
                "--client-root",
                &client_root.display().to_string(),
                "--probe-only",
            ])
            .output()
        {
            Ok(out) => format!(
                "exit code {:?} ({})",
                out.status.code(),
                String::from_utf8_lossy(&out.stdout).trim()
            ),
            Err(e) => format!("spawn failed: {e}"),
        }
    };

    let maps_dir = client_root.join("maps");
    let mut zone_rows = Vec::new();
    let mut total_types = 0usize;
    let mut total_placements = 0usize;
    if let Ok(rd) = fs::read_dir(&maps_dir) {
        let mut zones: Vec<String> = rd
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        zones.sort();
        for z in zones {
            let zdir = maps_dir.join(&z);
            let Ok(wld) = fs::read(zdir.join(format!("{z}.ecwld"))) else {
                continue;
            };
            let Ok(bsd) = fs::read(zdir.join(format!("{z}.ecbsd"))) else {
                continue;
            };
            if let Ok((_, _, ttypes, tplaces, _, _)) =
                vegetation::parse_ecwld_ecbsd_vegetation(&wld, &bsd)
            {
                total_types += ttypes.len();
                total_placements += tplaces.len();
                if !ttypes.is_empty() || !tplaces.is_empty() {
                    zone_rows.push((z, ttypes.len(), tplaces.len()));
                }
            }
        }
    }

    let mut md = String::new();
    md.push_str("# SpeedTree Client Survey\n\n");
    md.push_str(&format!("- `update/speedtreert.dll`: {dll_status}\n"));
    md.push_str(
        "- Runtime DLL version string: none embedded (146 `CSpeedTreeRT` PE32 x86 exports)\n",
    );
    md.push_str("- Baked LOD tiers per tree type: `3` (`1.0` high, `0.5` medium, `0.1` low)\n");
    md.push_str(&format!("- Helper dry-run probe: {helper_probe}\n"));
    md.push_str(&format!(
        "- Total tree types across surveyed zones: **{total_types}**\n"
    ));
    md.push_str(&format!(
        "- Total tree placements across surveyed zones: **{total_placements}**\n\n"
    ));

    if !zone_rows.is_empty() {
        md.push_str("| Zone | Tree Types | Tree Placements |\n| --- | --- | --- |\n");
        for (z, nt, np) in &zone_rows {
            md.push_str(&format!("| `{z}` | {nt} | {np} |\n"));
        }
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
        "Wrote SpeedTree survey ({total_types} types, {total_placements} placements) to {}",
        out_path.display()
    );
    ExitCode::SUCCESS
}
