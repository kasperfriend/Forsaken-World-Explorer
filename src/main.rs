use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde::Serialize;
use std::{collections::BTreeMap, fs, path::{Path, PathBuf}};
use walkdir::WalkDir;

#[derive(Parser)]
#[command(name="fw-explorer", version, about="Inspect Forsaken World client installs and inventory world assets")]
struct Args { #[command(subcommand)] command: Command }
#[derive(Subcommand)]
enum Command {
    /// Validate an install directory, discover its world assets, and summarize supported content.
    Inspect { /// Path to the game client root (the directory containing elementclient.exe or data/maps).
        path: PathBuf, /// Emit the full report as JSON.
        #[arg(long)] json: bool },
}
#[derive(Debug, Serialize)]
struct Report {
    client_root: PathBuf,
    client_executable: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    maps_dir: Option<PathBuf>,
    map_files: Vec<Asset>,
    counts_by_extension: BTreeMap<String, usize>,
    archives: Vec<Asset>,
    warnings: Vec<String>,
}
#[derive(Debug, Serialize)]
struct Asset { path: PathBuf, bytes: u64, extension: String }

fn main() -> Result<()> {
    match Args::parse().command { Command::Inspect { path, json } => {
        let report = inspect(&path)?;
        if json { println!("{}", serde_json::to_string_pretty(&report)?); }
        else { print_report(&report); }
    }}
    Ok(())
}

fn inspect(root: &Path) -> Result<Report> {
    let root = root.canonicalize().with_context(|| format!("Cannot open client path {}", root.display()))?;
    if !root.is_dir() { bail!("Client path is not a directory: {}", root.display()); }
    // Existing installations include both a top-level `maps` directory and `data/maps`;
    // prefer data/maps, but recognize either and never assume a particular install version.
    let data_dir = child_dir_case_insensitive(&root, "data");
    let maps_dir = data_dir.as_deref().and_then(|d| child_dir_case_insensitive(d, "maps"))
        .or_else(|| child_dir_case_insensitive(&root, "maps"));
    let client_executable = find_named(&root, "elementclient.exe", 3);
    let mut warnings = Vec::new();
    if data_dir.is_none() { warnings.push("No data/ directory found; this may be a partial install or an archive-extracted asset folder.".into()); }
    if maps_dir.is_none() { warnings.push("No maps/ directory found. The world viewer needs the client's unpacked map assets.".into()); }
    if client_executable.is_none() { warnings.push("elementclient.exe was not found within three directory levels (expected on Windows installs).".into()); }
    let mut counts = BTreeMap::new();
    let mut map_files = Vec::new();
    let mut archives = Vec::new();
    for entry in WalkDir::new(&root).follow_links(false).max_depth(12).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() { continue; }
        let path = entry.path();
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
        *counts.entry(if ext.is_empty() { "[none]".into() } else { ext.clone() }).or_insert(0) += 1;
        let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if is_archive_ext(&ext) { archives.push(Asset {path:path.to_path_buf(), bytes, extension:ext.clone()}); }
        if maps_dir.as_ref().is_some_and(|m| path.starts_with(m)) && is_world_ext(&ext) {
            map_files.push(Asset {path:path.to_path_buf(), bytes, extension:ext});
        }
    }
    map_files.sort_by(|a,b| a.path.cmp(&b.path)); archives.sort_by(|a,b| a.path.cmp(&b.path));
    Ok(Report {client_root:root, client_executable, data_dir, maps_dir, map_files, counts_by_extension:counts, archives, warnings})
}
fn child_dir_case_insensitive(parent:&Path, name:&str)->Option<PathBuf>{
    fs::read_dir(parent).ok()?.filter_map(Result::ok).find(|e| e.file_type().map(|t|t.is_dir()).unwrap_or(false) && e.file_name().to_string_lossy().eq_ignore_ascii_case(name)).map(|e|e.path())
}
fn find_named(root:&Path,name:&str,max_depth:usize)->Option<PathBuf>{
    WalkDir::new(root).follow_links(false).max_depth(max_depth).into_iter().filter_map(Result::ok).find(|e|e.file_type().is_file() && e.file_name().to_string_lossy().eq_ignore_ascii_case(name)).map(|e|e.path().to_path_buf())
}
fn is_archive_ext(ext:&str)->bool { matches!(ext,"pck"|"pak"|"zip"|"7z"|"rar") }
fn is_world_ext(ext:&str)->bool { matches!(ext,"map"|"smd"|"smo"|"sma"|"spt"|"dds"|"tga"|"bmp"|"jpg"|"png"|"gfx"|"ecm") }
fn print_report(r:&Report){
    println!("Forsaken World client inventory\nRoot: {}",r.client_root.display());
    println!("Client executable: {}",r.client_executable.as_deref().map(|p|p.display().to_string()).unwrap_or_else(||"not found".into()));
    println!("Data directory: {}",r.data_dir.as_deref().map(|p|p.display().to_string()).unwrap_or_else(||"not found".into()));
    println!("World maps directory: {}",r.maps_dir.as_deref().map(|p|p.display().to_string()).unwrap_or_else(||"not found".into()));
    println!("World assets: {} | archives: {}",r.map_files.len(),r.archives.len());
    for (ext,n) in &r.counts_by_extension {println!("  .{ext:<8} {n}");}
    if !r.map_files.is_empty(){println!("\nMap content (first 40 files):"); for f in r.map_files.iter().take(40){println!("  {} ({} bytes)",f.path.display(),f.bytes);}}
    for w in &r.warnings {eprintln!("Warning: {w}");}
}

#[cfg(test)] mod tests {
 use super::*;
 #[test] fn finds_case_insensitive_install_dirs_and_world_assets(){
  let t=tempfile::tempdir().unwrap(); let root=t.path();
  fs::create_dir_all(root.join("DATA/Maps/world")).unwrap(); fs::write(root.join("DATA/Maps/world/000.map"),b"map").unwrap();
  fs::write(root.join("DATA/world.pck"),b"archive").unwrap(); fs::write(root.join("elementclient.exe"),b"exe").unwrap();
  let report=inspect(root).unwrap(); assert!(report.maps_dir.is_some()); assert!(report.client_executable.is_some()); assert_eq!(report.map_files.len(),1); assert_eq!(report.archives.len(),1);
 }
 #[test] fn rejects_missing_path(){ assert!(inspect(Path::new("/definitely/not/a/game" )).is_err()); }
}
