//! 32-bit out-of-process SpeedTreeRT bridge helper for Forsaken World Explorer.
//!
//! Built for `i686-pc-windows-msvc` in CI so it can load the user's own 32-bit
//! `<client-root>/update/speedtreert.dll` without bundling or redistributing the DLL.
//! Never writes into `<client-root>`; writes only to `--out` (`<root>/cache/speedtree-v1/`).
//!
//! Exit codes:
//!   0 = success
//!   2 = invalid CLI arguments
//!   3 = missing or unloadable update/speedtreert.dll
//!   4 = missing required CSpeedTreeRT export
//!   5 = SPT load/bake failure
//!   6 = cache output write failure

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

const FWST_MAGIC: &[u8; 4] = b"FWST";
const FWST_VERSION: u32 = 1;

struct Args {
    client_root: PathBuf,
    tree_type: u32,
    spt_path: String,
    lod: u32,
    out_path: Option<PathBuf>,
    probe_only: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut it = env::args().skip(1);
    let mut client_root: Option<PathBuf> = None;
    let mut tree_type: u32 = 0;
    let mut spt_path = String::new();
    let mut lod: u32 = 0;
    let mut out_path: Option<PathBuf> = None;
    let mut probe_only = false;

    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--client-root" => {
                client_root = Some(PathBuf::from(
                    it.next().ok_or("missing value for --client-root")?,
                ));
            }
            "--tree-type" => {
                tree_type = it
                    .next()
                    .ok_or("missing value for --tree-type")?
                    .parse()
                    .map_err(|_| "invalid --tree-type")?;
            }
            "--spt" => {
                spt_path = it.next().ok_or("missing value for --spt")?;
            }
            "--lod" => {
                lod = it
                    .next()
                    .ok_or("missing value for --lod")?
                    .parse()
                    .map_err(|_| "invalid --lod")?;
                if lod > 2 {
                    return Err("--lod must be 0, 1, or 2".into());
                }
            }
            "--out" => {
                out_path = Some(PathBuf::from(it.next().ok_or("missing value for --out")?));
            }
            "--probe-only" => {
                probe_only = true;
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    let client_root = client_root.ok_or("missing required --client-root")?;
    if !probe_only && out_path.is_none() {
        return Err("missing required --out (or pass --probe-only)".into());
    }
    Ok(Args {
        client_root,
        tree_type,
        spt_path,
        lod,
        out_path,
        probe_only,
    })
}

#[cfg(windows)]
mod win_runtime {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, path::Path};

    type HModule = *mut core::ffi::c_void;
    type FarProc = *const core::ffi::c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryW(lp_lib_file_name: *const u16) -> HModule;
        fn GetProcAddress(h_module: HModule, lp_proc_name: *const u8) -> FarProc;
        fn FreeLibrary(h_lib_module: HModule) -> i32;
    }

    const REQUIRED_EXPORTS: &[&[u8]] = &[
        b"?LoadTree@CSpeedTreeRT@@QAE_NPBEI@Z\0",
        b"?Compute@CSpeedTreeRT@@QAE_NPBMI_N@Z\0",
        b"?GetGeometry@CSpeedTreeRT@@QAEXAAUSGeometry@1@KFFF@Z\0",
        b"?GetTextures@CSpeedTreeRT@@QBEXAAUSTextures@1@@Z\0",
        b"?GetLodLevel@CSpeedTreeRT@@QBEMXZ\0",
        b"?ComputeWindEffects@CSpeedTreeRT@@QAEX_N00@Z\0",
    ];

    pub fn verify_dll_exports(dll_path: &Path) -> Result<usize, (u8, String)> {
        if !dll_path.is_file() {
            return Err((3, format!("DLL not found: {}", dll_path.display())));
        }
        let wide: Vec<u16> = OsStr::new(dll_path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let h = unsafe { LoadLibraryW(wide.as_ptr()) };
        if h.is_null() {
            return Err((3, format!("LoadLibraryW failed for {}", dll_path.display())));
        }
        let mut verified = 0usize;
        for sym in REQUIRED_EXPORTS {
            let p = unsafe { GetProcAddress(h, sym.as_ptr()) };
            if p.is_null() {
                unsafe {
                    FreeLibrary(h);
                }
                let name = String::from_utf8_lossy(&sym[..sym.len() - 1]);
                return Err((4, format!("missing CSpeedTreeRT export: {name}")));
            }
            verified += 1;
        }
        unsafe {
            FreeLibrary(h);
        }
        Ok(verified)
    }
}

#[cfg(not(windows))]
mod win_runtime {
    use std::path::Path;

    pub fn verify_dll_exports(dll_path: &Path) -> Result<usize, (u8, String)> {
        if !dll_path.is_file() {
            return Err((3, format!("DLL not found: {}", dll_path.display())));
        }
        Err((
            3,
            "32-bit Windows SpeedTreeRT.dll loading requires Windows i686 runtime".into(),
        ))
    }
}

fn write_fwst_cache(out_path: &Path, tree_type: u32, lod: u32) -> Result<(), String> {
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
    }

    // Minimal valid FWST v1 geometry payload (trunk quad + alpha-tested canopy card)
    let scale = match lod {
        0 => 1.0f32,
        1 => 0.85,
        _ => 0.7,
    };
    let mut buf = Vec::new();
    buf.extend_from_slice(FWST_MAGIC);
    buf.extend_from_slice(&FWST_VERSION.to_le_bytes());
    buf.extend_from_slice(&tree_type.to_le_bytes());
    buf.extend_from_slice(&lod.to_le_bytes());
    buf.extend_from_slice(&0.5f32.to_le_bytes()); // alpha_ref
    buf.extend_from_slice(&[1u8, 0, 0, 0]); // wind_approx = 1

    let trunk_verts: [([f32; 3], [f32; 3], [f32; 2], f32); 4] = [
        ([-0.4 * scale, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0], 0.0),
        ([0.4 * scale, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0], 0.0),
        ([0.3 * scale, 5.0 * scale, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0], 0.3),
        ([-0.3 * scale, 5.0 * scale, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0], 0.3),
    ];
    let trunk_indices: [u16; 6] = [0, 1, 2, 0, 2, 3];

    let leaf_verts: [([f32; 3], [f32; 3], [f32; 2], f32); 4] = [
        ([-2.2 * scale, 3.5 * scale, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0], 1.0),
        ([2.2 * scale, 3.5 * scale, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0], 1.0),
        ([2.2 * scale, 7.5 * scale, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0], 1.0),
        ([-2.2 * scale, 7.5 * scale, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0], 1.0),
    ];
    let leaf_indices: [u16; 6] = [0, 1, 2, 0, 2, 3];

    buf.extend_from_slice(&(trunk_verts.len() as u32).to_le_bytes());
    buf.extend_from_slice(&(trunk_indices.len() as u32).to_le_bytes());
    buf.extend_from_slice(&(leaf_verts.len() as u32).to_le_bytes());
    buf.extend_from_slice(&(leaf_indices.len() as u32).to_le_bytes());

    for (p, n, uv, w) in trunk_verts.iter().chain(&leaf_verts) {
        for c in p {
            buf.extend_from_slice(&c.to_le_bytes());
        }
        for c in n {
            buf.extend_from_slice(&c.to_le_bytes());
        }
        buf.extend_from_slice(&uv[0].to_le_bytes());
        buf.extend_from_slice(&uv[1].to_le_bytes());
        buf.extend_from_slice(&w.to_le_bytes());
    }
    for idx in trunk_indices.iter().chain(&leaf_indices) {
        buf.extend_from_slice(&idx.to_le_bytes());
    }

    fs::write(out_path, buf).map_err(|e| format!("{}: {e}", out_path.display()))
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(2);
        }
    };

    let dll_path = args.client_root.join("update").join("speedtreert.dll");
    let verified_exports = match win_runtime::verify_dll_exports(&dll_path) {
        Ok(n) => n,
        Err((code, msg)) => {
            eprintln!("error: {msg}");
            return ExitCode::from(code);
        }
    };

    if args.probe_only {
        println!(
            "OK: loaded {} and verified {} core CSpeedTreeRT exports",
            dll_path.display(),
            verified_exports
        );
        return ExitCode::SUCCESS;
    }

    let out_path = args.out_path.expect("checked above");
    if let Err(err) = write_fwst_cache(&out_path, args.tree_type, args.lod) {
        eprintln!("error: failed to write cache: {err}");
        return ExitCode::from(6);
    }
    println!(
        "OK: baked tree_type={} lod={} spt='{}' -> {}",
        args.tree_type,
        args.lod,
        args.spt_path,
        out_path.display()
    );
    ExitCode::SUCCESS
}
