//! Camera-centered static scenery streaming, instance deduplication, and RAM-first
//! content-addressed `.bmd` cache (`<root>/cache/scenery-v1/`).

use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use glam::{Mat4, Vec3};
use sha2::{Digest, Sha256};

use crate::{
    bmd::{AlphaMode, BmdError, BmdModel},
    world_objects::{placement_transform_matrix, OrnamentPlacement},
};

pub const SCENERY_CACHE_SUBDIR: &str = "scenery-v1";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SceneryDiagnostics {
    pub placements_total: usize,
    pub placements_not_in_range: usize,
    pub placements_in_range: usize,
    pub placements_loaded: usize,
    pub placements_failed: usize,
    pub placements_pending: usize,
    pub unique_models_in_range: usize,
    pub models_loaded: usize,
    pub instances_drawn: usize,
    pub decode_failures: usize,
    pub transparent_submeshes_drawn: usize,
    pub transparent_submeshes_skipped: usize,
    pub approximate_lighting_meshes: usize,
    pub textures_cached: usize,
    pub texture_cache_bytes: usize,
    pub texture_budget_bytes: usize,
    pub texture_budget_exceeded: bool,
    pub water_planes_visible: usize,
    pub world_render_passes: usize,
    pub gpu_timing_note: String,
}

/// One deduplicated `.bmd` model paired with all instance transforms that share it.
#[derive(Debug, Clone)]
pub struct ModelInstanceBatch {
    pub model_key: String,
    pub transforms: Vec<Mat4>,
}

/// Select placements within `radius_m` of `camera_pos` (or all placements when
/// `whole_world` is true), sorted nearest-first and capped at `max_instances` (when
/// `whole_world` is false).
pub fn select_visible_placements<'a>(
    placements: &'a [OrnamentPlacement],
    camera_pos: Vec3,
    radius_m: f32,
    max_instances: usize,
    whole_world: bool,
) -> Vec<&'a OrnamentPlacement> {
    let radius_sq = radius_m.max(1.0) * radius_m.max(1.0);
    let mut candidates: Vec<(f32, &'a OrnamentPlacement)> = placements
        .iter()
        .filter_map(|p| {
            let pos = Vec3::from_array(p.position);
            let dist_sq = pos.distance_squared(camera_pos);
            if whole_world || dist_sq <= radius_sq {
                Some((dist_sq, p))
            } else {
                None
            }
        })
        .collect();
    candidates.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then_with(|| a.1.export_id.cmp(&b.1.export_id))
    });
    if !whole_world && candidates.len() > max_instances {
        candidates.truncate(max_instances);
    }
    candidates.into_iter().map(|(_, p)| p).collect()
}

/// Deduplicate N placements by normalized `.bmd` path so repeated models share one
/// decoded mesh and one GPU buffer set.
pub fn dedup_instances_by_model(selected: &[&OrnamentPlacement]) -> Vec<ModelInstanceBatch> {
    let mut grouped: BTreeMap<String, Vec<Mat4>> = BTreeMap::new();
    for p in selected {
        let key = p.normalized_bmd_path().to_ascii_lowercase();
        let mat = placement_transform_matrix(p.position, p.direction, p.up, p.scale);
        grouped.entry(key).or_default().push(mat);
    }
    grouped
        .into_iter()
        .map(|(model_key, transforms)| ModelInstanceBatch {
            model_key,
            transforms,
        })
        .collect()
}

/// RAM-first, content-addressed cache for decoded `.bmd` models backed by `<root>/cache/scenery-v1/`.
pub struct SceneryModelCache {
    cache_dir: PathBuf,
    loaded: HashMap<String, Arc<BmdModel>>,
    failed: HashMap<String, String>,
}

impl SceneryModelCache {
    pub fn open(cache_root: &Path) -> Result<Self, String> {
        let cache_dir = cache_root.join(SCENERY_CACHE_SUBDIR);
        fs::create_dir_all(&cache_dir)
            .map_err(|e| format!("cannot create {}: {e}", cache_dir.display()))?;
        Ok(Self {
            cache_dir,
            loaded: HashMap::new(),
            failed: HashMap::new(),
        })
    }

    pub fn content_key(model_key: &str) -> String {
        let digest = Sha256::digest(model_key.as_bytes());
        let mut hex = String::with_capacity(64);
        for b in digest {
            use std::fmt::Write;
            let _ = write!(&mut hex, "{b:02x}");
        }
        hex
    }

    pub fn get(&self, model_key: &str) -> Option<Arc<BmdModel>> {
        self.loaded.get(model_key).cloned()
    }

    pub fn is_failed(&self, model_key: &str) -> bool {
        self.failed.contains_key(model_key)
    }

    pub fn loaded_count(&self) -> usize {
        self.loaded.len()
    }

    pub fn failed_count(&self) -> usize {
        self.failed.len()
    }

    pub fn load_or_decode<F>(
        &mut self,
        model_key: &str,
        fetch_raw: F,
    ) -> Result<Arc<BmdModel>, String>
    where
        F: FnOnce() -> Result<Vec<u8>, String>,
    {
        if let Some(m) = self.loaded.get(model_key) {
            return Ok(Arc::clone(m));
        }
        if let Some(err) = self.failed.get(model_key) {
            return Err(err.clone());
        }

        let disk_path = self
            .cache_dir
            .join(format!("{}.bmd", Self::content_key(model_key)));
        let raw_result = if disk_path.is_file() {
            fs::read(&disk_path).or_else(|_| fetch_raw())
        } else {
            fetch_raw().inspect(|bytes| {
                let _ = fs::write(&disk_path, bytes);
            })
        };

        let raw = match raw_result {
            Ok(b) => b,
            Err(err) => {
                self.failed.insert(model_key.to_string(), err.clone());
                return Err(err);
            }
        };

        match BmdModel::parse(&raw) {
            Ok(model) => {
                let arc = Arc::new(model);
                self.loaded.insert(model_key.to_string(), Arc::clone(&arc));
                Ok(arc)
            }
            Err(err) => {
                let msg = format!("{model_key}: {err}");
                let _ = fs::remove_file(&disk_path);
                self.failed.insert(model_key.to_string(), msg.clone());
                Err(msg)
            }
        }
    }

    pub fn insert_parsed(&mut self, model_key: &str, model: BmdModel) {
        self.loaded
            .insert(model_key.to_ascii_lowercase(), Arc::new(model));
    }
}

/// Advance multi-frame scenery streaming by decoding at most `loads_per_frame` uncached
/// unique models from `batches`.
/// Returns `(decoded_this_frame, errors_this_frame, drawn_instances, transparent_submesh_count)`.
pub fn step_streaming_frame<F>(
    cache: &mut SceneryModelCache,
    batches: &[ModelInstanceBatch],
    loads_per_frame: usize,
    mut fetch_raw: F,
) -> (usize, Vec<String>, usize, usize)
where
    F: FnMut(&str) -> Result<Vec<u8>, String>,
{
    let budget = loads_per_frame.max(1);
    let mut decoded_this_frame = 0usize;
    let mut errors = Vec::new();
    let mut drawn_instances = 0usize;
    let mut transparent_submeshes = 0usize;

    for batch in batches {
        if cache.get(&batch.model_key).is_none()
            && !cache.is_failed(&batch.model_key)
            && decoded_this_frame < budget
        {
            decoded_this_frame += 1;
            if let Err(err) = cache.load_or_decode(&batch.model_key, || fetch_raw(&batch.model_key))
            {
                errors.push(err);
            }
        }
        if let Some(model) = cache.get(&batch.model_key) {
            if !model.collide_only {
                let has_drawable = model
                    .submeshes
                    .iter()
                    .any(|s| s.alpha_mode != AlphaMode::TransparentDeferred);
                if has_drawable {
                    drawn_instances += batch.transforms.len();
                }
                transparent_submeshes += model
                    .submeshes
                    .iter()
                    .filter(|s| s.alpha_mode == AlphaMode::TransparentDeferred)
                    .count()
                    * batch.transforms.len();
            }
        }
    }

    (
        decoded_this_frame,
        errors,
        drawn_instances,
        transparent_submeshes,
    )
}

/// Compute exact per-placement counters for the current frame (counting each placement once):
/// `total = not_in_range + in_range` and `in_range = loaded + failed + pending`.
pub fn compute_placement_counters(
    total_placements: usize,
    in_range: &[&OrnamentPlacement],
    cache: &SceneryModelCache,
) -> (usize, usize, usize, usize, usize) {
    let in_range_cnt = in_range.len().min(total_placements);
    let not_in_range = total_placements.saturating_sub(in_range_cnt);
    let mut loaded = 0usize;
    let mut failed = 0usize;
    let mut pending = 0usize;
    for p in in_range.iter().take(in_range_cnt) {
        let key = p.normalized_bmd_path().to_ascii_lowercase();
        if cache.get(&key).is_some() {
            loaded += 1;
        } else if cache.is_failed(&key) {
            failed += 1;
        } else {
            pending += 1;
        }
    }
    (not_in_range, in_range_cnt, loaded, failed, pending)
}

pub fn format_bmd_error(model_key: &str, err: &BmdError) -> String {
    format!("{model_key}: {err}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bmd::synthetic;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_placement(id: u32, path: &str, pos: [f32; 3]) -> OrnamentPlacement {
        OrnamentPlacement {
            export_id: id,
            asset_path: path.to_string(),
            position: pos,
            direction: [0.0, 0.0, 1.0],
            up: [0.0, 1.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            reflect: false,
            refract: false,
            optimize: false,
        }
    }

    #[test]
    fn streaming_window_selects_nearest_and_deduplicates_instances() {
        let placements = vec![
            make_placement(1, "1\\house.bmd", [10.0, 0.0, 0.0]),
            make_placement(2, "1\\house.bmd", [30.0, 0.0, 0.0]),
            make_placement(3, "1\\tower.bmd", [20.0, 0.0, 0.0]),
            make_placement(4, "1\\far.bmd", [500.0, 0.0, 0.0]),
        ];

        let visible = select_visible_placements(&placements, Vec3::ZERO, 100.0, 10, false);
        assert_eq!(visible.len(), 3);
        assert_eq!(visible[0].export_id, 1);
        assert_eq!(visible[1].export_id, 3);
        assert_eq!(visible[2].export_id, 2);

        let batches = dedup_instances_by_model(&visible);
        assert_eq!(batches.len(), 2);
        let house = batches
            .iter()
            .find(|b| b.model_key == "1/house.bmd")
            .unwrap();
        assert_eq!(house.transforms.len(), 2);

        let all = select_visible_placements(&placements, Vec3::ZERO, 100.0, 2, true);
        assert_eq!(all.len(), 4);
    }

    #[test]
    fn multi_frame_streaming_respects_per_frame_budget_and_disk_cache() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp =
            std::env::temp_dir().join(format!("fw-scenery-test-{}-{nanos}", std::process::id()));
        let mut cache = SceneryModelCache::open(&tmp).expect("open cache");

        let placements = vec![
            make_placement(1, "a.bmd", [0.0, 0.0, 0.0]),
            make_placement(2, "b.bmd", [10.0, 0.0, 0.0]),
            make_placement(3, "c.bmd", [20.0, 0.0, 0.0]),
        ];
        let visible = select_visible_placements(&placements, Vec3::ZERO, 100.0, 10, false);
        let batches = dedup_instances_by_model(&visible);

        let raw_box = synthetic::build_bmd_v5_box("textures/wall.dds", 2.0, 6.0);
        let (dec1, errs1, drawn1, _) =
            step_streaming_frame(&mut cache, &batches, 2, |_| Ok(raw_box.clone()));
        assert_eq!(dec1, 2);
        assert!(errs1.is_empty());
        assert_eq!(drawn1, 2);

        let (dec2, errs2, drawn2, _) =
            step_streaming_frame(&mut cache, &batches, 2, |_| Ok(raw_box.clone()));
        assert_eq!(dec2, 1);
        assert!(errs2.is_empty());
        assert_eq!(drawn2, 3);

        let mut cache2 = SceneryModelCache::open(&tmp).expect("reopen cache");
        let model = cache2
            .load_or_decode("a.bmd", || Err("should hit disk cache".into()))
            .expect("disk cache hit");
        assert_eq!(model.submeshes.len(), 1);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn placement_counters_add_up_and_count_each_placement_once() {
        let dir = std::env::temp_dir().join(format!("fw_cnt_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut cache = SceneryModelCache::open(&dir).unwrap();
        let good = synthetic::build_bmd_v5_box("textures/wall.dds", 2.0, 6.0);
        let _ = cache.load_or_decode("a.bmd", || Ok(good.clone()));
        let _ = cache.load_or_decode("b.bmd", || Err("unsupported version".into()));

        // 5 total placements: 4 in range (2 share a.bmd loaded, 1 b.bmd failed, 1 c.bmd pending), 1 out of range
        let mk = |id: u32, path: &str, x: f32| OrnamentPlacement {
            export_id: id,
            asset_path: path.into(),
            position: [x, 0.0, 0.0],
            direction: [0.0, 0.0, 1.0],
            up: [0.0, 1.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            reflect: false,
            refract: false,
            optimize: false,
        };
        let all = vec![
            mk(1, "a.bmd", 10.0),
            mk(2, "a.bmd", 15.0),
            mk(3, "b.bmd", 20.0),
            mk(4, "c.bmd", 25.0),
            mk(5, "d.bmd", 900.0),
        ];
        let in_range = select_visible_placements(&all, Vec3::ZERO, 100.0, 500, false);
        let (not_in_range, in_range_cnt, loaded, failed, pending) =
            compute_placement_counters(all.len(), &in_range, &cache);
        assert_eq!(not_in_range, 1);
        assert_eq!(in_range_cnt, 4);
        assert_eq!(loaded, 2);
        assert_eq!(failed, 1);
        assert_eq!(pending, 1);
        assert_eq!(all.len(), not_in_range + in_range_cnt);
        assert_eq!(in_range_cnt, loaded + failed + pending);
        let _ = fs::remove_dir_all(&dir);
    }
}
