//! `/feeds/atom/new-releases`, the one Atom feed SE leaves outside the Patrons
//! Circle. It answers conditional requests with `ETag` and `Last-Modified`, and
//! carries [`WINDOW`] entries: a delta, handled by [`crate::cache`].

/// Entries SE keeps in the feed, against `crate::cache::Freshness`.
pub const FEED_WINDOW: usize = 15;

/// One entry, carrying identity alone: the feed holds no cover URL, so a new
/// book takes one listing fetch before the grid draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: crate::se::url::BookPath,
    pub title: String,
}

fn tag_text(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = block.find(&open)? + open.len();
    let end = block[start..].find(&close)? + start;
    Some(decode(block[start..end].trim()))
}

fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Entries newest-first, the order SE emits. An unparseable `<id>` skips that
/// entry.
pub fn parse(xml: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel) = xml[cursor..].find("<entry") {
        let start = cursor + rel;
        let end = xml[start..]
            .find("</entry>")
            .map(|r| start + r)
            .unwrap_or(xml.len());
        let block = &xml[start..end];
        cursor = end + 1;

        // `<id>` is the canonical book URL, `<title>` the book's title.
        let (Some(id), Some(title)) = (tag_text(block, "id"), tag_text(block, "title")) else {
            continue;
        };
        let Ok(path) = crate::se::url::BookPath::parse(&id) else {
            continue;
        };
        out.push(Entry { path, title });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = include_str!("../../tests/fixtures/feed-new-releases.xml");

    #[test]
    fn parses_the_feed_window() {
        let entries = parse(FEED);
        assert_eq!(entries.len(), FEED_WINDOW);
    }

    #[test]
    fn entries_carry_a_usable_book_path() {
        let entries = parse(FEED);
        let first = &entries[0];
        assert!(!first.title.is_empty());
        // A full URL with a translator segment, both of which `BookPath` takes.
        assert!(!first.path.as_key().is_empty());
        assert!(!first.path.as_key().starts_with('/'));
    }

    #[test]
    fn every_entry_in_the_real_feed_parses() {
        // A changed `<id>` form drops this below [`WINDOW`].
        assert_eq!(parse(FEED).len(), FEED_WINDOW);
    }

    #[test]
    fn a_malformed_entry_is_skipped_not_fatal() {
        let xml = r#"
            <feed>
              <entry><id>not-a-book</id><title>Bad</title></entry>
              <entry><id>https://standardebooks.org/ebooks/a-author/a-title</id><title>Good</title></entry>
            </feed>"#;
        let entries = parse(xml);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "Good");
    }
}
