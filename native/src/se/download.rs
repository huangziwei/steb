//! An azw3 verified by [`is_ebook`], committed to `dir` by [`commit`], and
//! indexed on the next [`request_reindex`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Offset of the MOBI8 magic in a PalmDOC header.
const BOOKMOBI_OFFSET: usize = 0x3c;
const BOOKMOBI_MAGIC: &[u8] = b"BOOKMOBI";

/// Appended to by [`request_reindex`] to re-index `documents/`.
pub const CLEANINDEX: &str = "/mnt/us/system/.cleanindex";

#[derive(Debug)]
pub enum Error {
    /// `bytes` carry no [`BOOKMOBI_MAGIC`]: an interstitial or an error page
    /// under a `.azw3` name.
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

/// [`BOOKMOBI_MAGIC`] at [`BOOKMOBI_OFFSET`].
pub fn is_ebook(bytes: &[u8]) -> bool {
    bytes
        .get(BOOKMOBI_OFFSET..BOOKMOBI_OFFSET + BOOKMOBI_MAGIC.len())
        .is_some_and(|m| m == BOOKMOBI_MAGIC)
}

/// [`is_ebook`] over `bytes`, then a `.partial` sibling in `dir` renamed to
/// `file_name`.
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

/// Appends to [`CLEANINDEX`]. A failed open is dropped.
pub fn request_reindex() {
    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(CLEANINDEX);
}

/// What [`commit`] writes, and what `crate::convert` leaves in its place.
pub const TAKEN_SUFFIXES: [&str; 2] = [".azw3", ".kfx"];

/// Names in `dir` ending in a [`TAKEN_SUFFIXES`] entry.
pub fn existing_files(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| TAKEN_SUFFIXES.iter().any(|s| n.ends_with(s)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shortest bytes [`is_ebook`] accepts.
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
    fn a_converted_book_reads_as_taken_and_a_partial_does_not() {
        let dir = tmpdir("converted");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("bram-stoker_dracula.kfx"), b"kfx").unwrap();
        fs::write(dir.join("homer_the-iliad.kfx.partial"), b"half").unwrap();

        assert_eq!(existing_files(&dir), vec!["bram-stoker_dracula.kfx"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scanning_a_missing_directory_is_empty_not_an_error() {
        assert!(existing_files(Path::new("/nonexistent/steb")).is_empty());
    }
}
