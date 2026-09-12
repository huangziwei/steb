//! `bokai convert` over a downloaded `.azw3`, writing the `.kfx` beside it.
//! [`locate`] returning `None` leaves the `.azw3` in place.
//!
//! bokai is an add-on, not a dependency: it lives in an extension folder of
//! its own and `crate::install` fetches it from the release named by
//! [`RELEASES_URL`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// The add-on extension's root, beside this app's under `/mnt/us/extensions`.
pub const EXTENSION_DIR: &str = "/mnt/us/extensions/bokai";
/// Where the release archive installs bokai's builds.
pub const BIN_DIR: &str = "/mnt/us/extensions/bokai/bin";

/// Where `crate::install` fetches it from, and what the log names when that
/// fails and it has to be done on a computer instead.
pub const RELEASES_URL: &str = "github.com/huangziwei/sidle/releases";
/// The asset, `*` standing for the version. bokai versions on its own line and
/// moves without this app moving, so no one version belongs here.
pub const RELEASE_ASSET: &str = "bokai-*-kindle.zip";

/// bokai's two builds in [`locate_in`] order: hard-float first, soft-float
/// second. One zip carries both and a device starts one of them.
pub const ABI_VARIANTS: [&str; 2] = ["bokai", "bokai-armsf"];

/// The invocation that converts nothing and exits 0 on a build that runs here.
const VERSION_FLAG: &str = "--version";

/// How long [`Converter::convert_watched`] leaves between `try_wait` calls:
/// short enough that a cover arriving over this app is noticed promptly.
const POLL: Duration = Duration::from_millis(200);

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

/// [`locate_in`] over [`BIN_DIR`].
pub fn locate() -> Option<Converter> {
    locate_in(Path::new(BIN_DIR))
}

/// The first [`ABI_VARIANTS`] entry under `dir` that [`locate_at`] accepts.
///
/// Each variant targets a different float ABI, so at most one of them starts
/// on any one device.
pub fn locate_in(dir: &Path) -> Option<Converter> {
    ABI_VARIANTS
        .iter()
        .find_map(|name| locate_at(&dir.join(name)))
}

/// `exe`, if it is a file whose [`VERSION_FLAG`] exits 0.
///
/// The run costs one process and rules out a build for the wrong ABI, which
/// otherwise fails once per book with the panel mid-download.
pub fn locate_at(exe: &Path) -> Option<Converter> {
    let ok = exe.is_file()
        && Command::new(exe)
            .arg(VERSION_FLAG)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    ok.then(|| Converter {
        exe: exe.to_path_buf(),
    })
}

/// The version of the installed bokai, as it states itself.
///
/// `None` where nothing runnable is installed. Nothing else on the device
/// records it, so a copy unzipped by hand reports the same as one this app
/// fetched.
pub fn installed_version() -> Option<String> {
    locate().and_then(|c| c.version())
}

/// The version token of a `--version` line: `bokai 0.2.0` answers `0.2.0`.
///
/// clap prints `<name> <version>`, and bokai's own build stamp can put a
/// release tag in brackets after it. The first word opening on a digit is the
/// version in every one of those shapes.
fn version_token(said: &str) -> Option<&str> {
    said.split_whitespace()
        .find(|word| word.starts_with(|c: char| c.is_ascii_digit()))
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

    /// What `bokai --version` states, cut to the version itself.
    pub fn version(&self) -> Option<String> {
        let out = Command::new(&self.exe).arg(VERSION_FLAG).output().ok()?;
        let said = String::from_utf8_lossy(&out.stdout);
        version_token(said.trim()).map(str::to_string)
    }

    /// `bokai convert -t kfx <azw3> <staged>` run to its exit, [`staging_path`]
    /// renamed to [`output_path`], `azw3` removed. `-t` names the format the
    /// `.partial` extension does not.
    pub fn convert(&self, azw3: &Path) -> Result<PathBuf, Error> {
        self.convert_watched(azw3, |_| {})
    }

    /// [`Converter::convert`], running `tick` about every [`POLL`] with the
    /// elapsed time. A conversion runs for minutes, so it is polled rather
    /// than waited on: `tick` is the caller's chance to service the screen.
    pub fn convert_watched(
        &self,
        azw3: &Path,
        mut tick: impl FnMut(Duration),
    ) -> Result<PathBuf, Error> {
        let kfx = output_path(azw3);
        let staged = staging_path(&kfx);
        // `staged` from an interrupted run.
        remove_if_present(&staged)?;

        let started = Instant::now();
        let status = (|| -> std::io::Result<ExitStatus> {
            let mut child = Command::new(&self.exe)
                .arg("convert")
                .args(["-t", KFX])
                .arg(azw3)
                .arg(&staged)
                .stdin(Stdio::null())
                .spawn()?;
            loop {
                if let Some(status) = child.try_wait()? {
                    return Ok(status);
                }
                tick(started.elapsed());
                std::thread::sleep(POLL);
            }
        })();

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
        // Neither ABI build is there, so neither answers.
        assert_eq!(locate_in(Path::new("/nonexistent/bin")), None);
        assert_eq!(installed_version(), locate().and_then(|c| c.version()));
    }

    #[test]
    fn a_directory_at_the_path_resolves_to_no_converter() {
        assert_eq!(locate_at(&std::env::temp_dir()), None);
        assert_eq!(locate_in(&std::env::temp_dir()), None);
    }

    /// One zip carries both float ABIs and a device starts one of them, so
    /// both are probed rather than the hard-float one alone.
    #[test]
    fn both_abi_builds_are_probed_under_the_bin_folder() {
        assert_eq!(ABI_VARIANTS, ["bokai", "bokai-armsf"]);
        assert!(BIN_DIR.starts_with(EXTENSION_DIR));
        assert_eq!(BIN_DIR, format!("{EXTENSION_DIR}/bin"));
    }

    /// Whatever shape `bokai --version` takes, the version is the first word
    /// opening on a digit.
    #[test]
    fn the_version_is_read_out_of_the_line_bokai_prints() {
        assert_eq!(version_token("bokai 0.2.0"), Some("0.2.0"));
        // `build.rs` folds a release tag in after the version.
        assert_eq!(version_token("bokai 0.2.0 (v0.2.0)"), Some("0.2.0"));
        assert_eq!(version_token("bokai 0.1.10"), Some("0.1.10"));
        assert_eq!(version_token(""), None);
        assert_eq!(version_token("bokai"), None);
    }

    /// `exe` written without an execute bit.
    #[cfg(unix)]
    #[test]
    fn an_unrunnable_binary_resolves_to_no_converter() {
        let exe = std::env::temp_dir().join("steb-bokai-no-mode-bits");
        fs::write(&exe, "#!/bin/sh\nexit 0\n").unwrap();
        assert_eq!(locate_at(&exe), None);
        let _ = fs::remove_file(&exe);
    }

    #[test]
    fn a_staged_path_that_was_never_written_removes_clean() {
        let staged = staging_path(Path::new("/nonexistent/steb/bram-stoker_dracula.kfx"));
        assert!(remove_if_present(&staged).is_ok());
    }
}
