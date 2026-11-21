//! Desktop window, menu, native folder picker, portable root layout, live 3D World View
//! with sky, day/night terrain lightmaps, opaque & transparent `.bmd` static scenery,
//! experimental animated water (reflection/refraction/shoreline/underwater), free-flight
//! camera, resolution scaling, Settings, 1 Hz Diagnostics panel, and Modules.

use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use eframe::{egui, egui_wgpu, wgpu};
use forsaken_world_explorer::{
    atmosphere::{self, BoxAreaAtmosphere, WaterPlane},
    bmd,
    camera::{scaled_render_target, CameraInput, FreeCamera},
    client_detect::{self, DetectedClient},
    dxt,
    input::{CaptureAction, InputStateMachine, MenuPage},
    logging::{self, RunLogger},
    modules::Registry,
    pck::PckArchive,
    scenery::{self, SceneryDiagnostics, SceneryModelCache},
    settings::{
        GrassDensity, PortablePaths, ViewerSettings, WaterQuality, DEFAULT_FOG_DIST_MULT_PCT,
        DEFAULT_SCENERY_LOADS_PER_FRAME, DEFAULT_SCENERY_MAX_INSTANCES, DEFAULT_SCENERY_RADIUS_M,
        DEFAULT_TERRAIN_RAM_MIB, DEFAULT_TEX_BUDGET_MIB, DEFAULT_TREE_LOD_DIST_MULT_PCT,
        DEFAULT_VEG_LOADS_PER_FRAME, DEFAULT_VEG_MAX_INSTANCES, DEFAULT_VEG_RADIUS_M,
        MAX_FOG_DIST_MULT_PCT, MAX_RESOLUTION_SCALE_PCT, MAX_SCENERY_LOADS_PER_FRAME,
        MAX_SCENERY_MAX_INSTANCES, MAX_SCENERY_RADIUS_M, MAX_TREE_LOD_DIST_MULT_PCT,
        MAX_VEG_LOADS_PER_FRAME, MAX_VEG_MAX_INSTANCES, MAX_VEG_RADIUS_M, MIN_FOG_DIST_MULT_PCT,
        MIN_RESOLUTION_SCALE_PCT, MIN_SCENERY_LOADS_PER_FRAME, MIN_SCENERY_MAX_INSTANCES,
        MIN_SCENERY_RADIUS_M, MIN_TREE_LOD_DIST_MULT_PCT, MIN_VEG_LOADS_PER_FRAME,
        MIN_VEG_MAX_INSTANCES, MIN_VEG_RADIUS_M,
    },
    terrain_gpu::{
        build_scenery_batches_for_model, build_tree_batches, CameraUniform, SceneryDrawSnapshot,
        SceneryInstance, TerrainMeshSnapshot, TerrainSession, TerrainTextureSnapshot,
        TerrainViewportRenderer, VegetationDrawBatch, VegetationDrawSnapshot, WaterRenderSnapshot,
    },
    tex_cache::{DecodedTexture, TextureCache},
    vegetation::{
        self, FwstTreeMesh, GrassPlacement, GrassTypeDef, HelperRequest, TreePlacement,
        TreeTypeDef, DEFAULT_HELPER_TIMEOUT_SECS, SPEEDTREE_CACHE_SUBDIR,
    },
    world_objects::{self, OrnamentPlacement},
    zone_dump::{
        bmd_lookup_candidates_for_archive, open_all_resource_pcks,
        texture_lookup_candidates_for_archive, FailureGroup,
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Launcher,
    WorldView,
    Settings(SettingsTab),
    Modules,
    About,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    Graphics,
    World,
    Sound,
    Interface,
}

struct TerrainCallback {
    viewport_w: u32,
    viewport_h: u32,
    scale_pct: u16,
    uniform: CameraUniform,
    mesh: Arc<TerrainMeshSnapshot>,
    day_lightmap: Arc<TerrainTextureSnapshot>,
    night_lightmap: Arc<TerrainTextureSnapshot>,
    scenery: Arc<SceneryDrawSnapshot>,
    vegetation: Arc<VegetationDrawSnapshot>,
    water: Arc<WaterRenderSnapshot>,
    draw_sky_gradient: bool,
    terrain_has_real_textures: bool,
    terrain_wireframe_debug: bool,
}

impl egui_wgpu::CallbackTrait for TerrainCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(renderer) = callback_resources.get_mut::<TerrainViewportRenderer>() else {
            return Vec::new();
        };
        vec![renderer.prepare_frame(
            device,
            queue,
            self.viewport_w,
            self.viewport_h,
            self.scale_pct,
            &self.uniform,
            &self.mesh,
            &self.day_lightmap,
            &self.night_lightmap,
            &self.scenery,
            &self.vegetation,
            &self.water,
            self.draw_sky_gradient,
            self.terrain_has_real_textures,
            self.terrain_wireframe_debug,
        )]
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(renderer) = callback_resources.get::<TerrainViewportRenderer>() {
            renderer.paint_blit(render_pass);
        }
    }
}

struct ActiveSceneryState {
    client_root: Option<PathBuf>,
    zone: String,
    placements: Vec<OrnamentPlacement>,
    box_areas: Vec<BoxAreaAtmosphere>,
    water_planes: Vec<WaterPlane>,
    model_cache: SceneryModelCache,
    tex_cache: TextureCache,
    resource_archives: Vec<(String, String, PckArchive)>,
    archive_open_errors: Vec<(String, String)>,
    failure_groups: BTreeMap<String, FailureGroup>,
    bmd_tex_failed: usize,
    synthetic_bmd_bytes: Option<Vec<u8>>,
    revision: u64,
}

impl ActiveSceneryState {
    fn open_zone(
        client_root: &Path,
        zone: &str,
        cache_root: &Path,
        tex_budget_bytes: usize,
    ) -> Result<Self, String> {
        let zone_dir = client_root.join("maps").join(zone);
        let wld_bytes = fs::read(zone_dir.join(format!("{zone}.ecwld")))
            .map_err(|e| format!("failed to read {zone}.ecwld: {e}"))?;
        let bsd_bytes = fs::read(zone_dir.join(format!("{zone}.ecbsd")))
            .map_err(|e| format!("failed to read {zone}.ecbsd: {e}"))?;
        let (_, _, placements) =
            world_objects::parse_ecwld_ecbsd_ornaments(&wld_bytes, &bsd_bytes)?;
        let (_, _, box_areas, _water_refs) =
            atmosphere::parse_ecwld_ecbsd_atmosphere_and_water(&wld_bytes, &bsd_bytes)
                .unwrap_or_else(|_| {
                    (
                        world_objects::EcwldHeader {
                            version: 14,
                            world_width: 1024.0,
                            world_length: 1024.0,
                            block_size: 64.0,
                            block_grid: 32,
                            num_blocks: 0,
                            block_rows: 0,
                            block_cols: 0,
                        },
                        world_objects::EcbsdHeader {
                            version: 15,
                            sub_terrain_count: 1,
                        },
                        Vec::new(),
                        Vec::new(),
                    )
                });

        let (resource_archives, archive_open_errors) = open_all_resource_pcks(client_root);
        let model_cache = SceneryModelCache::open(cache_root)?;
        let tex_cache = TextureCache::new(tex_budget_bytes);
        Ok(Self {
            client_root: Some(client_root.to_path_buf()),
            zone: zone.to_string(),
            placements,
            box_areas,
            water_planes: Vec::new(),
            model_cache,
            tex_cache,
            resource_archives,
            archive_open_errors,
            failure_groups: BTreeMap::new(),
            bmd_tex_failed: 0,
            synthetic_bmd_bytes: None,
            revision: 0,
        })
    }

    fn open_synthetic(cache_root: &Path, tex_budget_bytes: usize) -> Result<Self, String> {
        let model_cache = SceneryModelCache::open(cache_root)?;
        let tex_cache = TextureCache::new(tex_budget_bytes);
        let raw_bmd = bmd::synthetic::build_bmd_v4_multi_submesh();
        let placements = vec![
            OrnamentPlacement {
                export_id: 1,
                asset_path: "synthetic/pavilion.bmd".into(),
                position: [96.0, 100.0, 96.0],
                direction: [0.0, 0.0, 1.0],
                up: [0.0, 1.0, 0.0],
                scale: [3.0, 4.0, 3.0],
                reflect: false,
                refract: false,
                optimize: false,
            },
            OrnamentPlacement {
                export_id: 2,
                asset_path: "synthetic/pavilion.bmd".into(),
                position: [152.0, 102.0, 118.0],
                direction: [1.0, 0.0, 0.0],
                up: [0.0, 1.0, 0.0],
                scale: [3.5, 4.0, 3.5],
                reflect: false,
                refract: false,
                optimize: false,
            },
        ];
        let water_planes = vec![WaterPlane {
            id: 1,
            center_x: 128.0,
            center_z: 128.0,
            half_x: 72.0,
            half_z: 72.0,
            height_y: 108.0,
            color_rgba: [0.06, 0.54, 0.84, 0.85],
        }];
        Ok(Self {
            client_root: None,
            zone: "synthetic-preview".to_string(),
            placements,
            box_areas: vec![BoxAreaAtmosphere::default()],
            water_planes,
            model_cache,
            tex_cache,
            resource_archives: Vec::new(),
            archive_open_errors: Vec::new(),
            failure_groups: BTreeMap::new(),
            bmd_tex_failed: 0,
            synthetic_bmd_bytes: Some(raw_bmd),
            revision: 0,
        })
    }

    fn active_box_area(&self) -> Option<&BoxAreaAtmosphere> {
        self.box_areas
            .iter()
            .find(|b| b.is_default)
            .or_else(|| self.box_areas.first())
    }

    fn step_frame(
        &mut self,
        camera_pos: glam::Vec3,
        view_proj: glam::Mat4,
        vp_w: u32,
        vp_h: u32,
        settings: &ViewerSettings,
        logger: &mut Option<RunLogger>,
    ) -> (SceneryDrawSnapshot, SceneryDiagnostics) {
        let tex_budget_bytes = (settings.tex_budget_mib as usize) * 1024 * 1024;
        self.tex_cache.set_budget_bytes(tex_budget_bytes);

        let vis_water = atmosphere::visible_water_planes(&self.water_planes, view_proj, vp_w, vp_h);
        let water_passes = if vis_water.is_empty() {
            0
        } else {
            usize::from(settings.water_reflection) + usize::from(settings.water_refraction)
        };
        let world_render_passes = 1 + water_passes;

        if !settings.scenery_enabled {
            let tstats = self.tex_cache.stats();
            let diag = SceneryDiagnostics {
                placements_total: self.placements.len(),
                placements_in_range: 0,
                unique_models_in_range: 0,
                models_loaded: self.model_cache.loaded_count(),
                instances_drawn: 0,
                decode_failures: self.model_cache.failed_count(),
                transparent_submeshes_drawn: 0,
                transparent_submeshes_skipped: 0,
                approximate_lighting_meshes: 0,
                textures_cached: tstats.count,
                texture_cache_bytes: tstats.total_bytes,
                texture_budget_bytes: tstats.budget_bytes,
                texture_budget_exceeded: tstats.budget_exceeded,
                water_planes_visible: vis_water.len(),
                world_render_passes,
                gpu_timing_note: "Timestamp queries unsupported or skipped".into(),
            };
            self.revision = self.revision.wrapping_add(1);
            return (
                SceneryDrawSnapshot {
                    opaque_batches: Vec::new(),
                    transparent_batches: Vec::new(),
                    revision: self.revision,
                },
                diag,
            );
        }

        let visible = scenery::select_visible_placements(
            &self.placements,
            camera_pos,
            settings.scenery_radius_m as f32,
            settings.scenery_max_instances as usize,
            settings.scenery_whole_world,
        );
        let dedup = scenery::dedup_instances_by_model(&visible);

        let zone = self.zone.clone();
        let client_root_opt = self.client_root.clone();
        let synth = self.synthetic_bmd_bytes.clone();
        let resource_archives = &mut self.resource_archives;
        let has_open_errors = !self.archive_open_errors.is_empty();
        let (_decoded_now, errs, drawn_instances, _total_trans) = scenery::step_streaming_frame(
            &mut self.model_cache,
            &dedup,
            settings.scenery_loads_per_frame as usize,
            |model_key| {
                if let Some(raw) = &synth {
                    return Ok(raw.clone());
                }
                for (_, mount, arch) in resource_archives.iter_mut() {
                    for cand in bmd_lookup_candidates_for_archive(&zone, model_key, Some(mount)) {
                        if let Ok(bytes) = arch.read_entry(&cand) {
                            return Ok(bytes);
                        }
                    }
                }
                if let Some(root) = &client_root_opt {
                    let loose = root
                        .join("litmodels")
                        .join(&zone)
                        .join(model_key.replace('\\', "/"));
                    if let Ok(bytes) = fs::read(&loose) {
                        return Ok(bytes);
                    }
                }
                if resource_archives.is_empty() {
                    if has_open_errors {
                        Err(format!("archive_open_error: {model_key}"))
                    } else {
                        Err(format!("archive_not_found: {model_key}"))
                    }
                } else {
                    Err(format!("entry_not_found: {model_key}"))
                }
            },
        );
        for e in &errs {
            let (reason, sample) = if e.contains("archive_open_error") {
                ("archive_open_error", e.as_str())
            } else if e.contains("archive_not_found") {
                ("archive_not_found", e.as_str())
            } else if e.contains("entry_not_found") {
                ("entry_not_found", e.as_str())
            } else if e.contains("version") {
                ("unsupported_bmd_version", e.as_str())
            } else {
                ("bmd_parse_error", e.as_str())
            };
            self.failure_groups
                .entry(reason.to_string())
                .or_default()
                .record(sample.to_string());
            if let Some(log) = logger.as_mut() {
                log.warn(&format!("Scenery decode error: {e}"));
            }
        }

        self.tex_cache.begin_visible_batch();
        let mut opaque_batches = Vec::new();
        let mut transparent_batches = Vec::new();
        let mut approx_lighting_meshes = 0usize;
        let mut transparent_skipped = 0usize;

        for (b_idx, batch) in dedup.iter().enumerate() {
            if let Some(model) = self.model_cache.get(&batch.model_key) {
                let resource_archives = &mut self.resource_archives;
                let tex_cache = &mut self.tex_cache;
                let bmd_tex_failed = &mut self.bmd_tex_failed;
                let (op, tr, approx, skipped) = build_scenery_batches_for_model(
                    &model,
                    &batch.transforms,
                    settings.transparent_scenery_enabled,
                    (b_idx as u64) << 32,
                    |tex_path| {
                        let res = tex_cache.get_or_insert_pinned(tex_path, || {
                            for (_, mount, arch) in resource_archives.iter_mut() {
                                if !matches!(
                                    mount.as_str(),
                                    "textures" | "litmodels" | "building" | "models" | "surfaces"
                                ) {
                                    continue;
                                }
                                for cand in
                                    texture_lookup_candidates_for_archive(tex_path, Some(mount))
                                {
                                    if let Ok(bytes) = arch.read_entry(&cand) {
                                        let surf = dxt::parse_dds(&bytes)?;
                                        let rgba = dxt::decode_rgba8(&surf);
                                        return Ok(DecodedTexture {
                                            width: surf.width,
                                            height: surf.height,
                                            rgba,
                                        });
                                    }
                                }
                            }
                            Err(format!("missing texture in archives: {tex_path}"))
                        });
                        if res.is_err() {
                            *bmd_tex_failed += 1;
                        }
                        res.ok()
                    },
                );
                opaque_batches.extend(op);
                transparent_batches.extend(tr);
                approx_lighting_meshes += approx;
                transparent_skipped += skipped;
            }
        }

        // Sort transparent submeshes strictly back-to-front per instance and per submesh
        atmosphere::sort_transparent_back_to_front(&mut transparent_batches, camera_pos, |b| {
            (b.sort_center, b.sort_id)
        });

        if let Some(warn_msg) = self.tex_cache.enforce_budget_after_batch() {
            if let Some(log) = logger.as_mut() {
                log.warn(&warn_msg);
            }
        }

        let tstats = self.tex_cache.stats();
        let diag = SceneryDiagnostics {
            placements_total: self.placements.len(),
            placements_in_range: visible.len(),
            unique_models_in_range: dedup.len(),
            models_loaded: self.model_cache.loaded_count(),
            instances_drawn: drawn_instances,
            decode_failures: self.model_cache.failed_count(),
            transparent_submeshes_drawn: transparent_batches.len(),
            transparent_submeshes_skipped: transparent_skipped,
            approximate_lighting_meshes: approx_lighting_meshes,
            textures_cached: tstats.count,
            texture_cache_bytes: tstats.total_bytes,
            texture_budget_bytes: tstats.budget_bytes,
            texture_budget_exceeded: tstats.budget_exceeded,
            water_planes_visible: vis_water.len(),
            world_render_passes,
            gpu_timing_note: "Timestamp queries checked on device init".into(),
        };

        self.revision = self.revision.wrapping_add(1);
        (
            SceneryDrawSnapshot {
                opaque_batches,
                transparent_batches,
                revision: self.revision,
            },
            diag,
        )
    }
}

#[derive(Clone, Debug, Default)]
#[allow(dead_code)]
struct VegetationDiagnostics {
    trees_total: usize,
    trees_in_range: usize,
    trees_loaded: usize,
    helper_status: String,
    grass_total: usize,
    grass_in_range: usize,
    grass_drawn: usize,
    speedtree_cache_bytes: u64,
}

struct ActiveVegetationState {
    client_root: Option<PathBuf>,
    helper_exe: PathBuf,
    cache_root: PathBuf,
    tree_types: Vec<TreeTypeDef>,
    tree_placements: Vec<TreePlacement>,
    grass_types: Vec<GrassTypeDef>,
    grass_placements: Vec<GrassPlacement>,
    tree_meshes: BTreeMap<(u32, u32), FwstTreeMesh>,
    failed_tree_keys: HashSet<(u32, u32)>,
    helper_ready: bool,
    dll_ready: bool,
    missing_notice: Option<String>,
    last_helper_status: String,
    revision: u64,
}

impl ActiveVegetationState {
    fn open_zone(
        client_root: &Path,
        zone: &str,
        portable_paths: &PortablePaths,
        logger: &mut Option<RunLogger>,
    ) -> Result<Self, String> {
        let zone_dir = client_root.join("maps").join(zone);
        let wld_bytes = fs::read(zone_dir.join(format!("{zone}.ecwld")))
            .map_err(|e| format!("failed to read {zone}.ecwld: {e}"))?;
        let bsd_bytes = fs::read(zone_dir.join(format!("{zone}.ecbsd")))
            .map_err(|e| format!("failed to read {zone}.ecbsd: {e}"))?;
        let (_, _, tree_types, tree_placements, grass_types, grass_placements) =
            vegetation::parse_ecwld_ecbsd_vegetation(&wld_bytes, &bsd_bytes)?;

        let helper_exe = vegetation::resolve_helper_path(&portable_paths.root);
        let helper_ready = helper_exe.is_file();
        let dll_check = vegetation::verify_client_speedtree_dll(client_root);
        let dll_ready = dll_check.is_ok();

        let mut notices = Vec::new();
        if !helper_ready {
            let msg = format!(
                "32-bit SpeedTree helper not found at {} — trees disabled; terrain, scenery, and grass continue normally.",
                helper_exe.display()
            );
            if let Some(log) = logger.as_mut() {
                log.warn(&msg);
            }
            notices.push(msg);
        }
        if let Err(err) = &dll_check {
            let msg = format!("{err} — trees disabled; viewer continues normally.");
            if let Some(log) = logger.as_mut() {
                log.warn(&msg);
            }
            notices.push(msg);
        }
        let missing_notice = (!notices.is_empty()).then(|| notices.join(" | "));
        let last_helper_status = if !helper_ready {
            "missing helper (tools/fw-speedtree-helper.exe)".to_string()
        } else if !dll_ready {
            "client update/speedtreert.dll missing or untrusted".to_string()
        } else if tree_placements.is_empty() {
            "idle (zone has 0 tree placements)".to_string()
        } else {
            "ready".to_string()
        };

        Ok(Self {
            client_root: Some(client_root.to_path_buf()),
            helper_exe,
            cache_root: portable_paths.cache_dir.clone(),
            tree_types,
            tree_placements,
            grass_types,
            grass_placements,
            tree_meshes: BTreeMap::new(),
            failed_tree_keys: HashSet::new(),
            helper_ready,
            dll_ready,
            missing_notice,
            last_helper_status,
            revision: 0,
        })
    }

    fn open_synthetic(portable_paths: &PortablePaths) -> Self {
        let mut tree_meshes = BTreeMap::new();
        for lod in 0..=2u32 {
            tree_meshes.insert(
                (0, lod),
                vegetation::synthetic::build_synthetic_tree_fwst(0, lod),
            );
        }
        let tree_placements = vec![
            TreePlacement {
                tree_type: 0,
                position: [72.0, 100.0, 86.0],
            },
            TreePlacement {
                tree_type: 0,
                position: [132.0, 102.0, 92.0],
            },
        ];
        let mut grass_placements = Vec::new();
        for i in 0..16u32 {
            let gx = (i % 4) as f32;
            let gz = (i / 4) as f32;
            grass_placements.push(GrassPlacement {
                export_id: 1000 + i,
                grass_type: 0,
                position: [82.0 + gx * 8.0, 100.0, 82.0 + gz * 8.0],
                grid_size: 2.0,
                grid_cols: 1,
                grid_rows: 1,
            });
        }
        let grass_types = vec![GrassTypeDef {
            type_id: 0,
            alpha_blend: false,
            alpha_ref: 84,
            size: 2.2,
            size_var: 0.3,
            softness: 1.0,
            sight_range: 50.0,
            texture_path: "synthetic/grass.dds".into(),
        }];
        Self {
            client_root: None,
            helper_exe: vegetation::resolve_helper_path(&portable_paths.root),
            cache_root: portable_paths.cache_dir.clone(),
            tree_types: vec![TreeTypeDef {
                type_id: 0,
                spt_path: "synthetic/tree0.spt".into(),
                dds_path: "synthetic/tree0.dds".into(),
            }],
            tree_placements,
            grass_types,
            grass_placements,
            tree_meshes,
            failed_tree_keys: HashSet::new(),
            helper_ready: true,
            dll_ready: true,
            missing_notice: None,
            last_helper_status: "synthetic preview (in-memory FWST v1)".to_string(),
            revision: 0,
        }
    }

    fn step_frame(
        &mut self,
        camera_pos: glam::Vec3,
        settings: &ViewerSettings,
        logger: &mut Option<RunLogger>,
    ) -> (VegetationDrawSnapshot, VegetationDiagnostics) {
        let mut batches = Vec::new();
        let mut trees_in_range = 0usize;
        let mut grass_in_range = 0usize;
        let mut grass_drawn = 0usize;

        if settings.trees_enabled && !self.tree_placements.is_empty() {
            let grouped = vegetation::dedup_trees_by_type_and_lod(
                &self.tree_placements,
                camera_pos,
                settings.veg_radius_m as f32,
                settings.veg_max_instances as usize,
                settings.tree_lod_distance_multiplier(),
            );
            trees_in_range = grouped.values().map(Vec::len).sum();

            let mut loads_left = settings.veg_loads_per_frame as usize;
            let cache_dir = self.cache_root.join(SPEEDTREE_CACHE_SUBDIR);

            for (&(tree_type, lod), transforms) in &grouped {
                let key = (tree_type, lod);
                if !self.tree_meshes.contains_key(&key)
                    && !self.failed_tree_keys.contains(&key)
                    && loads_left > 0
                {
                    let cache_file = cache_dir.join(format!("tree-{tree_type}-lod{lod}.fwst"));
                    if let Ok(bytes) = fs::read(&cache_file) {
                        match FwstTreeMesh::decode(&bytes) {
                            Ok(mesh) => {
                                self.tree_meshes.insert(key, mesh);
                                loads_left = loads_left.saturating_sub(1);
                            }
                            Err(err) => {
                                self.failed_tree_keys.insert(key);
                                if let Some(log) = logger.as_mut() {
                                    log.warn(&format!(
                                        "Corrupt FWST cache {}: {err}",
                                        cache_file.display()
                                    ));
                                }
                            }
                        }
                    } else if self.helper_ready && self.dll_ready {
                        if let (Some(client_root), Some(tdef)) = (
                            self.client_root.as_deref(),
                            self.tree_types.iter().find(|t| t.type_id == tree_type),
                        ) {
                            let _ = fs::create_dir_all(&cache_dir);
                            let req = HelperRequest {
                                tree_type,
                                spt_path: tdef.spt_path.clone(),
                                lod,
                                out_cache_file: cache_file.clone(),
                            };
                            loads_left = loads_left.saturating_sub(1);
                            match vegetation::run_speedtree_helper(
                                &self.helper_exe,
                                client_root,
                                &req,
                                Duration::from_secs(DEFAULT_HELPER_TIMEOUT_SECS),
                            ) {
                                Ok(rep) => {
                                    self.last_helper_status = if rep.timed_out {
                                        format!("timeout after {} ms", rep.duration_ms)
                                    } else {
                                        format!(
                                            "exit={:?} in {} ms",
                                            rep.exit_code, rep.duration_ms
                                        )
                                    };
                                    if rep.exit_code == Some(0) && !rep.timed_out {
                                        if let Ok(bytes) = fs::read(&cache_file) {
                                            if let Ok(mesh) = FwstTreeMesh::decode(&bytes) {
                                                self.tree_meshes.insert(key, mesh);
                                            } else {
                                                self.failed_tree_keys.insert(key);
                                            }
                                        } else {
                                            self.failed_tree_keys.insert(key);
                                        }
                                    } else {
                                        self.failed_tree_keys.insert(key);
                                        if let Some(log) = logger.as_mut() {
                                            log.warn(&format!(
                                                "SpeedTree helper failed for type {tree_type} lod {lod}: {} ({})",
                                                self.last_helper_status, rep.stderr_summary
                                            ));
                                        }
                                    }
                                }
                                Err(err) => {
                                    self.last_helper_status = format!("error: {err}");
                                    self.failed_tree_keys.insert(key);
                                    if let Some(log) = logger.as_mut() {
                                        log.warn(&format!("SpeedTree helper launch error: {err}"));
                                    }
                                }
                            }
                        }
                    }
                }

                if let Some(mesh) = self.tree_meshes.get(&key) {
                    batches.extend(build_tree_batches(
                        mesh,
                        transforms,
                        [0.42, 0.28, 0.14, 1.0],
                        [0.18, 0.56, 0.20, 1.0],
                        None,
                    ));
                }
            }
        }

        if settings.grass_enabled && !self.grass_placements.is_empty() {
            let selected = vegetation::select_visible_grass(
                &self.grass_placements,
                camera_pos,
                settings.veg_radius_m as f32,
                settings.veg_max_instances as usize,
                settings.grass_density,
            );
            grass_in_range = selected.len();
            let grouped = vegetation::dedup_grass_by_type(&selected);
            for (gtype, mats) in grouped {
                let gdef = self.grass_types.iter().find(|g| g.type_id == gtype);
                let size = gdef.map(|g| g.size).unwrap_or(1.5).clamp(0.5, 6.0);
                let alpha_cutoff = gdef
                    .map(|g| (g.alpha_ref as f32 / 255.0).clamp(0.05, 0.9))
                    .unwrap_or(0.33);
                let (verts, idxs) = vegetation::synthetic::build_grass_tuft_mesh(size);
                let instances: Vec<SceneryInstance> = mats
                    .iter()
                    .map(|m| SceneryInstance {
                        model_cols: m.to_cols_array_2d(),
                    })
                    .collect();
                grass_drawn += instances.len();
                batches.push(VegetationDrawBatch {
                    vertices: verts,
                    indices: idxs,
                    instances,
                    base_color: [0.32, 0.64, 0.22, 1.0],
                    alpha_cutoff,
                    wind_amplitude: 0.22,
                    texture: None,
                });
            }
        }

        self.revision = self.revision.wrapping_add(1);
        let diag = VegetationDiagnostics {
            trees_total: self.tree_placements.len(),
            trees_in_range,
            trees_loaded: self.tree_meshes.len(),
            helper_status: self.last_helper_status.clone(),
            grass_total: self.grass_placements.len(),
            grass_in_range,
            grass_drawn,
            speedtree_cache_bytes: vegetation::speedtree_cache_bytes(&self.cache_root),
        };
        (
            VegetationDrawSnapshot {
                batches,
                revision: self.revision,
            },
            diag,
        )
    }
}

pub struct ExplorerApp {
    page: Page,
    paths: PortablePaths,
    root_write_error: Option<String>,
    logger: Option<RunLogger>,
    settings: ViewerSettings,
    detected_client: Option<Result<DetectedClient, String>>,
    modules: Registry,
    status_line: String,
    camera: FreeCamera,
    selected_zone: String,
    streaming_radius: i32,
    input_sm: InputStateMachine,
    world_settings_tab: SettingsTab,
    show_diagnostics: bool,
    show_developer_tools: bool,
    terrain_wireframe_debug: bool,
    started_at: Instant,
    terrain_session: Option<TerrainSession>,
    scenery_state: Option<ActiveSceneryState>,
    veg_state: Option<ActiveVegetationState>,
    mesh_arc: Arc<TerrainMeshSnapshot>,
    day_lm_arc: Arc<TerrainTextureSnapshot>,
    night_lm_arc: Arc<TerrainTextureSnapshot>,
    scenery_arc: Arc<SceneryDrawSnapshot>,
    veg_arc: Arc<VegetationDrawSnapshot>,
    water_arc: Arc<WaterRenderSnapshot>,
    latest_diag: SceneryDiagnostics,
    displayed_diag: SceneryDiagnostics,
    latest_veg_diag: VegetationDiagnostics,
    displayed_veg_diag: VegetationDiagnostics,
    last_diag_refresh: Instant,
}

impl ExplorerApp {
    pub fn new() -> Self {
        let paths = PortablePaths::from_executable();
        Self::with_portable_paths(paths, true)
    }

    pub fn new_headless_smoke() -> Self {
        let paths = PortablePaths::from_executable();
        Self::with_portable_paths(paths, false)
    }

    fn with_portable_paths(paths: PortablePaths, allow_disk_init: bool) -> Self {
        let mut root_write_error = None;
        let mut logger = None;
        let mut import_note: Option<String> = None;

        if allow_disk_init {
            match paths.check_writable() {
                Ok(()) => {
                    match RunLogger::open(&paths.logs_dir) {
                        Ok(mut run_log) => {
                            run_log.info(&format!(
                                "Portable root: {} (settings: {}, logs: {}, cache: {})",
                                paths.root.display(),
                                paths.settings_dir.display(),
                                paths.logs_dir.display(),
                                paths.cache_dir.display()
                            ));
                            logger = Some(run_log);
                        }
                        Err(err) => {
                            root_write_error = Some(err);
                        }
                    }
                    let legacy = ViewerSettings::legacy_settings_path();
                    if let Ok(Some(old_path)) =
                        ViewerSettings::import_legacy_once(&paths, legacy.as_deref())
                    {
                        let msg =
                            format!("Imported legacy settings once from {}", old_path.display());
                        if let Some(log) = logger.as_mut() {
                            log.info(&msg);
                        }
                        import_note = Some(msg);
                    }
                }
                Err(err) => {
                    root_write_error = Some(err);
                }
            }
        }

        let (mut settings, mut status_line) = match ViewerSettings::load_from_paths(&paths) {
            Ok(loaded) => (
                loaded,
                format!("Settings path: {}", paths.settings_file.display()),
            ),
            Err(error) => (
                ViewerSettings::default(),
                format!("Using default settings ({error})"),
            ),
        };
        if let Some(note) = import_note {
            status_line = note;
        }
        if let Some(err) = &root_write_error {
            status_line = err.clone();
        }
        settings.apply_env_overrides();
        let detected_client = settings.client_dir.as_deref().map(client_detect::detect);
        let selected_zone = detected_client
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .and_then(|c| {
                if c.zones.iter().any(|z| z == "s0") {
                    Some("s0".to_string())
                } else {
                    c.zones.first().cloned()
                }
            })
            .unwrap_or_else(|| "s0".to_string());
        let modules = Registry::discover(Registry::default_root());
        let default_diag = SceneryDiagnostics {
            texture_budget_bytes: (settings.tex_budget_mib as usize) * 1024 * 1024,
            world_render_passes: 1,
            gpu_timing_note: "Timestamp queries checked on device init".into(),
            ..Default::default()
        };
        Self {
            page: Page::Launcher,
            paths,
            root_write_error,
            logger,
            settings,
            detected_client,
            modules,
            status_line,
            camera: FreeCamera::default(),
            selected_zone,
            streaming_radius: 4,
            input_sm: InputStateMachine::new_world_ready(),
            world_settings_tab: SettingsTab::Graphics,
            show_diagnostics: true,
            show_developer_tools: false,
            terrain_wireframe_debug: false,
            started_at: Instant::now(),
            terrain_session: None,
            scenery_state: None,
            veg_state: None,
            mesh_arc: Arc::new(TerrainMeshSnapshot::default()),
            day_lm_arc: Arc::new(TerrainTextureSnapshot::default()),
            night_lm_arc: Arc::new(TerrainTextureSnapshot::default()),
            scenery_arc: Arc::new(SceneryDrawSnapshot::default()),
            veg_arc: Arc::new(VegetationDrawSnapshot::default()),
            water_arc: Arc::new(WaterRenderSnapshot::default()),
            latest_diag: default_diag.clone(),
            displayed_diag: default_diag,
            latest_veg_diag: VegetationDiagnostics::default(),
            displayed_veg_diag: VegetationDiagnostics::default(),
            last_diag_refresh: Instant::now(),
        }
    }

    fn switch_portable_root(&mut self, new_root: PathBuf) {
        let new_paths = PortablePaths::from_root(new_root);
        match new_paths.check_writable() {
            Ok(()) => {
                self.paths = new_paths;
                self.root_write_error = None;
                self.logger = RunLogger::open(&self.paths.logs_dir).ok();
                if let Some(log) = self.logger.as_mut() {
                    log.info(&format!(
                        "Switched portable data root to {}",
                        self.paths.root.display()
                    ));
                }
                self.save_settings();
            }
            Err(err) => {
                self.root_write_error = Some(err.clone());
                self.status_line = err;
            }
        }
    }

    fn apply_selected_client_dir(&mut self, folder: PathBuf) {
        self.settings.client_dir = Some(folder.clone());
        let result = client_detect::detect(&folder);
        if let Ok(client) = &result {
            if client.zones.iter().any(|z| z == "s0") {
                self.selected_zone = "s0".to_string();
            } else if let Some(first_zone) = client.zones.first() {
                self.selected_zone = first_zone.clone();
            }
        }
        if let Some(log) = self.logger.as_mut() {
            match &result {
                Ok(client) => log.info(&format!(
                    "Selected client folder {}: {} ({} archives, {} zones)",
                    client.root.display(),
                    client.recognition.summary(),
                    client.pck_archives.len(),
                    client.zones.len()
                )),
                Err(err) => log.warn(&format!(
                    "Client folder validation failed for {}: {err}",
                    folder.display()
                )),
            }
        }
        self.detected_client = Some(result);
        self.terrain_session = None;
        self.scenery_state = None;
        self.veg_state = None;
        self.save_settings();
    }

    fn step_scenery(&mut self, view_proj: glam::Mat4, vp_w: u32, vp_h: u32) {
        if let Some(state) = self.scenery_state.as_mut() {
            let (snap, diag) = state.step_frame(
                self.camera.position,
                view_proj,
                vp_w,
                vp_h,
                &self.settings,
                &mut self.logger,
            );
            self.scenery_arc = Arc::new(snap);
            self.water_arc = Arc::new(WaterRenderSnapshot {
                planes: state.water_planes.clone(),
                reflection_enabled: self.settings.water_reflection,
                refraction_enabled: self.settings.water_refraction,
                quality: self.settings.water_quality,
            });
            self.latest_diag = diag;
        }
        if let Some(veg) = self.veg_state.as_mut() {
            let (vsnap, vdiag) =
                veg.step_frame(self.camera.position, &self.settings, &mut self.logger);
            self.veg_arc = Arc::new(vsnap);
            self.latest_veg_diag = vdiag;
        }
        if self.last_diag_refresh.elapsed() >= Duration::from_secs(1) {
            self.displayed_diag = self.latest_diag.clone();
            self.displayed_veg_diag = self.latest_veg_diag.clone();
            self.last_diag_refresh = Instant::now();
        }
    }

    fn load_selected_zone(&mut self) {
        let Some(Ok(client)) = &self.detected_client else {
            self.status_line = "Select a valid client folder first.".to_string();
            return;
        };
        let hot_bytes = (self.settings.terrain_ram_mib as usize) * 1024 * 1024;
        let tex_budget_bytes = (self.settings.tex_budget_mib as usize) * 1024 * 1024;
        let zone = self.selected_zone.clone();
        let archive_names = client.pck_archives.clone();
        if let Some(log) = self.logger.as_mut() {
            log.info(&format!(
                "=== Load World: zone '{zone}' | client_root={} | archives_searched=[{}] ===",
                client.root.display(),
                archive_names.join(", ")
            ));
        }
        // Stage 1: Terrain blocks, real layer textures/masks, and lightmaps
        match TerrainSession::open_zone(&client.root, &zone, &self.paths.cache_dir, hot_bytes) {
            Ok(mut session) => {
                for note in &session.load_notes {
                    if let Some(log) = self.logger.as_mut() {
                        log.info(&format!("[Stage 1 Terrain] {note}"));
                    }
                }
                let origin_h = session.sample_height(0.0, 0.0).unwrap_or(140.0);
                self.camera = FreeCamera {
                    position: glam::Vec3::new(0.0, origin_h + 30.0, 0.0),
                    yaw: 0.0,
                    pitch: -0.25,
                    ..Default::default()
                };
                if let Err(err) =
                    session.update_for_camera(self.camera.position, self.streaming_radius)
                {
                    if let Some(log) = self.logger.as_mut() {
                        log.error(&format!(
                            "[Stage 1 Terrain] update_for_camera FAILED: {err}"
                        ));
                    }
                }
                if let Some(log) = self.logger.as_mut() {
                    log.info(&format!(
                        "[Stage 1 Terrain] OK: {}x{} blocks, streamed={}, height(0,0)={origin_h:.2}m, spawn=({:.1}, {:.1}, {:.1})",
                        session.block_rows,
                        session.block_cols,
                        session.mesh.streamed_blocks,
                        self.camera.position.x,
                        self.camera.position.y,
                        self.camera.position.z
                    ));
                }
                self.mesh_arc = Arc::new(session.mesh.clone());
                self.day_lm_arc = Arc::new(session.day_lightmap.clone());
                self.night_lm_arc = Arc::new(session.night_lightmap.clone());

                // Stage 2: World placements & static .bmd buildings
                let scenery_note = match ActiveSceneryState::open_zone(
                    &client.root,
                    &zone,
                    &self.paths.cache_dir,
                    tex_budget_bytes,
                ) {
                    Ok(st) => {
                        let count = st.placements.len();
                        let boxes = st.box_areas.len();
                        let bmd_pcks = st.resource_archives.len();
                        if let Some(log) = self.logger.as_mut() {
                            log.info(&format!(
                                "[Stage 2 Scenery] OK: {count} placements, {bmd_pcks} resource .pck archive(s) opened, {} open error(s)",
                                st.archive_open_errors.len()
                            ));
                            for (aname, aerr) in &st.archive_open_errors {
                                log.error(&format!(
                                    "[Stage 2 Scenery] PCK open error {aname}: {aerr}"
                                ));
                            }
                            log.info(&format!(
                                "[Stage 4 Atmosphere/Water] box_areas={boxes}, water_planes={} | Sky: missing (Issue #15) | Effects: missing",
                                st.water_planes.len()
                            ));
                        }
                        self.scenery_state = Some(st);
                        format!("{count} placements, {boxes} atmosphere box(es)")
                    }
                    Err(err) => {
                        if let Some(log) = self.logger.as_mut() {
                            log.warn(&format!("[Stage 2 Scenery] FAILED: {err}"));
                        }
                        self.scenery_state = None;
                        format!("scenery unavailable ({err})")
                    }
                };

                // Stage 3: Trees & Grass
                match ActiveVegetationState::open_zone(
                    &client.root,
                    &zone,
                    &self.paths,
                    &mut self.logger,
                ) {
                    Ok(veg) => {
                        if let Some(log) = self.logger.as_mut() {
                            log.info(&format!(
                                "[Stage 3 Vegetation] OK: {} tree placements, {} grass placements",
                                veg.tree_placements.len(),
                                veg.grass_placements.len()
                            ));
                        }
                        self.veg_state = Some(veg);
                    }
                    Err(err) => {
                        if let Some(log) = self.logger.as_mut() {
                            log.warn(&format!("[Stage 3 Vegetation] FAILED: {err}"));
                        }
                        self.veg_state = None;
                    }
                }

                let vp = self.camera.view_proj(16.0 / 9.0);
                self.step_scenery(vp, 1080, 680);
                self.displayed_diag = self.latest_diag.clone();
                self.displayed_veg_diag = self.latest_veg_diag.clone();
                self.last_diag_refresh = Instant::now();
                self.input_sm = InputStateMachine::new_world_ready();
                let msg = format!(
                    "Loaded zone '{}' ({}x{} blocks, height(0,0)={origin_h:.1}m, {})",
                    session.zone_name, session.block_rows, session.block_cols, scenery_note
                );
                let reasons_summary = self
                    .scenery_state
                    .as_ref()
                    .map(|st| {
                        if st.failure_groups.is_empty() {
                            "none".to_string()
                        } else {
                            st.failure_groups
                                .iter()
                                .map(|(r, g)| format!("{r}={}", g.count))
                                .collect::<Vec<_>>()
                                .join(", ")
                        }
                    })
                    .unwrap_or_else(|| "none".to_string());
                let bmd_tex_failed = self
                    .scenery_state
                    .as_ref()
                    .map(|st| st.bmd_tex_failed)
                    .unwrap_or(0);
                let summary_line = format!(
                    "[Zone Summary] zone='{}' | placements total={}, loaded={}, drawn={}, failed={} (by_reason: {}) | terrain blocks streamed={} | textures loaded={}, failed={}",
                    session.zone_name,
                    self.latest_diag.placements_total,
                    self.latest_diag.models_loaded,
                    self.latest_diag.instances_drawn,
                    self.latest_diag.decode_failures,
                    reasons_summary,
                    session.mesh.streamed_blocks,
                    session.terrain_layers_loaded + self.latest_diag.textures_cached,
                    session.terrain_layers_failed + bmd_tex_failed
                );
                if let Some(log) = self.logger.as_mut() {
                    log.info(&msg);
                    log.info(&summary_line);
                }
                self.status_line = msg;
                self.terrain_session = Some(session);
            }
            Err(err) => {
                let msg = format!("Failed to load zone '{zone}': {err}");
                if let Some(log) = self.logger.as_mut() {
                    log.warn(&msg);
                }
                self.status_line = msg;
            }
        }
    }

    fn load_synthetic_preview(&mut self) {
        let hot_bytes = (self.settings.terrain_ram_mib as usize) * 1024 * 1024;
        let tex_budget_bytes = (self.settings.tex_budget_mib as usize) * 1024 * 1024;
        match TerrainSession::open_synthetic(&self.paths.cache_dir, hot_bytes) {
            Ok(mut session) => {
                self.camera = FreeCamera::default();
                let _ = session.update_for_camera(self.camera.position, self.streaming_radius);
                self.mesh_arc = Arc::new(session.mesh.clone());
                self.day_lm_arc = Arc::new(session.day_lightmap.clone());
                self.night_lm_arc = Arc::new(session.night_lightmap.clone());
                if let Ok(st) =
                    ActiveSceneryState::open_synthetic(&self.paths.cache_dir, tex_budget_bytes)
                {
                    self.scenery_state = Some(st);
                }
                self.veg_state = Some(ActiveVegetationState::open_synthetic(&self.paths));
                let vp = self.camera.view_proj(16.0 / 9.0);
                self.step_scenery(vp, 1080, 680);
                self.displayed_diag = self.latest_diag.clone();
                self.displayed_veg_diag = self.latest_veg_diag.clone();
                self.last_diag_refresh = Instant::now();
                let msg = format!(
                    "Loaded synthetic world preview ({} blocks, {} scenery instances, {} water plane)",
                    session.mesh.streamed_blocks,
                    self.latest_diag.instances_drawn,
                    self.latest_diag.water_planes_visible
                );
                if let Some(log) = self.logger.as_mut() {
                    log.info(&msg);
                }
                self.status_line = msg;
                self.terrain_session = Some(session);
            }
            Err(err) => {
                self.status_line = format!("Failed to initialize synthetic preview: {err}");
            }
        }
    }

    fn save_settings(&mut self) {
        match self.settings.save_to_paths(&self.paths) {
            Ok(path) => {
                let msg = format!("Saved settings to {}", path.display());
                if let Some(log) = self.logger.as_mut() {
                    log.info(&msg);
                }
                self.status_line = msg;
            }
            Err(error) => {
                let msg = format!("Failed to save settings: {error}");
                if let Some(log) = self.logger.as_mut() {
                    log.error(&msg);
                }
                self.root_write_error = Some(msg.clone());
                self.status_line = msg;
            }
        }
    }

    fn draw_unwritable_banner(&mut self, ui: &mut egui::Ui) {
        let Some(err) = self.root_write_error.clone() else {
            return;
        };
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.strong("Portable folder is not writable");
            ui.label(&err);
            if ui.button("Choose Writable Folder…").clicked() {
                if let Some(picked) = rfd::FileDialog::new()
                    .set_title("Select a writable folder for settings, logs, and cache")
                    .pick_folder()
                {
                    self.switch_portable_root(picked);
                }
            }
        });
        ui.add_space(8.0);
    }

    fn draw_launcher(&mut self, ui: &mut egui::Ui) {
        ui.heading("Forsaken World Explorer");
        ui.label("Free-flight viewer for a user-owned Forsaken World client.");
        ui.add_space(10.0);

        self.draw_unwritable_banner(ui);

        ui.label("Client folder (never modified by this viewer):");
        ui.horizontal(|ui| {
            let mut display_path = self
                .settings
                .client_dir
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(No client folder selected)".to_string());
            ui.add_enabled(
                false,
                egui::TextEdit::singleline(&mut display_path).desired_width(480.0),
            );
            if ui.button("Browse…").clicked() {
                let mut dialog =
                    rfd::FileDialog::new().set_title("Select Forsaken World client folder");
                if let Some(last) = &self.settings.client_dir {
                    if last.is_dir() {
                        dialog = dialog.set_directory(last);
                    }
                }
                if let Some(picked) = dialog.pick_folder() {
                    self.apply_selected_client_dir(picked);
                }
            }
        });
        ui.add_space(8.0);

        match &self.detected_client {
            None => {
                ui.label("Click Browse… to select your Forsaken World client root (containing update/ and resources/).");
            }
            Some(Ok(client)) => {
                ui.group(|ui| {
                    ui.strong(format!("Client status: {}", client.recognition.summary()));
                    ui.label(format!("Root: {}", client.root.display()));
                    ui.label(format!(
                        "Executable: update/pem.exe ({} bytes, SHA-256 {})",
                        client.executable_size, client.executable_sha256
                    ));
                    ui.label(format!(
                        "SpeedTree runtime (update/speedtreert.dll): {}",
                        if client.has_speedtree_dll {
                            "found"
                        } else {
                            "missing"
                        }
                    ));
                    ui.label(format!(
                        "Archives in resources/: {} (.pck)",
                        client.pck_archives.len()
                    ));
                    ui.label(format!("Readable map zones: {}", client.zones.len()));
                });
                ui.add_space(6.0);
                let zones = client.zones.clone();
                let mut clicked_load = false;
                ui.horizontal(|ui| {
                    ui.label("Zone:");
                    egui::ComboBox::from_id_salt("launcher_zone_selector")
                        .selected_text(&self.selected_zone)
                        .show_ui(ui, |ui| {
                            for z in &zones {
                                ui.selectable_value(&mut self.selected_zone, z.clone(), z);
                            }
                        });
                    if ui
                        .add_sized([160.0, 32.0], egui::Button::new("Load World"))
                        .clicked()
                    {
                        clicked_load = true;
                    }
                });
                if clicked_load {
                    self.load_selected_zone();
                    self.page = Page::WorldView;
                }
            }
            Some(Err(err)) => {
                ui.label(format!("Validation error: {err}"));
            }
        }

        ui.add_space(10.0);
        ui.separator();
        ui.label(format!("Portable root: {}", self.paths.root.display()));
        ui.label(format!("Settings: {}", self.paths.settings_file.display()));
        ui.label(format!("Cache: {}", self.paths.cache_dir.display()));
        ui.horizontal(|ui| {
            ui.label(format!("Logs: {}", self.paths.logs_dir.display()));
            if ui.button("Open logs folder").clicked() {
                if let Err(err) = logging::open_folder_in_file_manager(&self.paths.logs_dir) {
                    if let Some(log) = self.logger.as_mut() {
                        log.warn(&err);
                    }
                    self.status_line = err;
                }
            }
        });
        ui.add_space(6.0);
        ui.checkbox(
            &mut self.show_developer_tools,
            "Developer tools (off by default — synthetic test scene is not a game world)",
        );
        if self.show_developer_tools {
            ui.horizontal(|ui| {
                if ui.button("Open Synthetic Pipeline Test Scene").clicked() {
                    self.load_synthetic_preview();
                    self.input_sm = InputStateMachine::new_world_ready();
                    self.page = Page::WorldView;
                }
                ui.small("Pipeline check only; not Forsaken World data.");
            });
        }
    }

    fn apply_capture_action(ctx: &egui::Context, action: CaptureAction) {
        match action {
            CaptureAction::Capture => {
                ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::Locked));
                ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(false));
            }
            CaptureAction::Release => {
                ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::None));
                ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
            }
            CaptureAction::None => {}
        }
    }

    fn draw_world_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        // Track window focus changes so focus loss always releases mouse capture
        let focused = ctx.input(|i| i.focused);
        if focused != self.input_sm.window_focused {
            let act = self.input_sm.on_focus_changed(focused);
            Self::apply_capture_action(ctx, act);
        }

        // Escape and M keys (matching PW Explorer MenuPage state machine)
        let (esc_pressed, m_pressed) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::M),
            )
        });
        if esc_pressed {
            let act = self.input_sm.on_escape();
            Self::apply_capture_action(ctx, act);
        }
        if m_pressed {
            let act = self.input_sm.on_toggle_world_map();
            Self::apply_capture_action(ctx, act);
        }

        let avail = ui.available_size();
        let (rect, response) = ui.allocate_exact_size(
            avail.max(egui::vec2(64.0, 64.0)),
            egui::Sense::click_and_drag(),
        );

        // Left-click in 3D viewport captures and locks the mouse for looking
        if response.clicked_by(egui::PointerButton::Primary) {
            let act = self.input_sm.on_left_click_in_viewport();
            Self::apply_capture_action(ctx, act);
        }

        // Apply camera movement & raw mouse look ONLY while mouse is captured
        if self.input_sm.allows_camera_input() {
            let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 0.1);
            let raw_mouse = ctx.input(|i| {
                let mut sum = egui::Vec2::ZERO;
                for ev in &i.events {
                    if let egui::Event::MouseMoved(delta) = ev {
                        sum += *delta;
                    }
                }
                if sum == egui::Vec2::ZERO {
                    sum = i.pointer.delta();
                }
                sum
            });
            let cam_input = ctx.input(|i| CameraInput {
                forward: i.key_down(egui::Key::W),
                backward: i.key_down(egui::Key::S),
                left: i.key_down(egui::Key::A),
                right: i.key_down(egui::Key::D),
                ascend: i.key_down(egui::Key::Space) || i.key_down(egui::Key::E),
                descend: i.key_down(egui::Key::Q) || i.modifiers.ctrl,
                fast: i.modifiers.shift,
                look_left: i.key_down(egui::Key::ArrowLeft),
                look_right: i.key_down(egui::Key::ArrowRight),
                look_up: i.key_down(egui::Key::ArrowUp),
                look_down: i.key_down(egui::Key::ArrowDown),
                mouse_dx: raw_mouse.x,
                mouse_dy: raw_mouse.y,
            });
            let moving = cam_input.forward
                || cam_input.backward
                || cam_input.left
                || cam_input.right
                || cam_input.ascend
                || cam_input.descend
                || cam_input.look_left
                || cam_input.look_right
                || cam_input.look_up
                || cam_input.look_down
                || raw_mouse.length_sq() > 0.0;
            if moving
                && self
                    .input_sm
                    .apply_camera_input(&mut self.camera, &cam_input, dt)
            {
                if let Some(session) = self.terrain_session.as_mut() {
                    if let Ok(true) =
                        session.update_for_camera(self.camera.position, self.streaming_radius)
                    {
                        self.mesh_arc = Arc::new(session.mesh.clone());
                    }
                }
            }
            ctx.request_repaint();
        }

        let ppp = ctx.pixels_per_point();
        let vp_w = ((rect.width() * ppp).round() as u32).max(1);
        let vp_h = ((rect.height() * ppp).round() as u32).max(1);
        let (scaled_w, scaled_h) =
            scaled_render_target(vp_w, vp_h, self.settings.resolution_scale_pct);
        let aspect = (vp_w as f32) / (vp_h as f32);
        let vp = self.camera.view_proj(aspect);

        if self.scenery_state.is_some() {
            self.step_scenery(vp, scaled_w, scaled_h);
        }

        let active_box = self
            .scenery_state
            .as_ref()
            .and_then(|s| s.active_box_area());
        let water_height = self
            .scenery_state
            .as_ref()
            .and_then(|s| s.water_planes.first().map(|w| w.height_y));
        let active_zone_title = self
            .terrain_session
            .as_ref()
            .map(|s| s.zone_name.as_str())
            .unwrap_or(self.selected_zone.as_str());
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "Forsaken World Explorer — {active_zone_title}"
        )));
        let is_synthetic = self
            .terrain_session
            .as_ref()
            .is_some_and(|s| s.is_synthetic);

        let hours = self.settings.time_of_day_hours();
        let elapsed_sec = self.started_at.elapsed().as_secs_f32();
        let uniform = CameraUniform::build(
            vp,
            self.camera.position,
            hours,
            elapsed_sec,
            self.settings.fog_enabled,
            self.settings.fog_distance_multiplier(),
            active_box,
            water_height,
        );

        let cb = egui_wgpu::Callback::new_paint_callback(
            rect,
            TerrainCallback {
                viewport_w: vp_w,
                viewport_h: vp_h,
                scale_pct: self.settings.resolution_scale_pct,
                uniform,
                mesh: Arc::clone(&self.mesh_arc),
                day_lightmap: Arc::clone(&self.day_lm_arc),
                night_lightmap: Arc::clone(&self.night_lm_arc),
                scenery: Arc::clone(&self.scenery_arc),
                vegetation: Arc::clone(&self.veg_arc),
                water: Arc::clone(&self.water_arc),
                draw_sky_gradient: is_synthetic,
                terrain_has_real_textures: self
                    .terrain_session
                    .as_ref()
                    .is_some_and(|s| s.terrain_layers_loaded > 0),
                terrain_wireframe_debug: self.terrain_wireframe_debug,
            },
        );
        ui.painter().add(cb);

        // Subtle controls / capture banner (top-left)
        egui::Area::new(egui::Id::new("world_controls_hud"))
            .fixed_pos(rect.left_top() + egui::vec2(10.0, 10.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    if self.input_sm.mouse_captured {
                        ui.small("Mouse captured | WASD fly | Space/E up | Q/Ctrl down | Shift fast | Arrows look | M map | ESC menu");
                    } else {
                        ui.strong("Left-click in the 3D view to capture mouse and look around | ESC menu | M World Map");
                    }
                    let zone_label = self
                        .terrain_session
                        .as_ref()
                        .map(|s| s.zone_name.as_str())
                        .unwrap_or("(no zone loaded)");
                    ui.small(format!(
                        "Zone: {zone_label} | Pos: ({:.1}, {:.1}, {:.1}) | 3D: {scaled_w}x{scaled_h} ({}%)",
                        self.camera.position.x,
                        self.camera.position.y,
                        self.camera.position.z,
                        self.settings.resolution_scale_pct
                    ));
                    if let Some(notice) = self.veg_state.as_ref().and_then(|v| v.missing_notice.as_deref()) {
                        ui.small(format!("SpeedTree notice: {notice}"));
                    }
                });
            });

        // Minimap overlay (top-right, matching PW Explorer Interface -> Show minimap)
        if self.settings.show_minimap {
            egui::Area::new(egui::Id::new("world_minimap_overlay"))
                .fixed_pos(rect.right_top() + egui::vec2(-210.0, 10.0))
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.strong("Minimap");
                        ui.small(format!(
                            "X {:.0}  Y {:.0}  Z {:.0}",
                            self.camera.position.x, self.camera.position.y, self.camera.position.z
                        ));
                        ui.small("Minimap tiles: missing (not decoded yet)");
                    });
                });
        }

        // 1 Hz Diagnostics overlay (bottom-right)
        if self.show_diagnostics {
            let d = &self.displayed_diag;
            let terrain_layers_loaded = self
                .terrain_session
                .as_ref()
                .map(|s| s.terrain_layers_loaded)
                .unwrap_or(0);
            let terrain_layers_failed = self
                .terrain_session
                .as_ref()
                .map(|s| s.terrain_layers_failed)
                .unwrap_or(0);
            let terrain_tex_bytes = self
                .terrain_session
                .as_ref()
                .map(|s| s.terrain_tex_bytes)
                .unwrap_or(0);
            let terrain_drawn_blocks = if terrain_layers_loaded > 0 || self.terrain_wireframe_debug
            {
                self.mesh_arc.streamed_blocks
            } else {
                0
            };
            let terrain_reason = if terrain_drawn_blocks == 0 {
                if self.mesh_arc.streamed_blocks == 0 {
                    " (no zone loaded)"
                } else {
                    " (0 terrain textures resolved; enable wireframe debug in Settings -> World)"
                }
            } else if terrain_layers_loaded == 0 && self.terrain_wireframe_debug {
                " [WIREFRAME DEBUG]"
            } else {
                ""
            };
            let total_tex_loaded = terrain_layers_loaded + d.textures_cached;
            let bmd_tex_failed = self
                .scenery_state
                .as_ref()
                .map(|s| s.bmd_tex_failed)
                .unwrap_or(0);
            let total_tex_failed = terrain_layers_failed + bmd_tex_failed;
            let total_tex_bytes = terrain_tex_bytes + d.texture_cache_bytes;
            let tex_reason = if total_tex_loaded == 0 {
                " (check logs/ or --dump-zone for missing entries or PCK open errors)"
            } else {
                ""
            };
            let bmd_reason = if d.models_loaded == 0 {
                " (see failure reasons below)"
            } else {
                ""
            };

            egui::Area::new(egui::Id::new("world_diagnostics_panel"))
                .fixed_pos(rect.right_bottom() + egui::vec2(-380.0, -240.0))
                .show(ctx, |ui| {
                    let active_zone_diag = self
                        .terrain_session
                        .as_ref()
                        .map(|s| s.zone_name.as_str())
                        .unwrap_or(self.selected_zone.as_str());
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.strong(format!("Diagnostics (1 Hz) — Zone: {active_zone_diag}"));
                        ui.small(format!(
                            "Terrain blocks drawn: {} (streamed={}, {} KiB hot RAM){}",
                            terrain_drawn_blocks,
                            self.mesh_arc.streamed_blocks,
                            self.mesh_arc.hot_ram_bytes / 1024,
                            terrain_reason
                        ));
                        ui.small(format!(
                            "Textures: loaded={}, failed={}, bytes={:.2} MiB{}",
                            total_tex_loaded,
                            total_tex_failed,
                            total_tex_bytes as f64 / (1024.0 * 1024.0),
                            tex_reason
                        ));
                        ui.small(format!(
                            "Placements: in_range={}/{}, loaded={}, drawn={}, failed={}{}",
                            d.placements_in_range,
                            d.placements_total,
                            d.models_loaded,
                            d.instances_drawn,
                            d.decode_failures,
                            bmd_reason
                        ));
                        if let Some(st) = &self.scenery_state {
                            if !st.archive_open_errors.is_empty() {
                                for (name, err) in st.archive_open_errors.iter().take(2) {
                                    ui.small(format!("  PCK open error ({name}): {err}"));
                                }
                            }
                            if !st.failure_groups.is_empty() {
                                ui.small("Building failures by reason:");
                                for (reason, grp) in &st.failure_groups {
                                    let sample =
                                        grp.samples.first().map(String::as_str).unwrap_or("-");
                                    ui.small(format!(
                                        "  - {reason}: {} (e.g. {sample})",
                                        grp.count
                                    ));
                                }
                            }
                        }
                        ui.small(
                            "Sky: missing (Issue #15) | Effects: missing | Audio: not started",
                        );
                    });
                });
        }

        // PW Explorer ESC Menu & subpages
        match self.input_sm.menu_page {
            MenuPage::Closed => {}
            MenuPage::Main => {
                egui::Window::new("FORSAKEN WORLD EXPLORER  |  ESC MENU")
                    .default_pos(egui::pos2(16.0, 80.0))
                    .default_size(egui::vec2(340.0, 340.0))
                    .collapsible(false)
                    .resizable(true)
                    .show(ctx, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.label(egui::RichText::new("WORLD EXPLORER").size(20.0).strong());
                            ui.add_space(10.0);
                            let w = ui.available_width().min(290.0);
                            if ui
                                .add_sized([w, 36.0], egui::Button::new("RESUME EXPLORING"))
                                .clicked()
                            {
                                let act = self.input_sm.on_set_menu_page(MenuPage::Closed);
                                Self::apply_capture_action(ctx, act);
                            }
                            ui.add_space(6.0);
                            if ui
                                .add_sized(
                                    [w, 40.0],
                                    egui::Button::new(egui::RichText::new("SETTINGS").strong()),
                                )
                                .clicked()
                            {
                                let act = self.input_sm.on_set_menu_page(MenuPage::Settings);
                                Self::apply_capture_action(ctx, act);
                            }
                            ui.add_space(6.0);
                            if ui
                                .add_sized([w, 34.0], egui::Button::new("MODULES"))
                                .clicked()
                            {
                                let act = self.input_sm.on_set_menu_page(MenuPage::Modules);
                                Self::apply_capture_action(ctx, act);
                            }
                            ui.add_space(6.0);
                            if ui
                                .add_sized([w, 34.0], egui::Button::new("TELEPORT LIST"))
                                .clicked()
                            {
                                let act = self.input_sm.on_set_menu_page(MenuPage::Teleports);
                                Self::apply_capture_action(ctx, act);
                            }
                            ui.add_space(6.0);
                            if ui
                                .add_sized([w, 34.0], egui::Button::new("WORLD MAP  [M]"))
                                .clicked()
                            {
                                let act = self.input_sm.on_set_menu_page(MenuPage::WorldMap);
                                Self::apply_capture_action(ctx, act);
                            }
                            ui.add_space(6.0);
                            ui.checkbox(&mut self.show_diagnostics, "Show Diagnostics overlay");
                            ui.add_space(6.0);
                            if ui
                                .add_sized([w, 30.0], egui::Button::new("Back to Launcher"))
                                .clicked()
                            {
                                let act = self.input_sm.on_set_menu_page(MenuPage::Closed);
                                Self::apply_capture_action(ctx, act);
                                self.page = Page::Launcher;
                            }
                            if ui
                                .add_sized([w, 30.0], egui::Button::new("Exit Explorer"))
                                .clicked()
                            {
                                self.save_settings();
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        });
                    });
            }
            MenuPage::Settings => {
                egui::Window::new("SETTINGS  |  ESC to go back")
                    .default_pos(egui::pos2(16.0, 80.0))
                    .default_size(egui::vec2(480.0, 440.0))
                    .collapsible(false)
                    .resizable(true)
                    .vscroll(true)
                    .show(ctx, |ui| {
                        let tab = self.world_settings_tab;
                        self.draw_settings(ui, tab);
                        ui.separator();
                        if ui.button("Back to menu").clicked() {
                            self.input_sm.on_set_menu_page(MenuPage::Main);
                        }
                    });
            }
            MenuPage::Modules => {
                egui::Window::new("MODULES  |  ESC to go back")
                    .default_pos(egui::pos2(16.0, 80.0))
                    .default_size(egui::vec2(460.0, 360.0))
                    .collapsible(false)
                    .resizable(true)
                    .show(ctx, |ui| {
                        self.draw_modules(ui);
                        ui.separator();
                        if ui.button("Back to menu").clicked() {
                            self.input_sm.on_set_menu_page(MenuPage::Main);
                        }
                    });
            }
            MenuPage::Teleports => {
                egui::Window::new("TELEPORT LIST  |  ESC to go back")
                    .default_pos(egui::pos2(16.0, 80.0))
                    .default_size(egui::vec2(460.0, 360.0))
                    .collapsible(false)
                    .resizable(true)
                    .show(ctx, |ui| {
                        ui.heading("Explore destinations");
                        let zones: Vec<String> = self
                            .detected_client
                            .as_ref()
                            .and_then(|r| r.as_ref().ok())
                            .map(|c| c.zones.clone())
                            .unwrap_or_default();
                        if !zones.is_empty() {
                            ui.horizontal(|ui| {
                                ui.label("Zone:");
                                egui::ComboBox::from_id_salt("tp_zone_selector")
                                    .selected_text(&self.selected_zone)
                                    .show_ui(ui, |ui| {
                                        for z in &zones {
                                            ui.selectable_value(&mut self.selected_zone, z.clone(), z);
                                        }
                                    });
                                if ui.button("Load World").clicked() {
                                    self.load_selected_zone();
                                }
                            });
                        }
                        ui.separator();
                        ui.label("Teleport destinations: missing (client teleport table not decoded yet).");
                        ui.separator();
                        if ui.button("Back to menu").clicked() {
                            self.input_sm.on_set_menu_page(MenuPage::Main);
                        }
                    });
            }
            MenuPage::WorldMap => {
                egui::Window::new("WORLD MAP  |  M or ESC to close")
                    .default_pos(egui::pos2(80.0, 80.0))
                    .default_size(egui::vec2(420.0, 240.0))
                    .collapsible(false)
                    .resizable(true)
                    .show(ctx, |ui| {
                        ui.label("World map tiles: missing (client surfaces/maps tiles not decoded yet).");
                        ui.label(format!(
                            "Current camera: X={:.1}, Y={:.1}, Z={:.1}",
                            self.camera.position.x, self.camera.position.y, self.camera.position.z
                        ));
                        if ui.button("Close (M)").clicked() {
                            self.input_sm.on_set_menu_page(MenuPage::Closed);
                        }
                    });
            }
        }
    }

    fn draw_settings(&mut self, ui: &mut egui::Ui, tab: SettingsTab) {
        ui.heading("Settings");
        ui.horizontal(|ui| {
            for (candidate, label) in [
                (SettingsTab::Graphics, "Graphics"),
                (SettingsTab::World, "World"),
                (SettingsTab::Sound, "Sound"),
                (SettingsTab::Interface, "Interface"),
            ] {
                if ui.selectable_label(tab == candidate, label).clicked() {
                    self.world_settings_tab = candidate;
                    if matches!(self.page, Page::Settings(_)) {
                        self.page = Page::Settings(candidate);
                    }
                }
            }
        });
        ui.separator();

        let mut changed = false;
        match tab {
            SettingsTab::Graphics => {
                ui.label(
                    "Graphics, atmosphere, and water settings (persisted under <root>/settings/).",
                );
                ui.add_space(6.0);
                changed |= ui
                    .add(
                        egui::Slider::new(
                            &mut self.settings.resolution_scale_pct,
                            MIN_RESOLUTION_SCALE_PCT..=MAX_RESOLUTION_SCALE_PCT,
                        )
                        .suffix("%")
                        .text("Resolution scale (3D world; UI stays native)"),
                    )
                    .changed();
                ui.separator();
                changed |= ui
                    .checkbox(
                        &mut self.settings.fog_enabled,
                        "Environmental fog (default on)",
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(
                            &mut self.settings.fog_distance_mult_pct,
                            MIN_FOG_DIST_MULT_PCT..=MAX_FOG_DIST_MULT_PCT,
                        )
                        .suffix("%")
                        .text(format!(
                            "Fog distance multiplier (default {DEFAULT_FOG_DIST_MULT_PCT}%)"
                        )),
                    )
                    .changed();
                ui.separator();
                changed |= ui
                    .checkbox(
                        &mut self.settings.water_reflection,
                        "Water planar reflection (disable for weak GPUs)",
                    )
                    .changed();
                changed |= ui
                    .checkbox(
                        &mut self.settings.water_refraction,
                        "Water refraction (disable for weak GPUs)",
                    )
                    .changed();
                ui.horizontal(|ui| {
                    ui.label("Water quality (pass resolution):");
                    for (q, label) in [
                        (WaterQuality::Low, "Low (25%)"),
                        (WaterQuality::Medium, "Medium (50%)"),
                        (WaterQuality::High, "High (100%)"),
                    ] {
                        if ui
                            .selectable_label(self.settings.water_quality == q, label)
                            .clicked()
                        {
                            self.settings.water_quality = q;
                            changed = true;
                        }
                    }
                });
                ui.separator();
                changed |= ui
                    .checkbox(
                        &mut self.settings.follow_real_clock,
                        "Time of day follows real clock (UTC)",
                    )
                    .changed();
                if !self.settings.follow_real_clock {
                    let hh = (self.settings.time_of_day_minutes / 60) % 24;
                    let mm = self.settings.time_of_day_minutes % 60;
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut self.settings.time_of_day_minutes, 0..=1439)
                                .text(format!("Time of day ({hh:02}:{mm:02}; default 12:00)")),
                        )
                        .changed();
                }
                ui.separator();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut self.settings.tex_budget_mib, 8..=1024)
                            .suffix(" MiB")
                            .text(format!(
                                "Decoded texture cache budget (FW_TEX_BUDGET_MIB; default {DEFAULT_TEX_BUDGET_MIB} MiB)"
                            )),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut self.settings.terrain_ram_mib, 1..=256)
                            .suffix(" MiB")
                            .text(format!(
                                "Terrain hot RAM budget (FW_TERRAIN_RAM_MIB; default {DEFAULT_TERRAIN_RAM_MIB} MiB)"
                            )),
                    )
                    .changed();
                ui.separator();
                changed |= ui
                    .add(
                        egui::Slider::new(
                            &mut self.settings.tree_lod_dist_mult_pct,
                            MIN_TREE_LOD_DIST_MULT_PCT..=MAX_TREE_LOD_DIST_MULT_PCT,
                        )
                        .suffix("%")
                        .text(format!(
                            "Tree LOD distance multiplier (default {DEFAULT_TREE_LOD_DIST_MULT_PCT}%)"
                        )),
                    )
                    .changed();
            }
            SettingsTab::World => {
                ui.label("Static scenery streaming and transparency controls.");
                ui.add_space(6.0);
                ui.checkbox(
                    &mut self.terrain_wireframe_debug,
                    "Terrain wireframe debug mode (only when 0 terrain textures resolve; default off)",
                );
                changed |= ui
                    .checkbox(
                        &mut self.settings.scenery_enabled,
                        "Show static scenery (.bmd)",
                    )
                    .changed();
                changed |= ui
                    .checkbox(
                        &mut self.settings.transparent_scenery_enabled,
                        "Show transparent scenery submeshes (sorted back-to-front; default on)",
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(
                            &mut self.settings.scenery_radius_m,
                            MIN_SCENERY_RADIUS_M..=MAX_SCENERY_RADIUS_M,
                        )
                        .suffix(" m")
                        .text(format!(
                            "Scenery streaming radius (default {DEFAULT_SCENERY_RADIUS_M} m)"
                        )),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(
                            &mut self.settings.scenery_max_instances,
                            MIN_SCENERY_MAX_INSTANCES..=MAX_SCENERY_MAX_INSTANCES,
                        )
                        .text(format!(
                            "Max loaded scenery instances (default {DEFAULT_SCENERY_MAX_INSTANCES})"
                        )),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(
                            &mut self.settings.scenery_loads_per_frame,
                            MIN_SCENERY_LOADS_PER_FRAME..=MAX_SCENERY_LOADS_PER_FRAME,
                        )
                        .text(format!(
                            "New models decoded per frame (default {DEFAULT_SCENERY_LOADS_PER_FRAME})"
                        )),
                    )
                    .changed();
                changed |= ui
                    .checkbox(
                        &mut self.settings.scenery_whole_world,
                        "Whole-world scenery loading (default off)",
                    )
                    .changed();
                if self.settings.scenery_whole_world {
                    ui.label("Warning: whole-world loading ignores the streaming radius and max-instance cap, which can consume large amounts of RAM/VRAM on dense zones and is not verified on full-zone memory use.");
                }
                ui.separator();
                ui.strong("Vegetation (SpeedTree & Grass)");
                changed |= ui
                    .checkbox(
                        &mut self.settings.trees_enabled,
                        "Show trees (via 32-bit SpeedTree helper; wind sway approximate)",
                    )
                    .changed();
                changed |= ui
                    .checkbox(&mut self.settings.grass_enabled, "Show grass")
                    .changed();
                ui.horizontal(|ui| {
                    ui.label("Grass density:");
                    for (d, label) in [
                        (GrassDensity::Low, "Low (35%)"),
                        (GrassDensity::Medium, "Medium (65%)"),
                        (GrassDensity::High, "High (100%)"),
                    ] {
                        if ui
                            .selectable_label(self.settings.grass_density == d, label)
                            .clicked()
                        {
                            self.settings.grass_density = d;
                            changed = true;
                        }
                    }
                });
                changed |= ui
                    .add(
                        egui::Slider::new(
                            &mut self.settings.veg_radius_m,
                            MIN_VEG_RADIUS_M..=MAX_VEG_RADIUS_M,
                        )
                        .suffix(" m")
                        .text(format!(
                            "Vegetation streaming radius (default {DEFAULT_VEG_RADIUS_M} m)"
                        )),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(
                            &mut self.settings.veg_max_instances,
                            MIN_VEG_MAX_INSTANCES..=MAX_VEG_MAX_INSTANCES,
                        )
                        .text(format!(
                            "Max vegetation instances (default {DEFAULT_VEG_MAX_INSTANCES})"
                        )),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(
                            &mut self.settings.veg_loads_per_frame,
                            MIN_VEG_LOADS_PER_FRAME..=MAX_VEG_LOADS_PER_FRAME,
                        )
                        .text(format!(
                            "New vegetation loads per frame (default {DEFAULT_VEG_LOADS_PER_FRAME})"
                        )),
                    )
                    .changed();
            }
            SettingsTab::Sound => {
                ui.label("Sound settings (audio backend arrives in Phase 7).");
                ui.add_space(6.0);
                changed |= ui
                    .add(
                        egui::Slider::new(&mut self.settings.music_volume, 0..=100)
                            .suffix("%")
                            .text("Music volume"),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut self.settings.ambience_volume, 0..=100)
                            .suffix("%")
                            .text("Ambience volume"),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut self.settings.effects_volume, 0..=100)
                            .suffix("%")
                            .text("Effects volume"),
                    )
                    .changed();
            }
            SettingsTab::Interface => {
                ui.label("Interface settings.");
                ui.add_space(6.0);
                changed |= ui
                    .checkbox(&mut self.settings.show_minimap, "Show minimap")
                    .changed();
                changed |= ui
                    .checkbox(&mut self.settings.show_zone_notices, "Show zone notices")
                    .changed();
            }
        }
        if changed {
            self.save_settings();
        }
    }

    fn draw_modules(&mut self, ui: &mut egui::Ui) {
        ui.heading("Modules");
        ui.label(format!(
            "Module search root: {}",
            self.modules.root.display()
        ));
        if ui.button("Rescan Modules").clicked() {
            self.modules = Registry::discover(Registry::default_root());
        }
        ui.separator();

        if self.modules.packages.is_empty() {
            ui.label("No valid module packages found.");
        } else {
            for pkg in &self.modules.packages {
                ui.group(|ui| {
                    ui.strong(format!("{} (id: {}, v{})", pkg.name, pkg.id, pkg.version));
                    ui.label(&pkg.description);
                });
            }
        }
        if !self.modules.errors.is_empty() {
            ui.add_space(8.0);
            ui.strong("Module discovery warnings:");
            for err in &self.modules.errors {
                ui.label(err);
            }
        }
    }

    fn draw_about(&self, ui: &mut egui::Ui) {
        ui.heading("About Forsaken World Explorer");
        ui.label(format!("Version: {}", env!("CARGO_PKG_VERSION")));
        ui.label("License: GPL-2.0-or-later");
        ui.add_space(8.0);
        ui.label(
            "An independent, non-affiliated free-flight viewer for a user-owned Forsaken World client. \
             No game files, archives, DLLs, or assets are bundled or modified.",
        );
        ui.add_space(8.0);
        ui.label(
            "Portable layout: settings, logs, and cache live beside forsaken-world-explorer.exe.",
        );
    }

    fn render_ui(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(self.page == Page::Launcher, "Launcher")
                    .clicked()
                {
                    self.page = Page::Launcher;
                }
                if ui
                    .selectable_label(self.page == Page::WorldView, "World View")
                    .clicked()
                {
                    self.page = Page::WorldView;
                }
                let in_settings = matches!(self.page, Page::Settings(_));
                if ui.selectable_label(in_settings, "Settings").clicked() {
                    self.page = Page::Settings(SettingsTab::Graphics);
                }
                if ui
                    .selectable_label(self.page == Page::Modules, "Modules")
                    .clicked()
                {
                    self.page = Page::Modules;
                }
                if ui
                    .selectable_label(self.page == Page::About, "About")
                    .clicked()
                {
                    self.page = Page::About;
                }
            });
        });

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.label(&self.status_line);
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.page {
            Page::Launcher => self.draw_launcher(ui),
            Page::WorldView => self.draw_world_view(ctx, ui),
            Page::Settings(tab) => self.draw_settings(ui, tab),
            Page::Modules => self.draw_modules(ui),
            Page::About => self.draw_about(ui),
        });
    }
}

impl eframe::App for ExplorerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.render_ui(ctx);
    }
}

pub fn run_smoke_launch() -> Result<(), String> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let _adapters = instance.enumerate_adapters(wgpu::Backends::all());

    let mut app = ExplorerApp::new_headless_smoke();
    let ctx = egui::Context::default();
    let pages = [
        Page::Launcher,
        Page::WorldView,
        Page::Settings(SettingsTab::Graphics),
        Page::Settings(SettingsTab::World),
        Page::Settings(SettingsTab::Sound),
        Page::Settings(SettingsTab::Interface),
        Page::Modules,
        Page::About,
    ];
    for page in pages {
        app.page = page;
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_ui(ctx);
        });
        if output.shapes.is_empty() {
            return Err("headless UI smoke frame produced no shapes".into());
        }
    }
    Ok(())
}

fn try_create_fallback_wgpu_setup() -> Option<egui_wgpu::WgpuSetupExisting> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: None,
        force_fallback_adapter: true,
    }))?;
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("forsaken-world-explorer-fallback"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
        },
        None,
    ))
    .ok()?;
    Some(egui_wgpu::WgpuSetupExisting {
        instance,
        adapter,
        device,
        queue,
    })
}

fn try_create_fallback_wgpu_setup_if_no_hardware() -> Option<egui_wgpu::WgpuSetupExisting> {
    let probe = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let has_hw = probe
        .enumerate_adapters(wgpu::Backends::PRIMARY)
        .iter()
        .any(|a| {
            matches!(
                a.get_info().device_type,
                wgpu::DeviceType::DiscreteGpu | wgpu::DeviceType::IntegratedGpu
            )
        });
    if has_hw {
        None
    } else {
        try_create_fallback_wgpu_setup()
    }
}

pub fn run_window() -> Result<(), String> {
    let wgpu_setup = if let Some(existing) = try_create_fallback_wgpu_setup_if_no_hardware() {
        egui_wgpu::WgpuSetup::Existing(existing)
    } else {
        egui_wgpu::WgpuSetup::CreateNew(egui_wgpu::WgpuSetupCreateNew {
            native_adapter_selector: Some(Arc::new(|adapters, surface| {
                let rank = |device_type: wgpu::DeviceType| match device_type {
                    wgpu::DeviceType::DiscreteGpu => 0,
                    wgpu::DeviceType::IntegratedGpu => 1,
                    wgpu::DeviceType::VirtualGpu => 2,
                    wgpu::DeviceType::Cpu => 3,
                    wgpu::DeviceType::Other => 4,
                };
                adapters
                    .iter()
                    .filter(|adapter| {
                        surface
                            .map(|s| adapter.is_surface_supported(s))
                            .unwrap_or(true)
                    })
                    .min_by_key(|adapter| rank(adapter.get_info().device_type))
                    .cloned()
                    .ok_or_else(|| "no compatible wgpu adapter found".to_string())
            })),
            ..Default::default()
        })
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1080.0, 680.0])
            .with_title("Forsaken World Explorer"),
        wgpu_options: egui_wgpu::WgpuConfiguration {
            wgpu_setup,
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        "Forsaken World Explorer",
        options,
        Box::new(|cc| {
            let mut app = ExplorerApp::new();
            if let Some(render_state) = &cc.wgpu_render_state {
                let renderer = TerrainViewportRenderer::new(
                    &render_state.device,
                    &render_state.queue,
                    render_state.target_format,
                );
                let ts_msg = if renderer.timestamp_queries_supported {
                    "GPU timestamp queries supported on adapter".to_string()
                } else {
                    "GPU timestamp queries not supported on adapter (skipped)".to_string()
                };
                if let Some(log) = app.logger.as_mut() {
                    log.info(&ts_msg);
                }
                app.displayed_diag.gpu_timing_note = ts_msg;
                render_state
                    .renderer
                    .write()
                    .callback_resources
                    .insert(renderer);
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| format!("failed to launch viewer window: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_smoke_launch_renders_all_tabs() {
        run_smoke_launch().expect("headless smoke launch should succeed");
    }
}
