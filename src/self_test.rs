//! CPU-only `--self-test` checks for CI and local verification without a GPU.

use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{dxt, logging, modules, pck, settings, terrain_block, terrain_store};

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

fn check_portable_settings_and_logs() -> Result<CheckResult, String> {
    let root = unique_temp_dir("fw-selftest-portable")?;
    let paths = settings::PortablePaths::from_root(root.clone());
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
    let result = (|| -> Result<(), String> {
        paths.check_writable()?;
        expected.save_to_paths(&paths)?;
        let loaded = settings::ViewerSettings::load_from_paths(&paths)?;
        if loaded != expected {
            return Err(format!(
                "settings mismatch after round-trip: expected {expected:?}, got {loaded:?}"
            ));
        }
        let mut logger = logging::RunLogger::open(&paths.logs_dir)?;
        logger.info("self-test log entry");
        if !logger.log_path().is_file() {
            return Err("expected per-run log file to exist".into());
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&root);
    result?;
    Ok(CheckResult {
        name: "portable-settings-and-logs",
        detail: "verified <root>/settings/viewer-settings.txt round-trip and <root>/logs/ creation"
            .into(),
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

fn check_synthetic_pck_archive() -> Result<CheckResult, String> {
    let dir = unique_temp_dir("fw-selftest-pck")?;
    let res = (|| -> Result<CheckResult, String> {
        let pck_path = dir.join("synthetic.pck");
        let payload = b"Forsaken World Explorer synthetic PCK payload\n".repeat(16);
        pck::synthetic::write_pck(&pck_path, &[("Configs\\test.cfg", &payload)])?;
        let mut archive = pck::PckArchive::open(&pck_path)?;
        let extracted = archive.read_entry("configs/test.cfg")?;
        if extracted != payload {
            return Err("extracted synthetic PCK payload did not match".into());
        }

        let split_path = dir.join("split.pck");
        pck::synthetic::write_split_pck(&split_path, &[("data\\a.bin", &payload)], 64)?;
        let mut split_archive = pck::PckArchive::open(&split_path)?;
        if split_archive.read_entry("data/a.bin")? != payload {
            return Err("extracted split PCK/PKX payload did not match".into());
        }

        let bad_path = dir.join("corrupt.pck");
        fs::write(&bad_path, [0xffu8; 320]).map_err(|e| e.to_string())?;
        if pck::PckArchive::open(&bad_path).is_ok() {
            return Err("corrupt PCK archive unexpectedly opened".into());
        }

        Ok(CheckResult {
            name: "synthetic-pck-archive",
            detail: "verified single .pck, split .pck/.pkx, and corrupt-archive rejection".into(),
        })
    })();
    let _ = fs::remove_dir_all(&dir);
    res
}

fn check_synthetic_world_decoders() -> Result<CheckResult, String> {
    let cache_dir = unique_temp_dir("fw-selftest-terrain")?;
    let res = (|| -> Result<CheckResult, String> {
        let bytes = terrain_block::synthetic::build_t2bk_v6(8, 2);
        let t2bk = terrain_block::T2bkFile::parse_header(&bytes)?;
        let block0 = t2bk.decode_block(&bytes, 0, 8)?;
        if block0.heights.len() != 81 || block0.normals.len() != 81 {
            return Err("unexpected synthetic terrain block dimensions".into());
        }

        let mut store =
            terrain_store::TerrainStore::open(&cache_dir, terrain_store::DEFAULT_HOT_RAM_BYTES)?;
        let key = terrain_store::TerrainStore::content_key(&bytes);
        let page = store.get_or_insert_with(&key, || {
            Ok(terrain_store::TerrainPage {
                heights: block0.heights.clone(),
                normals: block0.normals.clone(),
            })
        })?;
        if page.heights != block0.heights {
            return Err("terrain cache round-trip mismatch".into());
        }

        let dxt1 = [0x00, 0xf8, 0x1f, 0x00, 0xe4, 0xe4, 0xe4, 0xe4];
        let dds = dxt::synthetic::build_dds(4, 4, b"DXT1", &dxt1);
        let surface = dxt::parse_dds(&dds)?;
        let rgba = dxt::decode_rgba8(&surface);
        if rgba.len() != 64 {
            return Err("unexpected decoded DDS pixel count".into());
        }

        Ok(CheckResult {
            name: "synthetic-world-decoders",
            detail: "verified v6 .t2bk decode, cache/terrain-v1, and DXT1 DDS decode".into(),
        })
    })();
    let _ = fs::remove_dir_all(&cache_dir);
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
                    match crate::client_detect::detect(&path) {
                        Ok(client) => format!(
                            "detected client at {} ({}, {} .pck archives, {} zones)",
                            client.root.display(),
                            client.recognition.summary(),
                            client.pck_archives.len(),
                            client.zones.len()
                        ),
                        Err(err) => format!("FW_CLIENT_DIR invalid: {err}"),
                    }
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

/// Run all CPU-only self-tests. Never opens any interactive UI dialog.
pub fn run() -> Result<Vec<CheckResult>, String> {
    Ok(vec![
        check_portable_settings_and_logs()?,
        check_module_discovery()?,
        check_synthetic_pck_archive()?,
        check_synthetic_world_decoders()?,
        check_optional_client_env(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_test_passes_on_cpu() {
        let results = run().expect("self-test should pass");
        assert!(results.len() >= 5);
    }
}
