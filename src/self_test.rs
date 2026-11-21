//! CPU-only `--self-test` checks for CI and local verification without a GPU.

use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use glam::Vec3;

use crate::{
    atmosphere, bmd, camera, dxt, logging, modules, pck, scenery, settings, terrain_block,
    terrain_store, tex_cache, world_objects,
};

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
        scenery_enabled: true,
        transparent_scenery_enabled: true,
        scenery_radius_m: 320,
        scenery_max_instances: 1500,
        scenery_loads_per_frame: 6,
        scenery_whole_world: false,
        fog_enabled: true,
        fog_distance_mult_pct: 120,
        water_reflection: true,
        water_refraction: false,
        water_quality: settings::WaterQuality::High,
        time_of_day_minutes: 900,
        follow_real_clock: false,
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

fn check_synthetic_scenery_pipeline() -> Result<CheckResult, String> {
    let cache_dir = unique_temp_dir("fw-selftest-scenery")?;
    let res = (|| -> Result<CheckResult, String> {
        let raw_bmd = bmd::synthetic::build_bmd_v5_box("textures/stone_tower.dds", 4.0, 12.0);
        let model = bmd::BmdModel::parse(&raw_bmd).map_err(|e| e.to_string())?;
        if model.submeshes.len() != 1 || model.submeshes[0].indices.len() != 36 {
            return Err("unexpected synthetic BMD mesh counts".into());
        }

        let placements = vec![
            world_objects::OrnamentPlacement {
                export_id: 1,
                asset_path: "1\\tower.bmd".into(),
                position: [10.0, 0.0, 10.0],
                direction: [0.0, 0.0, 1.0],
                up: [0.0, 1.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                reflect: false,
                refract: false,
                optimize: false,
            },
            world_objects::OrnamentPlacement {
                export_id: 2,
                asset_path: "1\\tower.bmd".into(),
                position: [30.0, 0.0, 10.0],
                direction: [1.0, 0.0, 0.0],
                up: [0.0, 1.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                reflect: false,
                refract: false,
                optimize: false,
            },
        ];
        let visible = scenery::select_visible_placements(&placements, Vec3::ZERO, 128.0, 64, false);
        let batches = scenery::dedup_instances_by_model(&visible);
        if batches.len() != 1 || batches[0].transforms.len() != 2 {
            return Err("instance deduplication did not merge identical BMD paths".into());
        }

        let mut model_cache = scenery::SceneryModelCache::open(&cache_dir)?;
        let (decoded, errs, drawn, _) =
            scenery::step_streaming_frame(&mut model_cache, &batches, 4, |_| Ok(raw_bmd.clone()));
        if decoded != 1 || !errs.is_empty() || drawn != 2 {
            return Err("scenery streaming frame counts mismatch".into());
        }

        let mut tex = tex_cache::TextureCache::new(4096);
        tex.begin_visible_batch();
        tex.get_or_insert_pinned("textures/stone_tower.dds", || {
            Ok(tex_cache::DecodedTexture {
                width: 32,
                height: 32,
                rgba: vec![255u8; 4096],
            })
        })?;
        let _ = tex.enforce_budget_after_batch();
        if tex.stats().count != 1 {
            return Err("texture cache failed to retain pinned texture".into());
        }

        Ok(CheckResult {
            name: "synthetic-scenery-pipeline",
            detail: "verified .bmd decode, instance dedup, multi-frame streaming, and pinned texture cache".into(),
        })
    })();
    let _ = fs::remove_dir_all(&cache_dir);
    res
}

fn check_phase5_atmosphere_and_water() -> Result<CheckResult, String> {
    let wrapped = atmosphere::advance_time_of_day(23.75, 0.5);
    if (wrapped - 0.25).abs() > 1e-4 {
        return Err(format!("midnight wraparound failed: {wrapped}"));
    }
    if (atmosphere::day_night_factor(5.0) - 0.5).abs() > 1e-4 {
        return Err("dawn day/night blend factor mismatch".into());
    }
    if (atmosphere::evaluate_fog_factor(150.0, 50.0, 250.0, 1.0, true) - 0.5).abs() > 1e-4 {
        return Err("fog midpoint factor mismatch".into());
    }
    let cam = camera::FreeCamera {
        position: Vec3::new(0.0, 40.0, 0.0),
        yaw: 0.0,
        pitch: -0.3,
        ..Default::default()
    };
    let plane = atmosphere::WaterPlane {
        id: 1,
        center_x: 0.0,
        center_z: 80.0,
        half_x: 30.0,
        half_z: 30.0,
        height_y: 10.0,
        color_rgba: [0.1, 0.4, 0.6, 0.85],
    };
    if atmosphere::compute_water_scissor(&plane, cam.view_proj(16.0 / 9.0), 800, 600).is_none() {
        return Err("visible water plane failed scissor rect computation".into());
    }
    let mut order = vec![
        (1u64, Vec3::new(0.0, 0.0, 5.0)),
        (2u64, Vec3::new(0.0, 0.0, 50.0)),
    ];
    atmosphere::sort_transparent_back_to_front(&mut order, Vec3::ZERO, |x| (x.1, x.0));
    if order[0].0 != 2 {
        return Err("transparent back-to-front sort failed".into());
    }
    Ok(CheckResult {
        name: "atmosphere-water-transparency",
        detail: "verified time-of-day wraparound, dn_factor, fog curves, water culling/scissor, and transparent sort".into(),
    })
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
        check_synthetic_scenery_pipeline()?,
        check_phase5_atmosphere_and_water()?,
        check_optional_client_env(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_test_passes_on_cpu() {
        let results = run().expect("self-test should pass");
        assert!(results.len() >= 7);
    }
}
