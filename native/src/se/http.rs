//! The one place Steb touches the network. [`Client`] takes an [`Endpoint`],
//! never a string, carrying [`super::url`]'s closed set into every request.
//! TLS rides `webpki-roots` and the RustCrypto provider.

use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use super::url::Endpoint;

/// The `User-Agent` every request carries. The `+URL` names the repository,
/// the software responsible for a request, and has to keep resolving.
pub const USER_AGENT: &str = concat!(
    "Steb/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/huangziwei/steb)"
);

/// Bounds a quiet socket, leaving total transfer time uncapped.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Shorter than [`TIMEOUT`], carrying an unreachable host to `crate::ui::diag`.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Body ceiling for a listing or feed, against a ~50 KB page and a ~60 KB feed.
const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug)]
pub enum Error {
    /// No DNS, no route, Wi-Fi off. The variant `crate::ui::diag` acts on.
    Unreachable(String),
    /// The server answered with a status.
    Status { code: u16, url: String },
    /// The body was unreadable, or past [`MAX_TEXT_BYTES`].
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
    /// A one-line hint for `crate::ui::diag`, by error class.
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

/// What a conditional GET carried back. [`Fresh::Unchanged`] is a 304, no body.
pub enum Fresh<T> {
    Unchanged,
    Changed { body: T, validators: Validators },
}

/// `ETag` and `Last-Modified`, replayed as `If-None-Match` and
/// `If-Modified-Since`. Persisted in `crate::cache`.
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

    /// No stored validators: the conditional GET carries a full body back.
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
        // Explicit: `rustls-no-provider` leaves no default to fall back on.
        let tls = ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::Rustls)
            .unversioned_rustls_crypto_provider(Arc::new(rustls_rustcrypto::provider()))
            .build();

        let config = ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_recv_response(Some(READ_TIMEOUT))
            .tls_config(tls)
            // `?source=download` skips the interstitial that sets SE's
            // `download-count` cookie. ureq carries no `cookies` feature.
            .build();

        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }

    /// A listing or a book page, bounded by [`MAX_TEXT_BYTES`].
    pub fn text(&self, endpoint: &Endpoint) -> Result<String> {
        let url = endpoint.to_url();
        let mut res = self.agent.get(&url).call().map_err(|e| classify(e, &url))?;
        read_text(&mut res)
    }

    /// Raw bytes: a cover, or an azw3.
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

    /// A conditional GET carrying `have`, short-circuiting on a 304.
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
            // A 304 surfaces as an `Ok` response or as an error, by
            // status-as-error configuration. Both read as unchanged.
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
        // DNS, a refused connection, a TLS failure, a timeout: all
        // [`Error::Unreachable`], the one distinction `crate::ui::diag` acts on.
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
        // The names SE's robots.txt disallows from the downloads path.
        for banned in ["chatgpt-user", "claude-user", "claude-web", "SemrushBot"] {
            assert!(!USER_AGENT.contains(banned));
        }
        assert!(!USER_AGENT.contains("Mozilla"), "no browser spoofing");
    }

    #[test]
    fn user_agent_points_at_the_repository() {
        // The repository, naming the software responsible for a request.
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
