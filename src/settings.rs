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

pub const MIN_SCENERY_RADIUS_M: u32 = 32;
pub const MAX_SCENERY_RADIUS_M: u32 = 2048;
pub const DEFAULT_SCENERY_RADIUS_M: u32 = 256;
pub const MIN_SCENERY_MAX_INSTANCES: u32 = 64;
pub const MAX_SCENERY_MAX_INSTANCES: u32 = 16_384;
pub const DEFAULT_SCENERY_MAX_INSTANCES: u32 = 1024;
pub const MIN_SCENERY_LOADS_PER_FRAME: u32 = 1;
pub const MAX_SCENERY_LOADS_PER_FRAME: u32 = 64;
pub const DEFAULT_SCENERY_LOADS_PER_FRAME: u32 = 4;

pub const MIN_FOG_DIST_MULT_PCT: u16 = 25;
pub const MAX_FOG_DIST_MULT_PCT: u16 = 400;
pub const DEFAULT_FOG_DIST_MULT_PCT: u16 = 100;
/// Default time-of-day: 12:00 (`MID_NOON` in `EC_SunMoon.h`) = 720 minutes.
pub const DEFAULT_TIME_OF_DAY_MINUTES: u16 = 720;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaterQuality {
    Low,
    Medium,
    High,
}

impl WaterQuality {
    /// Pass resolution percentage relative to the scaled 3D viewport.
    pub fn pass_scale_pct(self) -> u16 {
        match self {
            Self::Low => 25,
            Self::Medium => 50,
            Self::High => 100,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "low" => Some(Self::Low),
            "medium" | "med" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortablePaths {
    pub root: PathBuf,
    pub settings_dir: PathBuf,
    pub settings_file: PathBuf,
    pub logs_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl PortablePaths {
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

    pub fn from_executable() -> Self {
        let root = env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
            .unwrap_or_else(env::temp_dir);
        Self::from_root(root)
    }

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
    pub scenery_enabled: bool,
    pub transparent_scenery_enabled: bool,
    pub scenery_radius_m: u32,
    pub scenery_max_instances: u32,
    pub scenery_loads_per_frame: u32,
    pub scenery_whole_world: bool,
    pub fog_enabled: bool,
    pub fog_distance_mult_pct: u16,
    pub water_reflection: bool,
    pub water_refraction: bool,
    pub water_quality: WaterQuality,
    pub time_of_day_minutes: u16,
    pub follow_real_clock: bool,
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
            scenery_enabled: true,
            transparent_scenery_enabled: true,
            scenery_radius_m: DEFAULT_SCENERY_RADIUS_M,
            scenery_max_instances: DEFAULT_SCENERY_MAX_INSTANCES,
            scenery_loads_per_frame: DEFAULT_SCENERY_LOADS_PER_FRAME,
            scenery_whole_world: false,
            fog_enabled: true,
            fog_distance_mult_pct: DEFAULT_FOG_DIST_MULT_PCT,
            water_reflection: true,
            water_refraction: true,
            water_quality: WaterQuality::Medium,
            time_of_day_minutes: DEFAULT_TIME_OF_DAY_MINUTES,
            follow_real_clock: false,
            music_volume: 70,
            ambience_volume: 70,
            effects_volume: 80,
            show_minimap: true,
            show_zone_notices: true,
        }
    }
}

impl ViewerSettings {
    pub fn fog_distance_multiplier(&self) -> f32 {
        (self
            .fog_distance_mult_pct
            .clamp(MIN_FOG_DIST_MULT_PCT, MAX_FOG_DIST_MULT_PCT) as f32)
            / 100.0
    }

    pub fn time_of_day_hours(&self) -> f32 {
        if self.follow_real_clock {
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() % 86_400)
                .unwrap_or(43_200);
            (secs as f32) / 3600.0
        } else {
            ((self.time_of_day_minutes % 1440) as f32) / 60.0
        }
    }

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
        out.push_str(&format!("scenery_enabled={}\n", self.scenery_enabled));
        out.push_str(&format!(
            "transparent_scenery_enabled={}\n",
            self.transparent_scenery_enabled
        ));
        out.push_str(&format!(
            "scenery_radius_m={}\n",
            self.scenery_radius_m
                .clamp(MIN_SCENERY_RADIUS_M, MAX_SCENERY_RADIUS_M)
        ));
        out.push_str(&format!(
            "scenery_max_instances={}\n",
            self.scenery_max_instances
                .clamp(MIN_SCENERY_MAX_INSTANCES, MAX_SCENERY_MAX_INSTANCES)
        ));
        out.push_str(&format!(
            "scenery_loads_per_frame={}\n",
            self.scenery_loads_per_frame
                .clamp(MIN_SCENERY_LOADS_PER_FRAME, MAX_SCENERY_LOADS_PER_FRAME)
        ));
        out.push_str(&format!(
            "scenery_whole_world={}\n",
            self.scenery_whole_world
        ));
        out.push_str(&format!("fog_enabled={}\n", self.fog_enabled));
        out.push_str(&format!(
            "fog_distance_mult_pct={}\n",
            self.fog_distance_mult_pct
                .clamp(MIN_FOG_DIST_MULT_PCT, MAX_FOG_DIST_MULT_PCT)
        ));
        out.push_str(&format!("water_reflection={}\n", self.water_reflection));
        out.push_str(&format!("water_refraction={}\n", self.water_refraction));
        out.push_str(&format!("water_quality={}\n", self.water_quality.as_str()));
        out.push_str(&format!(
            "time_of_day_minutes={}\n",
            self.time_of_day_minutes % 1440
        ));
        out.push_str(&format!("follow_real_clock={}\n", self.follow_real_clock));
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
                "scenery_enabled" => {
                    if let Some(parsed) = parse_bool(value) {
                        settings.scenery_enabled = parsed;
                    }
                }
                "transparent_scenery_enabled" => {
                    if let Some(parsed) = parse_bool(value) {
                        settings.transparent_scenery_enabled = parsed;
                    }
                }
                "scenery_radius_m" => {
                    if let Ok(parsed) = value.parse::<u32>() {
                        settings.scenery_radius_m =
                            parsed.clamp(MIN_SCENERY_RADIUS_M, MAX_SCENERY_RADIUS_M);
                    }
                }
                "scenery_max_instances" => {
                    if let Ok(parsed) = value.parse::<u32>() {
                        settings.scenery_max_instances =
                            parsed.clamp(MIN_SCENERY_MAX_INSTANCES, MAX_SCENERY_MAX_INSTANCES);
                    }
                }
                "scenery_loads_per_frame" => {
                    if let Ok(parsed) = value.parse::<u32>() {
                        settings.scenery_loads_per_frame =
                            parsed.clamp(MIN_SCENERY_LOADS_PER_FRAME, MAX_SCENERY_LOADS_PER_FRAME);
                    }
                }
                "scenery_whole_world" => {
                    if let Some(parsed) = parse_bool(value) {
                        settings.scenery_whole_world = parsed;
                    }
                }
                "fog_enabled" => {
                    if let Some(parsed) = parse_bool(value) {
                        settings.fog_enabled = parsed;
                    }
                }
                "fog_distance_mult_pct" => {
                    if let Ok(parsed) = value.parse::<u16>() {
                        settings.fog_distance_mult_pct =
                            parsed.clamp(MIN_FOG_DIST_MULT_PCT, MAX_FOG_DIST_MULT_PCT);
                    }
                }
                "water_reflection" => {
                    if let Some(parsed) = parse_bool(value) {
                        settings.water_reflection = parsed;
                    }
                }
                "water_refraction" => {
                    if let Some(parsed) = parse_bool(value) {
                        settings.water_refraction = parsed;
                    }
                }
                "water_quality" => {
                    if let Some(parsed) = WaterQuality::parse(value) {
                        settings.water_quality = parsed;
                    }
                }
                "time_of_day_minutes" => {
                    if let Ok(parsed) = value.parse::<u16>() {
                        settings.time_of_day_minutes = parsed % 1440;
                    }
                }
                "follow_real_clock" => {
                    if let Some(parsed) = parse_bool(value) {
                        settings.follow_real_clock = parsed;
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

    pub fn load_from_paths(paths: &PortablePaths) -> Result<Self, String> {
        let text = match fs::read_to_string(&paths.settings_file) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(format!("{}: {error}", paths.settings_file.display())),
        };
        Ok(Self::from_text(&text))
    }

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
        paths
            .check_writable()
            .expect("temp root should be writable");

        let original = ViewerSettings {
            client_dir: Some(PathBuf::from("/example/forsaken-world")),
            resolution_scale_pct: 125,
            terrain_ram_mib: 8,
            tex_budget_mib: 64,
            scenery_enabled: true,
            transparent_scenery_enabled: false,
            scenery_radius_m: 384,
            scenery_max_instances: 2048,
            scenery_loads_per_frame: 6,
            scenery_whole_world: true,
            fog_enabled: false,
            fog_distance_mult_pct: 175,
            water_reflection: false,
            water_refraction: true,
            water_quality: WaterQuality::High,
            time_of_day_minutes: 1110,
            follow_real_clock: true,
            music_volume: 45,
            ambience_volume: 55,
            effects_volume: 90,
            show_minimap: false,
            show_zone_notices: true,
        };
        let saved_path = original.save_to_paths(&paths).expect("save settings");
        assert_eq!(saved_path, paths.settings_file);

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
        assert!(legacy_file.is_file());

        let loaded = ViewerSettings::load_from_paths(&paths).expect("load imported");
        assert_eq!(loaded.resolution_scale_pct, 150);
        assert_eq!(loaded.music_volume, 33);

        fs::write(&legacy_file, "resolution_scale_pct=50\n").unwrap();
        let second = ViewerSettings::import_legacy_once(&paths, Some(&legacy_file))
            .expect("second import check");
        assert_eq!(second, None);

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&legacy_dir);
    }

    #[test]
    fn unwritable_root_is_detected_cleanly() {
        let root = temp_test_dir("unwritable");
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
resolution_scale_pct=999
fog_distance_mult_pct=9999
water_quality=ultra
time_of_day_minutes=1500
scenery_radius_m=99999
scenery_max_instances=1
scenery_loads_per_frame=0
";
        let parsed = ViewerSettings::from_text(raw);
        assert_eq!(parsed.resolution_scale_pct, MAX_RESOLUTION_SCALE_PCT);
        assert_eq!(parsed.fog_distance_mult_pct, MAX_FOG_DIST_MULT_PCT);
        assert_eq!(parsed.water_quality, WaterQuality::Medium);
        assert_eq!(parsed.time_of_day_minutes, 60);
        assert_eq!(parsed.scenery_radius_m, MAX_SCENERY_RADIUS_M);
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
