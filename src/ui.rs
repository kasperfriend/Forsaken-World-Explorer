//! Desktop window, menu, native folder picker, portable root layout, live 3D World View
//! with free-flight camera and resolution scaling, Settings, and Modules.

use std::{path::PathBuf, sync::Arc};

use eframe::{egui, egui_wgpu, wgpu};
use forsaken_world_explorer::{
    camera::{scaled_render_target, CameraInput, FreeCamera},
    client_detect::{self, DetectedClient},
    logging::{self, RunLogger},
    modules::Registry,
    settings::{PortablePaths, ViewerSettings, MAX_RESOLUTION_SCALE_PCT, MIN_RESOLUTION_SCALE_PCT},
    terrain_gpu::{
        CameraUniform, TerrainMeshSnapshot, TerrainSession, TerrainTextureSnapshot,
        TerrainViewportRenderer,
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
    lightmap: Arc<TerrainTextureSnapshot>,
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
            &self.lightmap,
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
    terrain_session: Option<TerrainSession>,
    mesh_arc: Arc<TerrainMeshSnapshot>,
    lightmap_arc: Arc<TerrainTextureSnapshot>,
}

impl ExplorerApp {
    pub fn new() -> Self {
        let paths = PortablePaths::from_executable();
        Self::with_portable_paths(paths, true)
    }

    /// Construct app state for headless `--smoke-launch` without creating files
    /// in the build output directory or opening any native dialog.
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
            terrain_session: None,
            mesh_arc: Arc::new(TerrainMeshSnapshot::default()),
            lightmap_arc: Arc::new(TerrainTextureSnapshot::default()),
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
        self.save_settings();
    }

    fn load_selected_zone(&mut self) {
        let Some(Ok(client)) = &self.detected_client else {
            self.status_line = "Select a valid client folder first.".to_string();
            return;
        };
        let hot_bytes = (self.settings.terrain_ram_mib as usize) * 1024 * 1024;
        let zone = self.selected_zone.clone();
        match TerrainSession::open_zone(&client.root, &zone, &self.paths.cache_dir, hot_bytes) {
            Ok(mut session) => {
                self.camera = FreeCamera::default();
                let _ = session.update_for_camera(self.camera.position, self.streaming_radius);
                self.mesh_arc = Arc::new(session.mesh.clone());
                self.lightmap_arc = Arc::new(session.lightmap.clone());
                let msg = format!(
                    "Loaded zone '{}' ({}x{} blocks, {} streamed via cache/terrain-v1)",
                    session.zone_name,
                    session.block_rows,
                    session.block_cols,
                    session.mesh.streamed_blocks
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
        match TerrainSession::open_synthetic(&self.paths.cache_dir, hot_bytes) {
            Ok(mut session) => {
                self.camera = FreeCamera::default();
                let _ = session.update_for_camera(self.camera.position, self.streaming_radius);
                self.mesh_arc = Arc::new(session.mesh.clone());
                self.lightmap_arc = Arc::new(session.lightmap.clone());
                let msg = format!(
                    "Loaded synthetic terrain preview ({} blocks streamed via cache/terrain-v1)",
                    session.mesh.streamed_blocks
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
        // Keyboard toggles: M opens map placeholder, Escape opens/closes overlay menu.
        ctx.input(|i| {
            if i.key_pressed(egui::Key::M) {
                self.show_map_placeholder = !self.show_map_placeholder;
            }
            if i.key_pressed(egui::Key::Escape) {
                self.show_world_menu = !self.show_world_menu;
            }
        });

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
            if ui.button("Preview Synthetic Terrain").clicked() {
                self.load_synthetic_preview();
            }
            ui.separator();
            ui.label(format!(
                "Scale: {}% | Pos: ({:.1}, {:.1}, {:.1}) | Blocks: {} | Cache: {} KiB",
                self.settings.resolution_scale_pct,
                self.camera.position.x,
                self.camera.position.y,
                self.camera.position.z,
                self.mesh_arc.streamed_blocks,
                self.mesh_arc.hot_ram_bytes / 1024
            ));
        });

        let avail = ui.available_size();
        let (rect, response) =
            ui.allocate_exact_size(avail.max(egui::vec2(64.0, 64.0)), egui::Sense::drag());

        // Free-flight input handling (active when overlay menu is closed)
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
        let uniform = CameraUniform {
            view_proj: self.camera.view_proj(aspect).to_cols_array_2d(),
            sun_dir: [0.4, 0.85, 0.35, 0.0],
            fog_color: [0.15, 0.25, 0.40, 1.0],
        };

        let cb = egui_wgpu::Callback::new_paint_callback(
            rect,
            TerrainCallback {
                viewport_w: vp_w,
                viewport_h: vp_h,
                scale_pct: self.settings.resolution_scale_pct,
                uniform,
                mesh: Arc::clone(&self.mesh_arc),
                lightmap: Arc::clone(&self.lightmap_arc),
            },
        );
        ui.painter().add(cb);

        // Native-resolution HUD overlay (always crisp regardless of 3D resolution scale)
        egui::Area::new(egui::Id::new("world_controls_hud"))
            .fixed_pos(rect.left_top() + egui::vec2(10.0, 10.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.small("WASD move | Space/E up | Q/Ctrl down | Shift fast | Arrows/Drag look | M map | Esc menu");
                    ui.small(format!(
                        "3D render target: {scaled_w}x{scaled_h} ({}% of native {vp_w}x{vp_h})",
                        self.settings.resolution_scale_pct
                    ));
                });
            });

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
                    if ui.button("Graphics Settings (Resolution Scale)").clicked() {
                        self.show_world_menu = false;
                        self.page = Page::Settings(SettingsTab::Graphics);
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
                ui.label("Graphics settings (persisted under <root>/settings/).");
                ui.add_space(6.0);
                let response = ui.add(
                    egui::Slider::new(
                        &mut self.settings.resolution_scale_pct,
                        MIN_RESOLUTION_SCALE_PCT..=MAX_RESOLUTION_SCALE_PCT,
                    )
                    .suffix("%")
                    .text("Resolution scale (3D world; UI stays native)"),
                );
                changed |= response.changed();
            }
            SettingsTab::World => {
                ui.label("World & memory budget settings.");
                ui.add_space(6.0);
                changed |= ui
                    .add(
                        egui::Slider::new(&mut self.settings.terrain_ram_mib, 1..=256)
                            .suffix(" MiB")
                            .text("Terrain hot RAM budget (FW_TERRAIN_RAM_MIB)"),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut self.settings.tex_budget_mib, 8..=1024)
                            .suffix(" MiB")
                            .text("Decoded texture cache budget (FW_TEX_BUDGET_MIB)"),
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

/// CPU-only headless UI smoke test: initializes `ExplorerApp` non-interactively
/// (never opens any native folder dialog), probes `wgpu::Instance`, and renders
/// every UI page and Settings tab including World View.
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
            if let Some(render_state) = &cc.wgpu_render_state {
                let renderer = TerrainViewportRenderer::new(
                    &render_state.device,
                    &render_state.queue,
                    render_state.target_format,
                );
                render_state
                    .renderer
                    .write()
                    .callback_resources
                    .insert(renderer);
            }
            Ok(Box::new(ExplorerApp::new()))
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
