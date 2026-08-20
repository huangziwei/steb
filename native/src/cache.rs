//! The catalogue cache: book records keyed by [`crate::se::url::BookPath`].
//! A key names a book, never a page, and [`Catalogue::merge`] only ever adds.
//! Cover images live under `covers/`, held by [`crate::cover_cache`].

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::se::feed;
use crate::se::http::Validators;
use crate::se::listing::Hit;
use crate::se::url::BookPath;

/// One persisted book, carrying everything a grid cell draws.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub title: String,
    pub author: String,
    /// Cover URL path, persisted as a string and re-parsed on load.
    pub cover: Option<String>,
}

/// The on-disk shape. [`load`] discards a `version` it does not know.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalogue {
    version: u32,
    /// Validators from the last feed fetch, replayed as `If-None-Match` and
    /// `If-Modified-Since`.
    #[serde(default)]
    pub feed: Validators,
    /// Ordered by book path. A key-wise insert makes [`Catalogue::merge`]
    /// idempotent over a re-read listing page.
    #[serde(default)]
    books: BTreeMap<String, Record>,
}

const VERSION: u32 = 1;

impl Default for Catalogue {
    fn default() -> Self {
        Self {
            version: VERSION,
            feed: Validators::default(),
            books: BTreeMap::new(),
        }
    }
}

/// What a feed check says about the gap.
#[derive(Debug, PartialEq, Eq)]
pub enum Freshness {
    /// Every feed entry is a known book.
    UpToDate,
    /// Some entries are new, the oldest is known: one listing page closes it.
    Behind,
    /// Even the oldest feed entry is unknown: a gap past the 15-entry window,
    /// walked page by page until [`Catalogue::caught_up`].
    FarBehind,
}

impl Catalogue {
    pub fn len(&self) -> usize {
        self.books.len()
    }

    pub fn is_empty(&self) -> bool {
        self.books.is_empty()
    }

    pub fn get(&self, path: &BookPath) -> Option<&Record> {
        self.books.get(path.as_key())
    }

    pub fn contains(&self, path: &BookPath) -> bool {
        self.books.contains_key(path.as_key())
    }

    /// Merges listing hits, refreshing a present record in place and removing
    /// none. Returns the count of new books.
    pub fn merge(&mut self, hits: &[Hit]) -> usize {
        let mut added = 0;
        for hit in hits {
            let key = hit.path.as_key().to_string();
            let record = Record {
                title: hit.title.clone(),
                author: hit.author.clone(),
                cover: Some(hit.cover.as_path().to_string()),
            };
            if self.books.insert(key, record).is_none() {
                added += 1;
            }
        }
        added
    }

    /// [`Freshness`] for `entries`, fetching nothing.
    pub fn freshness(&self, entries: &[feed::Entry]) -> Freshness {
        if entries.is_empty() || entries.iter().all(|e| self.contains(&e.path)) {
            return Freshness::UpToDate;
        }
        // An empty catalogue is a first run. The browse fetch populates it
        // from page one.
        if self.is_empty() {
            return Freshness::UpToDate;
        }
        // Entries run newest-first: the last is the oldest the feed carries.
        match entries.last() {
            Some(oldest) if self.contains(&oldest.path) => Freshness::Behind,
            _ => Freshness::FarBehind,
        }
    }

    /// True once `hits` carries a known book, ending a [`Freshness::FarBehind`]
    /// walk.
    pub fn overlaps(&self, hits: &[Hit]) -> bool {
        hits.iter().any(|h| self.contains(&h.path))
    }
}

/// The cache file under the extension bundle.
pub fn catalogue_path(bundle_dir: &Path) -> PathBuf {
    bundle_dir.join("cache").join("catalogue.json")
}

pub fn covers_dir(bundle_dir: &Path) -> PathBuf {
    bundle_dir.join("cache").join("covers")
}

/// The cache at `path`. Missing, truncated or a foreign `version` reads as an
/// empty [`Catalogue`].
pub fn load(path: &Path) -> Catalogue {
    let Ok(text) = fs::read_to_string(path) else {
        return Catalogue::default();
    };
    match serde_json::from_str::<Catalogue>(&text) {
        Ok(c) if c.version == VERSION => c,
        _ => Catalogue::default(),
    }
}

/// `catalogue` into a `.partial` sibling, renamed over `path`.
pub fn store(path: &Path, catalogue: &Catalogue) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string(catalogue).map_err(io::Error::other)?;
    let partial = path.with_extension("json.partial");
    fs::write(&partial, json)?;
    fs::rename(&partial, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::se::listing;

    fn hits_from(fixture: &str) -> Vec<Hit> {
        listing::parse(fixture).unwrap().hits
    }

    const BROWSE: &str = include_str!("../tests/fixtures/listing-browse-p1.html");
    const SEARCH: &str = include_str!("../tests/fixtures/listing-search-dracula.html");
    const FEED: &str = include_str!("../tests/fixtures/feed-new-releases.xml");

    fn entry(key: &str) -> feed::Entry {
        feed::Entry {
            path: BookPath::parse(&format!("/ebooks/{key}")).unwrap(),
            title: key.to_string(),
        }
    }

    #[test]
    fn merging_is_additive_and_idempotent() {
        let mut cat = Catalogue::default();
        let hits = hits_from(BROWSE);
        assert_eq!(cat.merge(&hits), hits.len());
        let before = cat.len();
        // The same page twice.
        assert_eq!(cat.merge(&hits), 0);
        assert_eq!(cat.len(), before);
    }

    #[test]
    fn a_catalogue_update_adds_rows_and_invalidates_nothing() {
        // New releases arrive; every known book survives.
        let mut cat = Catalogue::default();
        cat.merge(&hits_from(SEARCH));
        let known: Vec<String> = cat.books.keys().cloned().collect();
        let before = cat.len();

        let added = cat.merge(&hits_from(BROWSE));

        assert!(added > 0, "the browse page should contain unseen books");
        assert_eq!(cat.len(), before + added);
        for key in known {
            assert!(
                cat.books.contains_key(&key),
                "{key} was dropped by a catalogue update"
            );
        }
    }

    #[test]
    fn an_empty_feed_response_means_up_to_date() {
        assert_eq!(Catalogue::default().freshness(&[]), Freshness::UpToDate);
    }

    #[test]
    fn a_known_feed_means_nothing_to_do() {
        let mut cat = Catalogue::default();
        let entries = feed::parse(FEED);
        // The catalogue seeded with the feed's own books.
        for e in &entries {
            cat.books.insert(
                e.path.as_key().to_string(),
                Record {
                    title: e.title.clone(),
                    author: String::new(),
                    cover: None,
                },
            );
        }
        assert_eq!(cat.freshness(&entries), Freshness::UpToDate);
    }

    #[test]
    fn knowing_the_oldest_entry_bounds_the_gap() {
        let entries = vec![entry("a/new"), entry("b/mid"), entry("c/old")];
        let mut cat = Catalogue::default();
        cat.books.insert(
            "c/old".into(),
            Record {
                title: "old".into(),
                author: String::new(),
                cover: None,
            },
        );
        // The oldest feed entry is known: one page closes the gap.
        assert_eq!(cat.freshness(&entries), Freshness::Behind);
    }

    #[test]
    fn a_first_run_is_not_behind() {
        // An empty catalogue: no gap to walk.
        let entries = vec![entry("a/new"), entry("c/old")];
        let cat = Catalogue::default();
        assert!(cat.is_empty());
        assert_eq!(cat.freshness(&entries), Freshness::UpToDate);
    }

    #[test]
    fn an_entirely_unknown_feed_means_we_fell_off_the_window() {
        // Even the oldest of the 15 is unseen, over a non-empty catalogue.
        let mut cat = Catalogue::default();
        cat.books.insert(
            "long/ago".into(),
            Record {
                title: "Something we saw once".into(),
                author: String::new(),
                cover: None,
            },
        );
        let entries = vec![entry("a/new"), entry("c/old")];
        assert_eq!(cat.freshness(&entries), Freshness::FarBehind);
    }

    #[test]
    fn overlap_is_the_signal_to_stop_walking_back() {
        let mut cat = Catalogue::default();
        cat.merge(&hits_from(SEARCH));
        assert!(cat.overlaps(&hits_from(SEARCH)));
        assert!(!cat.overlaps(&[]));
    }

    #[test]
    fn a_corrupt_cache_costs_a_slow_launch_not_a_launch() {
        let dir = std::env::temp_dir().join("steb-test-cache");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("catalogue.json");

        fs::write(&path, b"{ truncated").unwrap();
        assert!(load(&path).is_empty());

        // An unknown `version`.
        fs::write(&path, br#"{"version":999,"books":{}}"#).unwrap();
        assert!(load(&path).is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn store_then_load_round_trips_and_keeps_validators() {
        let dir = std::env::temp_dir().join("steb-test-cache-rt");
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("catalogue.json");

        let mut cat = Catalogue::default();
        cat.merge(&hits_from(SEARCH));
        cat.feed = Validators {
            etag: Some("\"abc\"".into()),
            last_modified: Some("Wed, 12 Aug 2026 05:11:33 GMT".into()),
        };
        store(&path, &cat).unwrap();

        let back = load(&path);
        assert_eq!(back.len(), cat.len());
        assert_eq!(back.feed, cat.feed);
        assert!(
            !dir.join("catalogue.json.partial").exists(),
            "partial must be renamed away"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
