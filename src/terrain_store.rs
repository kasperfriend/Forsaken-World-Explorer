//! RAM-first, content-addressed lossless terrain cache (`cache/terrain-v1`).
//!
//! Keeps decoded terrain pages bounded to a configurable RAM hot set (default 2 MiB,
//! configurable via `FW_TERRAIN_RAM_MIB`), spilling lossless compressed pages to disk
//! keyed by SHA-256 and verifying integrity on every read.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

pub const CACHE_SUBDIR: &str = "cache/terrain-v1";
pub const DEFAULT_HOT_RAM_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct TerrainPage {
    pub heights: Vec<f32>,
    pub normals: Vec<[i8; 3]>,
}

impl TerrainPage {
    pub fn owned_bytes(&self) -> usize {
        self.heights.capacity() * std::mem::size_of::<f32>()
            + self.normals.capacity() * std::mem::size_of::<[i8; 3]>()
            + std::mem::size_of::<Self>()
    }

    fn encode_lossless(&self) -> Result<Vec<u8>, String> {
        let mut raw = Vec::with_capacity(8 + self.heights.len() * 4 + self.normals.len() * 3);
        raw.extend_from_slice(&(self.heights.len() as u32).to_le_bytes());
        raw.extend_from_slice(&(self.normals.len() as u32).to_le_bytes());
        for &h in &self.heights {
            raw.extend_from_slice(&h.to_le_bytes());
        }
        for &[nx, ny, nz] in &self.normals {
            raw.extend_from_slice(&[nx as u8, ny as u8, nz as u8]);
        }
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::fast());
        enc.write_all(&raw).map_err(|e| e.to_string())?;
        enc.finish().map_err(|e| e.to_string())
    }

    fn decode_lossless(compressed: &[u8]) -> Result<Self, String> {
        let mut dec = ZlibDecoder::new(compressed);
        let mut raw = Vec::new();
        dec.read_to_end(&mut raw)
            .map_err(|e| format!("cache zlib decode failed: {e}"))?;
        if raw.len() < 8 {
            return Err("truncated terrain cache page".into());
        }
        let h_len = u32::from_le_bytes(raw[0..4].try_into().unwrap()) as usize;
        let n_len = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
        let expected = 8 + h_len * 4 + n_len * 3;
        if raw.len() != expected {
            return Err(format!(
                "terrain cache page length mismatch: got {}, expected {expected}",
                raw.len()
            ));
        }
        let mut heights = Vec::with_capacity(h_len);
        let h_slice = &raw[8..8 + h_len * 4];
        for &c in h_slice.as_chunks::<4>().0 {
            heights.push(f32::from_le_bytes(c));
        }
        let mut normals = Vec::with_capacity(n_len);
        let n_slice = &raw[8 + h_len * 4..];
        for &c in n_slice.as_chunks::<3>().0 {
            normals.push([c[0] as i8, c[1] as i8, c[2] as i8]);
        }
        Ok(Self { heights, normals })
    }
}

struct HotEntry {
    page: Arc<TerrainPage>,
    last_used: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub ram_hits: u64,
    pub disk_hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub owned_ram_bytes: usize,
    pub budget_bytes: usize,
}

pub struct TerrainStore {
    cache_dir: PathBuf,
    budget_bytes: usize,
    clock: u64,
    hot: HashMap<String, HotEntry>,
    disk_hashes: HashMap<String, [u8; 32]>,
    stats: CacheStats,
}

impl TerrainStore {
    pub fn open(cache_root: &Path, budget_bytes: usize) -> Result<Self, String> {
        let cache_dir = cache_root.join(CACHE_SUBDIR);
        fs::create_dir_all(&cache_dir)
            .map_err(|e| format!("cannot create {}: {e}", cache_dir.display()))?;
        let budget_bytes = budget_bytes.max(64 * 1024);
        Ok(Self {
            cache_dir,
            budget_bytes,
            clock: 0,
            hot: HashMap::new(),
            disk_hashes: HashMap::new(),
            stats: CacheStats {
                budget_bytes,
                ..CacheStats::default()
            },
        })
    }

    pub fn content_key(source_bytes: &[u8]) -> String {
        let digest = Sha256::digest(source_bytes);
        let mut hex = String::with_capacity(64);
        for b in digest {
            use std::fmt::Write;
            let _ = write!(&mut hex, "{b:02x}");
        }
        hex
    }

    pub fn stats(&self) -> CacheStats {
        self.stats
    }

    pub fn get_or_insert_with<F>(
        &mut self,
        key: &str,
        decode: F,
    ) -> Result<Arc<TerrainPage>, String>
    where
        F: FnOnce() -> Result<TerrainPage, String>,
    {
        self.clock += 1;
        if let Some(entry) = self.hot.get_mut(key) {
            entry.last_used = self.clock;
            self.stats.ram_hits += 1;
            return Ok(Arc::clone(&entry.page));
        }

        let disk_path = self.cache_dir.join(format!("{key}.tpage"));
        if disk_path.is_file() {
            if let Ok(blob) = fs::read(&disk_path) {
                let actual_hash: [u8; 32] = Sha256::digest(&blob).into();
                let hash_ok = self
                    .disk_hashes
                    .get(key)
                    .map(|expected| *expected == actual_hash)
                    .unwrap_or(true);
                if hash_ok {
                    if let Ok(page) = TerrainPage::decode_lossless(&blob) {
                        self.stats.disk_hits += 1;
                        return Ok(self.insert_hot(key.to_string(), page));
                    }
                }
            }
        }

        self.stats.misses += 1;
        let page = decode()?;
        let blob = page.encode_lossless()?;
        let hash: [u8; 32] = Sha256::digest(&blob).into();
        fs::write(&disk_path, &blob)
            .map_err(|e| format!("cannot write {}: {e}", disk_path.display()))?;
        self.disk_hashes.insert(key.to_string(), hash);
        Ok(self.insert_hot(key.to_string(), page))
    }

    fn insert_hot(&mut self, key: String, page: TerrainPage) -> Arc<TerrainPage> {
        let arc = Arc::new(page);
        let bytes = arc.owned_bytes();
        self.hot.insert(
            key,
            HotEntry {
                page: Arc::clone(&arc),
                last_used: self.clock,
            },
        );
        self.stats.owned_ram_bytes += bytes;
        self.evict_to_budget();
        arc
    }

    fn evict_to_budget(&mut self) {
        while self.stats.owned_ram_bytes > self.budget_bytes && self.hot.len() > 1 {
            let oldest_key = self
                .hot
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(k, _)| k.clone());
            let Some(k) = oldest_key else {
                break;
            };
            if let Some(removed) = self.hot.remove(&k) {
                self.stats.owned_ram_bytes = self
                    .stats
                    .owned_ram_bytes
                    .saturating_sub(removed.page.owned_bytes());
                self.stats.evictions += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_cache_root(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        env::temp_dir().join(format!(
            "fw-explorer-tstore-{tag}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn synthetic_page(seed: f32) -> TerrainPage {
        TerrainPage {
            heights: (0..1089).map(|i| seed + (i as f32) * 0.125).collect(),
            normals: vec![[0, 127, 0]; 1089],
        }
    }

    #[test]
    fn enforces_ram_budget_and_recovers_losslessly_from_disk() {
        let root = temp_cache_root("budget");
        // Budget of 128 KiB fits ~16 pages of 1089 verts (~7.6 KiB each).
        let budget = 128 * 1024;
        let mut store = TerrainStore::open(&root, budget).expect("open store");

        for i in 0..40 {
            let key = TerrainStore::content_key(&format!("block-{i}").into_bytes());
            let page = store
                .get_or_insert_with(&key, || Ok(synthetic_page(i as f32)))
                .expect("insert page");
            assert_eq!(page.heights.len(), 1089);
            assert!(store.stats().owned_ram_bytes <= budget);
        }
        assert!(store.stats().evictions > 0);

        // Re-reading an evicted page hits disk losslessly without re-running decode!
        let key0 = TerrainStore::content_key(b"block-0");
        let reloaded = store
            .get_or_insert_with(&key0, || Err("should have hit disk cache".into()))
            .expect("disk hit");
        assert_eq!(*reloaded, synthetic_page(0.0));
        assert_eq!(store.stats().disk_hits, 1);

        let _ = fs::remove_dir_all(&root);
    }
}
