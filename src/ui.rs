//! Desktop window, menu, client folder validation, Settings tabs, and Modules view.

use std::path::PathBuf;

use eframe::egui;
use forsaken_world_explorer::{
    client_detect::{self, DetectedClient},
    modules::Registry,
    settings::{ViewerSettings, MAX_RESOLUTION_SCALE_PCT, MIN_RESOLUTION_SCALE_PCT},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Launcher,
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

pub struct ExplorerApp {
    page: Page,
    settings_root: PathBuf,
    settings: ViewerSettings,
    client_dir_input: String,
    detected_client: Option<Result<DetectedClient, String>>,
    modules: Registry,
    status_line: String,
}

impl ExplorerApp {
    pub fn new() -> Self {
        let settings_root = ViewerSettings::default_root_dir();
        let (mut settings, status_line) = match ViewerSettings::load_from_dir(&settings_root) {
            Ok(loaded) => (
                loaded,
                format!(
                    "Settings loaded from {}",
                    ViewerSettings::settings_path_in(&settings_root).display()
                ),
            ),
            Err(error) => (
                ViewerSettings::default(),
                format!("Using default settings ({error})"),
            ),
        };
        settings.apply_env_overrides();
        let client_dir_input = settings
            .client_dir
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let detected_client = settings.client_dir.as_deref().map(client_detect::detect);
        let modules = Registry::discover(Registry::default_root());
        Self {
            page: Page::Launcher,
            settings_root,
            settings,
            client_dir_input,
            detected_client,
            modules,
            status_line,
        }
    }

    fn save_settings(&mut self) {
        let trimmed = self.client_dir_input.trim();
        self.settings.client_dir = if trimmed.is_empty() {
            None
        } else {
            Some(PathBuf::from(trimmed))
        };
        self.detected_client = self
            .settings
            .client_dir
            .as_deref()
            .map(client_detect::detect);
        match self.settings.save_to_dir(&self.settings_root) {
            Ok(path) => {
                self.status_line = format!("Saved settings to {}", path.display());
            }
            Err(error) => {
                self.status_line = format!("Failed to save settings: {error}");
            }
        }
    }

    fn draw_launcher(&mut self, ui: &mut egui::Ui) {
        ui.heading("Forsaken World Explorer");
        ui.label("Free-flight viewer for a user-owned Forsaken World client.");
        ui.add_space(10.0);

        ui.label("Client folder (never modified by this viewer):");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.client_dir_input);
            if ui.button("Validate & Save Path").clicked() {
                self.save_settings();
            }
        });
        ui.add_space(8.0);

        match &self.detected_client {
            None => {
                ui.label("No client folder selected yet. Enter the Forsaken World client root (containing update/ and resources/).");
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
            }
            Some(Err(err)) => {
                ui.label(format!("Validation error: {err}"));
            }
        }

        ui.add_space(8.0);
        ui.label(format!(
            "Cache & settings root: {}",
            self.settings_root.display()
        ));
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
                ui.label("Graphics settings (persisted to viewer-settings.txt).");
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
            "Current stage: Phase 1 (client detection and .pck/.pkx archive layer). \
             No 3D rendering or visual parity is claimed yet.",
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

/// CPU-only headless UI smoke test: initializes `ExplorerApp` and executes an `egui`
/// frame pass across every page and Settings tab to prove startup UI code does not panic.
pub fn run_smoke_launch() -> Result<(), String> {
    let mut app = ExplorerApp::new();
    let ctx = egui::Context::default();
    let pages = [
        Page::Launcher,
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

pub fn run_window() -> Result<(), String> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 620.0])
            .with_title("Forsaken World Explorer"),
        ..Default::default()
    };
    eframe::run_native(
        "Forsaken World Explorer",
        options,
        Box::new(|_cc| Ok(Box::new(ExplorerApp::new()))),
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
