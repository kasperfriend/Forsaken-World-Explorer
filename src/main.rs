mod ui;

use std::{env, process::ExitCode};

use forsaken_world_explorer::self_test;

fn print_help() {
    println!(
        "Forsaken World Explorer {}\n\
         \n\
         USAGE:\n\
         \tforsaken-world-explorer [OPTIONS]\n\
         \n\
         OPTIONS:\n\
         \t--self-test     Run CPU-only self-checks (settings, modules, synthetic PCK/PKX) and exit\n\
         \t--smoke-launch  Initialize app state and render every UI page headlessly without a GPU\n\
         \t-V, --version   Print version information\n\
         \t-h, --help      Print this help message",
        env!("CARGO_PKG_VERSION")
    );
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
