//! Portable root-folder paths and persisted viewer settings.
//!
//! Settings live in `<root>/settings/viewer-settings.txt`, logs in `<root>/logs/`,
//! and derived caches in `<root>/cache/` (overridable via `FW_CACHE_DIR`), where
//! `<root>` is the directory containing `forsaken-world-explorer.exe` — never the
//! current working directory and never the user's Forsaken World client folder.

use std::{
    env, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const LEGACY_APP_DIR_NAME: &str = "ForsakenWorldExplorer";
pub const SETTINGS_SUBDIR: &str = "settings";
pub const LOGS_SUBDIR: &str = "logs";
pub const CACHE_SUBDIR_ROOT: &str = "cache";
pub const SETTINGS_FILE_NAME: &str = "viewer-settings.txt";

pub const ENV_CACHE_DIR: &str = "FW_CACHE_DIR";
pub const ENV_CLIENT_DIR: &str = "FW_CLIENT_DIR";
pub const ENV_TERRAIN_RAM_MIB: &str = "FW_TERRAIN_RAM_MIB";
pub const ENV_TEX_BUDGET_MIB: &str = "FW_TEX_BUDGET_MIB";

pub const MIN_RESOLUTION_SCALE_PCT: u16 = 10;
pub const MAX_RESOLUTION_SCALE_PCT: u16 = 200;
pub const DEFAULT_RESOLUTION_SCALE_PCT: u16 = 100;
pub const DEFAULT_TERRAIN_RAM_MIB: u32 = 2;
pub const DEFAULT_TEX_BUDGET_MIB: u32 = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortablePaths {
    pub root: PathBuf,
    pub settings_dir: PathBuf,
    pub settings_file: PathBuf,
    pub logs_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl PortablePaths {
    /// Build portable paths rooted at `root`. Honors `FW_CACHE_DIR` for `cache_dir`.
    pub fn from_root(root: PathBuf) -> Self {
        let settings_dir = root.join(SETTINGS_SUBDIR);
        let settings_file = settings_dir.join(SETTINGS_FILE_NAME);
        let logs_dir = root.join(LOGS_SUBDIR);
        let cache_dir = env::var_os(ENV_CACHE_DIR)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join(CACHE_SUBDIR_ROOT));
        Self {
            root,
            settings_dir,
            settings_file,
            logs_dir,
            cache_dir,
        }
    }

    /// Resolve `<root>` from the running executable's directory (never the current working directory).
    pub fn from_executable() -> Self {
        let root = env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
            .unwrap_or_else(env::temp_dir);
        Self::from_root(root)
    }

    /// Check whether `<root>` can be written to by creating and removing a temporary probe file.
    pub fn check_writable(&self) -> Result<(), String> {
        fs::create_dir_all(&self.settings_dir).map_err(|e| {
            format!(
                "Portable folder {} is not writable ({e}). Choose a writable folder or move the app out of a protected directory (such as Program Files).",
                self.root.display()
            )
        })?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let probe = self
            .settings_dir
            .join(format!(".write-probe-{}-{nanos}", std::process::id()));
        fs::write(&probe, b"ok").map_err(|e| {
            format!(
                "Portable folder {} is not writable ({e}). Choose a writable folder or move the app out of a protected directory (such as Program Files).",
                self.root.display()
            )
        })?;
        let _ = fs::remove_file(&probe);
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewerSettings {
    pub client_dir: Option<PathBuf>,
    pub resolution_scale_pct: u16,
    pub terrain_ram_mib: u32,
    pub tex_budget_mib: u32,
    pub music_volume: u8,
    pub ambience_volume: u8,
    pub effects_volume: u8,
    pub show_minimap: bool,
    pub show_zone_notices: bool,
}

impl Default for ViewerSettings {
    fn default() -> Self {
        Self {
            client_dir: None,
            resolution_scale_pct: DEFAULT_RESOLUTION_SCALE_PCT,
            terrain_ram_mib: DEFAULT_TERRAIN_RAM_MIB,
            tex_budget_mib: DEFAULT_TEX_BUDGET_MIB,
            music_volume: 70,
            ambience_volume: 70,
            effects_volume: 80,
            show_minimap: true,
            show_zone_notices: true,
        }
    }
}

impl ViewerSettings {
    /// Optional legacy settings location (`%LOCALAPPDATA%\ForsakenWorldExplorer\viewer-settings.txt`).
    pub fn legacy_settings_path() -> Option<PathBuf> {
        #[cfg(windows)]
        {
            return env::var_os("LOCALAPPDATA")
                .filter(|v| !v.is_empty())
                .map(|dir| {
                    PathBuf::from(dir)
                        .join(LEGACY_APP_DIR_NAME)
                        .join(SETTINGS_FILE_NAME)
                });
        }
        #[cfg(not(windows))]
        {
            if let Some(xdg) = env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
                return Some(
                    PathBuf::from(xdg)
                        .join(LEGACY_APP_DIR_NAME)
                        .join(SETTINGS_FILE_NAME),
                );
            }
            env::var_os("HOME").filter(|v| !v.is_empty()).map(|home| {
                PathBuf::from(home)
                    .join(".config")
                    .join(LEGACY_APP_DIR_NAME)
                    .join(SETTINGS_FILE_NAME)
            })
        }
    }

    /// One-time import from `legacy_path` into `paths.settings_file` if the portable
    /// settings file does not exist yet. Leaves the legacy file untouched.
    pub fn import_legacy_once(
        paths: &PortablePaths,
        legacy_path: Option<&Path>,
    ) -> Result<Option<PathBuf>, String> {
        if paths.settings_file.is_file() {
            return Ok(None);
        }
        let Some(old) = legacy_path else {
            return Ok(None);
        };
        if !old.is_file() {
            return Ok(None);
        }
        let text = fs::read_to_string(old)
            .map_err(|e| format!("failed to read legacy settings {}: {e}", old.display()))?;
        let parsed = Self::from_text(&text);
        parsed.save_to_paths(paths)?;
        Ok(Some(old.to_path_buf()))
    }

    /// Serialize settings as stable `key=value` lines.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("# Forsaken World Explorer settings\n");
        if let Some(client_dir) = &self.client_dir {
            out.push_str(&format!("client_dir={}\n", client_dir.display()));
        }
        out.push_str(&format!(
            "resolution_scale_pct={}\n",
            self.resolution_scale_pct
                .clamp(MIN_RESOLUTION_SCALE_PCT, MAX_RESOLUTION_SCALE_PCT)
        ));
        out.push_str(&format!("terrain_ram_mib={}\n", self.terrain_ram_mib));
        out.push_str(&format!("tex_budget_mib={}\n", self.tex_budget_mib));
        out.push_str(&format!("music_volume={}\n", self.music_volume.min(100)));
        out.push_str(&format!(
            "ambience_volume={}\n",
            self.ambience_volume.min(100)
        ));
        out.push_str(&format!(
            "effects_volume={}\n",
            self.effects_volume.min(100)
        ));
        out.push_str(&format!("show_minimap={}\n", self.show_minimap));
        out.push_str(&format!("show_zone_notices={}\n", self.show_zone_notices));
        out
    }

    /// Parse settings text; malformed lines or out-of-range values fall back safely.
    pub fn from_text(text: &str) -> Self {
        let mut settings = Self::default();
        for raw_line in text.trim_start_matches('\u{feff}').lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            let Some((raw_key, raw_value)) = line.split_once('=') else {
                continue;
            };
            let key = raw_key.trim().to_ascii_lowercase();
            let value = raw_value.trim();
            match key.as_str() {
                "client_dir" if !value.is_empty() => {
                    settings.client_dir = Some(PathBuf::from(value));
                }
                "resolution_scale_pct" => {
                    if let Ok(parsed) = value.parse::<u16>() {
                        settings.resolution_scale_pct =
                            parsed.clamp(MIN_RESOLUTION_SCALE_PCT, MAX_RESOLUTION_SCALE_PCT);
                    }
                }
                "terrain_ram_mib" => {
                    if let Ok(parsed) = value.parse::<u32>() {
                        settings.terrain_ram_mib = parsed.max(1);
                    }
                }
                "tex_budget_mib" => {
                    if let Ok(parsed) = value.parse::<u32>() {
                        settings.tex_budget_mib = parsed.max(1);
                    }
                }
                "music_volume" => {
                    if let Ok(parsed) = value.parse::<u8>() {
                        settings.music_volume = parsed.min(100);
                    }
                }
                "ambience_volume" => {
                    if let Ok(parsed) = value.parse::<u8>() {
                        settings.ambience_volume = parsed.min(100);
                    }
                }
                "effects_volume" => {
                    if let Ok(parsed) = value.parse::<u8>() {
                        settings.effects_volume = parsed.min(100);
                    }
                }
                "show_minimap" => {
                    if let Some(parsed) = parse_bool(value) {
                        settings.show_minimap = parsed;
                    }
                }
                "show_zone_notices" => {
                    if let Some(parsed) = parse_bool(value) {
                        settings.show_zone_notices = parsed;
                    }
                }
                _ => {}
            }
        }
        settings
    }

    /// Apply runtime environment overrides (`FW_CLIENT_DIR`, `FW_TERRAIN_RAM_MIB`, `FW_TEX_BUDGET_MIB`).
    pub fn apply_env_overrides(&mut self) {
        if let Some(dir) = env::var_os(ENV_CLIENT_DIR).filter(|value| !value.is_empty()) {
            self.client_dir = Some(PathBuf::from(dir));
        }
        if let Ok(raw) = env::var(ENV_TERRAIN_RAM_MIB) {
            if let Ok(parsed) = raw.trim().parse::<u32>() {
                self.terrain_ram_mib = parsed.max(1);
            }
        }
        if let Ok(raw) = env::var(ENV_TEX_BUDGET_MIB) {
            if let Ok(parsed) = raw.trim().parse::<u32>() {
                self.tex_budget_mib = parsed.max(1);
            }
        }
    }

    /// Load settings from `<root>/settings/viewer-settings.txt`, falling back to defaults if absent.
    pub fn load_from_paths(paths: &PortablePaths) -> Result<Self, String> {
        let text = match fs::read_to_string(&paths.settings_file) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(format!("{}: {error}", paths.settings_file.display())),
        };
        Ok(Self::from_text(&text))
    }

    /// Persist settings to `<root>/settings/viewer-settings.txt`, creating `<root>/settings/` if needed.
    pub fn save_to_paths(&self, paths: &PortablePaths) -> Result<PathBuf, String> {
        fs::create_dir_all(&paths.settings_dir)
            .map_err(|error| format!("{}: {error}", paths.settings_dir.display()))?;
        fs::write(&paths.settings_file, self.to_text())
            .map_err(|error| format!("{}: {error}", paths.settings_file.display()))?;
        Ok(paths.settings_file.clone())
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Some(true),
        "false" | "0" | "no" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_test_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = env::temp_dir().join(format!(
            "fw-explorer-settings-{tag}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp test dir");
        dir
    }

    #[test]
    fn portable_root_resolution_and_settings_round_trip() {
        let root = temp_test_dir("portable-rt");
        let paths = PortablePaths::from_root(root.clone());
        assert_eq!(paths.settings_dir, root.join("settings"));
        assert_eq!(
            paths.settings_file,
            root.join("settings").join("viewer-settings.txt")
        );
        assert_eq!(paths.logs_dir, root.join("logs"));
        paths
            .check_writable()
            .expect("temp root should be writable");

        let original = ViewerSettings {
            client_dir: Some(PathBuf::from("/example/forsaken-world")),
            resolution_scale_pct: 125,
            terrain_ram_mib: 8,
            tex_budget_mib: 64,
            music_volume: 45,
            ambience_volume: 55,
            effects_volume: 90,
            show_minimap: false,
            show_zone_notices: true,
        };
        let saved_path = original.save_to_paths(&paths).expect("save settings");
        assert_eq!(saved_path, paths.settings_file);
        assert!(saved_path.is_file());

        let loaded = ViewerSettings::load_from_paths(&paths).expect("load settings");
        assert_eq!(loaded, original);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn one_time_legacy_import_preserves_old_file_and_does_not_overwrite_new() {
        let root = temp_test_dir("legacy-import");
        let legacy_dir = temp_test_dir("legacy-src");
        let legacy_file = legacy_dir.join(SETTINGS_FILE_NAME);
        fs::write(&legacy_file, "resolution_scale_pct=150\nmusic_volume=33\n").unwrap();

        let paths = PortablePaths::from_root(root.clone());
        let imported = ViewerSettings::import_legacy_once(&paths, Some(&legacy_file))
            .expect("import should succeed");
        assert_eq!(imported, Some(legacy_file.clone()));
        assert!(legacy_file.is_file(), "legacy file must be left in place");

        let loaded = ViewerSettings::load_from_paths(&paths).expect("load imported");
        assert_eq!(loaded.resolution_scale_pct, 150);
        assert_eq!(loaded.music_volume, 33);

        // Mutate legacy file; second import must be a no-op because portable file exists.
        fs::write(&legacy_file, "resolution_scale_pct=50\n").unwrap();
        let second = ViewerSettings::import_legacy_once(&paths, Some(&legacy_file))
            .expect("second import check");
        assert_eq!(second, None);
        assert_eq!(
            ViewerSettings::load_from_paths(&paths)
                .unwrap()
                .resolution_scale_pct,
            150
        );

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&legacy_dir);
    }

    #[test]
    fn unwritable_root_is_detected_cleanly() {
        let root = temp_test_dir("unwritable");
        // Place a regular file at `<root>/settings` so creating the `settings/` directory
        // fails reliably across both Windows and Unix (even if running as root).
        fs::write(root.join(SETTINGS_SUBDIR), b"blocking-file").unwrap();
        let paths = PortablePaths::from_root(root.clone());
        let err = paths
            .check_writable()
            .expect_err("blocked settings path must fail writability check");
        assert!(err.contains("not writable"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn malformed_and_out_of_range_values_fall_back_safely() {
        let raw = "\
# comment
not-a-kv-line
resolution_scale_pct=999
terrain_ram_mib=0
tex_budget_mib=abc
music_volume=250
show_minimap=maybe
show_zone_notices=false
unknown_key=123
";
        let parsed = ViewerSettings::from_text(raw);
        assert_eq!(parsed.resolution_scale_pct, MAX_RESOLUTION_SCALE_PCT);
        assert_eq!(parsed.terrain_ram_mib, 1);
        assert_eq!(parsed.tex_budget_mib, DEFAULT_TEX_BUDGET_MIB);
        assert_eq!(parsed.music_volume, 100);
        assert!(parsed.show_minimap);
        assert!(!parsed.show_zone_notices);
    }

    #[test]
    fn missing_settings_file_returns_defaults() {
        let root = temp_test_dir("missing");
        let paths = PortablePaths::from_root(root.clone());
        let loaded = ViewerSettings::load_from_paths(&paths).expect("load missing settings");
        assert_eq!(loaded, ViewerSettings::default());
        let _ = fs::remove_dir_all(&root);
    }
}
