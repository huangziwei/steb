//! Parse a `/ebooks` listing page.
//!
//! **One parser, both modes.** `/ebooks` and `/ebooks?query=…` return identical
//! markup, so browsing is search with an empty query and there is nothing to
//! branch on. Anything that looks like it wants a second parser is a mistake.
//!
//! Scanning rather than a DOM: the markup is RDFa-annotated and machine-regular
//! (`typeof="schema:Book"`, `property="schema:name"`), a listing is ~50 KB, and
//! an HTML5 parser would be the single largest thing in the binary for no gain.
//! What we give up is resilience to markup drift — so every extractor here
//! fails loudly rather than silently yielding an empty list, and the fixture
//! tests exist to catch the drift when it comes.

use super::url::{BadUrl, BookPath, CoverHref};

/// One book as it appears in a listing — everything the grid needs to draw a
/// cell, and nothing more. The `.azw3` href is deliberately absent: it lives on
/// the book's own page and is fetched only when the user taps to download.
///
/// The cover is **not** optional, and that is the type doing real work: a
/// listing entry without cover art is not a book Steb can download, so one
/// cannot be constructed. See [`parse`].
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
    /// Highest page number offered by the pagination nav. SE lists every page
    /// rather than a rolling window, so this is a real total — but it is a
    /// total *at the requested `per-page`*, not a count of books: the same
    /// catalogue is ~125 pages at 12 per page and ~32 at 48. Useful for a
    /// progress indicator, not for deciding whether to fetch more.
    pub total_pages: u32,
    /// Entries dropped because they are not downloadable — see [`parse`].
    /// Surfaced only so a page that looks half-empty has an explanation.
    pub unavailable: usize,
    /// Whether a further page exists. Prefer this over comparing against
    /// [`Self::total_pages`] when deciding whether to fetch more — it comes
    /// from SE's own `rel="next"` control and does not depend on how many links
    /// the nav renders.
    pub has_next: bool,
    /// The subject-tag vocabulary, straight from the page's own
    /// `<select name="tags[]">`.
    ///
    /// Taken from the markup rather than hardcoded so the filter menu cannot
    /// drift out of step with the site — if SE adds a subject, it appears in
    /// the menu the next time a listing is fetched, with no release on our
    /// side. SE's `all` sentinel is dropped here: it means "no filter", which
    /// [`crate::ui::filter::Filters`] expresses as an empty selection.
    pub tags: Vec<String>,
}

#[derive(Debug)]
pub enum ParseError {
    /// The page parsed but held no books and no "no results" marker — which
    /// means the markup changed under us, not that the query found nothing.
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

/// Undo the five XML entities SE's escaper emits. Titles carry `&amp;` and
/// curly quotes arrive as literal UTF-8, so this is the whole set in practice.
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

    // Each result opens with `<li typeof="schema:Book" about="/ebooks/…">`.
    // The `about` is the book path, so a hit never needs a URL guessed for it.
    while let Some(rel) = html[cursor..].find("typeof=\"schema:Book\"") {
        let item = cursor + rel;
        // Bound the item at the next book so a malformed entry cannot swallow
        // the rest of the page and attribute the next book's cover to it.
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

        // Two `schema:name` spans per item, in document order: the title (in
        // the heading link), then the author (inside `<p class="author">`).
        let Some((title, after_title)) = schema_name_after(block, 0) else {
            continue;
        };
        let author = schema_name_after(block, after_title)
            .map(|(a, _)| a)
            .unwrap_or_default();

        // Cover: the `<img src>` @2x, the largest raster SE offers here. The
        // `<source srcset>` siblings are AVIF and a JPEG srcset pair — AVIF the
        // `image` crate cannot decode, and a srcset needs splitting on
        // descriptors, so `src` is both simpler and right.
        //
        // **Its absence is what marks an entry as not downloadable**, which is
        // why it is required rather than optional. Standard Ebooks lists three
        // kinds of entry Steb cannot serve, and none of them has cover art:
        // announced but not yet public domain (`class="ribbon not-pd"`), public
        // domain but not yet produced (`class="ribbon wanted"`), and pages with
        // neither ribbon nor any download on them. Measured across the
        // fixtures: no ribboned entry ever carries a cover, and the cover test
        // additionally catches the unribboned ones — so this single check
        // subsumes the ribbon vocabulary and survives SE adding another label.
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

    // A genuinely empty result set is normal — SE has no typo tolerance, so a
    // mistyped query on a device keyboard lands here often. Distinguish it from
    // markup drift by looking for the page furniture that is present either
    // way; if even that is gone, we are not looking at a listing page.
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

/// Subject tags from the page's own `<select name="tags[]">`, minus the `all`
/// sentinel. Empty when the select is absent, in which case the caller keeps
/// whatever vocabulary it already had.
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
        // SE's "all tags" option means "no filter" and is not a subject.
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

/// Highest `page=N` in the pagination nav, or 1 when there is no nav (a result
/// set that fits one page).
///
/// The boundary check is load-bearing and was not obvious: hrefs look like
/// `/ebooks?page=2&amp;per-page=48`, so a naive search for `page=` also matches
/// the `page=48` *inside* `per-page=48`. Steb always sends `per-page=48`, so
/// without this every listing would report at least 48 pages — a total that
/// looks plausible, is wrong on every page, and reconciles with nothing. A real
/// parameter is preceded by `?`, `&`, or the `;` ending an `&amp;` entity.
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

/// Is there a page after this one?
///
/// The authoritative signal, and independent of how many links the nav happens
/// to render: SE marks the forward control `rel="next"` and gives it an href
/// only when there is somewhere to go. On the last page it becomes
/// `aria-disabled="true"` with no href.
fn has_next(html: &str) -> bool {
    nav_block(html).is_some_and(|nav| {
        nav.find("rel=\"next\"").is_some_and(|at| {
            // Walk back to the opening `<a` and check it carried an href.
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
        // Bare /ebooks defaults to 12 per page, newest first.
        assert_eq!(page.hits.len(), 12);
        // SE lists every page, not a window — so this is the real catalogue
        // size and the pager can show it honestly.
        assert!(
            page.total_pages > 100,
            "expected the full catalogue, got {} pages",
            page.total_pages
        );
    }

    #[test]
    fn the_same_parser_handles_a_search_page() {
        // The point of one parser: browse and search markup are identical.
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
        // Guards the bounding of one <li> against the next: a title bleeding
        // into the following book shows up here first.
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
        // `arthur-b-reeve/the-war-terror` carries no ribbon, so a class-based
        // filter would keep it — but its book page offers no download at all.
        // The missing cover is the signal that catches it.
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
        // Counterweight: cover extraction failing wholesale would mark every
        // entry unavailable and empty the grid while still "working".
        let page = parse(BROWSE).unwrap();
        assert!(
            page.hits.len() > page.unavailable * 4,
            "{} kept vs {} dropped — cover extraction is probably broken",
            page.hits.len(),
            page.unavailable
        );
    }

    /// The nav shape when `per-page` is in play.
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
        // `per-page=48` contains the substring `page=48`. Steb always sends
        // per-page=48, so a scanner without a parameter-boundary check reports
        // 48 pages on every listing — plausible-looking and wrong everywhere.
        let page = parse(NAV_WITH_PER_PAGE).unwrap();
        assert_eq!(page.total_pages, 32, "should read the real highest page");
    }

    #[test]
    fn has_next_follows_ses_own_control() {
        assert!(parse(NAV_WITH_PER_PAGE).unwrap().has_next);

        // Last page: `rel="next"` is still there but carries no href.
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
        // This query is mostly entries Steb cannot serve, in two flavours SE
        // labels differently — `ribbon not-pd` (not yet public domain) and
        // `ribbon wanted` (public domain, not yet produced). Neither has cover
        // art, which is why one check handles both without knowing the labels.
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
        // Hardcoding this list would let the filter menu drift out of step
        // with the site; reading it means a new SE subject just appears.
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
        // The caller keeps whatever list it already had rather than clearing.
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
        // Routine on a device keyboard, since SE has no typo tolerance.
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
