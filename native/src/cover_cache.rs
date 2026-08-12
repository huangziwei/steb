//! On-device cover thumbnail cache.
//!
//! The grid draws a ~30–50 KB cover per book. Fetching those over TLS on every
//! launch would be the single rudest thing Steb does to Standard Ebooks, so a
//! cover fetched once is read off `/mnt/us` forever after.
//!
//! **Keyed by content.** SE's cover URLs carry a content hash —
//! `/images/covers/{slug}/{sha}/cover@2x.jpg` — so the key is
//! `<slug>.<sha>.jpg` and self-invalidation is structural: a re-produced cover
//! arrives under a new sha, misses, and refetches. Nothing has to notice a
//! change or decide to expire anything.
//!
//! `store` prunes the book's other shas, so the cache holds one file per book
//! rather than one per cover version. That is the *only* pruning: there is
//! deliberately no size ceiling, no LRU, no sweeper. The entire catalogue is
//! roughly 50 MB of covers against a 16–32 GB device, and a cover fetched once
//! should stay fetched.
//!
//! FAT-safe atomic write: bytes go to a `.partial` sibling and are renamed over
//! the target, so a crash mid-write cannot leave a truncated JPEG that would
//! later decode to garbage.

use std::path::{Path, PathBuf};

/// Cache file for one cover. `name` comes from
/// [`crate::se::url::CoverHref::cache_name`], which is `<slug>.<sha>.jpg`.
fn cache_file(dir: &Path, name: &str) -> PathBuf {
    dir.join(name)
}

/// Read a cached cover. `None` on any miss — absent, unreadable, or a sha we
/// have never seen — and the caller falls back to a network fetch.
pub fn load(dir: &Path, name: &str) -> Option<Vec<u8>> {
    std::fs::read(cache_file(dir, name)).ok()
}

/// Write a cover to the cache.
///
/// Callers treat caching as best-effort: a failure here must never fail the
/// fetch that produced the bytes, so the `io::Result` is for logging only.
pub fn store(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let dest = cache_file(dir, name);
    let tmp = dest.with_extension("partial");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &dest)?;
    prune_old(dir, name, &dest);
    Ok(())
}

/// Remove this book's other `<slug>.*.jpg` files, keeping only `keep`.
///
/// Best-effort — a failure just orphans a thumbnail. The trailing dot on the
/// prefix is what stops `jane-austen_emma.` from matching
/// `jane-austen_emma-and-more.…`.
fn prune_old(dir: &Path, name: &str, keep: &Path) {
    // `<slug>.<sha>.jpg` → prefix `<slug>.`
    let Some(slug) = name.split('.').next() else {
        return;
    };
    let prefix = format!("{slug}.");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path == *keep {
            continue;
        }
        let Some(n) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if n.starts_with(&prefix) && n.ends_with(".jpg") {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("steb-covers-{tag}"));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn round_trips() {
        let dir = scratch("rt");
        store(&dir, "a_b.abc123.jpg", b"jpegbytes").unwrap();
        assert_eq!(load(&dir, "a_b.abc123.jpg").unwrap(), b"jpegbytes");
        assert!(load(&dir, "a_b.other.jpg").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_sha_replaces_the_old_one() {
        // A re-produced cover arrives under a new URL; the stale file goes.
        let dir = scratch("prune");
        store(&dir, "a_b.old.jpg", b"1").unwrap();
        store(&dir, "a_b.new.jpg", b"2").unwrap();
        assert!(
            load(&dir, "a_b.old.jpg").is_none(),
            "old sha should be pruned"
        );
        assert!(load(&dir, "a_b.new.jpg").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pruning_leaves_other_books_alone() {
        let dir = scratch("others");
        store(&dir, "other_book.zzz.jpg", b"keep").unwrap();
        store(&dir, "a_b.new.jpg", b"2").unwrap();
        assert!(load(&dir, "other_book.zzz.jpg").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_partial_survives_a_store() {
        let dir = scratch("partial");
        store(&dir, "a_b.abc.jpg", b"x").unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("partial"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
