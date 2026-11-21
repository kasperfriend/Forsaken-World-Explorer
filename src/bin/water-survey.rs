//! Local client water survey tool.
//!
//! Reads `<client-root>/maps/<zone>/<zone>.ecwld` + `<zone>.ecbsd` and writes a Markdown
//! summary of `ECBSDFILEWATER` area counts, sub-terrain indices, and `BSData/*.dat`
//! presence (counts and format fields only; no client assets).
//!
//! Usage:
//!   cargo run --locked --bin water-survey -- <client-root> <zone> --out <report.md>

use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use forsaken_world_explorer::atmosphere;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 4 || args[2] != "--out" {
        eprintln!("Usage: water-survey <client-root> <zone> --out <report.md>");
        return ExitCode::FAILURE;
    }
    let client_root = PathBuf::from(&args[0]);
    let zone = &args[1];
    let out_path = PathBuf::from(&args[3]);

    let zone_dir = client_root.join("maps").join(zone);
    let wld = match fs::read(zone_dir.join(format!("{zone}.ecwld"))) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: failed to read {zone}.ecwld: {e}");
            return ExitCode::FAILURE;
        }
    };
    let bsd = match fs::read(zone_dir.join(format!("{zone}.ecbsd"))) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: failed to read {zone}.ecbsd: {e}");
            return ExitCode::FAILURE;
        }
    };

    let (wh, bh, _boxes, waters) =
        match atmosphere::parse_ecwld_ecbsd_atmosphere_and_water(&wld, &bsd) {
            Ok(parsed) => parsed,
            Err(e) => {
                eprintln!("error: failed to parse water data for '{zone}': {e}");
                return ExitCode::FAILURE;
            }
        };

    let sub_terrains: BTreeSet<i32> = waters.iter().map(|w| w.sub_terrain).collect();
    let bsdata_dir = zone_dir.join("BSData");
    let bsdata_count = fs::read_dir(&bsdata_dir)
        .map(|rd| rd.flatten().count())
        .unwrap_or(0);

    let mut md = String::new();
    md.push_str(&format!("# Water Survey — Zone `{zone}`\n\n"));
    md.push_str(&format!(
        "- `.ecwld` version: `{}` ({} blocks)\n",
        wh.version, wh.num_blocks
    ));
    md.push_str(&format!("- `.ecbsd` version: `{}`\n", bh.version));
    md.push_str(&format!(
        "- Unique `ECBSDFILEWATER` references: **{}**\n",
        waters.len()
    ));
    md.push_str(&format!(
        "- Distinct sub-terrain indices referenced: **{}**\n",
        sub_terrains.len()
    ));
    md.push_str(&format!(
        "- Files in `maps/{zone}/BSData/`: **{bsdata_count}**\n\n"
    ));

    if !waters.is_empty() {
        md.push_str("| Export ID | Sub-Terrain | Data Index |\n| --- | --- | --- |\n");
        for w in waters.iter().take(64) {
            md.push_str(&format!(
                "| {} | {} | {} |\n",
                w.export_id, w.sub_terrain, w.data_index
            ));
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
        "Wrote water survey for zone '{zone}' ({} water refs) to {}",
        waters.len(),
        out_path.display()
    );
    ExitCode::SUCCESS
}
