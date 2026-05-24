//! Warm-tier cache for projections backed by on-disk artefacts.
//!
//! Pattern: each cached projection records (a) the mtime of its source
//! file at fetch time and (b) the wall-clock at fetch time. A subsequent
//! request re-stats the source; cache is valid iff the stat mtime equals
//! the recorded mtime AND we are within `TTL` of the fetch time.
//!
//! Spec: `docs/spec/studio/part-4-data.md` §4.3.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

pub const DEFAULT_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct CacheEntry<T> {
    pub value: T,
    pub source_mtime: SystemTime,
    pub fetched_at: Instant,
}

#[derive(Debug)]
pub struct Cache<K: std::hash::Hash + Eq, T> {
    entries: Mutex<std::collections::HashMap<K, CacheEntry<T>>>,
    ttl: Duration,
}

impl<K: std::hash::Hash + Eq + Clone, T: Clone> Cache<K, T> {
    pub fn new(ttl: Duration) -> Self {
        Self {
            entries: Mutex::new(Default::default()),
            ttl,
        }
    }

    /// Return cached value iff the source mtime AND TTL both pass.
    pub fn get_if_fresh(&self, key: &K, source: &Path) -> Option<T> {
        let entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let cached = entries.get(key)?;
        if cached.fetched_at.elapsed() > self.ttl {
            return None;
        }
        let live_mtime = std::fs::metadata(source).ok()?.modified().ok()?;
        if live_mtime != cached.source_mtime {
            return None;
        }
        Some(cached.value.clone())
    }

    pub fn put(&self, key: K, value: T, source: &Path) {
        if let Ok(meta) = std::fs::metadata(source) {
            if let Ok(mtime) = meta.modified() {
                let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
                entries.insert(
                    key,
                    CacheEntry {
                        value,
                        source_mtime: mtime,
                        fetched_at: Instant::now(),
                    },
                );
            }
        }
    }

    pub fn invalidate(&self, key: &K) {
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(key);
    }

    pub fn invalidate_all(&self) {
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }
}

/// Workspace-level cache key for the exec list (singleton).
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct ExecListKey;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::thread::sleep;

    #[test]
    fn cache_hit_within_ttl_and_unchanged_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("src.db");
        fs::write(&p, b"v1").unwrap();
        let c: Cache<String, i32> = Cache::new(Duration::from_secs(30));
        c.put("k".into(), 42, &p);
        assert_eq!(c.get_if_fresh(&"k".into(), &p), Some(42));
    }

    #[test]
    fn cache_miss_when_mtime_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("src.db");
        fs::write(&p, b"v1").unwrap();
        let c: Cache<String, i32> = Cache::new(Duration::from_secs(30));
        c.put("k".into(), 42, &p);
        // Force mtime delta.
        sleep(Duration::from_millis(10));
        fs::write(&p, b"v2").unwrap();
        assert_eq!(c.get_if_fresh(&"k".into(), &p), None);
    }

    #[test]
    fn cache_miss_when_ttl_expires() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("src.db");
        fs::write(&p, b"v1").unwrap();
        let c: Cache<String, i32> = Cache::new(Duration::from_millis(20));
        c.put("k".into(), 42, &p);
        sleep(Duration::from_millis(40));
        assert_eq!(c.get_if_fresh(&"k".into(), &p), None);
    }

    #[test]
    fn invalidate_drops_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("src.db");
        fs::write(&p, b"v1").unwrap();
        let c: Cache<String, i32> = Cache::new(DEFAULT_TTL);
        c.put("k".into(), 42, &p);
        c.invalidate(&"k".into());
        assert_eq!(c.get_if_fresh(&"k".into(), &p), None);
    }
}
