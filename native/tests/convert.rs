//! [`convert::Converter::convert`] against a `#!/bin/sh` stand-in for bokai.
//! Unix-only: [`Tree::bokai`] sets mode bits.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use steb_native::convert::{self, Error};

/// A scratch download directory, emptied by [`Tree::new`] and [`Tree::drop`].
struct Tree(PathBuf);

impl Tree {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("steb-convert-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Tree(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// A `bram-stoker_dracula.azw3` in this tree.
    fn azw3(&self) -> PathBuf {
        let path = self.path("bram-stoker_dracula.azw3");
        fs::write(&path, b"...a book...").unwrap();
        path
    }

    /// A `bokai` exiting 0 on `--version`, running `body` on any other argv.
    /// `body` sees that argv as `$1..`.
    fn bokai(&self, body: &str) -> convert::Converter {
        let exe = self.path("bokai");
        fs::write(
            &exe,
            format!("#!/bin/sh\nif [ \"$1\" = --version ]; then exit 0; fi\n{body}\n"),
        )
        .unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        convert::locate_at(&exe).expect("a runnable bokai")
    }

    /// Names in the tree, sorted, minus `bokai` and `argv`.
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.0)
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n != "bokai" && n != "argv")
            .collect();
        names.sort();
        names
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_conversion_takes_the_azw3s_place() {
    let t = Tree::new("replace");
    let bokai = t.bokai(r#"shift 3; printf 'kfx' > "$2""#);
    let azw3 = t.azw3();

    let kfx = bokai.convert(&azw3).unwrap();

    assert_eq!(kfx, t.path("bram-stoker_dracula.kfx"));
    assert_eq!(fs::read(&kfx).unwrap(), b"kfx");
    assert_eq!(
        t.names(),
        ["bram-stoker_dracula.kfx"],
        "the azw3 and the partial should both be gone"
    );
}

#[test]
fn the_call_names_the_format_the_staged_name_cannot() {
    let t = Tree::new("argv");
    let log = t.path("argv");
    let bokai = t.bokai(&format!(
        "printf '%s\\n' \"$@\" > {}; shift 3; printf 'kfx' > \"$2\"",
        log.display()
    ));
    let azw3 = t.azw3();

    bokai.convert(&azw3).unwrap();

    let argv: Vec<String> = fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(
        argv,
        [
            "convert".to_string(),
            "-t".to_string(),
            "kfx".to_string(),
            azw3.display().to_string(),
            t.path("bram-stoker_dracula.kfx.partial")
                .display()
                .to_string(),
        ]
    );
}

#[test]
fn a_failed_conversion_leaves_the_book_alone() {
    let t = Tree::new("failed");
    let bokai = t.bokai(r#"shift 3; printf 'half' > "$2"; exit 1"#);
    let azw3 = t.azw3();

    let err = bokai.convert(&azw3).unwrap_err();

    assert!(matches!(err, Error::NoOutput(_)), "{err}");
    assert_eq!(
        t.names(),
        ["bram-stoker_dracula.azw3"],
        "the download must survive, and the half-written container must not"
    );
}

#[test]
fn a_zero_exit_with_no_container_is_still_a_failure() {
    let t = Tree::new("silent");
    let bokai = t.bokai("exit 0");
    let azw3 = t.azw3();

    assert!(matches!(
        bokai.convert(&azw3).unwrap_err(),
        Error::NoOutput(_)
    ));
    assert_eq!(t.names(), ["bram-stoker_dracula.azw3"]);
}

#[test]
fn a_partial_from_an_earlier_run_is_never_renamed_into_place() {
    let t = Tree::new("stale");
    let stale = t.path("bram-stoker_dracula.kfx.partial");
    fs::write(&stale, b"...from a run that was killed...").unwrap();

    let bokai = t.bokai("exit 1");
    let azw3 = t.azw3();

    assert!(bokai.convert(&azw3).is_err());
    assert_eq!(
        t.names(),
        ["bram-stoker_dracula.azw3"],
        "the stale partial must be cleared, not promoted to a book"
    );
}

#[test]
fn the_converter_has_to_run_before_it_counts_as_installed() {
    let t = Tree::new("locate");
    let exe = t.path("bokai");

    assert_eq!(convert::locate_at(&exe), None, "nothing there");

    fs::write(&exe, "#!/bin/sh\nexit 0\n").unwrap();
    assert_eq!(convert::locate_at(&exe), None, "not executable");

    fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(convert::locate_at(&exe).is_some_and(|c| c.exe() == exe));

    fs::write(&exe, "#!/bin/sh\nexit 1\n").unwrap();
    assert_eq!(convert::locate_at(&exe), None, "--version failed");
}

#[test]
fn a_missing_directory_is_an_error_not_a_replaced_book() {
    let t = Tree::new("missing");
    let bokai = t.bokai(r#"shift 3; printf 'kfx' > "$2""#);
    assert!(
        bokai
            .convert(Path::new("/nonexistent/steb/book.azw3"))
            .is_err()
    );
}
