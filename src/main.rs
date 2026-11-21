mod ui;

use std::{env, fs, path::PathBuf, process::ExitCode};

use forsaken_world_explorer::{self_test, zone_dump};

fn print_help() {
    println!(
        "Forsaken World Explorer {}\n\
         \n\
         USAGE:\n\
         \tforsaken-world-explorer [OPTIONS]\n\
         \n\
         OPTIONS:\n\
         \t--dump-zone <ZONE>                                   Decode a real client zone stage-by-stage and write counts/bindings\n\
         \t--extract-entry <ARCHIVE.pck> <ENTRY> --out <FILE>   Extract a single entry from a client .pck archive\n\
         \t--hexdump <FILE> [--bytes N] --out <FILE.txt>        Write a hex + ASCII dump of the first N bytes (default 256)\n\
         \t--export-terrain-block <ZONE> <BX> <BZ> --out <FILE> Export decoded terrain block header and first 16 heights\n\
         \t--export-texture-png <ARCHIVE> <ENTRY> --out <PNG>   Decode a DDS entry from a .pck archive and save as PNG\n\
         \t--export-terrain-pngs <ZONE> --out-dir <DIR>         Export diffuse, lightmaps, lit output & layer masks as PNGs + stats\n\
         \t--bmd-trace <FILE> [--client <PATH>]                 Trace every byte/field of a .bmd file and verify texture resolution\n\
         \t--list-entries <ARCHIVE.pck> [--filter <SUBSTR>]     List matching entries in a client .pck archive\n\
         \t--bench-flight <ZONE>                                Run a 60-frame flight benchmark and print per-stage avg/max ms\n\
         \t--client <PATH>                                      Client root folder (defaults to FW_CLIENT_DIR)\n\
         \t--out <PATH>                                         Output file path for dump/export commands\n\
         \t--self-test                                          Run CPU-only self-checks and exit\n\
         \t--smoke-launch                                       Initialize app state and render every UI page headlessly\n\
         \t-V, --version                                        Print version information\n\
         \t-h, --help                                           Print this help message",
        env!("CARGO_PKG_VERSION")
    );
}

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
}

fn arg_values_n(args: &[String], flag: &str, n: usize) -> Option<Vec<String>> {
    let idx = args.iter().position(|a| a == flag)?;
    if idx + n < args.len() {
        Some(args[idx + 1..=idx + n].to_vec())
    } else {
        None
    }
}

fn resolve_client_root(args: &[String], cmd: &str) -> Result<PathBuf, String> {
    arg_value(args, "--client")
        .or_else(|| env::var("FW_CLIENT_DIR").ok())
        .map(PathBuf::from)
        .ok_or_else(|| format!("{cmd} requires --client <PATH> or FW_CLIENT_DIR"))
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print_help();
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|arg| arg == "-V" || arg == "--version") {
        println!("forsaken-world-explorer {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    if let Some(archive) = arg_value(&args, "--list-entries") {
        let client_root = match resolve_client_root(&args, "--list-entries") {
            Ok(p) => p,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        };
        let filter = arg_value(&args, "--filter");
        return match zone_dump::list_archive_entries(&client_root, &archive, filter.as_deref()) {
            Ok(listing) => {
                print!("{listing}");
                if let Some(out_path) = arg_value(&args, "--out") {
                    let _ = fs::write(&out_path, &listing);
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if let Some(zone) = arg_value(&args, "--bench-flight") {
        let client_root = match resolve_client_root(&args, "--bench-flight") {
            Ok(p) => p,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        };
        return match zone_dump::bench_flight(&client_root, &zone) {
            Ok(report) => {
                print!("{report}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if let Some(zone) = arg_value(&args, "--export-terrain-pngs") {
        let client_root = match resolve_client_root(&args, "--export-terrain-pngs") {
            Ok(p) => p,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        };
        let Some(out_dir) = arg_value(&args, "--out-dir") else {
            eprintln!("error: --export-terrain-pngs requires --out-dir <DIR>");
            return ExitCode::FAILURE;
        };
        return match zone_dump::export_terrain_pngs(&client_root, &zone, &PathBuf::from(out_dir)) {
            Ok(report) => {
                print!("{report}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if let Some(bmd_file) = arg_value(&args, "--bmd-trace") {
        let client_root = arg_value(&args, "--client")
            .or_else(|| env::var("FW_CLIENT_DIR").ok())
            .map(PathBuf::from);
        return match zone_dump::bmd_trace_file(&PathBuf::from(bmd_file), client_root.as_deref()) {
            Ok(trace) => {
                print!("{trace}");
                ExitCode::SUCCESS
            }
            Err(trace) => {
                eprint!("{trace}");
                ExitCode::FAILURE
            }
        };
    }

    if let Some(vals) = arg_values_n(&args, "--extract-entry", 2) {
        let client_root = match resolve_client_root(&args, "--extract-entry") {
            Ok(p) => p,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        };
        let Some(out_path) = arg_value(&args, "--out") else {
            eprintln!("error: --extract-entry requires --out <FILE>");
            return ExitCode::FAILURE;
        };
        return match zone_dump::extract_entry(
            &client_root,
            &vals[0],
            &vals[1],
            &PathBuf::from(out_path),
        ) {
            Ok(msg) => {
                println!("{msg}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if let Some(file_arg) = arg_value(&args, "--hexdump") {
        let max_bytes: usize = arg_value(&args, "--bytes")
            .and_then(|s| s.parse().ok())
            .unwrap_or(256);
        let Some(out_path) = arg_value(&args, "--out") else {
            eprintln!("error: --hexdump requires --out <FILE.txt>");
            return ExitCode::FAILURE;
        };
        return match zone_dump::hexdump_file(
            &PathBuf::from(file_arg),
            max_bytes,
            &PathBuf::from(out_path),
        ) {
            Ok(msg) => {
                println!("{msg}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if let Some(vals) = arg_values_n(&args, "--export-terrain-block", 3) {
        let client_root = match resolve_client_root(&args, "--export-terrain-block") {
            Ok(p) => p,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        };
        let Some(out_path) = arg_value(&args, "--out") else {
            eprintln!("error: --export-terrain-block requires --out <FILE.txt>");
            return ExitCode::FAILURE;
        };
        let (Ok(bx), Ok(bz)) = (vals[1].parse::<u32>(), vals[2].parse::<u32>()) else {
            eprintln!("error: block_x and block_z must be non-negative integers");
            return ExitCode::FAILURE;
        };
        return match zone_dump::export_terrain_block(
            &client_root,
            &vals[0],
            bx,
            bz,
            &PathBuf::from(out_path),
        ) {
            Ok(msg) => {
                println!("{msg}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if let Some(vals) = arg_values_n(&args, "--export-texture-png", 2) {
        let client_root = match resolve_client_root(&args, "--export-texture-png") {
            Ok(p) => p,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        };
        let Some(out_path) = arg_value(&args, "--out") else {
            eprintln!("error: --export-texture-png requires --out <FILE.png>");
            return ExitCode::FAILURE;
        };
        return match zone_dump::export_texture_png(
            &client_root,
            &vals[0],
            &vals[1],
            &PathBuf::from(out_path),
        ) {
            Ok(msg) => {
                println!("{msg}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if let Some(zone) = arg_value(&args, "--dump-zone") {
        let client_root = match resolve_client_root(&args, "--dump-zone") {
            Ok(p) => p,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        };
        let summary = zone_dump::dump_zone(&client_root, &zone);
        print!("{}", summary.report_text);
        if let Some(out_path) = arg_value(&args, "--out") {
            let p = PathBuf::from(&out_path);
            if let Some(parent) = p.parent() {
                if !parent.as_os_str().is_empty() {
                    let _ = fs::create_dir_all(parent);
                }
            }
            if let Err(err) = fs::write(&p, &summary.report_text) {
                eprintln!("error: failed to write {}: {err}", p.display());
                return ExitCode::FAILURE;
            }
        }
        return if summary.trn2_ok && summary.decoded_blocks > 0 {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    if args.iter().any(|arg| arg == "--self-test") {
        return match self_test::run() {
            Ok(results) => {
                for check in results {
                    println!("[PASS] {}: {}", check.name, check.detail);
                }
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("[FAIL] self-test: {error}");
                ExitCode::FAILURE
            }
        };
    }
    if args.iter().any(|arg| arg == "--smoke-launch") {
        return match ui::run_smoke_launch() {
            Ok(()) => {
                println!("[PASS] smoke-launch: initialized app state and rendered all UI tabs");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("[FAIL] smoke-launch: {error}");
                ExitCode::FAILURE
            }
        };
    }

    if let Err(error) = ui::run_window() {
        eprintln!("error: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
