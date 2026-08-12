//! The public new-releases Atom feed — our delta channel.
//!
//! Standard Ebooks gates its full OPDS and Atom feeds behind the Patrons
//! Circle. `/feeds/atom/new-releases` is the exception and is open to everyone,
//! which makes it the one polite way to ask "has anything shipped?" without
//! pulling listing pages.
//!
//! It answers conditional requests with `ETag` and `Last-Modified`, so the
//! steady state on launch is a **304 with no body**. That is the single most
//! important property here: a device that opens Steb every day and reads
//! nothing new costs SE one small request per launch.
//!
//! The feed carries the newest 15 entries only, so it is a delta and not a
//! catalogue. Falling behind it is handled by the caller — see
//! [`crate::cache`].

/// How many entries SE keeps in the feed. Used to reason about whether a gap is
/// possible, not to size any buffer.
pub const FEED_WINDOW: usize = 15;

/// One entry. Deliberately thin: the feed has no cover URL, so a genuinely new
/// book still needs one listing fetch before it can be drawn. What the feed
/// gives us cheaply is *identity* — which books exist that we have not seen.
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

/// Parse entries, newest first (the order SE emits them).
///
/// Unparseable entries are skipped rather than failing the whole feed: one odd
/// `<id>` should cost us that book until the next listing fetch, not the entire
/// freshness check.
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

        // <id> is the canonical book URL; <title> is the book's title. The
        // <author><name> here is the author, but we take the title only — a
        // new book gets its full record from the listing fetch that follows.
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
        // The feed's <id> is a full URL and can carry a translator segment;
        // BookPath handles both, which is why identity survives the round trip.
        assert!(!first.path.as_key().is_empty());
        assert!(!first.path.as_key().starts_with('/'));
    }

    #[test]
    fn every_entry_in_the_real_feed_parses() {
        // If SE changes the <id> form this drops below the window and the
        // fallen-behind logic would start firing spuriously.
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
