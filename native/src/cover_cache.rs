//! Cover thumbnails on `/mnt/us`, keyed `<slug>.<sha>.jpg` by
//! [`crate::se::url::CoverHref::cache_name`]. A new sha misses and refetches;
//! [`store`] prunes the book's other shas and nothing else.

use std::path::{Path, PathBuf};

/// The cache file for `name`.
fn cache_file(dir: &Path, name: &str) -> PathBuf {
    dir.join(name)
}

/// A cached cover. `None` on an absent, unreadable or unseen `name`.
pub fn load(dir: &Path, name: &str) -> Option<Vec<u8>> {
    std::fs::read(cache_file(dir, name)).ok()
}

/// `bytes` under `name`, then [`prune_other_shas`]. The `io::Result` is for
/// [`crate::log`].
pub fn store(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let dest = cache_file(dir, name);
    let tmp = dest.with_extension("partial");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &dest)?;
    prune_old(dir, name, &dest);
    Ok(())
}

/// The book's other `<slug>.*.jpg` files, past `keep`. The trailing dot on the
/// prefix keeps `jane-austen_emma.` clear of `jane-austen_emma-and-more.…`.
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
        // A new URL for the same book.
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
