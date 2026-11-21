mod modules;
mod self_test;
mod settings;
mod ui;

use std::{env, process::ExitCode};

fn print_help() {
    println!(
        "Forsaken World Explorer {}\n\
         \n\
         USAGE:\n\
         \tforsaken-world-explorer [OPTIONS]\n\
         \n\
         OPTIONS:\n\
         \t--self-test    Run CPU-only self-checks (settings, modules, synthetic fixtures) and exit\n\
         \t-V, --version  Print version information\n\
         \t-h, --help     Print this help message",
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

    if let Err(error) = ui::run_window() {
        eprintln!("error: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
