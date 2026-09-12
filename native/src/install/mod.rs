//! [`STEB`] and [`BOKAI`], from GitHub releases.
//!
//! [`run`] takes the newest release carrying [`Source::asset`], checks the
//! download against its `.sha256` sidecar, unpacks it under [`STAGING`], and
//! passes that copy to [`Source::verify`].
//!
//! [`place`] moves each unpacked file over its counterpart under
//! [`Source::dest`], [`Source::marker`] last. A path the archive does not
//! carry keeps its contents, [`EXTENSION_DIR`]`/cache` included.

pub mod archive;
pub mod http;

use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::convert;
use crate::log;

/// Releases to look through, newest first.
const RELEASES_PER_PAGE: u32 = 30;

/// This build's version, as `Cargo.toml` spells it: `0.2.1`, never `v0.2.1`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// [`STEB`]'s folder, holding `crate::cache` as well as the binary.
pub const EXTENSION_DIR: &str = "/mnt/us/extensions/steb";
/// [`STEB`]'s releases page, as [`by_hand`] and [`Outcome::message`] print it.
pub const RELEASES_URL: &str = "github.com/huangziwei/steb/releases";

/// Paths [`fetch`] writes under [`Source::dest`], on the one filesystem
/// [`replace`] renames across.
const ARCHIVE: &str = ".new.zip";
const STAGING: &str = ".new";

/// [`STEB`]'s [`Source::marker`], and the path its [`Source::verify`] runs.
const STEB_MARKER: &str = "bin/steb";

/// What can be fetched, and what a staged copy has to pass.
pub struct Source {
    /// The name [`Outcome::message`] prints.
    pub name: &'static str,
    /// `owner/repo`, for [`available`]'s API path.
    pub repo: &'static str,
    /// The releases page [`by_hand`] prints.
    pub releases: &'static str,
    /// The asset's name, `*` standing for the version it carries.
    pub asset_name: &'static str,
    /// Which asset of a release this installs, by filename.
    pub asset: fn(&str) -> bool,
    /// The archive entry [`archive::prefix_for`] reads, and [`place`]'s last
    /// move.
    pub marker: &'static str,
    /// Where the unpacked copy lands.
    pub dest: &'static str,
    /// Whether the copy staged at this path runs here, at this version.
    pub verify: fn(&Path, &str) -> bool,
    /// The version [`run_into`] compares [`Release::version`] against.
    pub installed: fn() -> Option<String>,
}

/// steb itself. `documents/Steb.sh` and `LICENSE` sit outside
/// [`STEB_MARKER`]'s root in the archive, and [`archive::unpack`] writes
/// neither.
pub static STEB: Source = Source {
    name: "Steb",
    repo: "huangziwei/steb",
    releases: RELEASES_URL,
    asset_name: "steb-*-kindle.zip",
    asset: |name| between(name, "steb-", "-kindle.zip").is_some(),
    marker: STEB_MARKER,
    dest: EXTENSION_DIR,
    verify: |dir, version| states_version(&dir.join(STEB_MARKER), version),
    installed: || Some(VERSION.to_string()),
};

/// The converter, an add-on in an extension folder of its own.
pub static BOKAI: Source = Source {
    name: "bokai",
    repo: "huangziwei/sidle",
    releases: convert::RELEASES_URL,
    asset_name: convert::RELEASE_ASSET,
    // The version rides the filename; the tag names sidle.
    asset: |name| between(name, "bokai-", "-kindle.zip").is_some(),
    // The archive's root: `extensions/bokai/bin/bokai`.
    marker: "bin/bokai",
    dest: convert::EXTENSION_DIR,
    // `convert::locate_in` starting the copy is the whole of bokai's proof.
    verify: |dir, _version| convert::locate_in(&dir.join("bin")).is_some(),
    installed: convert::installed_version,
};

/// What `name` holds between `prefix` and `suffix`, when it has both and
/// something in between: `v0.2.0` out of `bokai-v0.2.0-kindle.zip`.
pub fn between<'a>(name: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    let middle = name.strip_prefix(prefix)?.strip_suffix(suffix)?;
    (!middle.is_empty()).then_some(middle)
}

/// The release a [`Source`] installs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The version the asset's filename carries, falling back to the tag.
    pub version: String,
    /// The asset to download.
    pub url: String,
    pub name: String,
    /// A `.sha256` sidecar, where the release publishes one.
    pub sha: Option<String>,
}

/// One release as the GitHub API serves it, cut to what [`pick_release`] reads.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ApiRelease {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<ApiAsset>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ApiAsset {
    pub name: String,
    pub browser_download_url: String,
}

/// Why an install did not go through. [`Failure::line`] gives each one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// GitHub could not be reached, or gave no answer.
    NoAnswer,
    /// No release the API lists carries [`Source::asset`].
    NoRelease,
    /// What arrived is not the archive the release published.
    BadDownload,
    /// [`Source::verify`] refused the staged copy.
    WrongBuild,
    /// [`place`] could not move a file into [`Source::dest`].
    NotPlaced,
}

impl Failure {
    /// The one line saying what went wrong.
    pub fn line(self) -> &'static str {
        match self {
            Failure::NoAnswer => "GitHub did not answer",
            Failure::NoRelease => "no release carries it",
            Failure::BadDownload => "the download is not the archive",
            Failure::WrongBuild => "that build does not run here",
            Failure::NotPlaced => "the folder would not take it",
        }
    }
}

/// How an install ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// `crate::net::is_offline` answered true.
    Offline,
    /// [`Source::installed`] is at or past [`Release::version`].
    UpToDate(String),
    /// In place, at this version.
    Installed(String),
    /// `cancel` was set mid-transfer.
    Stopped,
    Failed(Failure),
}

impl Outcome {
    /// Two lines of banner text: what happened, and what to do.
    pub fn message(&self, source: &Source) -> String {
        let name = source.name;
        match self {
            Outcome::Offline => format!("{name}\nNo Wi-Fi — turn it on and try again"),
            Outcome::UpToDate(version) => format!("{name} {version}\nAlready the newest release"),
            Outcome::Installed(version) => {
                format!("{name} {version} is in place\nClose Steb and open it again")
            }
            Outcome::Stopped => format!("{name}\nStopped"),
            Outcome::Failed(why) => {
                format!("{name}: {}\nOr fetch it at {}", why.line(), source.releases)
            }
        }
    }

    /// Whether this is [`Outcome::Installed`].
    pub fn is_installed(&self) -> bool {
        matches!(self, Outcome::Installed(_))
    }
}

//------------------------------------------------------------------------------
// Pure: what to fetch, and what to check it against
//------------------------------------------------------------------------------

/// The newest release carrying `source`'s asset.
///
/// A `draft` release is never read. A `prerelease` one is taken only when no
/// release with `prerelease` false carries the asset.
pub fn pick_release(releases: &[ApiRelease], source: &Source) -> Option<Release> {
    newest(releases, source, false).or_else(|| newest(releases, source, true))
}

fn newest(releases: &[ApiRelease], source: &Source, prereleases: bool) -> Option<Release> {
    for release in releases {
        if release.draft || release.prerelease != prereleases {
            continue;
        }
        // A tag published before its assets carries none.
        let Some(found) = release.assets.iter().find(|a| (source.asset)(&a.name)) else {
            continue;
        };
        let sidecar = format!("{}.sha256", found.name);
        return Some(Release {
            version: version_of(&found.name, &release.tag_name, source),
            url: found.browser_download_url.clone(),
            name: found.name.clone(),
            sha: release
                .assets
                .iter()
                .find(|a| a.name == sidecar)
                .map(|a| a.browser_download_url.clone()),
        });
    }
    None
}

/// The version `asset` carries between the two halves of
/// [`Source::asset_name`], falling back to `tag`.
fn version_of(asset: &str, tag: &str, source: &Source) -> String {
    let (prefix, suffix) = source
        .asset_name
        .split_once('*')
        .unwrap_or((source.asset_name, ""));
    between(asset, prefix, suffix).unwrap_or(tag).to_string()
}

/// A version as its dotted numbers. A leading `v` and anything trailing a
/// number — `-rc1`, `+build` — are not read.
fn numbers(version: &str) -> Vec<u64> {
    version
        .trim()
        .trim_start_matches(['v', 'V'])
        .split('.')
        .map(|part| {
            part.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        })
        .collect()
}

/// Whether `version` carries anything past its dotted numbers, as `0.2.5-dev`
/// does.
fn prerelease(version: &str) -> bool {
    version
        .trim()
        .trim_start_matches(['v', 'V'])
        .split('.')
        .any(|part| !part.chars().all(|c| c.is_ascii_digit()))
}

/// Is `offered` a later version than `running`? A missing part reads as zero,
/// and equal numbers are not later. Over equal numbers a [`prerelease`]
/// `running` sits below an untagged `offered`.
pub fn newer(offered: &str, running: &str) -> bool {
    let (a, b) = (numbers(offered), numbers(running));
    for at in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(at).copied().unwrap_or(0),
            b.get(at).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    prerelease(running) && !prerelease(offered)
}

/// The `sha256sum`-style line for `name`, or the whole file when it carries a
/// bare digest.
pub fn digest_from(text: &str, name: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        let mut parts = line.splitn(2, char::is_whitespace);
        let digest = parts.next().unwrap_or_default();
        if !is_sha256(digest) {
            continue;
        }
        match parts.next() {
            // A file holding nothing but the digest.
            None => return Some(digest.to_ascii_lowercase()),
            Some(rest) => {
                // `sha256sum -b` marks a binary with a `*`, and a digest taken
                // in a build directory names the file by its whole path.
                let named = rest.trim_start().trim_start_matches('*');
                if named == name || named.rsplit('/').next() == Some(name) {
                    return Some(digest.to_ascii_lowercase());
                }
            }
        }
    }
    None
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `got` against `total`, as a percentage or a count.
fn transferred(got: u64, total: Option<u64>) -> String {
    match total {
        Some(total) if total > 0 => format!("{}%", got * 100 / total),
        // No `Content-Length`: the count is all there is to say.
        _ => format!("{:.1} MB", got as f64 / (1024.0 * 1024.0)),
    }
}

//------------------------------------------------------------------------------
// The whole flow
//------------------------------------------------------------------------------

/// Ask, fetch and install `source` into [`Source::dest`].
///
/// Blocks. `cancel` is read while the release list is awaited and between
/// chunks of the download; `say` takes the line under the banner's title.
pub fn run(source: &Source, cancel: &AtomicBool, say: &dyn Fn(String)) -> Outcome {
    run_into(source, Path::new(source.dest), cancel, say)
}

/// [`run`] against a named `dest`.
pub fn run_into(
    source: &Source,
    dest: &Path,
    cancel: &AtomicBool,
    say: &dyn Fn(String),
) -> Outcome {
    if crate::net::is_offline() {
        return Outcome::Offline;
    }
    say("Asking GitHub…".into());

    let client = http::Client::new();
    let release = match available(&client, source) {
        Ok(release) => release,
        Err(why) => {
            by_hand(source);
            return Outcome::Failed(why);
        }
    };
    // `cancel` set while the release list was awaited.
    if cancel.load(Ordering::Relaxed) {
        return Outcome::Stopped;
    }

    // `Source::installed` answering `None` falls through to `fetch`.
    let installed = (source.installed)();
    log(format!(
        "install: {} here {:?}, newest {}",
        source.name, installed, release.version
    ));
    if let Some(have) = installed
        && !newer(&release.version, &have)
    {
        return Outcome::UpToDate(have);
    }

    match fetch(&client, source, &release, dest, say, cancel) {
        Ok(()) => {
            log(format!(
                "install: {} {} into {}",
                source.name,
                release.version,
                dest.display()
            ));
            Outcome::Installed(release.version)
        }
        Err(None) => Outcome::Stopped,
        Err(Some(why)) => {
            log(format!("install: {} failed — {}", source.name, why.line()));
            by_hand(source);
            Outcome::Failed(why)
        }
    }
}

/// `source`'s asset name, releases page and destination, into the log.
fn by_hand(source: &Source) {
    log(format!(
        "install: by hand, unzip {} from {} over {}",
        source.asset_name, source.releases, source.dest
    ));
}

/// The release `source` installs.
pub fn available(client: &http::Client, source: &Source) -> Result<Release, Failure> {
    let url = format!(
        "https://api.github.com/repos/{}/releases?per_page={RELEASES_PER_PAGE}",
        source.repo
    );
    let body = client
        .text(&url, "application/vnd.github+json")
        .map_err(|e| {
            log(format!("install: {} — {e}", source.name));
            Failure::NoAnswer
        })?;

    let releases: Vec<ApiRelease> = serde_json::from_str(&body).map_err(|e| {
        log(format!(
            "install: {} — unreadable release list: {e}",
            source.name
        ));
        Failure::NoAnswer
    })?;

    pick_release(&releases, source).ok_or(Failure::NoRelease)
}

/// Download, unpack, prove and move in. `Err(None)` is a tap on Cancel.
///
/// Nothing under `dest` is written until [`Source::verify`] has passed on the
/// copy under [`STAGING`].
fn fetch(
    client: &http::Client,
    source: &Source,
    release: &Release,
    dest: &Path,
    say: &dyn Fn(String),
    cancel: &AtomicBool,
) -> Result<(), Option<Failure>> {
    let staging = dest.join(STAGING);
    let zip = dest.join(ARCHIVE);
    let _ = fs::remove_dir_all(&staging);
    let _ = fs::remove_file(&zip);

    // `last` holds the banner to one repaint every other percent.
    let last = std::cell::Cell::new(u64::MAX);
    let progress = |got: u64, total: Option<u64>| {
        let mark = match total {
            Some(total) if total > 0 => got * 50 / total,
            _ => got / (512 * 1024),
        };
        if mark != last.get() {
            last.set(mark);
            say(format!("Downloading  {}", transferred(got, total)));
        }
    };
    say("Downloading…".into());
    if let Err(e) = client.download(&release.url, &zip, cancel, &progress) {
        log(format!("install: {} — {e}", source.name));
        return Err(match e {
            http::Error::Cancelled => None,
            _ => Some(Failure::BadDownload),
        });
    }

    say("Checking it…".into());
    if let Some(sidecar) = &release.sha
        && !matches(client, sidecar, &release.name, &zip)
    {
        let _ = fs::remove_file(&zip);
        return Err(Some(Failure::BadDownload));
    }

    let unpacked = archive::unpack(&zip, source.marker, &staging);
    let _ = fs::remove_file(&zip);
    match unpacked {
        Ok(written) => log(format!(
            "install: {} — {written} files into {}",
            source.name,
            staging.display()
        )),
        Err(e) => {
            log(format!("install: {} — {e}", source.name));
            let _ = fs::remove_dir_all(&staging);
            return Err(Some(Failure::BadDownload));
        }
    }

    // The modes a zip carries depend on what wrote it.
    mark_executable(&staging.join("bin"));

    if !(source.verify)(&staging, &release.version) {
        let _ = fs::remove_dir_all(&staging);
        return Err(Some(Failure::WrongBuild));
    }

    say("Putting it in place…".into());
    let placed = place(&staging, dest, source.marker);
    let _ = fs::remove_dir_all(&staging);
    placed.map_err(Some)
}

/// Does `exe` start here, and does it print `version`?
///
/// `--version` opens neither the framebuffer nor the log, and settles the
/// float ABI, that `exe` starts at all, and which release the archive holds.
fn states_version(exe: &Path, version: &str) -> bool {
    let Ok(out) = Command::new(exe).arg("--version").output() else {
        log("install: the staged copy would not run");
        return false;
    };
    let said = String::from_utf8_lossy(&out.stdout).trim().to_string();
    // `newer` answers false both ways on one version.
    let same = !newer(&said, version) && !newer(version, &said);
    if !out.status.success() || !same {
        log(format!(
            "install: staged copy says {said:?}, release says {version:?}"
        ));
        return false;
    }
    true
}

/// Every file under `staging` over its counterpart in `dest`. `marker` goes
/// last: [`STEB_MARKER`] is this process's own binary.
fn place(staging: &Path, dest: &Path, marker: &str) -> Result<(), Failure> {
    let mut files = Vec::new();
    walk(staging, Path::new(""), &mut files);
    files.sort();
    files.sort_by_key(|rel| rel == Path::new(marker));

    for rel in &files {
        let to = dest.join(rel);
        if let Some(parent) = to.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if !replace(&staging.join(rel), &to) {
            log(format!(
                "install: {} would not go into place",
                rel.display()
            ));
            return Err(Failure::NotPlaced);
        }
    }
    Ok(())
}

/// Every file under `dir`, as a path relative to `at`.
fn walk(dir: &Path, at: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let rel = at.join(entry.file_name());
        match path.is_dir() {
            true => walk(&path, &rel, out),
            false => out.push(rel),
        }
    }
}

/// `from` over `to`, whether or not anything stands at `to`. The copy at `to`
/// is renamed aside first, and put back where the move in fails.
fn replace(from: &Path, to: &Path) -> bool {
    let aside = PathBuf::from(format!("{}.old", to.display()));
    let _ = fs::remove_file(&aside);
    let stood = to.exists();
    if stood && fs::rename(to, &aside).is_err() {
        return false;
    }
    if fs::rename(from, to).is_ok() {
        // A copy held open cannot always be unlinked.
        let _ = fs::remove_file(&aside);
        return true;
    }
    if stood {
        let _ = fs::rename(&aside, to);
    }
    false
}

/// The downloaded file against the digest the release publishes for it. A
/// sidecar that cannot be read is not a mismatch.
fn matches(client: &http::Client, sidecar: &str, name: &str, zip: &Path) -> bool {
    let Ok(text) = client.text(sidecar, "text/plain") else {
        log("install: the checksum could not be read");
        return true;
    };
    let Some(want) = digest_from(&text, name) else {
        log("install: the checksum names no digest for this file");
        return true;
    };
    let Some(got) = digest_of(zip) else {
        log("install: the download could not be read back");
        return true;
    };
    if want != got {
        log(format!("install: checksum wanted {want}, got {got}"));
        return false;
    }
    true
}

/// SHA-256 of `path`, hex.
fn digest_of(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};

    let mut file = fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    )
}

/// Every file directly under `dir`, executable. Best effort on FAT.
pub fn mark_executable(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        if meta.is_file() {
            let mut perms = meta.permissions();
            perms.set_mode(perms.mode() | 0o755);
            let _ = fs::set_permissions(&path, perms);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> ApiAsset {
        ApiAsset {
            name: name.to_string(),
            browser_download_url: format!("https://example.invalid/{name}"),
        }
    }

    fn release(tag: &str, assets: &[&str]) -> ApiRelease {
        ApiRelease {
            tag_name: tag.to_string(),
            draft: false,
            prerelease: false,
            assets: assets.iter().map(|n| asset(n)).collect(),
        }
    }

    #[test]
    fn each_source_names_only_its_own_asset() {
        assert!((STEB.asset)("steb-v0.3.0-kindle.zip"));
        assert!(!(STEB.asset)("steb-v0.3.0-kindle.zip.sha256"));
        assert!(!(STEB.asset)("bokai-v0.2.0-kindle.zip"));
        // `steb-feat-x-kindle.zip` is the shape a `/`-bearing tag squashes to.
        assert!((STEB.asset)("steb-feat-x-kindle.zip"));

        assert!((BOKAI.asset)("bokai-v0.2.0-kindle.zip"));
        assert!((BOKAI.asset)("bokai-v0.1.10-kindle.zip"));
        assert!(!(BOKAI.asset)("bokai-v0.2.0-kindle.zip.sha256"));
        assert!(!(BOKAI.asset)("sidle-v0.2.0-kindle.zip"));
        assert!(!(BOKAI.asset)("steb-v0.3.0-kindle.zip"));
    }

    #[test]
    fn each_source_names_the_page_its_release_comes_from() {
        // `convert` owns bokai's two strings; nothing here restates them.
        assert_eq!(BOKAI.releases, convert::RELEASES_URL);
        assert_eq!(BOKAI.asset_name, convert::RELEASE_ASSET);
        assert_eq!(BOKAI.dest, convert::EXTENSION_DIR);
        assert_eq!(STEB.dest, EXTENSION_DIR);
        // The marker names a file inside the folder it installs into.
        for source in [&STEB, &BOKAI] {
            assert!(source.marker.starts_with("bin/"), "{}", source.name);
            assert!(source.asset_name.contains('*'), "{}", source.name);
        }
    }

    #[test]
    fn the_newest_release_carrying_the_asset_is_the_one_taken() {
        let list = [
            // Published ahead of its assets: skipped for the one below it.
            release("v0.4.0", &[]),
            release(
                "v0.3.0",
                &["steb-v0.3.0-kindle.zip", "steb-v0.3.0-kindle.zip.sha256"],
            ),
            release("v0.2.0", &["steb-v0.2.0-kindle.zip"]),
        ];
        let picked = pick_release(&list, &STEB).unwrap();
        assert_eq!(picked.version, "v0.3.0");
        assert_eq!(picked.name, "steb-v0.3.0-kindle.zip");
        assert!(picked.sha.is_some_and(|s| s.ends_with(".sha256")));
    }

    #[test]
    fn bokais_version_is_the_filenames_not_the_sidle_tag() {
        let list = [release(
            "bokai-v0.2.0",
            &["bokai-v0.2.0-kindle.zip", "sidle-v1.0.0-kindle.zip"],
        )];
        let picked = pick_release(&list, &BOKAI).unwrap();
        assert_eq!(picked.version, "v0.2.0");
        assert_eq!(picked.name, "bokai-v0.2.0-kindle.zip");
        // No sidecar in this release, and that is not a failure.
        assert_eq!(picked.sha, None);
    }

    #[test]
    fn a_draft_is_never_read_and_a_prerelease_only_as_a_last_resort() {
        let mut draft = release("v9.9.9", &["steb-v9.9.9-kindle.zip"]);
        draft.draft = true;
        let mut rc = release("v0.4.0-rc1", &["steb-v0.4.0-rc1-kindle.zip"]);
        rc.prerelease = true;
        let full = release("v0.3.0", &["steb-v0.3.0-kindle.zip"]);

        // A full release outranks a newer prerelease.
        assert_eq!(
            pick_release(&[draft.clone(), rc.clone(), full], &STEB)
                .unwrap()
                .version,
            "v0.3.0"
        );
        // A prerelease alone is reachable.
        assert_eq!(
            pick_release(&[rc, draft], &STEB).unwrap().version,
            "v0.4.0-rc1"
        );
        assert_eq!(pick_release(&[], &STEB), None);
        // A release carrying only the other project's asset is not this one's.
        assert_eq!(
            pick_release(&[release("v1", &["bokai-v1-kindle.zip"])], &STEB),
            None
        );
    }

    #[test]
    fn a_later_release_is_offered_and_this_build_is_not_offered_to_itself() {
        assert!(newer("v0.3.0", "0.2.1"));
        assert!(newer("v0.2.2", "0.2.1"));
        assert!(newer("v1.0.0", "0.9.9"));
        assert!(!newer("v0.2.1", "0.2.1"));
        assert!(!newer("v0.2.0", "0.2.1"));
        // A downgrade is never offered, whichever side carries the `v`.
        assert!(!newer("0.2.0", "v0.2.1"));
        // A missing part reads as zero.
        assert!(!newer("v0.3", "0.3.0"));
        assert!(newer("v0.3.1", "0.3"));
        // A release outranks the same numbers with a suffix, never the reverse.
        assert!(newer("v0.3.0", "0.3.0-dev"));
        assert!(!newer("v0.3.0-rc1", "0.3.0"));
        assert!(!newer("v0.3.0-rc2", "0.3.0-rc1"));
    }

    #[test]
    fn this_build_names_a_version_that_reads_as_one() {
        assert!(!numbers(VERSION).is_empty());
        assert!(!prerelease(VERSION), "{VERSION}");
        // Cargo carries no `v`; a release tag does.
        assert!(!VERSION.starts_with('v'));
        assert!(newer(&format!("v{VERSION}.1"), VERSION));
    }

    #[test]
    fn a_digest_is_read_off_whichever_shape_the_sidecar_takes() {
        let d = "a".repeat(64);
        let name = "steb-v0.3.0-kindle.zip";
        assert_eq!(digest_from(&format!("{d}  {name}"), name), Some(d.clone()));
        // `sha256sum -b` marks a binary file.
        assert_eq!(digest_from(&format!("{d} *{name}"), name), Some(d.clone()));
        // A digest taken in a build directory names the whole path.
        assert_eq!(
            digest_from(&format!("{d}  /tmp/build/{name}"), name),
            Some(d.clone())
        );
        // A file holding nothing but the digest is for whatever it came with.
        assert_eq!(digest_from(&format!("{d}\n"), name), Some(d.clone()));
        assert_eq!(
            digest_from(&format!("{}  f.zip", "A".repeat(64)), "f.zip"),
            Some("a".repeat(64))
        );
        assert_eq!(digest_from("abc  f.zip", "f.zip"), None);
        assert_eq!(digest_from("", "f.zip"), None);
        // A sidecar for a different file is not this file's digest.
        assert_eq!(digest_from(&format!("{d}  other.zip"), name), None);
    }

    #[test]
    fn the_transferred_line_prefers_a_percentage_and_falls_back_to_a_count() {
        assert_eq!(transferred(0, Some(200)), "0%");
        assert_eq!(transferred(100, Some(200)), "50%");
        assert_eq!(transferred(200, Some(200)), "100%");
        // No `Content-Length`, or one that says nothing.
        assert_eq!(transferred(1024 * 1024, None), "1.0 MB");
        assert_eq!(transferred(1024 * 1024, Some(0)), "1.0 MB");
    }

    #[test]
    fn every_outcome_says_which_add_on_it_is_about() {
        for outcome in [
            Outcome::Offline,
            Outcome::UpToDate("0.2.1".into()),
            Outcome::Installed("v0.3.0".into()),
            Outcome::Stopped,
            Outcome::Failed(Failure::NoAnswer),
        ] {
            let msg = outcome.message(&STEB);
            assert!(msg.contains(STEB.name), "{msg}");
            assert_eq!(msg.lines().count(), 2, "{msg}");
        }
        assert!(Outcome::Installed("v0.3.0".into()).is_installed());
        assert!(!Outcome::UpToDate("0.2.1".into()).is_installed());
        // A failure names `Source::releases`.
        let failed = Outcome::Failed(Failure::WrongBuild).message(&BOKAI);
        assert!(failed.contains(convert::RELEASES_URL), "{failed}");
    }

    #[test]
    fn a_file_hashes_to_what_sha256sum_would_say() {
        let path = std::env::temp_dir().join("steb-install-digest.txt");
        fs::write(&path, b"abc").unwrap();
        assert_eq!(
            digest_of(&path).as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        let _ = fs::remove_file(&path);
        assert_eq!(digest_of(Path::new("/nonexistent/steb")), None);
    }

    /// `place` moves files over their counterparts and leaves everything the
    /// archive does not carry.
    #[test]
    fn an_update_replaces_the_files_it_ships_and_keeps_the_rest() {
        let root = std::env::temp_dir().join("steb-install-place");
        let _ = fs::remove_dir_all(&root);
        let (dest, staging) = (root.join("steb"), root.join("steb/.new"));
        fs::create_dir_all(dest.join("bin")).unwrap();
        fs::create_dir_all(dest.join("cache/covers")).unwrap();
        fs::create_dir_all(staging.join("bin")).unwrap();

        fs::write(dest.join("bin/steb"), "old binary").unwrap();
        fs::write(dest.join("config.xml"), "old config").unwrap();
        fs::write(dest.join("cache/catalogue.json"), "{}").unwrap();
        fs::write(dest.join("cache/covers/a.jpg"), "cover").unwrap();
        fs::write(staging.join("bin/steb"), "new binary").unwrap();
        fs::write(staging.join("config.xml"), "new config").unwrap();

        place(&staging, &dest, STEB.marker).unwrap();

        assert_eq!(
            fs::read_to_string(dest.join("bin/steb")).unwrap(),
            "new binary"
        );
        assert_eq!(
            fs::read_to_string(dest.join("config.xml")).unwrap(),
            "new config"
        );
        // Neither cache is in the archive.
        assert_eq!(
            fs::read_to_string(dest.join("cache/catalogue.json")).unwrap(),
            "{}"
        );
        assert_eq!(
            fs::read_to_string(dest.join("cache/covers/a.jpg")).unwrap(),
            "cover"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// `STEB_MARKER` sorts after every other staged file.
    #[test]
    fn the_marker_is_the_last_file_moved() {
        let root = std::env::temp_dir().join("steb-install-order");
        let _ = fs::remove_dir_all(&root);
        let staging = root.join(".new");
        fs::create_dir_all(staging.join("bin")).unwrap();
        for name in ["bin/steb", "bin/steb.sh", "config.xml", "menu.json"] {
            fs::write(staging.join(name), name).unwrap();
        }

        let mut files = Vec::new();
        walk(&staging, Path::new(""), &mut files);
        files.sort();
        files.sort_by_key(|rel| rel == Path::new(STEB.marker));
        assert_eq!(files.last().unwrap(), Path::new(STEB.marker));
        assert_eq!(files.len(), 4);
        let _ = fs::remove_dir_all(&root);
    }

    /// A first install lands where there was nothing, and a second one over
    /// what the first left.
    #[test]
    fn a_file_goes_into_place_whether_or_not_one_stood_there() {
        let root = std::env::temp_dir().join("steb-install-replace");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let (from, to) = (root.join("from"), root.join("to"));

        fs::write(&from, "first").unwrap();
        assert!(replace(&from, &to));
        assert_eq!(fs::read_to_string(&to).unwrap(), "first");
        assert!(!from.exists(), "the staged copy is moved, not copied");

        fs::write(&from, "second").unwrap();
        assert!(replace(&from, &to));
        assert_eq!(fs::read_to_string(&to).unwrap(), "second");

        // Nothing to move leaves what stood there alone.
        assert!(!replace(&root.join("absent"), &to));
        assert_eq!(fs::read_to_string(&to).unwrap(), "second");
        let _ = fs::remove_dir_all(&root);
    }
}
