//! Local client grass survey tool.
//!
//! Reads `<client-root>/maps/<zone>/<zone>.ecwld` + `<zone>.ecbsd` and writes a Markdown
//! summary of grass types, `ECWDFILEGRASSTYPE` parameters, total grass placements, and
//! placements within default streaming range (counts and format fields only; no client data).
//!
//! Usage:
//!   cargo run --locked --bin grass-survey -- <client-root> <zone> --out <report.md>

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use glam::Vec3;

use forsaken_world_explorer::{
    settings::{GrassDensity, DEFAULT_VEG_RADIUS_M},
    vegetation,
};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 4 || args[2] != "--out" {
        eprintln!("Usage: grass-survey <client-root> <zone> --out <report.md>");
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

    let (wh, bh, _, _, grass_types, grasses) =
        match vegetation::parse_ecwld_ecbsd_vegetation(&wld, &bsd) {
            Ok(parsed) => parsed,
            Err(e) => {
                eprintln!("error: failed to parse grass data for '{zone}': {e}");
                return ExitCode::FAILURE;
            }
        };

    let in_range_high = vegetation::select_visible_grass(
        &grasses,
        Vec3::ZERO,
        DEFAULT_VEG_RADIUS_M as f32,
        10_000,
        GrassDensity::High,
    )
    .len();
    let in_range_med = vegetation::select_visible_grass(
        &grasses,
        Vec3::ZERO,
        DEFAULT_VEG_RADIUS_M as f32,
        10_000,
        GrassDensity::Medium,
    )
    .len();

    let mut md = String::new();
    md.push_str(&format!("# Grass Survey — Zone `{zone}`\n\n"));
    md.push_str(&format!(
        "- `.ecwld` version: `{}` ({} blocks)\n",
        wh.version, wh.num_blocks
    ));
    md.push_str(&format!("- `.ecbsd` version: `{}`\n", bh.version));
    md.push_str(&format!(
        "- Grass types (`ECWDFILEGRASSTYPE`): **{}**\n",
        grass_types.len()
    ));
    md.push_str(&format!(
        "- Grass area placements (`ECBSDFILEGRASS7`): **{}**\n",
        grasses.len()
    ));
    md.push_str(&format!(
        "- Placements within `{DEFAULT_VEG_RADIUS_M} m` of origin: high={in_range_high}, medium={in_range_med}\n\n"
    ));

    if !grass_types.is_empty() {
        md.push_str("| Type ID | Alpha Blend | Alpha Ref | Size | Size Var | Softness | Sight Range (m) |\n");
        md.push_str("| --- | --- | --- | --- | --- | --- | --- |\n");
        for g in &grass_types {
            md.push_str(&format!(
                "| {} | {} | {} | {:.2} | {:.2} | {:.2} | {:.1} |\n",
                g.type_id,
                g.alpha_blend,
                g.alpha_ref,
                g.size,
                g.size_var,
                g.softness,
                g.sight_range
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
        "Wrote grass survey for zone '{zone}' ({} types, {} placements) to {}",
        grass_types.len(),
        grasses.len(),
        out_path.display()
    );
    ExitCode::SUCCESS
}
