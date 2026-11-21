//! Persisted viewer settings stored under `%LOCALAPPDATA%\ForsakenWorldExplorer`
//! (or `FW_CACHE_DIR` when overridden). Malformed values fall back safely to defaults.

use std::{
    env, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

pub const APP_DIR_NAME: &str = "ForsakenWorldExplorer";
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
    /// Resolve the root directory for persisted settings, cache, and logs.
    ///
    /// Never writes into the user's Forsaken World client directory.
    pub fn default_root_dir() -> PathBuf {
        if let Some(dir) = env::var_os(ENV_CACHE_DIR).filter(|value| !value.is_empty()) {
            return PathBuf::from(dir);
        }
        #[cfg(windows)]
        {
            if let Some(local_app_data) =
                env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty())
            {
                return PathBuf::from(local_app_data).join(APP_DIR_NAME);
            }
        }
        if let Some(xdg) = env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
            return PathBuf::from(xdg).join(APP_DIR_NAME);
        }
        if let Some(home) = env::var_os("HOME").filter(|value| !value.is_empty()) {
            return PathBuf::from(home).join(".config").join(APP_DIR_NAME);
        }
        env::temp_dir().join(APP_DIR_NAME)
    }

    pub fn settings_path_in(root: &Path) -> PathBuf {
        root.join(SETTINGS_FILE_NAME)
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

    /// Load settings from `root/viewer-settings.txt`, falling back to defaults if absent.
    pub fn load_from_dir(root: &Path) -> Result<Self, String> {
        let path = Self::settings_path_in(root);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        Ok(Self::from_text(&text))
    }

    /// Persist settings to `root/viewer-settings.txt`, creating `root` if needed.
    pub fn save_to_dir(&self, root: &Path) -> Result<PathBuf, String> {
        fs::create_dir_all(root).map_err(|error| format!("{}: {error}", root.display()))?;
        let path = Self::settings_path_in(root);
        fs::write(&path, self.to_text()).map_err(|error| format!("{}: {error}", path.display()))?;
        Ok(path)
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
    use std::time::{SystemTime, UNIX_EPOCH};

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
    fn round_trip_save_and_load() {
        let root = temp_test_dir("roundtrip");
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
        let saved_path = original.save_to_dir(&root).expect("save settings");
        assert!(saved_path.is_file());
        let loaded = ViewerSettings::load_from_dir(&root).expect("load settings");
        assert_eq!(loaded, original);
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
        let loaded = ViewerSettings::load_from_dir(&root).expect("load missing settings");
        assert_eq!(loaded, ViewerSettings::default());
        let _ = fs::remove_dir_all(&root);
    }
}
