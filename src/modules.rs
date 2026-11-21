//! Discovery of installed, executable-adjacent module packages.
//!
//! Mirrors the reference project's `Modules/<Name>/module.ini` schema.
//! A broken or malformed manifest is recorded in `Registry::errors` and never
//! prevents the core viewer from starting.

use std::{
    collections::{HashMap, HashSet},
    env, fs,
    io::ErrorKind,
    path::PathBuf,
};

pub const MODULES_DIRECTORY: &str = "Modules";
pub const MODULE_MANIFEST: &str = "module.ini";
pub const ENV_MODULES_DIR: &str = "FW_MODULES_DIR";
const MAX_MANIFEST_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub directory: PathBuf,
}

impl Package {
    pub fn load(directory: PathBuf) -> Result<Self, String> {
        let manifest = directory.join(MODULE_MANIFEST);
        let metadata =
            fs::metadata(&manifest).map_err(|error| format!("{}: {error}", manifest.display()))?;
        if metadata.len() > MAX_MANIFEST_BYTES as u64 {
            return Err(format!(
                "{} exceeds the {} byte manifest limit",
                manifest.display(),
                MAX_MANIFEST_BYTES
            ));
        }
        let text = fs::read_to_string(&manifest)
            .map_err(|error| format!("{}: {error}", manifest.display()))?;
        let mut fields = HashMap::new();
        for (index, raw_line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("manifest line {} is not key=value", index + 1))?;
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim();
            if key.is_empty() || value.is_empty() {
                return Err(format!(
                    "manifest line {} has an empty key or value",
                    index + 1
                ));
            }
            if fields.insert(key.clone(), value.to_owned()).is_some() {
                return Err(format!("manifest key '{key}' is repeated"));
            }
        }
        let mut required = |key: &str| {
            fields
                .remove(key)
                .ok_or_else(|| format!("manifest key '{key}' is required"))
        };
        let format = required("format")?;
        if format != "1" {
            return Err(format!("unsupported module manifest format '{format}'"));
        }
        let id = required("id")?.to_ascii_lowercase();
        if id.is_empty()
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(
                "module id must contain only lowercase ASCII letters, digits or '-'".into(),
            );
        }
        Ok(Self {
            id,
            name: required("name")?,
            version: required("version")?,
            description: required("description")?,
            directory,
        })
    }
}

#[derive(Debug)]
pub struct Registry {
    pub root: PathBuf,
    pub packages: Vec<Package>,
    pub errors: Vec<String>,
}

impl Registry {
    /// Resolve the `Modules` directory: `FW_MODULES_DIR` env override, else
    /// beside the current executable.
    pub fn default_root() -> PathBuf {
        if let Some(dir) = env::var_os(ENV_MODULES_DIR).filter(|value| !value.is_empty()) {
            return PathBuf::from(dir);
        }
        if let Ok(exe) = env::current_exe() {
            if let Some(parent) = exe.parent() {
                return parent.join(MODULES_DIRECTORY);
            }
        }
        PathBuf::from(MODULES_DIRECTORY)
    }

    pub fn discover(root: PathBuf) -> Self {
        let mut registry = Self {
            root: root.clone(),
            packages: Vec::new(),
            errors: Vec::new(),
        };
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return registry,
            Err(error) => {
                registry.errors.push(format!("{}: {error}", root.display()));
                return registry;
            }
        };
        let mut directories = Vec::new();
        for entry in entries {
            match entry {
                Ok(entry) => match entry.file_type() {
                    Ok(kind) if kind.is_dir() => directories.push(entry.path()),
                    Ok(_) => {}
                    Err(error) => registry
                        .errors
                        .push(format!("{}: {error}", entry.path().display())),
                },
                Err(error) => registry.errors.push(error.to_string()),
            }
        }
        directories.sort();
        let mut seen_ids = HashSet::new();
        for dir in directories {
            match Package::load(dir.clone()) {
                Ok(package) => {
                    if seen_ids.insert(package.id.clone()) {
                        registry.packages.push(package);
                    } else {
                        registry.errors.push(format!(
                            "{}: duplicate module id '{}'",
                            dir.display(),
                            package.id
                        ));
                    }
                }
                Err(error) => registry.errors.push(error),
            }
        }
        registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_modules_root(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = env::temp_dir().join(format!(
            "fw-explorer-modules-{tag}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp modules dir");
        dir
    }

    #[test]
    fn discovers_valid_and_isolates_broken_manifests() {
        let root = temp_modules_root("mixed");
        fs::create_dir_all(root.join("Walker")).unwrap();
        fs::write(
            root.join("Walker/module.ini"),
            "format=1\nid=walker\nname=Walker\nversion=0.1.0\ndescription=Test walker\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("Broken")).unwrap();
        fs::write(root.join("Broken/module.ini"), "format=99\nid=broken\n").unwrap();

        let registry = Registry::discover(root.clone());
        assert_eq!(registry.packages.len(), 1);
        assert_eq!(registry.packages[0].id, "walker");
        assert_eq!(registry.errors.len(), 1);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_duplicate_module_ids() {
        let root = temp_modules_root("dup");
        for folder in ["A", "B"] {
            fs::create_dir_all(root.join(folder)).unwrap();
            fs::write(
                root.join(folder).join("module.ini"),
                "format=1\nid=same\nname=Same\nversion=1.0.0\ndescription=Dup\n",
            )
            .unwrap();
        }
        let registry = Registry::discover(root.clone());
        assert_eq!(registry.packages.len(), 1);
        assert_eq!(registry.errors.len(), 1);
        assert!(registry.errors[0].contains("duplicate module id"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_directory_returns_empty_without_errors() {
        let root = temp_modules_root("absent").join("nonexistent");
        let registry = Registry::discover(root);
        assert!(registry.packages.is_empty());
        assert!(registry.errors.is_empty());
    }
}
