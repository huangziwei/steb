//! One line to stderr, which `bin/steb.sh` redirects into `/mnt/us/logs`.
//! Never that file directly: writing it here too doubles every line.

/// A timestamped line, for the run loop and for anything it drives.
pub fn log(msg: impl AsRef<str>) {
    eprintln!("[{}] {}", now(), msg.as_ref());
}

fn now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "?".into())
}
