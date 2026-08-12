//! Which sort the user picked.
//!
//! Standard Ebooks sorts server-side, so this holds a choice and nothing else:
//! there is no comparator here, and the choice reaches the server as a query
//! parameter.
//!
//! There is no sort *direction* either. SE's options bake the direction into
//! the option itself ("Author name (a → z)", "Length (short → long)"), so there
//! is nothing to toggle.

use crate::se::url::Sort;

/// What the sort menu offers, in SE's own order.
///
/// [`None`] is *SE's default* rather than an absence of opinion, and it is
/// deliberately first: it is what the app opens on, and picking it again is how
/// the user gets back. Sending no `sort` parameter is what makes SE apply
/// release-date-newest while browsing and relevance while searching — the right
/// thing in each mode, without us having to know which mode we are in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SortState(pub Option<Sort>);

impl SortState {
    /// Menu rows, in display order.
    pub const ALL: [SortState; 7] = [
        SortState(None),
        SortState(Some(Sort::Relevance)),
        SortState(Some(Sort::Newest)),
        SortState(Some(Sort::AuthorAlpha)),
        SortState(Some(Sort::ReadingEase)),
        SortState(Some(Sort::Length)),
        SortState(Some(Sort::Popularity)),
    ];

    /// SE's own wording, so the menu matches what the site says.
    pub fn label(self) -> &'static str {
        match self.0 {
            None => "Default",
            Some(Sort::Relevance) => "Relevance",
            Some(Sort::Newest) => "Release date (new → old)",
            Some(Sort::AuthorAlpha) => "Author name (a → z)",
            Some(Sort::ReadingEase) => "Reading ease (easy → hard)",
            Some(Sort::Length) => "Length (short → long)",
            Some(Sort::Popularity) => "Popularity (most → least)",
        }
    }

    /// Whether this row is worth offering right now.
    ///
    /// SE only offers relevance when there is something to be relevant to, so
    /// the row is hidden while browsing rather than shown and silently ignored.
    pub fn available(self, has_query: bool) -> bool {
        has_query || self.0 != Some(Sort::Relevance)
    }

    /// The grid header line.
    pub fn header(self) -> String {
        match self.0 {
            None => "Latest releases".to_string(),
            Some(_) => self.label().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_ses_default() {
        assert_eq!(SortState::default(), SortState(None));
        assert_eq!(SortState::default().header(), "Latest releases");
    }

    #[test]
    fn relevance_is_hidden_while_browsing() {
        let relevance = SortState(Some(Sort::Relevance));
        assert!(!relevance.available(false));
        assert!(relevance.available(true));
        // Everything else is always offered.
        for s in SortState::ALL.iter().filter(|s| **s != relevance) {
            assert!(s.available(false), "{} should always be offered", s.label());
        }
    }

    #[test]
    fn every_row_has_a_label() {
        for s in SortState::ALL {
            assert!(!s.label().is_empty());
        }
    }
}
