//! The catalogue cache — book records that survive a monthly update.
//!
//! We are scraping someone else's server for free, so the design goal is that a
//! launch with nothing new costs one conditional request and no listing fetch
//! at all.
//!
//! **The key is a book, never a page number.** Standard Ebooks orders by
//! release date newest-first, so publishing N new books shifts every page
//! boundary — page 3 today is not page 3 next month. A page-keyed cache would
//! therefore self-destruct on exactly the monthly event it needs to survive.
//! Keyed by the book's own path, a record is stable forever and a catalogue
//! update can only ever **add** rows. That is why there is no invalidation
//! logic in this module: none is reachable.
//!
//! Cover images are not here. They are content-addressed by the sha in their
//! URL and live as files under `covers/` — a re-produced cover arrives under a
//! new name and orphans the old one, so that cache needs no invalidation
//! either. There is deliberately no size ceiling on it: the whole catalogue is
//! roughly 50 MB of covers against a 16–32 GB device, and a cover fetched once
//! should stay fetched.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::se::feed;
use crate::se::http::Validators;
use crate::se::listing::Hit;
use crate::se::url::BookPath;

/// One book as we persist it. Everything needed to draw a grid cell without
/// touching the network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub title: String,
    pub author: String,
    /// Cover URL path. Stored as a string because it is only ever re-parsed on
    /// load, and keeping the validated type out of the on-disk shape means a
    /// future markup change cannot make the whole cache unreadable.
    pub cover: Option<String>,
}

/// On-disk shape. Versioned so a format change can be detected and discarded
/// rather than misread — the only circumstance in which the cache is dropped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalogue {
    version: u32,
    /// Validators from the last feed fetch, replayed as `If-None-Match` /
    /// `If-Modified-Since` to get a 304 next launch.
    #[serde(default)]
    pub feed: Validators,
    /// Ordered by book path. A `BTreeMap` rather than a `Vec` so a merge is a
    /// key-wise insert and duplicate arrivals are idempotent — re-reading a
    /// listing page must never double a book.
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

/// What a feed check implies about how much we missed.
#[derive(Debug, PartialEq, Eq)]
pub enum Freshness {
    /// Every entry in the feed is already known — nothing to fetch.
    UpToDate,
    /// Some entries are new, but the oldest is known, so the feed window spans
    /// the whole gap and one listing page will close it.
    Behind,
    /// Even the oldest feed entry is unknown. The device has been off longer
    /// than the 15-entry window covers, so the gap is of unknown size and must
    /// be walked page by page until a known book reappears.
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

    /// Merge listing hits. Purely additive in the sense that matters: no book
    /// is ever removed, and the count can only grow. A record already present
    /// is refreshed in place, which is how a re-produced cover's new URL lands
    /// without any invalidation step.
    ///
    /// Returns how many books were genuinely new.
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

    /// Classify what the feed tells us, without fetching anything.
    ///
    /// The window is 15 entries and SE ships a few books a week, so a device
    /// left off for a couple of months can have a gap the feed cannot show.
    /// Detecting that is the difference between a catalogue with a permanent
    /// hole in it and one that heals.
    pub fn freshness(&self, entries: &[feed::Entry]) -> Freshness {
        if entries.is_empty() || entries.iter().all(|e| self.contains(&e.path)) {
            return Freshness::UpToDate;
        }
        // An empty catalogue is a first run, not a gap. Every feed entry is
        // unknown because we have never known anything, and the ordinary
        // browse fetch is about to populate us from page one — so reporting
        // `FarBehind` here would send a brand-new install off to walk the
        // catalogue backwards looking for an overlap that cannot exist.
        if self.is_empty() {
            return Freshness::UpToDate;
        }
        // Entries are newest-first, so the last is the oldest the feed knows.
        // If we have never seen even that one, everything between it and our
        // newest is invisible to us.
        match entries.last() {
            Some(oldest) if self.contains(&oldest.path) => Freshness::Behind,
            _ => Freshness::FarBehind,
        }
    }

    /// Have we caught up with a listing page? True once a page contains a book
    /// we already knew, which is the signal to stop walking backwards.
    pub fn overlaps(&self, hits: &[Hit]) -> bool {
        hits.iter().any(|h| self.contains(&h.path))
    }
}

/// Where the cache lives, relative to the extension bundle.
pub fn catalogue_path(bundle_dir: &Path) -> PathBuf {
    bundle_dir.join("cache").join("catalogue.json")
}

pub fn covers_dir(bundle_dir: &Path) -> PathBuf {
    bundle_dir.join("cache").join("covers")
}

/// Read the cache. Any failure — missing, truncated, wrong version — yields an
/// empty catalogue rather than an error: a corrupt cache should cost a slower
/// first launch, never a launch.
pub fn load(path: &Path) -> Catalogue {
    let Ok(text) = fs::read_to_string(path) else {
        return Catalogue::default();
    };
    match serde_json::from_str::<Catalogue>(&text) {
        Ok(c) if c.version == VERSION => c,
        _ => Catalogue::default(),
    }
}

/// Write atomically. The user partition is FAT, so a crash mid-write must not
/// be able to leave a half-written JSON that the next launch would discard —
/// bytes land in a `.partial` sibling and are renamed over the target.
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
        // The same page again must not double anything.
        assert_eq!(cat.merge(&hits), 0);
        assert_eq!(cat.len(), before);
    }

    #[test]
    fn a_catalogue_update_adds_rows_and_invalidates_nothing() {
        // The requirement, stated as a test: new releases arrive, and every
        // book we already knew is still there afterwards.
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
        // Seed the catalogue with exactly the feed's books.
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
        // We know where the feed's window bottoms out, so one page closes it.
        assert_eq!(cat.freshness(&entries), Freshness::Behind);
    }

    #[test]
    fn a_first_run_is_not_behind() {
        // Every entry is unknown because nothing is known yet — the browse
        // fetch populates from page one, so there is no gap to walk.
        let entries = vec![entry("a/new"), entry("c/old")];
        let cat = Catalogue::default();
        assert!(cat.is_empty());
        assert_eq!(cat.freshness(&entries), Freshness::UpToDate);
    }

    #[test]
    fn an_entirely_unknown_feed_means_we_fell_off_the_window() {
        // A device left off for months: even the oldest of the 15 is unseen,
        // so the gap is bigger than the feed can describe.
        //
        // The catalogue must be non-empty for this to mean anything — with
        // nothing cached it is a first run, not a gap (see the test above).
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

        // A future format is discarded rather than misread.
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
