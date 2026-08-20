//! Every URL Steb fetches, as a closed set of types. [`Endpoint`] carries no
//! general constructor; the two hrefs read from markup go through
//! [`DownloadHref::parse`] and [`CoverHref::parse`].

use std::fmt;

/// Scheme and host, prepended to every path built here. HTTPS only.
pub const ORIGIN: &str = "https://standardebooks.org";

/// A rejected URL shape, carrying the offending value.
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

/// Characters legal in a path segment: lowercase ASCII, digits, hyphen.
fn is_slug(seg: &str) -> bool {
    !seg.is_empty()
        && seg
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// One or more slugs joined by `_`, as in
/// `/ebooks/leo-tolstoy/war-and-peace/louise-maude_aylmer-maude`.
fn is_slug_group(seg: &str) -> bool {
    !seg.is_empty() && seg.split('_').all(is_slug)
}

/// `/ebooks/{author}/{title}`, with an optional third segment for a translator
/// or editor. Stored without the `/ebooks/` prefix, as [`crate::cache`] keys it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BookPath(String);

impl BookPath {
    /// A bare path (`/ebooks/bram-stoker/dracula`) or a full URL on [`ORIGIN`].
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

    /// `bram-stoker/dracula`, the [`crate::cache`] key.
    pub fn as_key(&self) -> &str {
        &self.0
    }

    /// `/ebooks/bram-stoker/dracula`.
    pub fn as_path(&self) -> String {
        format!("/ebooks/{}", self.0)
    }
}

/// A `.azw3` href from a book page's `class="amazon"` link. The slug carries
/// translator segments (`homer_the-iliad_alexander-pope.azw3`).
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

    /// The filename, as `se::download::commit` writes it.
    pub fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or_default()
    }
}

/// A cover href: `/images/covers/{slug}/{sha}/cover@2x.jpg`. The `sha` makes
/// these content-addressed.
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

    /// Cache filename `<slug>.<sha>.jpg`, one file per book.
    pub fn cache_name(&self) -> String {
        format!("{}.{}.jpg", self.slug, self.sha)
    }

    pub fn slug(&self) -> &str {
        &self.slug
    }

    /// The URL path, persisted as a plain string and re-parsed on load.
    pub fn as_path(&self) -> &str {
        &self.path
    }
}

/// `/ebooks/{…}/downloads/thumbnail_<id>_EBOK_portrait.jpg`, `<id>` the ASIN
/// embedded in the azw3. A path [`CoverHref`] rejects.
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

    /// The name to write into `/mnt/us/system/thumbnails/`, used verbatim.
    pub fn file_name(&self) -> &str {
        &self.file
    }
}

/// The chosen sort. `None` omits the parameter.
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
    /// The `sort=` value for `has_query`. `None` drops the parameter.
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

/// Results per listing request. One of three values SE accepts.
pub const PER_PAGE: u16 = 48;

/// One listing request. `query: None` browses.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    pub query: Option<String>,
    pub page: u32,
    pub sort: Option<Sort>,
    pub tags: Vec<String>,
}

/// Percent-encode a query-string value: the RFC 3986 unreserved set, space
/// as `+`.
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

/// The closed set of fetchable URLs. No general constructor.
#[derive(Debug, Clone)]
pub enum Endpoint {
    /// `/ebooks`, with or without a query.
    Listing(Listing),
    /// A book's own page, carrying its `.azw3` href.
    Book(BookPath),
    /// The `.azw3` itself, under `?source=download`.
    Download(DownloadHref),
    /// A cover image for the grid.
    Cover(CoverHref),
    /// The Kindle home-screen thumbnail for a downloaded book.
    Thumbnail(ThumbnailHref),
    /// The public new-releases Atom feed.
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
                // Omitted for a [`SortState`] default, leaving SE's own.
                let has_query = l.query.as_deref().is_some_and(|t| !t.is_empty());
                if let Some(s) = l.sort.and_then(|s| s.as_param(has_query)) {
                    q.push(format!("sort={s}"));
                }
                q.push(format!("per-page={PER_PAGE}"));
                format!("{ORIGIN}/ebooks?{}", q.join("&"))
            }
            Endpoint::Book(p) => format!("{ORIGIN}{}", p.as_path()),
            // `?source=download` in place of the interstitial the bare href serves.
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
        // The feed's <id> form: a full URL, translator third segment.
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
        // Two translators join with `_` inside the third segment.
        assert_eq!(
            BookPath::parse("/ebooks/leo-tolstoy/war-and-peace/louise-maude_aylmer-maude")
                .unwrap()
                .as_key(),
            "leo-tolstoy/war-and-peace/louise-maude_aylmer-maude"
        );
    }

    #[test]
    fn book_path_rejects_anything_that_is_not_a_book() {
        // `/honeypot`, the path robots.txt disallows.
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
        // A translator segment in the filename slug.
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
        // One sort, two parameter values.
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
        // `relevance` is a search-only value.
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
