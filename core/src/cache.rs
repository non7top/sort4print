//! A cache of decoded images on disk.
//!
//! Decoding a 50-megapixel phone JPEG and shrinking it takes long enough to be
//! felt on every single photo, and it was being redone from scratch every time
//! the program started. What goes in here instead is the finished, upright,
//! already-shrunk image as a small JPEG, so the second visit to a folder costs
//! a few tens of milliseconds a picture rather than seconds.
//!
//! Three properties matter more than speed:
//!
//! * **It cannot go stale.** The key includes the photo's size and modification
//!   time, so editing or replacing a file simply produces a different key and
//!   the old entry is never consulted again.
//! * **It cannot be the only copy.** Everything in here is derived and can be
//!   rebuilt from the original. Deleting the whole directory costs time, never
//!   data.
//! * **It cannot grow without limit.** A budget is enforced by discarding the
//!   least recently used entries.
//!
//! It lives with the user's other caches rather than beside the photos: eleven
//! thousand thumbnails is not something to scatter through someone's pictures.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Filmstrip-sized.
    Thumb,
    /// Editor-sized.
    View,
}

impl Kind {
    fn suffix(self) -> &'static str {
        match self {
            Kind::Thumb => "t",
            Kind::View => "v",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiskCache {
    root: PathBuf,
    budget_bytes: u64,
}

impl DiskCache {
    pub fn new(root: PathBuf, budget_bytes: u64) -> DiskCache {
        DiskCache {
            root,
            budget_bytes,
        }
    }

    /// Alongside the user's other caches, not with their photos.
    pub fn default_root() -> PathBuf {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            if !local.is_empty() {
                return PathBuf::from(local).join("sort4print").join("cache");
            }
        }
        if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
            if !xdg.is_empty() {
                return PathBuf::from(xdg).join("sort4print");
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            if !home.is_empty() {
                return PathBuf::from(home).join(".cache").join("sort4print");
            }
        }
        std::env::temp_dir().join("sort4print-cache")
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn budget_bytes(&self) -> u64 {
        self.budget_bytes
    }

    /// Identifies a photo by where it is and what state it is in. A photo that
    /// changes gets a new key rather than a stale entry, which is what makes
    /// invalidation a non-problem instead of a source of bugs.
    pub fn key_for(path: &Path) -> Option<String> {
        let meta = std::fs::metadata(path).ok()?;
        let modified = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis();

        let mut hash = FNV_OFFSET;
        for byte in path.to_string_lossy().as_bytes() {
            hash = fnv_step(hash, *byte);
        }
        for byte in meta.len().to_le_bytes() {
            hash = fnv_step(hash, byte);
        }
        for byte in (modified as u64).to_le_bytes() {
            hash = fnv_step(hash, byte);
        }
        Some(format!("{hash:016x}"))
    }

    /// Sharded by the first two characters of the key: eleven thousand photos
    /// is twenty-two thousand files, and directories that size are slow to
    /// enumerate on Windows.
    ///
    /// The size an entry was made at is part of its name, for two reasons. A
    /// preview cached for one screen is the wrong size for a bigger one, and
    /// without this the stored copy would be served regardless and the setting
    /// would appear to do nothing. And with it, a laptop and a large monitor
    /// each keep their own entry instead of evicting each other's on every
    /// swap.
    fn entry_path(&self, key: &str, kind: Kind, size_px: u32) -> PathBuf {
        let shard = &key[..2.min(key.len())];
        self.root
            .join(shard)
            .join(format!("{key}.{}{size_px}.jpg", kind.suffix()))
    }

    pub fn read(&self, key: &str, kind: Kind, size_px: u32) -> Option<Vec<u8>> {
        let path = self.entry_path(key, kind, size_px);
        let bytes = std::fs::read(&path).ok()?;
        if bytes.is_empty() {
            return None;
        }
        // Touching the file on a hit is what makes the eviction order "least
        // recently used" rather than "oldest".
        let _ = filetime_now(&path);
        Some(bytes)
    }

    pub fn write(&self, key: &str, kind: Kind, size_px: u32, bytes: &[u8]) -> Result<()> {
        let path = self.entry_path(key, kind, size_px);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        // Via a temporary, so a half-written entry is never readable as a whole
        // one. Worst case a reader misses and decodes the original.
        let temporary = path.with_extension("part");
        std::fs::write(&temporary, bytes)
            .with_context(|| format!("writing {}", temporary.display()))?;
        std::fs::rename(&temporary, &path)
            .with_context(|| format!("moving into {}", path.display()))
    }

    pub fn contains(&self, key: &str, kind: Kind, size_px: u32) -> bool {
        self.entry_path(key, kind, size_px).is_file()
    }

    /// Sizes a preview is cached at.
    ///
    /// Bucketed rather than following the window exactly, so that dragging a
    /// window about or moving between two screens settles on one of a handful of
    /// entries instead of making a new one for every width the window has ever
    /// had.
    pub const SIZE_BUCKETS: &'static [u32] = &[1000, 1400, 1800, 2200, 2800, 3600];

    /// The bucket that covers `wanted`, never exceeding `cap`.
    pub fn bucket_for(wanted: u32, cap: u32) -> u32 {
        let cap = cap.max(Self::SIZE_BUCKETS[0]);
        Self::SIZE_BUCKETS
            .iter()
            .copied()
            .find(|bucket| *bucket >= wanted)
            .unwrap_or(*Self::SIZE_BUCKETS.last().expect("never empty"))
            .min(cap)
    }

    /// Every entry, with its size and when it was last used.
    fn entries(&self) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
        let mut out = Vec::new();
        let Ok(shards) = std::fs::read_dir(&self.root) else {
            return out;
        };
        for shard in shards.flatten() {
            let Ok(files) = std::fs::read_dir(shard.path()) else {
                continue;
            };
            for file in files.flatten() {
                let Ok(meta) = file.metadata() else { continue };
                if !meta.is_file() {
                    continue;
                }
                let used = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                out.push((file.path(), meta.len(), used));
            }
        }
        out
    }

    pub fn total_bytes(&self) -> u64 {
        self.entries().iter().map(|(_, len, _)| *len).sum()
    }

    pub fn entry_count(&self) -> usize {
        self.entries().len()
    }

    /// Discards least-recently-used entries until the total is within budget.
    /// Returns how many bytes went.
    pub fn prune(&self) -> u64 {
        let mut entries = self.entries();
        let mut total: u64 = entries.iter().map(|(_, len, _)| *len).sum();
        if total <= self.budget_bytes {
            return 0;
        }

        entries.sort_by_key(|(_, _, used)| *used);
        let mut freed = 0;
        for (path, len, _) in entries {
            if total <= self.budget_bytes {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
                freed += len;
            }
        }
        freed
    }

    pub fn clear(&self) -> Result<()> {
        if self.root.exists() {
            std::fs::remove_dir_all(&self.root)
                .with_context(|| format!("removing {}", self.root.display()))?;
        }
        Ok(())
    }
}

/// Marks a file as just used, for the eviction order. Best effort: failing to
/// touch it only makes it look older than it is.
fn filetime_now(path: &Path) -> std::io::Result<()> {
    // Rewriting the file's own first byte would be a real write; opening for
    // append with no data is enough to move the modification time on the
    // platforms that matter, and harmless where it is not.
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
    file.write_all(&[])?;
    file.flush()
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a, written out rather than using `DefaultHasher`, whose values are
/// explicitly not stable between Rust releases — that would silently orphan
/// every entry in the cache after a toolchain upgrade.
fn fnv_step(hash: u64, byte: u8) -> u64 {
    (hash ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sort4print-cache-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn photo(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn a_key_depends_on_the_file_not_just_its_name() {
        let dir = scratch("keys");
        let a = photo(&dir, "a.jpg", b"one");
        let b = photo(&dir, "b.jpg", b"one");
        assert_ne!(DiskCache::key_for(&a), DiskCache::key_for(&b), "path matters");

        let before = DiskCache::key_for(&a).unwrap();
        // Same name, different contents: the key has to change or a stale entry
        // would be served for a photo that was replaced.
        std::fs::write(&a, b"different length").unwrap();
        let after = DiskCache::key_for(&a).unwrap();
        assert_ne!(before, after, "contents matter");

        assert!(DiskCache::key_for(&dir.join("missing.jpg")).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_key_is_stable_for_an_unchanged_file() {
        let dir = scratch("stable");
        let a = photo(&dir, "a.jpg", b"one");
        assert_eq!(DiskCache::key_for(&a), DiskCache::key_for(&a));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn entries_round_trip_and_the_two_kinds_do_not_collide() {
        let dir = scratch("roundtrip");
        let cache = DiskCache::new(dir.join("cache"), 10_000_000);

        cache.write("abcdef0123456789", Kind::Thumb, 220, b"thumb bytes").unwrap();
        cache.write("abcdef0123456789", Kind::View, 1800, b"view bytes").unwrap();

        assert_eq!(
            cache.read("abcdef0123456789", Kind::Thumb, 220).as_deref(),
            Some(&b"thumb bytes"[..])
        );
        assert_eq!(
            cache.read("abcdef0123456789", Kind::View, 1800).as_deref(),
            Some(&b"view bytes"[..])
        );
        assert!(cache.contains("abcdef0123456789", Kind::View, 1800));
        assert!(cache.read("no such key here!", Kind::View, 1800).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_partial_files_are_left_behind() {
        let dir = scratch("partial");
        let cache = DiskCache::new(dir.join("cache"), 10_000_000);
        cache.write("0123456789abcdef", Kind::View, 1800, b"x").unwrap();

        let shard = cache.root().join("01");
        let leftovers: Vec<_> = std::fs::read_dir(&shard)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "part"))
            .collect();
        assert!(leftovers.is_empty(), "a .part file survived the write");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pruning_brings_the_total_within_budget() {
        let dir = scratch("prune");
        // Budget of 3 KB against 10 entries of 1 KB each.
        let cache = DiskCache::new(dir.join("cache"), 3_000);
        let payload = vec![b'x'; 1_000];
        for i in 0..10 {
            cache
                .write(&format!("{i:016x}"), Kind::View, 1800, &payload)
                .unwrap();
        }
        assert!(cache.total_bytes() > 3_000);

        let freed = cache.prune();
        assert!(freed > 0);
        assert!(
            cache.total_bytes() <= 3_000,
            "still over budget: {}",
            cache.total_bytes()
        );
        assert!(cache.entry_count() > 0, "it should not have emptied itself");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The whole point of the size being in the entry name: a preview made for
    /// a laptop screen must not be handed back for a large monitor, or the
    /// setting would appear to do nothing at all.
    #[test]
    fn the_same_photo_at_two_sizes_is_two_entries() {
        let dir = scratch("sizes");
        let cache = DiskCache::new(dir.join("cache"), 10_000_000);

        cache.write("aaaaaaaaaaaaaaaa", Kind::View, 1400, b"laptop").unwrap();
        cache.write("aaaaaaaaaaaaaaaa", Kind::View, 2800, b"monitor").unwrap();

        assert_eq!(
            cache.read("aaaaaaaaaaaaaaaa", Kind::View, 1400).as_deref(),
            Some(&b"laptop"[..])
        );
        assert_eq!(
            cache.read("aaaaaaaaaaaaaaaa", Kind::View, 2800).as_deref(),
            Some(&b"monitor"[..])
        );
        // A size never cached is a miss, not the nearest thing to hand.
        assert!(cache.read("aaaaaaaaaaaaaaaa", Kind::View, 1800).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn buckets_round_up_and_respect_the_cap() {
        // Just over a bucket goes to the next one, not back to the last.
        assert_eq!(DiskCache::bucket_for(1401, 3600), 1800);
        assert_eq!(DiskCache::bucket_for(1400, 3600), 1400);
        assert_eq!(DiskCache::bucket_for(1, 3600), 1000);
        // Beyond the largest bucket, the largest is used.
        assert_eq!(DiskCache::bucket_for(99_999, 3600), 3600);
        // The cap wins over the bucket.
        assert_eq!(DiskCache::bucket_for(2800, 1800), 1800);
        // A nonsensical cap still yields something usable.
        assert_eq!(DiskCache::bucket_for(2800, 0), 1000);
    }

    #[test]
    fn pruning_within_budget_does_nothing() {
        let dir = scratch("noprune");
        let cache = DiskCache::new(dir.join("cache"), 10_000_000);
        cache.write("0000000000000001", Kind::View, 1800, &vec![b'x'; 100]).unwrap();
        assert_eq!(cache.prune(), 0);
        assert_eq!(cache.entry_count(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clearing_removes_everything_and_is_safe_when_absent() {
        let dir = scratch("clear");
        let cache = DiskCache::new(dir.join("cache"), 10_000);
        cache.write("0000000000000001", Kind::View, 1800, b"x").unwrap();
        cache.clear().unwrap();
        assert_eq!(cache.entry_count(), 0);
        // Clearing again, with nothing there, is not an error.
        cache.clear().unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_absent_cache_directory_reads_as_empty_rather_than_failing() {
        let cache = DiskCache::new(PathBuf::from("/nonexistent/sort4print"), 1_000);
        assert_eq!(cache.total_bytes(), 0);
        assert_eq!(cache.entry_count(), 0);
        assert!(cache.read("0000000000000001", Kind::View, 1800).is_none());
        assert_eq!(cache.prune(), 0);
    }
}
