//! The one place Steb touches the network.
//!
//! Everything goes through [`Client`], and [`Client`] only accepts an
//! [`Endpoint`] — never a string — so the honeypot rule from [`super::url`] is
//! structural rather than a convention someone has to remember.
//!
//! standardebooks.org is HTTPS-with-HSTS on a current Let's Encrypt chain, so
//! TLS is mandatory. Two consequences:
//!
//! - We bundle our own root store (`webpki-roots`) and never consult the
//!   device's. A Kindle's CA bundle is years stale and would reject the chain
//!   SE serves today.
//! - The crypto provider is RustCrypto's, not `ring`. That is not a preference
//!   about cryptography but about the build: `ring` carries a C build script,
//!   Cargo unifies features so it would be *compiled* whether or not it were
//!   ever called, and that alone breaks the pure-Rust `rust-lld` cross path
//!   the whole fleet's single static binary depends on. ureq 3's
//!   `rustls-no-provider` feature is what makes choosing possible; ureq 2 has
//!   no such seam. See `native/Cargo.toml` for the full reasoning.

use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use super::url::Endpoint;

/// Identify honestly. SE's robots.txt disallows the downloads path for a named
/// list of SEO and AI crawlers; we are neither, and a user-driven client that
/// says what it is has nothing to hide. Spoofing a browser — or worse, wearing
/// one of the disallowed names — would be the thing that makes this rude.
///
/// The `+URL` points at the **repository**, not at the author's homepage. Steb
/// is not a hosted service — every copy runs on a different person's Kindle
/// from their own IP — so what this identifies is the *software* responsible
/// for a request, which is what Standard Ebooks would want to look up and, if
/// it came to it, block. Scoping it to the repo keeps that useful without
/// making one person's whole identity the reference for anyone else's traffic.
///
/// Keep it resolving. A `+URL` that 404s reads as a fake courtesy and is worse
/// than none at all.
pub const USER_AGENT: &str = concat!(
    "Steb/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/huangziwei/steb)"
);

/// Bounds a stalled socket without capping total transfer time — an 800 KB
/// azw3 over hotel Wi-Fi is slow but healthy, whereas a socket that goes quiet
/// for this long is not coming back.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Connect separately and sooner: an unreachable host should reach the
/// Diagnostics screen quickly rather than looking like a hang.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Refuse a listing or feed body larger than this. A listing page is ~50 KB and
/// the feed ~60 KB, so a megabyte means something has gone wrong and we should
/// not spend a device's RAM finding out what.
const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug)]
pub enum Error {
    /// Could not reach the host at all — no DNS, no route, Wi-Fi off. This is
    /// the variant the Diagnostics screen exists for.
    Unreachable(String),
    /// Reached the server and it said no.
    Status { code: u16, url: String },
    /// Reached the server, but the body was unreadable or absurdly large.
    Body(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unreachable(e) => write!(f, "cannot reach standardebooks.org: {e}"),
            Error::Status { code, url } => {
                write!(f, "standardebooks.org returned {code} for {url}")
            }
            Error::Body(e) => write!(f, "unreadable response: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    /// A one-line hint for the Diagnostics screen, chosen by error class. The
    /// user is holding an e-reader, not a terminal — "check Wi-Fi" is more
    /// use to them than a transport error string.
    pub fn hint(&self) -> &'static str {
        match self {
            Error::Unreachable(_) => "Check that Wi-Fi is on and connected.",
            Error::Status { code: 404, .. } => "That book may have been renamed or withdrawn.",
            Error::Status { code: 429, .. } => "Too many requests — wait a few minutes.",
            Error::Status { .. } => "The site is reachable but returned an error.",
            Error::Body(_) => "The response could not be read. Try again.",
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// What a conditional GET came back with. The steady state on launch is
/// [`Fresh::Unchanged`] — SE sends a 304 with no body, so a launch with no new
/// releases costs one small request and nothing else.
pub enum Fresh<T> {
    Unchanged,
    Changed { body: T, validators: Validators },
}

/// `ETag` / `Last-Modified` from a response, to be sent back as
/// `If-None-Match` / `If-Modified-Since` next launch. Persisted alongside the
/// catalogue cache.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Validators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl Validators {
    fn from_response<B>(res: &ureq::http::Response<B>) -> Self {
        let header = |name: &str| {
            res.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        Self {
            etag: header("etag"),
            last_modified: header("last-modified"),
        }
    }

    /// No validators stored yet — a first run, or a cache that was discarded.
    /// The conditional GET still works without them; it just cannot come back
    /// 304, so the caller knows to expect a full body.
    pub fn is_empty(&self) -> bool {
        self.etag.is_none() && self.last_modified.is_none()
    }
}

pub struct Client {
    agent: ureq::Agent,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    pub fn new() -> Self {
        // The RustCrypto provider, chosen for the build reasons in the module
        // docs. Passed explicitly because ureq is built with
        // `rustls-no-provider`, so there is no default to fall back on — which
        // is deliberate: a missing provider should be a compile-time-visible
        // wiring decision, not a runtime surprise.
        let tls = ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::Rustls)
            .unversioned_rustls_crypto_provider(Arc::new(rustls_rustcrypto::provider()))
            .build();

        let config = ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_recv_response(Some(READ_TIMEOUT))
            .tls_config(tls)
            // SE sets a `download-count` cookie to drive its donation prompt.
            // We ask for the file directly with `?source=download` and never
            // see the interstitial, so the cookie has nothing to do — ureq is
            // built without the `cookies` feature at all, so there is no jar
            // to accumulate anything we do not use.
            .build();

        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }

    /// Fetch a text resource — a listing or a book page.
    pub fn text(&self, endpoint: &Endpoint) -> Result<String> {
        let url = endpoint.to_url();
        let mut res = self.agent.get(&url).call().map_err(|e| classify(e, &url))?;
        read_text(&mut res)
    }

    /// Fetch raw bytes — a cover, or an azw3.
    pub fn bytes(&self, endpoint: &Endpoint) -> Result<Vec<u8>> {
        let url = endpoint.to_url();
        let mut res = self.agent.get(&url).call().map_err(|e| classify(e, &url))?;
        let mut buf = Vec::new();
        res.body_mut()
            .as_reader()
            .read_to_end(&mut buf)
            .map_err(|e| Error::Body(e.to_string()))?;
        Ok(buf)
    }

    /// Conditional GET. Sends the stored validators and short-circuits on a
    /// 304, which is the whole point of the launch freshness check: when SE has
    /// published nothing, this transfers no body at all.
    pub fn text_if_modified(
        &self,
        endpoint: &Endpoint,
        known: &Validators,
    ) -> Result<Fresh<String>> {
        let url = endpoint.to_url();
        let mut req = self.agent.get(&url);
        if let Some(etag) = &known.etag {
            req = req.header("If-None-Match", etag);
        }
        if let Some(lm) = &known.last_modified {
            req = req.header("If-Modified-Since", lm);
        }

        match req.call() {
            // A 304 carries no body and is the outcome we want. Depending on
            // status-as-error configuration it can surface either as an Ok
            // response or as an error, so both are treated as "unchanged"
            // rather than relying on which one ureq picks.
            Ok(res) if res.status() == 304 => Ok(Fresh::Unchanged),
            Ok(mut res) => {
                let validators = Validators::from_response(&res);
                let body = read_text(&mut res)?;
                Ok(Fresh::Changed { body, validators })
            }
            Err(ureq::Error::StatusCode(304)) => Ok(Fresh::Unchanged),
            Err(e) => Err(classify(e, &url)),
        }
    }
}

fn classify(e: ureq::Error, url: &str) -> Error {
    match e {
        ureq::Error::StatusCode(code) => Error::Status {
            code,
            url: url.to_string(),
        },
        // Everything else — DNS, refused connection, TLS failure, timeout —
        // is the device not being able to reach the site, which is the one
        // distinction the Diagnostics screen actually acts on.
        other => Error::Unreachable(other.to_string()),
    }
}

fn read_text(res: &mut ureq::http::Response<ureq::Body>) -> Result<String> {
    res.body_mut()
        .with_config()
        .limit(MAX_TEXT_BYTES as u64)
        .read_to_string()
        .map_err(|e| Error::Body(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_agent_is_honest_and_not_a_disallowed_crawler() {
        assert!(USER_AGENT.starts_with("Steb/"));
        // These are the names SE's robots.txt disallows from the downloads
        // path. Wearing one would be both a lie and a violation.
        for banned in ["chatgpt-user", "claude-user", "claude-web", "SemrushBot"] {
            assert!(!USER_AGENT.contains(banned));
        }
        assert!(!USER_AGENT.contains("Mozilla"), "no browser spoofing");
    }

    #[test]
    fn user_agent_points_at_the_repository() {
        // Not the author's homepage: this identifies the software responsible
        // for a request, which is what SE would look up or block. It must also
        // actually resolve — a `+URL` that 404s reads as a fake courtesy.
        assert!(
            USER_AGENT.contains("github.com/huangziwei/steb"),
            "{USER_AGENT}"
        );
    }

    #[test]
    fn empty_validators_are_recognised() {
        assert!(Validators::default().is_empty());
        assert!(
            !Validators {
                etag: Some("\"abc\"".into()),
                last_modified: None,
            }
            .is_empty()
        );
    }

    #[test]
    fn hints_are_class_specific() {
        assert!(
            Error::Unreachable("dns".into())
                .hint()
                .to_lowercase()
                .contains("wi-fi")
        );
        assert_ne!(
            Error::Status {
                code: 404,
                url: String::new()
            }
            .hint(),
            Error::Status {
                code: 429,
                url: String::new()
            }
            .hint()
        );
    }
}
