//! Every URL Steb is allowed to fetch, as a closed set of types.
//!
//! Standard Ebooks pages carry a honeypot link — labelled "Following this link
//! will ban your IP for 24 hours" — and `/honeypot` is the one path their
//! robots.txt disallows for every user agent. A client that follows hrefs it
//! finds in markup will eventually follow that one and get the device's IP
//! banned.
//!
//! So the rule is: **never follow a discovered href; only build a URL from a
//! known shape.** That rule is enforced here rather than written in a comment
//! somewhere, by giving [`Endpoint`] no general constructor. There is no
//! `Endpoint::parse(&str)` and no way to hand [`crate::se::http`] a bare
//! string. The hrefs we *do* take from markup — the `.azw3` download and the
//! cover image — go through [`DownloadHref::parse`] / [`CoverHref::parse`],
//! which validate the shape and reject anything else, so a markup change that
//! swapped one for a link to `/honeypot` yields an error instead of a ban.

use std::fmt;

/// Scheme + host, prepended to every path we build. HTTPS only: the site sends
/// HSTS and there is nothing to gain from allowing a downgrade.
pub const ORIGIN: &str = "https://standardebooks.org";

/// A rejected URL shape. Carries the offending value so the caller can log what
/// the markup actually contained — a parse failure here means SE changed their
/// HTML, and the failing string is the whole diagnosis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadUrl(pub String);

impl fmt::Display for BadUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "refusing unrecognised standardebooks.org URL: {}",
            self.0
        )
    }
}

impl std::error::Error for BadUrl {}

/// Characters allowed in a path segment we build a URL from. SE slugs are
/// lowercase ASCII words joined by hyphens, with digits in a few titles.
/// Deliberately strict: anything outside this set (a `.`, a `/`, a `%`, a
/// space) means we are not looking at a slug and should not be building a URL.
fn is_slug(seg: &str) -> bool {
    !seg.is_empty()
        && seg
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// One or more slugs joined by `_`.
///
/// A book credited to two translators puts both in one path segment —
/// `/ebooks/leo-tolstoy/war-and-peace/louise-maude_aylmer-maude` — and the same
/// joiner appears in cover slugs (`author_title`) and download filenames. So
/// `_` is a separator *within* a segment, never a character inside a slug.
fn is_slug_group(seg: &str) -> bool {
    !seg.is_empty() && seg.split('_').all(is_slug)
}

/// A book's page path: `/ebooks/{author}/{title}` with an optional third
/// segment for a translator or editor (`/ebooks/aristophanes/the-birds/the-athenian-society`).
///
/// Stored without the leading `/ebooks/` prefix so it round-trips as the
/// catalogue cache key — see [`crate::cache`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BookPath(String);

impl BookPath {
    /// Parse the `about=` / feed `<id>` form. Accepts a bare path
    /// (`/ebooks/bram-stoker/dracula`) or a full URL on our own origin, since
    /// the listing markup uses the first and the Atom feed the second.
    pub fn parse(raw: &str) -> Result<Self, BadUrl> {
        let path = raw.strip_prefix(ORIGIN).unwrap_or(raw);
        let rest = path
            .strip_prefix("/ebooks/")
            .ok_or_else(|| BadUrl(raw.to_string()))?;
        let rest = rest.strip_suffix('/').unwrap_or(rest);

        let segs: Vec<&str> = rest.split('/').collect();
        if !matches!(segs.len(), 2 | 3) || !segs.iter().all(|s| is_slug_group(s)) {
            return Err(BadUrl(raw.to_string()));
        }
        Ok(Self(rest.to_string()))
    }

    /// `bram-stoker/dracula` — the cache key and the stable identity of a book.
    pub fn as_key(&self) -> &str {
        &self.0
    }

    /// `/ebooks/bram-stoker/dracula`.
    pub fn as_path(&self) -> String {
        format!("/ebooks/{}", self.0)
    }
}

/// A `.azw3` download href lifted from a book page's `class="amazon"` link.
///
/// Never constructed by hand. The filename slug gains segments for translators
/// (`homer_the-iliad_alexander-pope.azw3`), so guessing `{author}_{title}.azw3`
/// 404s across a large minority of the catalogue — the href must come from the
/// page. This type is what makes taking it from markup safe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadHref(String);

impl DownloadHref {
    pub fn parse(raw: &str) -> Result<Self, BadUrl> {
        let path = raw.strip_prefix(ORIGIN).unwrap_or(raw);
        let bad = || BadUrl(raw.to_string());
        // Shape, in full: /ebooks/<segments>/downloads/<file>.azw3
        let rest = path.strip_prefix("/ebooks/").ok_or_else(bad)?;
        let (book, file) = rest.rsplit_once("/downloads/").ok_or_else(bad)?;
        if !path.ends_with(".azw3") {
            return Err(bad());
        }
        let ok_book = book.split('/').all(is_slug_group);
        // Filenames are slugs joined by `_`, plus the extension.
        let stem = file.trim_end_matches(".azw3");
        let ok_file = is_slug_group(stem);
        if !ok_book || !ok_file {
            return Err(bad());
        }
        Ok(Self(path.to_string()))
    }

    /// The filename SE serves it under — also what we write into
    /// `documents/standardebooks/`, so a re-download is detectable by name.
    pub fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or_default()
    }
}

/// A cover image href from listing markup: `/images/covers/{slug}/{sha}/cover@2x.jpg`.
///
/// The `sha` makes these content-addressed, which is why the cover cache never
/// needs invalidating — a re-produced cover arrives under a new URL and the old
/// file is simply orphaned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverHref {
    path: String,
    slug: String,
    sha: String,
}

impl CoverHref {
    pub fn parse(raw: &str) -> Result<Self, BadUrl> {
        let path = raw.strip_prefix(ORIGIN).unwrap_or(raw);
        let bad = || BadUrl(raw.to_string());
        let rest = path.strip_prefix("/images/covers/").ok_or_else(bad)?;
        let mut segs = rest.split('/');
        let (slug, sha, file) = (
            segs.next().ok_or_else(bad)?,
            segs.next().ok_or_else(bad)?,
            segs.next().ok_or_else(bad)?,
        );
        if segs.next().is_some() {
            return Err(bad());
        }
        // Slug here is `author_title`, so `_` is legal on top of the slug set.
        let ok_slug = is_slug_group(slug);
        let ok_sha = !sha.is_empty() && sha.bytes().all(|b| b.is_ascii_hexdigit());
        if !ok_slug || !ok_sha || !file.ends_with(".jpg") {
            return Err(bad());
        }
        Ok(Self {
            path: path.to_string(),
            slug: slug.to_string(),
            sha: sha.to_string(),
        })
    }

    /// Cache filename: `<slug>.<sha>.jpg` — one file per book, with prior
    /// shas pruned on store.
    pub fn cache_name(&self) -> String {
        format!("{}.{}.jpg", self.slug, self.sha)
    }

    pub fn slug(&self) -> &str {
        &self.slug
    }

    /// The URL path, as persisted in the catalogue cache. Stored as a plain
    /// string and re-parsed on load, so a markup change that produced an
    /// unparseable cover can never make the whole cache unreadable.
    pub fn as_path(&self) -> &str {
        &self.path
    }
}

/// The Kindle cover thumbnail a book page links:
/// `/ebooks/{…}/downloads/thumbnail_<id>_EBOK_portrait.jpg`.
///
/// `<id>` is exactly the ASIN embedded in the azw3, which is what makes this
/// worth fetching: dropping the file into `/mnt/us/system/thumbnails/` under
/// its own given name is precisely what the framework looks for, so a
/// sideloaded book gets a real cover on the home screen instead of the grey
/// placeholder. Nothing has to parse the book to find the ASIN — Standard
/// Ebooks has already named the file after it.
///
/// Distinct from [`CoverHref`] despite both being JPEGs: this lives under
/// `/ebooks/…/downloads/`, not `/images/covers/`, so it fails that type's shape
/// check and needs its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbnailHref {
    path: String,
    file: String,
}

impl ThumbnailHref {
    pub fn parse(raw: &str) -> Result<Self, BadUrl> {
        let path = raw.strip_prefix(ORIGIN).unwrap_or(raw);
        let bad = || BadUrl(raw.to_string());
        let file = path.rsplit('/').next().ok_or_else(bad)?;
        if !path.starts_with("/ebooks/")
            || !path.contains("/downloads/")
            || !file.starts_with("thumbnail_")
            || !file.ends_with("_EBOK_portrait.jpg")
        {
            return Err(bad());
        }
        Ok(Self {
            path: path.to_string(),
            file: file.to_string(),
        })
    }

    /// The name to write into `/mnt/us/system/thumbnails/`. Used verbatim —
    /// Standard Ebooks' filename is already the one the framework expects.
    pub fn file_name(&self) -> &str {
        &self.file
    }
}

/// Which sort the user picked. `None` anywhere this appears means "SE's own
/// default", which we express by omitting the parameter entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Relevance,
    Newest,
    AuthorAlpha,
    ReadingEase,
    Length,
    Popularity,
}

impl Sort {
    /// SE uses **different values for the same sort** depending on whether a
    /// query is present, and offers `relevance` only when one is:
    ///
    /// | sort                  | browse    | search      |
    /// |-----------------------|-----------|-------------|
    /// | release date new→old  | `default` | `newest`    |
    /// | relevance             | *absent*  | `relevance` |
    ///
    /// So the value cannot be baked into the enum; it is resolved here, at
    /// URL-build time, from whether the listing has a query. Returning `None`
    /// drops the parameter — which is what `Relevance` does while browsing,
    /// since there is nothing to be relevant to.
    fn as_param(self, has_query: bool) -> Option<&'static str> {
        Some(match self {
            Sort::Relevance => {
                if !has_query {
                    return None;
                }
                "relevance"
            }
            Sort::Newest => {
                if has_query {
                    "newest"
                } else {
                    "default"
                }
            }
            Sort::AuthorAlpha => "author-alpha",
            Sort::ReadingEase => "reading-ease",
            Sort::Length => "length",
            Sort::Popularity => "popularity",
        })
    }
}

/// How many results per listing request. SE allows only these three; 48 keeps
/// one handshake filling roughly five device grid pages.
pub const PER_PAGE: u16 = 48;

/// What a listing request asks for. Browsing is this with `query: None` — SE
/// serves both from the same endpoint with identical markup, which is why Steb
/// has one parser and one grid rather than separate browse and search modes.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    pub query: Option<String>,
    pub page: u32,
    pub sort: Option<Sort>,
    pub tags: Vec<String>,
}

/// Percent-encode for a query-string value. Hand-rolled because the only
/// alternative is pulling a URL crate in for one function; the unreserved set
/// is from RFC 3986 and space becomes `+` as the form encoding expects.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The closed set of fetchable URLs. No general constructor — see the module
/// docs for why that is the point rather than an oversight.
#[derive(Debug, Clone)]
pub enum Endpoint {
    /// `/ebooks` with or without a query — browse and search are one endpoint.
    Listing(Listing),
    /// A book's own page, fetched to read its `.azw3` href.
    Book(BookPath),
    /// The `.azw3` itself, with `?source=download` so SE serves the file
    /// instead of the "Your Download Has Started!" interstitial.
    Download(DownloadHref),
    /// A cover image for the grid.
    Cover(CoverHref),
    /// The Kindle home-screen thumbnail for a downloaded book.
    Thumbnail(ThumbnailHref),
    /// The public new-releases Atom feed — the only feed not gated behind the
    /// Patrons Circle, and our delta channel for catalogue updates.
    Feed,
}

impl Endpoint {
    pub fn to_url(&self) -> String {
        match self {
            Endpoint::Listing(l) => {
                let mut q: Vec<String> = Vec::new();
                if let Some(term) = l.query.as_deref().filter(|t| !t.is_empty()) {
                    q.push(format!("query={}", encode(term)));
                }
                if l.page > 1 {
                    q.push(format!("page={}", l.page));
                }
                for tag in &l.tags {
                    q.push(format!("tags%5B%5D={}", encode(tag)));
                }
                // Omitted unless the user actively chose a sort, so SE applies
                // its own default: release date new→old browsing, relevance
                // searching. Right in both modes, and it sidesteps the
                // vocabulary split above for the common case.
                let has_query = l.query.as_deref().is_some_and(|t| !t.is_empty());
                if let Some(s) = l.sort.and_then(|s| s.as_param(has_query)) {
                    q.push(format!("sort={s}"));
                }
                q.push(format!("per-page={PER_PAGE}"));
                format!("{ORIGIN}/ebooks?{}", q.join("&"))
            }
            Endpoint::Book(p) => format!("{ORIGIN}{}", p.as_path()),
            // The bare href returns a ~10 KB interstitial whose meta refresh
            // points back at this same URL with `?source=download`. Asking for
            // it directly skips a request and a parse.
            Endpoint::Download(d) => format!("{ORIGIN}{}?source=download", d.0),
            Endpoint::Cover(c) => format!("{ORIGIN}{}", c.path),
            Endpoint::Thumbnail(t) => format!("{ORIGIN}{}", t.path),
            Endpoint::Feed => format!("{ORIGIN}/feeds/atom/new-releases"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn book_path_accepts_two_and_three_segments() {
        assert_eq!(
            BookPath::parse("/ebooks/bram-stoker/dracula")
                .unwrap()
                .as_key(),
            "bram-stoker/dracula"
        );
        // Translator third segment — the feed's <id> form, full URL.
        assert_eq!(
            BookPath::parse(
                "https://standardebooks.org/ebooks/aristophanes/the-birds/the-athenian-society"
            )
            .unwrap()
            .as_key(),
            "aristophanes/the-birds/the-athenian-society"
        );
    }

    #[test]
    fn book_path_accepts_two_translators_in_one_segment() {
        // A book credited to two translators joins them with `_` inside the
        // third segment. A hyphens-only slug rule rejects every such book, and
        // there are many.
        assert_eq!(
            BookPath::parse("/ebooks/leo-tolstoy/war-and-peace/louise-maude_aylmer-maude")
                .unwrap()
                .as_key(),
            "leo-tolstoy/war-and-peace/louise-maude_aylmer-maude"
        );
    }

    #[test]
    fn book_path_rejects_anything_that_is_not_a_book() {
        // The honeypot is the whole reason this type exists.
        assert!(BookPath::parse("/honeypot").is_err());
        assert!(BookPath::parse("https://example.com/ebooks/a/b").is_err());
        assert!(BookPath::parse("/ebooks/bram-stoker").is_err());
        assert!(BookPath::parse("/ebooks/a/b/c/d").is_err());
        assert!(BookPath::parse("/ebooks/../../etc/passwd").is_err());
        assert!(BookPath::parse("/ebooks/Bram-Stoker/dracula").is_err());
    }

    #[test]
    fn download_href_round_trips_and_adds_source_param() {
        let d =
            DownloadHref::parse("/ebooks/bram-stoker/dracula/downloads/bram-stoker_dracula.azw3")
                .unwrap();
        assert_eq!(d.file_name(), "bram-stoker_dracula.azw3");
        assert_eq!(
            Endpoint::Download(d).to_url(),
            "https://standardebooks.org/ebooks/bram-stoker/dracula/downloads/bram-stoker_dracula.azw3?source=download"
        );
    }

    #[test]
    fn download_href_accepts_translator_segments() {
        // Guessing {author}_{title}.azw3 would miss these entirely.
        assert!(
            DownloadHref::parse(
                "/ebooks/homer/the-iliad/alexander-pope/downloads/homer_the-iliad_alexander-pope.azw3"
            )
            .is_ok()
        );
    }

    #[test]
    fn download_href_rejects_other_formats_and_foreign_paths() {
        assert!(
            DownloadHref::parse("/ebooks/a/b/downloads/a_b.epub").is_err(),
            "azw3 only — we do not ship other formats"
        );
        assert!(DownloadHref::parse("/honeypot").is_err());
        assert!(DownloadHref::parse("https://example.com/x.azw3").is_err());
    }

    #[test]
    fn cover_href_yields_a_content_addressed_cache_name() {
        let c = CoverHref::parse(
            "/images/covers/bram-stoker_dracula/a02c17aca72aec342c861065c62cd25559a6d960/cover@2x.jpg",
        )
        .unwrap();
        assert_eq!(
            c.cache_name(),
            "bram-stoker_dracula.a02c17aca72aec342c861065c62cd25559a6d960.jpg"
        );
    }

    #[test]
    fn cover_href_rejects_non_hex_revisions() {
        assert!(CoverHref::parse("/images/covers/a_b/not-a-sha/cover.jpg").is_err());
        assert!(CoverHref::parse("/images/covers/a_b/abc/cover.gif").is_err());
    }

    #[test]
    fn browsing_omits_query_and_sort() {
        let url = Endpoint::Listing(Listing::default()).to_url();
        assert_eq!(url, "https://standardebooks.org/ebooks?per-page=48");
    }

    #[test]
    fn sort_value_depends_on_whether_a_query_is_present() {
        // The trap: same human-facing sort, two different parameter values.
        let browse = Endpoint::Listing(Listing {
            sort: Some(Sort::Newest),
            ..Default::default()
        })
        .to_url();
        assert!(browse.contains("sort=default"), "{browse}");

        let search = Endpoint::Listing(Listing {
            query: Some("dracula".into()),
            sort: Some(Sort::Newest),
            ..Default::default()
        })
        .to_url();
        assert!(search.contains("sort=newest"), "{search}");
    }

    #[test]
    fn relevance_is_dropped_when_browsing() {
        // SE does not offer it without a query, so sending it would be a lie.
        let browse = Endpoint::Listing(Listing {
            sort: Some(Sort::Relevance),
            ..Default::default()
        })
        .to_url();
        assert!(!browse.contains("sort="), "{browse}");
    }

    #[test]
    fn multi_word_queries_are_encoded() {
        let url = Endpoint::Listing(Listing {
            query: Some("war and peace".into()),
            page: 3,
            ..Default::default()
        })
        .to_url();
        assert!(url.contains("query=war+and+peace"), "{url}");
        assert!(url.contains("page=3"), "{url}");
    }
}
