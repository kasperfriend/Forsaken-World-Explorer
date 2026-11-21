//! Desktop window, menu, native folder picker, portable root layout, live 3D World View
//! with sky, day/night terrain lightmaps, opaque & transparent `.bmd` static scenery,
//! experimental animated water (reflection/refraction/shoreline/underwater), free-flight
//! camera, resolution scaling, Settings, 1 Hz Diagnostics panel, and Modules.

use std::{
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
    logging::{self, RunLogger},
    modules::Registry,
    pck::PckArchive,
    scenery::{self, SceneryDiagnostics, SceneryModelCache},
    settings::{
        PortablePaths, ViewerSettings, WaterQuality, DEFAULT_FOG_DIST_MULT_PCT,
        DEFAULT_SCENERY_LOADS_PER_FRAME, DEFAULT_SCENERY_MAX_INSTANCES, DEFAULT_SCENERY_RADIUS_M,
        DEFAULT_TERRAIN_RAM_MIB, DEFAULT_TEX_BUDGET_MIB, MAX_FOG_DIST_MULT_PCT,
        MAX_RESOLUTION_SCALE_PCT, MAX_SCENERY_LOADS_PER_FRAME, MAX_SCENERY_MAX_INSTANCES,
        MAX_SCENERY_RADIUS_M, MIN_FOG_DIST_MULT_PCT, MIN_RESOLUTION_SCALE_PCT,
        MIN_SCENERY_LOADS_PER_FRAME, MIN_SCENERY_MAX_INSTANCES, MIN_SCENERY_RADIUS_M,
    },
    terrain_gpu::{
        build_scenery_batches_for_model, CameraUniform, SceneryDrawSnapshot, TerrainMeshSnapshot,
        TerrainSession, TerrainTextureSnapshot, TerrainViewportRenderer, WaterRenderSnapshot,
    },
    tex_cache::{DecodedTexture, TextureCache},
    world_objects::{self, OrnamentPlacement},
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
    water: Arc<WaterRenderSnapshot>,
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
            &self.water,
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
    zone: String,
    placements: Vec<OrnamentPlacement>,
    box_areas: Vec<BoxAreaAtmosphere>,
    water_planes: Vec<WaterPlane>,
    model_cache: SceneryModelCache,
    tex_cache: TextureCache,
    bmd_archives: Vec<PckArchive>,
    tex_archives: Vec<PckArchive>,
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

        let res_dir = client_root.join("resources");
        let mut bmd_archives = Vec::new();
        for name in ["litmodels.pck", "building.pck"] {
            let p = res_dir.join(name);
            if p.is_file() {
                if let Ok(a) = PckArchive::open(&p) {
                    bmd_archives.push(a);
                }
            }
        }
        let mut tex_archives = Vec::new();
        for name in ["textures.pck", "litmodels.pck", "building.pck"] {
            let p = res_dir.join(name);
            if p.is_file() {
                if let Ok(a) = PckArchive::open(&p) {
                    tex_archives.push(a);
                }
            }
        }

        let model_cache = SceneryModelCache::open(cache_root)?;
        let tex_cache = TextureCache::new(tex_budget_bytes);
        Ok(Self {
            zone: zone.to_string(),
            placements,
            box_areas,
            water_planes: Vec::new(),
            model_cache,
            tex_cache,
            bmd_archives,
            tex_archives,
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
            zone: "synthetic-preview".to_string(),
            placements,
            box_areas: vec![BoxAreaAtmosphere::default()],
            water_planes,
            model_cache,
            tex_cache,
            bmd_archives: Vec::new(),
            tex_archives: Vec::new(),
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
        let synth = self.synthetic_bmd_bytes.clone();
        let bmd_archives = &mut self.bmd_archives;
        let (_decoded_now, errs, drawn_instances, _total_trans) = scenery::step_streaming_frame(
            &mut self.model_cache,
            &dedup,
            settings.scenery_loads_per_frame as usize,
            |model_key| {
                if let Some(raw) = &synth {
                    return Ok(raw.clone());
                }
                let candidates = [
                    model_key.to_string(),
                    format!("litmodels/{zone}/{model_key}"),
                    format!("litmodels/{model_key}"),
                    format!("building/{model_key}"),
                ];
                for arch in bmd_archives.iter_mut() {
                    for cand in &candidates {
                        if let Ok(bytes) = arch.read_entry(cand) {
                            return Ok(bytes);
                        }
                    }
                }
                Err(format!("model not found in archives: {model_key}"))
            },
        );
        if let Some(log) = logger.as_mut() {
            for e in &errs {
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
                let tex_archives = &mut self.tex_archives;
                let tex_cache = &mut self.tex_cache;
                let (op, tr, approx, skipped) = build_scenery_batches_for_model(
                    &model,
                    &batch.transforms,
                    settings.transparent_scenery_enabled,
                    (b_idx as u64) << 32,
                    |tex_path| {
                        tex_cache
                            .get_or_insert_pinned(tex_path, || {
                                let candidates = [
                                    tex_path.to_string(),
                                    format!("textures/{tex_path}"),
                                    format!("litmodels/{tex_path}"),
                                ];
                                for arch in tex_archives.iter_mut() {
                                    for cand in &candidates {
                                        if let Ok(bytes) = arch.read_entry(cand) {
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
                                Ok(DecodedTexture {
                                    width: 1,
                                    height: 1,
                                    rgba: vec![235, 220, 200, 255],
                                })
                            })
                            .ok()
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
    show_map_placeholder: bool,
    show_world_menu: bool,
    show_diagnostics: bool,
    started_at: Instant,
    terrain_session: Option<TerrainSession>,
    scenery_state: Option<ActiveSceneryState>,
    mesh_arc: Arc<TerrainMeshSnapshot>,
    day_lm_arc: Arc<TerrainTextureSnapshot>,
    night_lm_arc: Arc<TerrainTextureSnapshot>,
    scenery_arc: Arc<SceneryDrawSnapshot>,
    water_arc: Arc<WaterRenderSnapshot>,
    latest_diag: SceneryDiagnostics,
    displayed_diag: SceneryDiagnostics,
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
            .and_then(|c| c.zones.first().cloned())
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
            streaming_radius: 2,
            show_map_placeholder: false,
            show_world_menu: false,
            show_diagnostics: true,
            started_at: Instant::now(),
            terrain_session: None,
            scenery_state: None,
            mesh_arc: Arc::new(TerrainMeshSnapshot::default()),
            day_lm_arc: Arc::new(TerrainTextureSnapshot::default()),
            night_lm_arc: Arc::new(TerrainTextureSnapshot::default()),
            scenery_arc: Arc::new(SceneryDrawSnapshot::default()),
            water_arc: Arc::new(WaterRenderSnapshot::default()),
            latest_diag: default_diag.clone(),
            displayed_diag: default_diag,
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
            if let Some(first_zone) = client.zones.first() {
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
        if self.last_diag_refresh.elapsed() >= Duration::from_secs(1) {
            self.displayed_diag = self.latest_diag.clone();
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
        match TerrainSession::open_zone(&client.root, &zone, &self.paths.cache_dir, hot_bytes) {
            Ok(mut session) => {
                self.camera = FreeCamera::default();
                let _ = session.update_for_camera(self.camera.position, self.streaming_radius);
                self.mesh_arc = Arc::new(session.mesh.clone());
                self.day_lm_arc = Arc::new(session.day_lightmap.clone());
                self.night_lm_arc = Arc::new(session.night_lightmap.clone());
                let scenery_note = match ActiveSceneryState::open_zone(
                    &client.root,
                    &zone,
                    &self.paths.cache_dir,
                    tex_budget_bytes,
                ) {
                    Ok(st) => {
                        let count = st.placements.len();
                        let boxes = st.box_areas.len();
                        self.scenery_state = Some(st);
                        format!("{count} placements, {boxes} atmosphere box(es)")
                    }
                    Err(err) => {
                        self.scenery_state = None;
                        format!("scenery unavailable ({err})")
                    }
                };
                let vp = self.camera.view_proj(16.0 / 9.0);
                self.step_scenery(vp, 1080, 680);
                self.displayed_diag = self.latest_diag.clone();
                self.last_diag_refresh = Instant::now();
                let msg = format!(
                    "Loaded zone '{}' ({}x{} blocks, {})",
                    session.zone_name, session.block_rows, session.block_cols, scenery_note
                );
                if let Some(log) = self.logger.as_mut() {
                    log.info(&msg);
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
                let vp = self.camera.view_proj(16.0 / 9.0);
                self.step_scenery(vp, 1080, 680);
                self.displayed_diag = self.latest_diag.clone();
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
                if ui.button("Enter World View").clicked() {
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
    }

    fn draw_world_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        ctx.input(|i| {
            if i.key_pressed(egui::Key::M) {
                self.show_map_placeholder = !self.show_map_placeholder;
            }
            if i.key_pressed(egui::Key::Escape) {
                self.show_world_menu = !self.show_world_menu;
            }
        });

        let mut settings_changed = false;
        ui.horizontal(|ui| {
            let zones: Vec<String> = self
                .detected_client
                .as_ref()
                .and_then(|r| r.as_ref().ok())
                .map(|c| c.zones.clone())
                .unwrap_or_default();

            if !zones.is_empty() {
                egui::ComboBox::from_id_salt("zone_selector")
                    .selected_text(&self.selected_zone)
                    .show_ui(ui, |ui| {
                        for z in &zones {
                            ui.selectable_value(&mut self.selected_zone, z.clone(), z);
                        }
                    });
                if ui.button("Load Zone").clicked() {
                    self.load_selected_zone();
                }
            }
            if ui.button("Preview Synthetic Scene").clicked() {
                self.load_synthetic_preview();
            }
            ui.checkbox(&mut self.show_diagnostics, "Diagnostics");
            ui.separator();
            let hours = self.settings.time_of_day_hours();
            let hh = (hours.floor() as u32) % 24;
            let mm = ((hours.fract() * 60.0).floor() as u32) % 60;
            ui.label(format!("Time: {hh:02}:{mm:02}"));
            if !self.settings.follow_real_clock {
                settings_changed |= ui
                    .add(
                        egui::Slider::new(&mut self.settings.time_of_day_minutes, 0..=1439)
                            .show_value(false),
                    )
                    .changed();
            }
            settings_changed |= ui
                .checkbox(&mut self.settings.follow_real_clock, "Real clock")
                .changed();
        });
        if settings_changed {
            self.save_settings();
        }

        let avail = ui.available_size();
        let (rect, response) =
            ui.allocate_exact_size(avail.max(egui::vec2(64.0, 64.0)), egui::Sense::drag());

        if !self.show_world_menu {
            let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 0.1);
            let drag_delta = if response.dragged_by(egui::PointerButton::Secondary)
                || response.dragged_by(egui::PointerButton::Primary)
            {
                response.drag_delta()
            } else {
                egui::Vec2::ZERO
            };
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
                mouse_dx: drag_delta.x,
                mouse_dy: drag_delta.y,
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
                || drag_delta.length_sq() > 0.0;
            if moving {
                self.camera.step(&cam_input, dt);
                if let Some(session) = self.terrain_session.as_mut() {
                    if let Ok(true) =
                        session.update_for_camera(self.camera.position, self.streaming_radius)
                    {
                        self.mesh_arc = Arc::new(session.mesh.clone());
                    }
                }
                ctx.request_repaint();
            }
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
                water: Arc::clone(&self.water_arc),
            },
        );
        ui.painter().add(cb);

        egui::Area::new(egui::Id::new("world_controls_hud"))
            .fixed_pos(rect.left_top() + egui::vec2(10.0, 10.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.small("WASD move | Space/E up | Q/Ctrl down | Shift fast | Arrows/Drag look | M map | Esc menu");
                    ui.small(format!(
                        "3D target: {scaled_w}x{scaled_h} ({}% of {vp_w}x{vp_h}) | Terrain blocks: {} ({} KiB)",
                        self.settings.resolution_scale_pct,
                        self.mesh_arc.streamed_blocks,
                        self.mesh_arc.hot_ram_bytes / 1024
                    ));
                });
            });

        if self.show_diagnostics {
            let d = &self.displayed_diag;
            let dn = atmosphere::day_night_factor(hours);
            let lighting_label = if d.approximate_lighting_meshes > 0 {
                format!(
                    "client day/night (dn={dn:.2}) + {} approx fallback mesh(es)",
                    d.approximate_lighting_meshes
                )
            } else {
                format!("client day/night (dn={dn:.2}; sky gradient approximate)")
            };
            egui::Area::new(egui::Id::new("world_diagnostics_panel"))
                .fixed_pos(rect.right_top() + egui::vec2(-315.0, 10.0))
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.strong("World Diagnostics (1 Hz)");
                        ui.small(format!("Lighting: {lighting_label}"));
                        ui.small(format!(
                            "Placements in range: {} / {}",
                            d.placements_in_range, d.placements_total
                        ));
                        ui.small(format!(
                            "Unique models loaded: {} (in range: {})",
                            d.models_loaded, d.unique_models_in_range
                        ));
                        ui.small(format!(
                            "Opaque/alpha-test instances: {}",
                            d.instances_drawn
                        ));
                        ui.small(format!(
                            "Transparent submeshes: drawn={}, skipped={}",
                            d.transparent_submeshes_drawn, d.transparent_submeshes_skipped
                        ));
                        ui.small(format!("Failed to decode: {}", d.decode_failures));
                        ui.small(format!(
                            "Water planes visible: {} | World render passes/frame: {}",
                            d.water_planes_visible, d.world_render_passes
                        ));
                        ui.small(format!(
                            "Textures cached: {} ({:.2} / {:.2} MiB){}",
                            d.textures_cached,
                            d.texture_cache_bytes as f64 / (1024.0 * 1024.0),
                            d.texture_budget_bytes as f64 / (1024.0 * 1024.0),
                            if d.texture_budget_exceeded {
                                " [PINNED > BUDGET]"
                            } else {
                                ""
                            }
                        ));
                        if !d.gpu_timing_note.is_empty() {
                            ui.small(format!("GPU timing: {}", d.gpu_timing_note));
                        }
                    });
                });
        }

        if self.show_map_placeholder {
            egui::Window::new("World Map (Placeholder)")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("Original client map tiles and teleport search arrive in Phase 7.");
                    ui.label(format!(
                        "Current camera: X={:.1}, Y={:.1}, Z={:.1}",
                        self.camera.position.x, self.camera.position.y, self.camera.position.z
                    ));
                    if ui.button("Close (M)").clicked() {
                        self.show_map_placeholder = false;
                    }
                });
        }

        if self.show_world_menu {
            egui::Window::new("Explorer Menu (Esc)")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    if ui.button("Resume Free Flight").clicked() {
                        self.show_world_menu = false;
                    }
                    if ui.button("Graphics Settings").clicked() {
                        self.show_world_menu = false;
                        self.page = Page::Settings(SettingsTab::Graphics);
                    }
                    if ui.button("World & Scenery Settings").clicked() {
                        self.show_world_menu = false;
                        self.page = Page::Settings(SettingsTab::World);
                    }
                    if ui.button("Back to Launcher").clicked() {
                        self.show_world_menu = false;
                        self.page = Page::Launcher;
                    }
                });
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
                    self.page = Page::Settings(candidate);
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
            }
            SettingsTab::World => {
                ui.label("Static scenery streaming and transparency controls.");
                ui.add_space(6.0);
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
