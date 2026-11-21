//! CPU-only `--self-test` checks for CI and local verification without a GPU.

use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{modules, settings};

#[derive(Debug)]
pub struct CheckResult {
    pub name: &'static str,
    pub detail: String,
}

fn unique_temp_dir(prefix: &str) -> Result<PathBuf, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

fn check_settings_round_trip() -> Result<CheckResult, String> {
    let dir = unique_temp_dir("fw-selftest-settings")?;
    let expected = settings::ViewerSettings {
        client_dir: Some(PathBuf::from("C:\\Games\\ForsakenWorld")),
        resolution_scale_pct: 110,
        terrain_ram_mib: 4,
        tex_budget_mib: 32,
        music_volume: 65,
        ambience_volume: 50,
        effects_volume: 85,
        show_minimap: true,
        show_zone_notices: false,
    };
    let saved = expected.save_to_dir(&dir);
    let loaded = saved
        .as_ref()
        .map_err(|e| e.clone())
        .and_then(|_| settings::ViewerSettings::load_from_dir(&dir));
    let _ = fs::remove_dir_all(&dir);

    let loaded = loaded?;
    if loaded != expected {
        return Err(format!(
            "settings mismatch after round-trip: expected {expected:?}, got {loaded:?}"
        ));
    }
    Ok(CheckResult {
        name: "settings-round-trip",
        detail: "wrote and re-read viewer-settings.txt cleanly".into(),
    })
}

fn check_module_discovery() -> Result<CheckResult, String> {
    let default_root = modules::Registry::default_root();
    if default_root.is_dir() {
        let registry = modules::Registry::discover(default_root.clone());
        if !registry.errors.is_empty() {
            return Err(format!(
                "module errors under {}: {}",
                default_root.display(),
                registry.errors.join("; ")
            ));
        }
        if registry.packages.is_empty() {
            return Err(format!(
                "no module packages found under {}",
                default_root.display()
            ));
        }
        let ids: Vec<&str> = registry.packages.iter().map(|p| p.id.as_str()).collect();
        return Ok(CheckResult {
            name: "module-discovery",
            detail: format!(
                "discovered {} package(s) beside executable: {}",
                registry.packages.len(),
                ids.join(", ")
            ),
        });
    }

    // Fallback when invoked directly from `target/debug` without copied `Modules/`:
    // validate discovery and broken-manifest isolation on a temporary synthetic tree.
    let synthetic = unique_temp_dir("fw-selftest-modules")?;
    let res = (|| -> Result<CheckResult, String> {
        let valid_dir = synthetic.join("Synthetic");
        let broken_dir = synthetic.join("Broken");
        fs::create_dir_all(&valid_dir).map_err(|e| e.to_string())?;
        fs::create_dir_all(&broken_dir).map_err(|e| e.to_string())?;
        fs::write(
            valid_dir.join(modules::MODULE_MANIFEST),
            "format=1\nid=synthetic\nname=Synthetic\nversion=0.1.0\ndescription=Self-test module\n",
        )
        .map_err(|e| e.to_string())?;
        fs::write(broken_dir.join(modules::MODULE_MANIFEST), "corrupt_line\n")
            .map_err(|e| e.to_string())?;

        let registry = modules::Registry::discover(synthetic.clone());
        if registry.packages.len() != 1 || registry.errors.len() != 1 {
            return Err(format!(
                "expected 1 valid package and 1 isolated error, got {} package(s) and {} error(s)",
                registry.packages.len(),
                registry.errors.len()
            ));
        }
        Ok(CheckResult {
            name: "module-discovery",
            detail: "verified synthetic package discovery and broken-manifest isolation".into(),
        })
    })();
    let _ = fs::remove_dir_all(&synthetic);
    res
}

fn check_synthetic_fixture_parsing() -> Result<CheckResult, String> {
    // Phase 0 has no client archive parser yet (Phase 1); verify deterministic
    // parsing and malformed-input rejection on a synthetic manifest fixture.
    let dir = unique_temp_dir("fw-selftest-fixture")?;
    let res = (|| -> Result<CheckResult, String> {
        let good = dir.join("good");
        fs::create_dir_all(&good).map_err(|e| e.to_string())?;
        fs::write(
            good.join(modules::MODULE_MANIFEST),
            "format=1\nid=fixture-ok\nname=Fixture\nversion=0.1.0\ndescription=Synthetic fixture\n",
        )
        .map_err(|e| e.to_string())?;
        let pkg = modules::Package::load(good)?;
        if pkg.id != "fixture-ok" {
            return Err(format!("unexpected fixture id '{}'", pkg.id));
        }

        let bad = dir.join("bad");
        fs::create_dir_all(&bad).map_err(|e| e.to_string())?;
        fs::write(bad.join(modules::MODULE_MANIFEST), [0xff, 0x00, 0xfe, 0x01])
            .map_err(|e| e.to_string())?;
        if modules::Package::load(bad).is_ok() {
            return Err("malformed binary manifest unexpectedly succeeded".into());
        }
        Ok(CheckResult {
            name: "synthetic-fixture",
            detail: "verified valid fixture load and malformed-bytes rejection".into(),
        })
    })();
    let _ = fs::remove_dir_all(&dir);
    res
}

fn check_optional_client_env() -> CheckResult {
    match env::var_os(settings::ENV_CLIENT_DIR).filter(|v| !v.is_empty()) {
        Some(path) => {
            let path = PathBuf::from(path);
            let exists = Path::new(&path).is_dir();
            CheckResult {
                name: "real-client-check",
                detail: if exists {
                    format!(
                        "FW_CLIENT_DIR is set and exists ({}); archive parsing arrives in Phase 1",
                        path.display()
                    )
                } else {
                    format!(
                        "FW_CLIENT_DIR is set but directory not found ({})",
                        path.display()
                    )
                },
            }
        }
        None => CheckResult {
            name: "real-client-check",
            detail: "skipped: no client (FW_CLIENT_DIR not set)".into(),
        },
    }
}

/// Run all Phase 0 CPU-only self-tests.
pub fn run() -> Result<Vec<CheckResult>, String> {
    Ok(vec![
        check_settings_round_trip()?,
        check_module_discovery()?,
        check_synthetic_fixture_parsing()?,
        check_optional_client_env(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_test_passes_on_cpu() {
        let results = run().expect("self-test should pass");
        assert!(results.len() >= 4);
    }
}
