//! A `/ebooks` listing page. `/ebooks` and `/ebooks?query=…` return identical
//! markup, so one parser serves both. Scanned against the RDFa annotations
//! (`typeof="schema:Book"`, `property="schema:name"`), never parsed into a DOM.

use super::url::{BadUrl, BookPath, CoverHref};

/// One listed book, holding what a grid cell draws. `cover` is not `Option`:
/// [`parse`] drops an entry without cover art.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub path: BookPath,
    pub title: String,
    pub author: String,
    pub cover: CoverHref,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub hits: Vec<Hit>,
    /// Highest page in the nav, at the requested `per-page`. [`Self::has_next`]
    /// decides whether to fetch more.
    pub total_pages: u32,
    /// Entries [`parse`] dropped as undownloadable.
    pub unavailable: usize,
    /// SE’s own `rel="next"` control, past [`Self::total_pages`].
    pub has_next: bool,
    /// The subject vocabulary from the page’s own `<select name="tags[]">`,
    /// minus SE’s `all` sentinel.
    pub tags: Vec<String>,
}

#[derive(Debug)]
pub enum ParseError {
    /// No books and no "no results" marker: the markup moved.
    Unrecognised,
    Url(BadUrl),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Unrecognised => {
                write!(f, "standardebooks.org markup changed — no books found")
            }
            ParseError::Url(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Value of `attr="…"` within `hay`, searching from `from`.
fn attr_after(hay: &str, from: usize, attr: &str) -> Option<(String, usize)> {
    let pat = format!("{attr}=\"");
    let start = hay[from..].find(&pat)? + from + pat.len();
    let end = hay[start..].find('"')? + start;
    Some((hay[start..end].to_string(), end))
}

/// Text of the first `<span property="schema:name">…</span>` at or after `from`.
fn schema_name_after(hay: &str, from: usize) -> Option<(String, usize)> {
    let pat = "property=\"schema:name\">";
    let start = hay[from..].find(pat)? + from + pat.len();
    let end = hay[start..].find('<')? + start;
    Some((decode_entities(hay[start..end].trim()), end))
}

/// The five XML entities SE’s escaper emits. Curly quotes arrive as UTF-8.
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

pub fn parse(html: &str) -> Result<Page, ParseError> {
    let mut hits = Vec::new();
    let mut unavailable = 0usize;
    let mut cursor = 0usize;

    // Each result opens `<li typeof="schema:Book" about="/ebooks/…">`, the
    // `about` carrying the book path.
    while let Some(rel) = html[cursor..].find("typeof=\"schema:Book\"") {
        let item = cursor + rel;
        // Bounded at the next book: a malformed entry stops here.
        let end = html[item + 1..]
            .find("typeof=\"schema:Book\"")
            .map(|r| item + 1 + r)
            .unwrap_or(html.len());
        let block = &html[item..end];
        cursor = end;

        let Some((about, _)) = attr_after(block, 0, "about") else {
            continue;
        };
        let path = BookPath::parse(&about).map_err(ParseError::Url)?;

        // Two `schema:name` spans in document order: title, then author.
        let Some((title, after_title)) = schema_name_after(block, 0) else {
            continue;
        };
        let author = schema_name_after(block, after_title)
            .map(|(a, _)| a)
            .unwrap_or_default();

        // The `<img src>` @2x; its `<source srcset>` siblings are AVIF. An
        // absent cover marks an undownloadable entry: `ribbon not-pd`,
        // `ribbon wanted` and unribboned ones alike carry none.
        let Some(cover) =
            attr_after(block, 0, "src").and_then(|(src, _)| CoverHref::parse(&src).ok())
        else {
            unavailable += 1;
            continue;
        };

        hits.push(Hit {
            path,
            title,
            author,
            cover,
        });
    }

    // An empty result set against markup drift: the page furniture is present
    // either way.
    if hits.is_empty() && !html.contains("<nav class=\"pagination\"") && !html.contains("/ebooks") {
        return Err(ParseError::Unrecognised);
    }

    Ok(Page {
        total_pages: total_pages(html),
        has_next: has_next(html),
        tags: tags(html),
        unavailable,
        hits,
    })
}

/// Subject tags from `<select name="tags[]">`, minus the `all` sentinel. Empty
/// where the select is absent.
fn tags(html: &str) -> Vec<String> {
    let Some(start) = html.find("name=\"tags[]\"") else {
        return Vec::new();
    };
    let end = html[start..]
        .find("</select>")
        .map(|r| start + r)
        .unwrap_or(html.len());
    let block = &html[start..end];

    let mut out = Vec::new();
    let mut cursor = 0usize;
    while let Some((value, next)) = attr_after(block, cursor, "value") {
        cursor = next;
        // SE’s "all tags" option names no subject.
        if value != "all" && !value.is_empty() {
            out.push(value);
        }
    }
    out
}

/// The pagination nav, if this page has one.
fn nav_block(html: &str) -> Option<&str> {
    let nav = html.find("<nav class=\"pagination\"")?;
    let end = html[nav..]
        .find("</nav>")
        .map(|r| nav + r)
        .unwrap_or(html.len());
    Some(&html[nav..end])
}

/// Highest `page=N` in the nav, or 1 with no nav. A real parameter follows
/// `?`, `&`, or the `;` ending an `&amp;`: `page=` also sits inside
/// `per-page=48`.
fn total_pages(html: &str) -> u32 {
    let Some(block) = nav_block(html) else {
        return 1;
    };

    let mut max = 1u32;
    let mut cursor = 0usize;
    while let Some(rel) = block[cursor..].find("page=") {
        let abs = cursor + rel;
        let start = abs + "page=".len();
        let boundary = abs == 0 || matches!(block.as_bytes()[abs - 1], b'?' | b'&' | b';');
        let digits: String = block[start..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        cursor = start + digits.len().max(1);
        if boundary && let Ok(n) = digits.parse::<u32>() {
            max = max.max(n);
        }
    }
    max
}

/// An `href` on the `rel="next"` control. The last page carries
/// `aria-disabled="true"` and none.
fn has_next(html: &str) -> bool {
    nav_block(html).is_some_and(|nav| {
        nav.find("rel=\"next\"").is_some_and(|at| {
            // Back to the opening `<a`, for its href.
            nav[..at]
                .rfind("<a ")
                .is_some_and(|open| nav[open..at].contains("href=\""))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BROWSE: &str = include_str!("../../tests/fixtures/listing-browse-p1.html");
    const SEARCH_48: &str = include_str!("../../tests/fixtures/listing-search-thewar-48.html");
    const SEARCH_DRACULA: &str = include_str!("../../tests/fixtures/listing-search-dracula.html");
    /// A query where most results are announced but not yet public domain.
    const SEARCH_NOT_PD: &str = include_str!("../../tests/fixtures/listing-search-not-pd.html");

    #[test]
    fn browse_page_one_is_the_opening_screen() {
        let page = parse(BROWSE).unwrap();
        // Bare `/ebooks`: 12 per page, newest first.
        assert_eq!(page.hits.len(), 12);
        // SE lists every page, so this is the whole catalogue.
        assert!(
            page.total_pages > 100,
            "expected the full catalogue, got {} pages",
            page.total_pages
        );
    }

    #[test]
    fn the_same_parser_handles_a_search_page() {
        // Browse and search markup, through one parser.
        let page = parse(SEARCH_48).unwrap();
        assert_eq!(page.hits.len(), 28);
        assert_eq!(page.total_pages, 1, "the results fit one page of 48");
    }

    #[test]
    fn a_hit_carries_everything_a_grid_cell_needs() {
        let page = parse(SEARCH_DRACULA).unwrap();
        let dracula = page
            .hits
            .iter()
            .find(|h| h.path.as_key() == "bram-stoker/dracula")
            .expect("Dracula should be the first hit for `dracula`");
        assert_eq!(dracula.title, "Dracula");
        assert_eq!(dracula.author, "Bram Stoker");
        assert_eq!(dracula.cover.slug(), "bram-stoker_dracula");
        assert!(dracula.cover.cache_name().ends_with(".jpg"));
    }

    #[test]
    fn every_hit_parses_cleanly_across_all_fixtures() {
        // One `<li>` bounded against the next.
        for (name, html) in [
            ("browse", BROWSE),
            ("search-48", SEARCH_48),
            ("search-dracula", SEARCH_DRACULA),
        ] {
            for hit in parse(html).unwrap().hits {
                assert!(!hit.title.is_empty(), "{name}: empty title");
                assert!(
                    !hit.author.is_empty(),
                    "{name}: empty author for {}",
                    hit.title
                );
            }
        }
    }

    #[test]
    fn an_entry_without_cover_art_is_not_downloadable_and_is_dropped() {
        // `arthur-b-reeve/the-war-terror` carries no ribbon and no download.
        // The missing cover catches it.
        let page = parse(SEARCH_48).unwrap();
        assert!(
            !page
                .hits
                .iter()
                .any(|h| h.path.as_key() == "arthur-b-reeve/the-war-terror"),
            "an entry with no cover art has nothing to download"
        );
        assert!(page.unavailable > 0);
    }

    #[test]
    fn the_bulk_of_a_browse_page_survives() {
        // Wholesale cover-extraction failure empties the grid silently.
        let page = parse(BROWSE).unwrap();
        assert!(
            page.hits.len() > page.unavailable * 4,
            "{} kept vs {} dropped — cover extraction is probably broken",
            page.hits.len(),
            page.unavailable
        );
    }

    /// The nav shape carrying `per-page`.
    const NAV_WITH_PER_PAGE: &str = r##"<html><nav class="pagination" aria-label="Pagination">
        <a aria-disabled="true">Back</a>
        <ol>
          <li><a aria-current="page" href="#">1</a></li>
          <li><a href="/ebooks?page=2&amp;per-page=48">2</a></li>
          <li><a href="/ebooks?page=32&amp;per-page=48">32</a></li>
        </ol>
        <a href="/ebooks?page=2&amp;per-page=48" rel="next">Next</a>
        </nav></html>"##;

    #[test]
    fn per_page_is_not_mistaken_for_a_page_number() {
        // `per-page=48` contains the substring `page=48`.
        let page = parse(NAV_WITH_PER_PAGE).unwrap();
        assert_eq!(page.total_pages, 32, "should read the real highest page");
    }

    #[test]
    fn has_next_follows_ses_own_control() {
        assert!(parse(NAV_WITH_PER_PAGE).unwrap().has_next);

        // The last page: a `rel="next"` carrying no href.
        let last = r##"<html><nav class="pagination">
            <a href="/ebooks?page=31&amp;per-page=48">Back</a>
            <ol><li><a aria-current="page" href="#">32</a></li></ol>
            <a aria-disabled="true" rel="next">Next</a>
            </nav></html>"##;
        assert!(
            !parse(last).unwrap().has_next,
            "an href-less Next is the end of results"
        );
    }

    #[test]
    fn a_single_page_result_has_no_next() {
        assert!(!parse(SEARCH_48).unwrap().has_next, "31 hits fit one page");
    }

    #[test]
    fn browse_page_one_does_have_a_next() {
        assert!(parse(BROWSE).unwrap().has_next);
    }

    #[test]
    fn undownloadable_entries_are_dropped_whatever_their_label() {
        // `ribbon not-pd` and `ribbon wanted` entries, neither carrying a cover.
        let page = parse(SEARCH_NOT_PD).unwrap();
        assert_eq!(page.unavailable, 11);
        assert_eq!(page.hits.len(), 1, "one real book on this page");
        assert_eq!(
            page.hits[0].path.as_key(),
            "ellery-queen/the-roman-hat-mystery"
        );
    }

    #[test]
    fn the_tag_vocabulary_comes_from_the_page() {
        // Read from the markup: a new SE subject appears on the next fetch.
        let tags = parse(BROWSE).unwrap().tags;
        assert!(tags.len() > 10, "expected SE's subject list, got {tags:?}");
        for expected in ["fantasy", "poetry", "mystery"] {
            assert!(tags.contains(&expected.to_string()), "missing {expected}");
        }
        assert!(
            !tags.iter().any(|t| t == "all"),
            "`all` means no filter and must not appear as a subject"
        );
    }

    #[test]
    fn a_page_without_a_tag_select_yields_no_vocabulary() {
        // An absent select leaves the caller’s list alone.
        let html = r#"<html><nav class="pagination"></nav></html>"#;
        assert!(parse(html).unwrap().tags.is_empty());
    }

    #[test]
    fn a_page_with_no_books_and_no_furniture_is_markup_drift_not_zero_results() {
        assert!(matches!(
            parse("<html><body>nothing here</body></html>"),
            Err(ParseError::Unrecognised)
        ));
    }

    #[test]
    fn zero_results_is_not_an_error() {
        // SE carries no typo tolerance.
        let empty = r#"<html><nav class="pagination" aria-label="Pagination"></nav></html>"#;
        let page = parse(empty).unwrap();
        assert!(page.hits.is_empty());
        assert_eq!(page.total_pages, 1);
    }

    #[test]
    fn entities_in_titles_are_decoded() {
        let html = r#"<li typeof="schema:Book" about="/ebooks/a-author/a-title">
            <img src="/images/covers/a-author_a-title/abc123/cover@2x.jpg"/>
            <span property="schema:name">Cakes &amp; Ale</span>
            <p class="author"><span property="schema:name">W. S.</span></p></li>"#;
        assert_eq!(parse(html).unwrap().hits[0].title, "Cakes & Ale");
    }
}
