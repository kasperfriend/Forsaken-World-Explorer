//! Local client sky & fog survey tool.
//!
//! Reads `<client-root>/maps/<zone>/<zone>.ecwld` + `<zone>.ecbsd` and writes a Markdown
//! summary of `ECBSDFILEBOXAREA15` box areas, day/night fog distances, and light fields
//! (counts and format fields only; no client assets).
//!
//! Usage:
//!   cargo run --locked --bin sky-survey -- <client-root> <zone> --out <report.md>

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use forsaken_world_explorer::atmosphere;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 4 || args[2] != "--out" {
        eprintln!("Usage: sky-survey <client-root> <zone> --out <report.md>");
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

    let (wh, bh, boxes, _waters) =
        match atmosphere::parse_ecwld_ecbsd_atmosphere_and_water(&wld, &bsd) {
            Ok(parsed) => parsed,
            Err(e) => {
                eprintln!("error: failed to parse atmosphere data for '{zone}': {e}");
                return ExitCode::FAILURE;
            }
        };

    let default_count = boxes.iter().filter(|b| b.is_default).count();
    let mut md = String::new();
    md.push_str(&format!("# Sky & Fog Survey — Zone `{zone}`\n\n"));
    md.push_str(&format!(
        "- `.ecwld` version: `{}` ({} blocks)\n",
        wh.version, wh.num_blocks
    ));
    md.push_str(&format!("- `.ecbsd` version: `{}`\n", bh.version));
    md.push_str(&format!(
        "- Unique `ECBSDFILEBOXAREA15` entries: **{}** (default: {}, regional: {})\n\n",
        boxes.len(),
        default_count,
        boxes.len() - default_count
    ));

    md.push_str("| Export ID | Default | Day Fog (m) | Night Fog (m) | Underwater Fog (m) | Day Fog ARGB | Night Fog ARGB |\n");
    md.push_str("| --- | --- | --- | --- | --- | --- | --- |\n");
    for b in &boxes {
        md.push_str(&format!(
            "| {} | {} | {:.1}..{:.1} | {:.1}..{:.1} | {:.1}..{:.1} | `0x{:08x}` | `0x{:08x}` |\n",
            b.export_id,
            b.is_default,
            b.day_fog_start,
            b.day_fog_end,
            b.night_fog_start,
            b.night_fog_end,
            b.uw_fog_start,
            b.uw_fog_end,
            b.day_fog_color,
            b.night_fog_color
        ));
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
        "Wrote sky/fog survey for zone '{zone}' ({} box areas) to {}",
        boxes.len(),
        out_path.display()
    );
    ExitCode::SUCCESS
}
