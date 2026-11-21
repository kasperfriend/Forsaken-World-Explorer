//! Offline WGSL shader validator using `naga`.
//! Parses and validates every `.wgsl` file in `src/shaders/` and fails non-zero on any error.

use std::{fs, path::Path, process::ExitCode};

use naga::{
    front::wgsl,
    valid::{Capabilities, ValidationFlags, Validator},
};

fn validate_shader(path: &Path) -> Result<(), String> {
    let source =
        fs::read_to_string(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let module = wgsl::parse_str(&source).map_err(|e| {
        format!(
            "WGSL parse error in {}:\n{}",
            path.display(),
            e.emit_to_string(&source)
        )
    })?;
    let mut validator = Validator::new(ValidationFlags::all(), Capabilities::empty());
    validator.validate(&module).map_err(|e| {
        format!(
            "WGSL validation error in {}:\n{}",
            path.display(),
            e.emit_to_string(&source)
        )
    })?;
    Ok(())
}

fn main() -> ExitCode {
    let shader_dir = Path::new("src/shaders");
    let entries = match fs::read_dir(shader_dir) {
        Ok(rd) => rd,
        Err(e) => {
            eprintln!("error: could not list {}: {e}", shader_dir.display());
            return ExitCode::FAILURE;
        }
    };

    let mut shader_paths = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("wgsl"))
        {
            shader_paths.push(path);
        }
    }
    shader_paths.sort();

    if shader_paths.is_empty() {
        eprintln!("error: no .wgsl shaders found in {}", shader_dir.display());
        return ExitCode::FAILURE;
    }

    let mut failed = false;
    for path in &shader_paths {
        match validate_shader(path) {
            Ok(()) => println!("[PASS] {}", path.display()),
            Err(err) => {
                eprintln!("[FAIL] {err}");
                failed = true;
            }
        }
    }

    if failed {
        ExitCode::FAILURE
    } else {
        println!("Validated {} WGSL shader(s) with naga.", shader_paths.len());
        ExitCode::SUCCESS
    }
}
