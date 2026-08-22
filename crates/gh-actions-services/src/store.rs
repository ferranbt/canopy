//! Where cache entries are kept: plain files in a directory.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::cache_server::{Cache, CacheEntry};

#[derive(Debug)]
pub struct LocalCache {
    root: PathBuf,
    entries: Mutex<BTreeMap<i64, CacheEntry>>,
    next_id: Mutex<i64>,
}

impl LocalCache {
    /// The index is on disk: an archive says nothing about the key it belongs to.
    pub fn open(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;

        let entries: BTreeMap<i64, CacheEntry> = std::fs::read_to_string(root.join("index.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let highest = entries.keys().copied().max().unwrap_or(1);

        Ok(Self {
            root,
            entries: Mutex::new(entries),
            next_id: Mutex::new(highest),
        })
    }

    fn persist(&self, entries: &BTreeMap<i64, CacheEntry>) {
        if let Ok(text) = serde_json::to_string_pretty(entries) {
            let _ = std::fs::write(self.root.join("index.json"), text);
        }
    }

    fn path(&self, id: i64) -> PathBuf {
        self.root.join(format!("{id}.tzst"))
    }
}

impl Cache for LocalCache {
    fn reserve(&self, key: &str, version: &str) -> Option<i64> {
        let mut entries = self.entries.lock().expect("cache lock");
        if entries
            .values()
            .any(|entry| entry.key == key && entry.version == version && entry.committed)
        {
            return None;
        }

        let mut next = self.next_id.lock().expect("id lock");
        *next += 1;
        entries.insert(
            *next,
            CacheEntry {
                key: key.to_owned(),
                version: version.to_owned(),
                id: *next,
                committed: false,
            },
        );
        Some(*next)
    }

    fn commit(&self, id: i64) -> bool {
        let mut entries = self.entries.lock().expect("cache lock");
        let Some(entry) = entries.get_mut(&id) else {
            return false;
        };

        entry.committed = true;
        self.persist(&entries);
        true
    }

    /// What `restore-keys` needs: the first key exact, the rest by prefix, newest first.
    fn lookup(&self, keys: &[String], version: &str) -> Option<CacheEntry> {
        let entries = self.entries.lock().expect("cache lock");
        let usable = |entry: &&CacheEntry| entry.committed && entry.version == version;

        if let Some(exact) = keys.first().and_then(|key| {
            entries
                .values()
                .filter(usable)
                .find(|entry| entry.key == *key)
        }) {
            return Some(exact.clone());
        }

        keys.iter().skip(1).find_map(|prefix| {
            entries
                .values()
                .rev()
                .filter(usable)
                .find(|entry| entry.key.starts_with(prefix))
                .cloned()
        })
    }

    fn write(&self, id: i64, offset: u64, bytes: &[u8]) -> std::io::Result<()> {
        use std::io::{Seek, SeekFrom, Write};

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(self.path(id))?;
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(bytes)
    }

    fn read(&self, id: i64) -> std::io::Result<Vec<u8>> {
        std::fs::read(self.path(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn behaves_as_a_cache() {
        let root = std::env::temp_dir().join("local-cache-conformance");
        let _ = std::fs::remove_dir_all(&root);

        crate::cache_server::conformance(Box::new(LocalCache::open(root).expect("store opens")));
    }
}
