//! Per-run file logging under `<root>/logs/` with file-count and byte-size caps.

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

pub const LOG_PREFIX: &str = "fw-explorer-";
pub const LOG_EXT: &str = ".log";
pub const MAX_LOG_FILES: usize = 10;
pub const MAX_LOG_BYTES: usize = 2 * 1024 * 1024;

pub struct RunLogger {
    logs_dir: PathBuf,
    log_path: PathBuf,
    file: File,
    bytes_written: usize,
    capped: bool,
}

impl RunLogger {
    /// Create `<logs_dir>` if needed, prune old log files beyond `MAX_LOG_FILES - 1`,
    /// and open a fresh per-run log file.
    pub fn open(logs_dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(logs_dir)
            .map_err(|e| format!("cannot create logs directory {}: {e}", logs_dir.display()))?;
        prune_old_logs(logs_dir, MAX_LOG_FILES.saturating_sub(1));

        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let file_name = format!("{LOG_PREFIX}{millis}-{}{LOG_EXT}", std::process::id());
        let log_path = logs_dir.join(file_name);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|e| format!("cannot open log file {}: {e}", log_path.display()))?;

        let mut logger = Self {
            logs_dir: logs_dir.to_path_buf(),
            log_path,
            file,
            bytes_written: 0,
            capped: false,
        };
        logger.write_line("INFO", "Forsaken World Explorer log initialized");
        Ok(logger)
    }

    pub fn logs_dir(&self) -> &Path {
        &self.logs_dir
    }

    pub fn log_path(&self) -> &Path {
        &self.log_path
    }

    pub fn info(&mut self, msg: &str) {
        self.write_line("INFO", msg);
    }

    pub fn warn(&mut self, msg: &str) {
        self.write_line("WARN", msg);
    }

    pub fn error(&mut self, msg: &str) {
        self.write_line("ERROR", msg);
    }

    fn write_line(&mut self, level: &str, msg: &str) {
        if self.capped {
            return;
        }
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let line = format!("[{secs}] [{level}] {msg}\n");
        if self.bytes_written.saturating_add(line.len()) > MAX_LOG_BYTES {
            let notice = format!(
                "[{secs}] [WARN] Log file reached {MAX_LOG_BYTES} byte limit; further entries suppressed.\n"
            );
            let _ = self.file.write_all(notice.as_bytes());
            let _ = self.file.flush();
            self.capped = true;
            return;
        }
        if self.file.write_all(line.as_bytes()).is_ok() {
            let _ = self.file.flush();
            self.bytes_written += line.len();
        }
    }
}

fn prune_old_logs(logs_dir: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(logs_dir) else {
        return;
    };
    let mut files: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if !path.is_file() {
                return None;
            }
            let name = path.file_name()?.to_str()?.to_string();
            if name.starts_with(LOG_PREFIX) && name.ends_with(LOG_EXT) {
                Some((name, path))
            } else {
                None
            }
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    if files.len() > keep {
        let to_remove = files.len() - keep;
        for (_, path) in files.into_iter().take(to_remove) {
            let _ = fs::remove_file(path);
        }
    }
}

/// Open a folder in the OS file manager. Must only be invoked by an explicit UI action.
pub fn open_folder_in_file_manager(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir)
        .map_err(|e| format!("cannot create directory {}: {e}", dir.display()))?;
    #[cfg(windows)]
    let status = Command::new("explorer.exe").arg(dir).spawn();
    #[cfg(target_os = "macos")]
    let status = Command::new("open").arg(dir).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let status = Command::new("xdg-open").arg(dir).spawn();

    status
        .map(|_| ())
        .map_err(|e| format!("failed to open {}: {e}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn temp_logs_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = env::temp_dir().join(format!(
            "fw-explorer-logs-{tag}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp logs dir");
        dir
    }

    #[test]
    fn creates_per_run_log_file_and_prunes_oldest_beyond_limit() {
        let dir = temp_logs_dir("prune");
        for i in 0..14 {
            let name = format!("{LOG_PREFIX}00000000000{i:02}-1{LOG_EXT}");
            fs::write(dir.join(name), b"old log\n").unwrap();
        }
        let mut logger = RunLogger::open(&dir).expect("open run logger");
        logger.info("hello from test");
        assert!(logger.log_path().is_file());
        let contents = fs::read_to_string(logger.log_path()).unwrap();
        assert!(contents.contains("hello from test"));

        let count = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().is_file())
            .count();
        assert!(
            count <= MAX_LOG_FILES,
            "expected <= {MAX_LOG_FILES} log files, found {count}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
