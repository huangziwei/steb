//! `bokai convert` over a downloaded `.azw3`, writing the `.kfx` beside it.
//! [`locate`] returning `None` leaves the `.azw3` in place.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

/// The bokai binary [`locate`] probes.
pub const BIN_PATH: &str = "/mnt/us/extensions/bokai/bin/bokai";

/// Extension of [`Converter::convert`]'s output.
const KFX: &str = "kfx";

#[derive(Debug)]
pub enum Error {
    /// A non-zero exit, or a zero exit with no file at the output path.
    NoOutput(ExitStatus),
    Io(io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoOutput(status) => write!(f, "bokai wrote no kfx ({status})"),
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

/// The binary [`locate_at`] resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converter {
    exe: PathBuf,
}

/// [`locate_at`] over [`BIN_PATH`].
pub fn locate() -> Option<Converter> {
    locate_at(Path::new(BIN_PATH))
}

/// `exe`, if it is a file whose `--version` exits 0.
pub fn locate_at(exe: &Path) -> Option<Converter> {
    let ok = exe.is_file()
        && Command::new(exe)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    ok.then(|| Converter {
        exe: exe.to_path_buf(),
    })
}

/// `azw3` under the [`KFX`] extension, stem unchanged.
pub fn output_path(azw3: &Path) -> PathBuf {
    azw3.with_extension(KFX)
}

/// The path [`Converter::convert`] writes before renaming it to `kfx`.
/// The `.partial` suffix matches no `se::download::TAKEN_SUFFIXES` entry.
fn staging_path(kfx: &Path) -> PathBuf {
    let mut name = kfx.file_name().unwrap_or_default().to_os_string();
    name.push(".partial");
    kfx.with_file_name(name)
}

impl Converter {
    /// The path [`Converter::convert`] spawns.
    pub fn exe(&self) -> &Path {
        &self.exe
    }

    /// `bokai convert -t kfx <azw3> <staged>` run to its exit, [`staging_path`]
    /// renamed to [`output_path`], `azw3` removed. `-t` names the format the
    /// `.partial` extension does not.
    pub fn convert(&self, azw3: &Path) -> Result<PathBuf, Error> {
        let kfx = output_path(azw3);
        let staged = staging_path(&kfx);
        // `staged` from an interrupted run.
        remove_if_present(&staged)?;

        let status = Command::new(&self.exe)
            .arg("convert")
            .args(["-t", KFX])
            .arg(azw3)
            .arg(&staged)
            .status();

        match status {
            Ok(s) if s.success() && staged.is_file() => {}
            Ok(s) => {
                let _ = remove_if_present(&staged);
                return Err(Error::NoOutput(s));
            }
            Err(e) => {
                let _ = remove_if_present(&staged);
                return Err(Error::Io(e));
            }
        }

        if let Err(e) = fs::rename(&staged, &kfx) {
            let _ = remove_if_present(&staged);
            return Err(Error::Io(e));
        }
        remove_if_present(azw3)?;
        Ok(kfx)
    }
}

/// `remove_file`, with an absent `path` reading as success.
fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kfx_keeps_the_azw3s_stem() {
        assert_eq!(
            output_path(Path::new("/d/bram-stoker_dracula.azw3")),
            Path::new("/d/bram-stoker_dracula.kfx")
        );
        assert_eq!(
            output_path(Path::new("/d/homer_the-iliad_alexander-pope.azw3")),
            Path::new("/d/homer_the-iliad_alexander-pope.kfx")
        );
    }

    #[test]
    fn the_staged_name_ends_in_neither_taken_suffix() {
        let staged = staging_path(Path::new("/d/bram-stoker_dracula.kfx"));
        assert_eq!(staged, Path::new("/d/bram-stoker_dracula.kfx.partial"));
        let name = staged.to_string_lossy();
        assert!(!name.ends_with(".kfx") && !name.ends_with(".azw3"));
    }

    #[test]
    fn a_missing_binary_resolves_to_no_converter() {
        assert_eq!(locate_at(Path::new("/nonexistent/bokai")), None);
    }
}
