//! Fetch an azw3 and commit it to the device library.
//!
//! Two things here are not obvious:
//!
//! 1. The bare `.azw3` URL does **not** serve the file. It returns a ~10 KB
//!    "Your Download Has Started!" page whose meta refresh points back at the
//!    same URL with `?source=download`. [`super::url::Endpoint::Download`]
//!    appends that, so this module never sees the interstitial — but it still
//!    checks, because a silently-HTML "azw3" landing in the library would be a
//!    confusing failure to diagnose later.
//! 2. The framework does not notice a new file on its own. `touch`ing
//!    `/mnt/us/system/.cleanindex` is what makes it index one, and without that
//!    the book simply never appears on the home screen.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// MOBI8 magic at offset 0x3c of a PalmDOC header. An azw3 is a MOBI8
/// container, so this is present in every file SE produces.
const BOOKMOBI_OFFSET: usize = 0x3c;
const BOOKMOBI_MAGIC: &[u8] = b"BOOKMOBI";

/// Touching this is what tells the framework to re-index `documents/`.
pub const CLEANINDEX: &str = "/mnt/us/system/.cleanindex";

#[derive(Debug)]
pub enum Error {
    /// The bytes are not a Kindle book. In practice this means SE served an
    /// interstitial or an error page under a `.azw3` name.
    NotAnEbook,
    Io(io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotAnEbook => write!(f, "downloaded file is not a Kindle book"),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

/// Does this look like a real azw3?
pub fn is_ebook(bytes: &[u8]) -> bool {
    bytes
        .get(BOOKMOBI_OFFSET..BOOKMOBI_OFFSET + BOOKMOBI_MAGIC.len())
        .is_some_and(|m| m == BOOKMOBI_MAGIC)
}

/// Write `bytes` into `dir` as `file_name`, verifying first.
///
/// The write is atomic: bytes land in a `.partial` sibling and are renamed over
/// the target. The user partition is FAT, so a crash or a yanked USB cable
/// mid-write would otherwise leave a truncated file that the framework happily
/// indexes as a corrupt book.
pub fn commit(dir: &Path, file_name: &str, bytes: &[u8]) -> Result<PathBuf, Error> {
    if !is_ebook(bytes) {
        return Err(Error::NotAnEbook);
    }
    fs::create_dir_all(dir)?;
    let dest = dir.join(file_name);
    let partial = dir.join(format!("{file_name}.partial"));
    fs::write(&partial, bytes)?;
    fs::rename(&partial, &dest)?;
    Ok(dest)
}

/// Ask the framework to index what we just wrote. Best-effort: a failure here
/// means the book is on disk but not yet on the home screen, which the next
/// reboot fixes — never worth failing a completed download over.
pub fn request_reindex() {
    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(CLEANINDEX);
}

/// Names already present in the download directory, so the grid can mark books
/// the user has taken. Keyed on SE's own filename, which is stable per book.
pub fn existing_files(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".azw3"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smallest byte string that satisfies the magic check.
    fn fake_azw3() -> Vec<u8> {
        let mut v = vec![0u8; BOOKMOBI_OFFSET];
        v.extend_from_slice(BOOKMOBI_MAGIC);
        v.extend_from_slice(b"...rest of book...");
        v
    }

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("steb-test-{name}"));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn the_real_interstitial_is_rejected() {
        // The exact page the bare .azw3 URL serves — this is the failure the
        // magic check exists to catch.
        let interstitial = include_bytes!("../../tests/fixtures/download-interstitial.html");
        assert!(!is_ebook(interstitial));
    }

    #[test]
    fn a_mobi8_container_is_accepted() {
        assert!(is_ebook(&fake_azw3()));
    }

    #[test]
    fn commit_writes_the_file_and_leaves_no_partial() {
        let dir = tmpdir("commit");
        let path = commit(&dir, "a_b.azw3", &fake_azw3()).unwrap();
        assert!(path.exists());
        assert!(
            !dir.join("a_b.azw3.partial").exists(),
            "partial must be renamed away, not left behind"
        );
        assert_eq!(existing_files(&dir), vec!["a_b.azw3".to_string()]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bad_download_never_reaches_the_library() {
        let dir = tmpdir("reject");
        let err = commit(&dir, "a_b.azw3", b"<!DOCTYPE html>");
        assert!(matches!(err, Err(Error::NotAnEbook)));
        assert!(
            existing_files(&dir).is_empty(),
            "nothing should be written when verification fails"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scanning_a_missing_directory_is_empty_not_an_error() {
        assert!(existing_files(Path::new("/nonexistent/steb")).is_empty());
    }
}
