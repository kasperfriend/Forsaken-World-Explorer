//! Decoded RGBA8 texture cache with shared lookup and pinned-batch budget enforcement.
//!
//! Default budget: 32 MiB (`FW_TEX_BUDGET_MIB`). Textures used by the currently visible
//! draw batch are pinned and may temporarily exceed the budget rather than being silently
//! downscaled or dropped. Exceedances are counted and logged.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

#[derive(Debug, Clone)]
pub struct DecodedTexture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl DecodedTexture {
    pub fn byte_size(&self) -> usize {
        self.rgba.len()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextureCacheStats {
    pub count: usize,
    pub total_bytes: usize,
    pub pinned_bytes: usize,
    pub budget_bytes: usize,
    pub budget_exceeded: bool,
    pub budget_exceeded_events: u64,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

struct Entry {
    tex: Arc<DecodedTexture>,
    last_used: u64,
}

pub struct TextureCache {
    budget_bytes: usize,
    clock: u64,
    entries: HashMap<String, Entry>,
    pinned: HashSet<String>,
    total_bytes: usize,
    was_exceeded: bool,
    budget_exceeded_events: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
}

impl TextureCache {
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            budget_bytes: budget_bytes.max(1024),
            clock: 0,
            entries: HashMap::new(),
            pinned: HashSet::new(),
            total_bytes: 0,
            was_exceeded: false,
            budget_exceeded_events: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    pub fn normalize_key(path: &str) -> String {
        path.trim()
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_ascii_lowercase()
    }

    pub fn set_budget_bytes(&mut self, budget_bytes: usize) {
        self.budget_bytes = budget_bytes.max(1024);
    }

    /// Clear previous frame's pin set before pinning the newly visible batch.
    pub fn begin_visible_batch(&mut self) {
        self.pinned.clear();
    }

    /// Retrieve a shared texture by path (decoding once on miss) and pin it for the
    /// current visible batch. Never downscales or drops a pinned texture.
    pub fn get_or_insert_pinned<F>(
        &mut self,
        path: &str,
        decode: F,
    ) -> Result<Arc<DecodedTexture>, String>
    where
        F: FnOnce() -> Result<DecodedTexture, String>,
    {
        let key = Self::normalize_key(path);
        self.clock += 1;
        self.pinned.insert(key.clone());

        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_used = self.clock;
            self.hits += 1;
            return Ok(Arc::clone(&entry.tex));
        }

        self.misses += 1;
        let decoded = Arc::new(decode()?);
        self.total_bytes += decoded.byte_size();
        self.entries.insert(
            key,
            Entry {
                tex: Arc::clone(&decoded),
                last_used: self.clock,
            },
        );
        Ok(decoded)
    }

    /// Evict unpinned LRU textures until `total_bytes <= budget_bytes` (or no unpinned
    /// textures remain). Returns `Some(warning_message)` when the pinned batch alone
    /// newly exceeds the budget so the caller can log it.
    pub fn enforce_budget_after_batch(&mut self) -> Option<String> {
        while self.total_bytes > self.budget_bytes {
            let victim = self
                .entries
                .iter()
                .filter(|(k, _)| !self.pinned.contains(*k))
                .min_by_key(|(_, e)| e.last_used)
                .map(|(k, _)| k.clone());
            let Some(key) = victim else {
                break;
            };
            if let Some(removed) = self.entries.remove(&key) {
                self.total_bytes = self.total_bytes.saturating_sub(removed.tex.byte_size());
                self.evictions += 1;
            }
        }

        let exceeded = self.total_bytes > self.budget_bytes;
        let warning = if exceeded && !self.was_exceeded {
            self.budget_exceeded_events += 1;
            Some(format!(
                "Texture cache pinned batch ({:.2} MiB across {} textures) exceeded budget ({:.2} MiB); keeping pinned textures intact without downscaling.",
                self.total_bytes as f64 / (1024.0 * 1024.0),
                self.entries.len(),
                self.budget_bytes as f64 / (1024.0 * 1024.0),
            ))
        } else {
            None
        };
        self.was_exceeded = exceeded;
        warning
    }

    pub fn stats(&self) -> TextureCacheStats {
        let pinned_bytes: usize = self
            .pinned
            .iter()
            .filter_map(|k| self.entries.get(k))
            .map(|e| e.tex.byte_size())
            .sum();
        TextureCacheStats {
            count: self.entries.len(),
            total_bytes: self.total_bytes,
            pinned_bytes,
            budget_bytes: self.budget_bytes,
            budget_exceeded: self.total_bytes > self.budget_bytes,
            budget_exceeded_events: self.budget_exceeded_events,
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_tex(w: u32, h: u32) -> DecodedTexture {
        DecodedTexture {
            width: w,
            height: h,
            rgba: vec![255u8; (w * h * 4) as usize],
        }
    }

    #[test]
    fn shared_lookup_decodes_once_case_insensitively() {
        let mut cache = TextureCache::new(64 * 1024);
        let mut calls = 0usize;
        cache.begin_visible_batch();
        let a = cache
            .get_or_insert_pinned("Textures\\Stone_01.DDS", || {
                calls += 1;
                Ok(fake_tex(16, 16))
            })
            .unwrap();
        let b = cache
            .get_or_insert_pinned("textures/stone_01.dds", || {
                calls += 1;
                Ok(fake_tex(16, 16))
            })
            .unwrap();
        assert_eq!(calls, 1);
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(cache.stats().hits, 1);
        assert_eq!(cache.stats().misses, 1);
    }

    #[test]
    fn pinned_batch_exceeds_budget_without_dropping_and_evicts_unpinned_later() {
        // Budget = 4096 bytes; each 32x32 RGBA8 texture is 4096 bytes
        let mut cache = TextureCache::new(4096);
        cache.begin_visible_batch();
        cache
            .get_or_insert_pinned("a.dds", || Ok(fake_tex(32, 32)))
            .unwrap();
        cache
            .get_or_insert_pinned("b.dds", || Ok(fake_tex(32, 32)))
            .unwrap();
        let warn = cache.enforce_budget_after_batch();
        assert!(
            warn.is_some(),
            "must emit warning when pinned batch exceeds budget"
        );
        let st = cache.stats();
        assert_eq!(st.count, 2, "must not drop pinned textures");
        assert_eq!(st.total_bytes, 8192);
        assert!(st.budget_exceeded);
        assert_eq!(st.budget_exceeded_events, 1);

        // Next frame only pins b.dds -> a.dds is unpinned and evicted cleanly
        cache.begin_visible_batch();
        cache
            .get_or_insert_pinned("b.dds", || panic!("should be cached"))
            .unwrap();
        assert!(cache.enforce_budget_after_batch().is_none());
        let st2 = cache.stats();
        assert_eq!(st2.count, 1);
        assert_eq!(st2.total_bytes, 4096);
        assert!(!st2.budget_exceeded);
        assert_eq!(st2.evictions, 1);
    }
}
