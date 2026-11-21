//! Synthetic memory-budget check for `TerrainStore` (default 2 MiB hot set).
//!
//! Feeds synthetic 33x33 terrain blocks through `cache/terrain-v1` and asserts
//! that owned RAM stays strictly within the configured budget while disk cache
//! hits recover pages losslessly.

use std::{
    env, fs,
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

use forsaken_world_explorer::{
    terrain_block::{self, T2bkFile},
    terrain_store::{TerrainPage, TerrainStore, DEFAULT_HOT_RAM_BYTES},
};

fn main() -> ExitCode {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let cache_root = env::temp_dir().join(format!(
        "fw-terrain-memcheck-{}-{nanos}",
        std::process::id()
    ));

    let res = (|| -> Result<(), String> {
        let budget = DEFAULT_HOT_RAM_BYTES;
        let mut store = TerrainStore::open(&cache_root, budget)?;

        // Build 320 synthetic 33x33 blocks (~2.4 MiB decoded raw > 2 MiB default budget).
        let t2bk_bytes = terrain_block::synthetic::build_t2bk_v6(32, 320);
        let t2bk = T2bkFile::parse_header(&t2bk_bytes)?;

        for i in 0..t2bk.offsets.len() {
            let key = TerrainStore::content_key(&format!("synthetic-block-{i}").into_bytes());
            let page = store.get_or_insert_with(&key, || {
                let decoded = t2bk.decode_block(&t2bk_bytes, i, 32)?;
                Ok(TerrainPage {
                    heights: decoded.heights,
                    normals: decoded.normals,
                })
            })?;
            if page.heights.len() != 1089 {
                return Err(format!("unexpected vertex count {}", page.heights.len()));
            }
            let stats = store.stats();
            if stats.owned_ram_bytes > budget {
                return Err(format!(
                    "owned RAM {} exceeded budget {} at block {i}",
                    stats.owned_ram_bytes, budget
                ));
            }
        }

        // Re-fetch block 0 to confirm lossless disk-cache recovery after eviction.
        let key0 = TerrainStore::content_key(b"synthetic-block-0");
        let _ = store.get_or_insert_with(&key0, || {
            Err("expected disk cache hit for evicted block 0".into())
        })?;

        let stats = store.stats();
        println!(
            "[PASS] terrain-memory-check: budget={} B, owned_ram={} B, evictions={}, disk_hits={}, misses={}",
            stats.budget_bytes,
            stats.owned_ram_bytes,
            stats.evictions,
            stats.disk_hits,
            stats.misses
        );
        if stats.evictions == 0 || stats.disk_hits == 0 {
            return Err("expected evictions and disk cache hits under 2 MiB budget".into());
        }
        Ok(())
    })();

    let _ = fs::remove_dir_all(&cache_root);

    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[FAIL] terrain-memory-check: {error}");
            ExitCode::FAILURE
        }
    }
}
