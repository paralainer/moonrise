//! Disk cache for cut segments and init sections, trimmed to a size cap (oldest first).
//! Prepared downloads live in `dl/` and aren't trimmed here (see `downloads.rs`).

use std::{path::PathBuf, sync::Arc, time::Duration};

use bytes::Bytes;

pub struct Cache {
    dir: PathBuf,
    max_bytes: u64,
}

impl Cache {
    pub fn new(dir: PathBuf, max_bytes: u64) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Cache { dir, max_bytes }
    }

    pub fn path_of(&self, key: &str) -> PathBuf {
        self.dir.join(key)
    }

    pub async fn get(&self, key: &str) -> Option<Bytes> {
        let path = self.path_of(key);
        let data = tokio::fs::read(&path).await.ok()?;
        // Mark as recently used for the janitor.
        let _ = std::fs::File::open(&path).and_then(|f| f.set_modified(std::time::SystemTime::now()));
        Some(Bytes::from(data))
    }

    pub async fn put(&self, key: &str, data: &[u8]) {
        let path = self.path_of(key);
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let tmp = path.with_extension("part");
        if tokio::fs::write(&tmp, data).await.is_ok() {
            let _ = tokio::fs::rename(&tmp, &path).await;
        }
    }

    /// Every few minutes, delete the least recently used files above the cap.
    pub async fn janitor(cache: Arc<Cache>) {
        loop {
            let c = cache.clone();
            let _ = tokio::task::spawn_blocking(move || c.trim()).await;
            tokio::time::sleep(Duration::from_secs(300)).await;
        }
    }

    /// Prepared downloads (`dl/`) and scrubbing thumbnails (`trickplay/`) are left alone: they are
    /// capped on their own.
    fn trim(&self) {
        let mut files = Vec::new();
        let downloads = self.dir.join("dl");
        // Scrubbing thumbnails keep to their own budget (`trickplay.rs`).
        let thumbnails = self.dir.join("trickplay");
        let mut stack = vec![self.dir.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let Ok(meta) = e.metadata() else { continue };
                if meta.is_dir() {
                    if e.path() != downloads && e.path() != thumbnails {
                        stack.push(e.path());
                    }
                } else if let Ok(m) = meta.modified() {
                    files.push((m, meta.len(), e.path()));
                }
            }
        }
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        if total <= self.max_bytes {
            return;
        }
        files.sort_by_key(|f| f.0);
        for (_, len, path) in files {
            if total <= self.max_bytes * 9 / 10 {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total -= len;
            }
        }
    }
}
