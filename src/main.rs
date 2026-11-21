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
         \t--dump-zone <ZONE>  Decode a real client zone stage-by-stage and print/write counts and samples\n\
         \t--client <PATH>     Client root folder for --dump-zone (defaults to FW_CLIENT_DIR)\n\
         \t--out <PATH>        Optional output file path for --dump-zone report\n\
         \t--self-test         Run CPU-only self-checks and exit\n\
         \t--smoke-launch      Initialize app state and render every UI page headlessly without a GPU\n\
         \t-V, --version       Print version information\n\
         \t-h, --help          Print this help message",
        env!("CARGO_PKG_VERSION")
    );
}

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
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
    if let Some(zone) = arg_value(&args, "--dump-zone") {
        let client_root = arg_value(&args, "--client")
            .or_else(|| env::var("FW_CLIENT_DIR").ok())
            .map(PathBuf::from);
        let Some(client_root) = client_root else {
            eprintln!("error: --dump-zone requires --client <PATH> or FW_CLIENT_DIR");
            return ExitCode::FAILURE;
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
