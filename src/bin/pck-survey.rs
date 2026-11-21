//! Survey tool for listing every `.pck`/`.pkx` archive in a Forsaken World client.
//!
//! Writes a Markdown report containing counts, sizes, extensions, and any read errors
//! with no proprietary client asset data included.

use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use forsaken_world_explorer::{client_detect, pck::PckArchive, settings::ENV_CLIENT_DIR};

fn print_usage() {
    println!(
        "pck-survey — list and optionally verify every .pck/.pkx archive in a Forsaken World client\n\
         \n\
         USAGE:\n\
         \tpck-survey [CLIENT_DIR] [--verify-payloads] [--out REPORT.md]\n\
         \n\
         If CLIENT_DIR is omitted, FW_CLIENT_DIR is checked. If unset, exits 0 with 'skipped: no client'."
    );
}

fn main() -> ExitCode {
    let mut client_arg: Option<PathBuf> = None;
    let mut out_path: Option<PathBuf> = None;
    let mut verify_payloads = false;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                return ExitCode::SUCCESS;
            }
            "--verify-payloads" => verify_payloads = true,
            "--out" => {
                let Some(path) = args.next() else {
                    eprintln!("error: --out requires a file path");
                    return ExitCode::FAILURE;
                };
                out_path = Some(PathBuf::from(path));
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

    let mut report = String::new();
    report.push_str("# Forsaken World Archive Survey\n\n");
    report.push_str(&format!(
        "- **Executable:** `update/pem.exe` ({} bytes, SHA-256 `{}`)\n",
        client.executable_size, client.executable_sha256
    ));
    report.push_str(&format!(
        "- **Recognition:** {}\n",
        client.recognition.summary()
    ));
    report.push_str(&format!(
        "- **SpeedTree DLL present:** {}\n",
        client.has_speedtree_dll
    ));
    report.push_str(&format!(
        "- **Readable map zones:** {} ({})\n\n",
        client.zones.len(),
        client.zones.join(", ")
    ));
    report.push_str(
        "| Archive | Version | Entries | Logical Size (B) | Companion `.pkx` (B) | Status |\n",
    );
    report.push_str("| --- | --- | ---: | ---: | ---: | --- |\n");

    let mut ext_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures = 0usize;

    for name in &client.pck_archives {
        let path = client.resources_dir.join(name);
        match PckArchive::open(&path) {
            Ok(mut archive) => {
                let info = archive.info().clone();
                for entry in archive.entries() {
                    let ext = Path::new(&entry.name)
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| format!(".{}", e.to_ascii_lowercase()))
                        .unwrap_or_else(|| "(none)".to_string());
                    *ext_counts.entry(ext).or_insert(0) += 1;
                }
                let status = if verify_payloads {
                    let mut payload_errors = 0usize;
                    for i in 0..info.entry_count {
                        if archive.read_entry_by_index(i).is_err() {
                            payload_errors += 1;
                        }
                    }
                    if payload_errors == 0 {
                        "ok (all payloads verified)".to_string()
                    } else {
                        failures += 1;
                        format!("error ({payload_errors} payload failures)")
                    }
                } else {
                    "ok".to_string()
                };
                report.push_str(&format!(
                    "| `{name}` | `0x{:08x}` | {} | {} | {} | {} |\n",
                    info.version, info.entry_count, info.logical_size, info.pkx_size, status
                ));
            }
            Err(error) => {
                failures += 1;
                report.push_str(&format!("| `{name}` | — | — | — | — | error: {error} |\n"));
            }
        }
    }

    report.push_str("\n## Entry Extension Census\n\n| Extension | Count |\n| --- | ---: |\n");
    for (ext, count) in ext_counts {
        report.push_str(&format!("| `{ext}` | {count} |\n"));
    }

    if let Some(out) = out_path {
        if let Err(error) = fs::write(&out, &report) {
            eprintln!("error writing {}: {error}", out.display());
            return ExitCode::FAILURE;
        }
        println!("Wrote archive survey report to {}", out.display());
    } else {
        print!("{report}");
    }

    if failures > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
