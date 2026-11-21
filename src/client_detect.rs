//! Client folder layout detection and executable fingerprint reporting for Forsaken World.
//!
//! Never modifies the client directory and never blocks on an unrecognized executable hash.

use std::{
    ffi::OsStr,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

/// Recognized `update/pem.exe` SHA-256 hashes (populated once confirmed locally).
///
/// The Drive client (`Forsaken World 0.525 Client`) ships `update/pem.exe`
/// (10,703,784 bytes), whose SHA-256 is marked `pending` in `Findings/client-inventory.md`.
pub const RECOGNIZED_EXECUTABLES: &[(&str, &str)] = &[];

/// Verified 32-bit `update/speedtreert.dll` SHA-256 from `Forsaken World 0.525 Client`.
pub const KNOWN_SPEEDTREE_SHA256: &str =
    "1a62ff4dc25eeb719ef885fcd77b35c35e81b3be32994c9852de31a5955eaf83";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recognition {
    Recognized { label: String },
    Unrecognized { reason: String },
}

impl Recognition {
    pub fn is_recognized(&self) -> bool {
        matches!(self, Self::Recognized { .. })
    }

    pub fn summary(&self) -> String {
        match self {
            Self::Recognized { label } => format!("Recognized ({label})"),
            Self::Unrecognized { reason } => format!("Unrecognized ({reason})"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DetectedClient {
    pub root: PathBuf,
    pub update_dir: PathBuf,
    pub resources_dir: PathBuf,
    pub maps_dir: PathBuf,
    pub executable: PathBuf,
    pub executable_size: u64,
    pub executable_sha256: String,
    pub recognition: Recognition,
    pub has_speedtree_dll: bool,
    pub pck_archives: Vec<String>,
    pub zones: Vec<String>,
}

fn find_child_ci(dir: &Path, target: &str) -> Option<PathBuf> {
    let exact = dir.join(target);
    if exact.exists() {
        return Some(exact);
    }
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(target)
        })
        .map(|entry| entry.path())
}

fn sha256_file(path: &Path) -> Result<(u64, String), String> {
    let mut file = File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    Ok((total, hex))
}

fn is_readable_zone_dir(dir: &Path) -> bool {
    let Some(id) = dir.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    let stem = if find_child_ci(dir, &format!("{id}.ecbsd")).is_some() {
        id
    } else {
        "world"
    };
    ["trn2", "ecwld", "ecbsd"]
        .iter()
        .all(|ext| find_child_ci(dir, &format!("{stem}.{ext}")).is_some())
}

fn resolve_client_root(selected: &Path) -> Option<PathBuf> {
    let candidates = [
        Some(selected.to_path_buf()),
        selected.parent().map(Path::to_path_buf),
    ];
    for candidate in candidates.into_iter().flatten() {
        let has_update = find_child_ci(&candidate, "update")
            .map(|p| p.is_dir())
            .unwrap_or(false);
        let has_resources = find_child_ci(&candidate, "resources")
            .map(|p| p.is_dir())
            .unwrap_or(false);
        if has_update && has_resources {
            return Some(candidate);
        }
    }
    None
}

/// Detect and validate a Forsaken World client folder without modifying it.
pub fn detect(selected: &Path) -> Result<DetectedClient, String> {
    let root = resolve_client_root(selected).ok_or_else(|| {
        format!(
            "{} is not a Forsaken World client folder (expected update/ and resources/ directories).",
            selected.display()
        )
    })?;

    let update_dir = find_child_ci(&root, "update")
        .filter(|p| p.is_dir())
        .ok_or_else(|| "Client folder is missing update/ directory.".to_string())?;
    let resources_dir = find_child_ci(&root, "resources")
        .filter(|p| p.is_dir())
        .ok_or_else(|| "Client folder is missing resources/ directory.".to_string())?;
    let maps_dir = find_child_ci(&root, "maps")
        .filter(|p| p.is_dir())
        .ok_or_else(|| "Client folder is missing maps/ directory.".to_string())?;

    let executable = find_child_ci(&update_dir, "pem.exe")
        .filter(|p| p.is_file())
        .ok_or_else(|| "Client folder is missing update/pem.exe.".to_string())?;

    let (executable_size, executable_sha256) = sha256_file(&executable)?;
    let recognition = RECOGNIZED_EXECUTABLES
        .iter()
        .find(|(_, hash)| hash.eq_ignore_ascii_case(&executable_sha256))
        .map(|(label, _)| Recognition::Recognized {
            label: (*label).to_string(),
        })
        .unwrap_or_else(|| {
            let reason = if RECOGNIZED_EXECUTABLES.is_empty() {
                "recognized pem.exe SHA-256 pending local confirmation".to_string()
            } else {
                "executable hash does not match a known build".to_string()
            };
            Recognition::Unrecognized { reason }
        });

    let has_speedtree_dll = find_child_ci(&update_dir, "speedtreert.dll")
        .map(|p| p.is_file())
        .unwrap_or(false);

    let mut pck_archives = Vec::new();
    if let Ok(entries) = fs::read_dir(&resources_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .and_then(OsStr::to_str)
                    .map(|ext| ext.eq_ignore_ascii_case("pck"))
                    .unwrap_or(false)
            {
                if let Some(name) = path.file_name().and_then(OsStr::to_str) {
                    pck_archives.push(name.to_string());
                }
            }
        }
    }
    pck_archives.sort();
    if pck_archives.is_empty() {
        return Err("Client resources/ directory contains no .pck archives.".into());
    }

    let mut zones = Vec::new();
    if let Ok(entries) = fs::read_dir(&maps_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && is_readable_zone_dir(&path) {
                if let Some(name) = path.file_name().and_then(OsStr::to_str) {
                    zones.push(name.to_string());
                }
            }
        }
    }
    zones.sort();

    Ok(DetectedClient {
        root,
        update_dir,
        resources_dir,
        maps_dir,
        executable,
        executable_size,
        executable_sha256,
        recognition,
        has_speedtree_dll,
        pck_archives,
        zones,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_client_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = env::temp_dir().join(format!(
            "fw-explorer-client-{tag}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp client dir");
        dir
    }

    fn populate_synthetic_client(root: &Path) {
        fs::create_dir_all(root.join("update")).unwrap();
        fs::create_dir_all(root.join("resources")).unwrap();
        fs::create_dir_all(root.join("maps/s0")).unwrap();
        fs::write(root.join("update/pem.exe"), b"MZsynthetic-pem").unwrap();
        fs::write(root.join("update/speedtreert.dll"), b"MZsynthetic-st").unwrap();
        fs::write(root.join("resources/configs.pck"), b"stub").unwrap();
        fs::write(root.join("maps/s0/s0.trn2"), b"stub").unwrap();
        fs::write(root.join("maps/s0/s0.ecwld"), b"stub").unwrap();
        fs::write(root.join("maps/s0/s0.ecbsd"), b"stub").unwrap();
    }

    #[test]
    fn detects_client_root_and_update_subfolder_without_blocking() {
        let root = temp_client_dir("valid");
        populate_synthetic_client(&root);

        let from_root = detect(&root).expect("detect from root");
        assert_eq!(from_root.root, root);
        assert!(from_root.has_speedtree_dll);
        assert_eq!(from_root.pck_archives, vec!["configs.pck"]);
        assert_eq!(from_root.zones, vec!["s0"]);
        assert_eq!(from_root.executable_sha256.len(), 64);
        assert!(!from_root.recognition.is_recognized());

        let from_update = detect(&root.join("update")).expect("detect from update/");
        assert_eq!(from_update.root, root);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_incomplete_client_directory() {
        let root = temp_client_dir("incomplete");
        fs::create_dir_all(root.join("update")).unwrap();
        assert!(detect(&root).is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
