//! The chosen sort. SE sorts server-side and bakes direction into each option
//! ("Author name (a → z)"), so this holds a choice: no comparator, no toggle.

use crate::se::url::Sort;

/// The menu rows, in SE's own order. [`None`] leads: it sends no `sort`
/// parameter, leaving SE's own default in either mode.
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

    /// SE's own wording, for anywhere with a line to spare.
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

    /// [`SortState::label`] cut to a chip, which is as wide as its own text:
    /// SE's parenthesised direction would leave two chips to a line.
    pub fn chip(self) -> &'static str {
        match self.0 {
            None => "Default",
            Some(Sort::Relevance) => "Relevance",
            Some(Sort::Newest) => "Newest",
            Some(Sort::AuthorAlpha) => "Author a–z",
            Some(Sort::ReadingEase) => "Easiest",
            Some(Sort::Length) => "Shortest",
            Some(Sort::Popularity) => "Most read",
        }
    }

    /// Whether this row is offered under `has_query`. `Relevance` is
    /// search-only.
    pub fn available(self, has_query: bool) -> bool {
        has_query || self.0 != Some(Sort::Relevance)
    }

    /// The states offered under `has_query`, in display order.
    pub fn offered(has_query: bool) -> Vec<SortState> {
        SortState::ALL
            .into_iter()
            .filter(|s| s.available(has_query))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_ses_default() {
        assert_eq!(SortState::default(), SortState(None));
        assert_eq!(SortState::default().chip(), "Default");
    }

    #[test]
    fn relevance_is_hidden_while_browsing() {
        let relevance = SortState(Some(Sort::Relevance));
        assert!(!relevance.available(false));
        assert!(relevance.available(true));
        for s in SortState::ALL.iter().filter(|s| **s != relevance) {
            assert!(s.available(false), "{} should always be offered", s.label());
        }
        // Only that one row comes and goes with a query.
        assert_eq!(SortState::offered(true).len(), SortState::ALL.len());
        assert_eq!(SortState::offered(false).len(), SortState::ALL.len() - 1);
        assert!(!SortState::offered(false).contains(&relevance));
        assert_eq!(SortState::offered(true)[0], SortState::default());
    }

    #[test]
    fn every_row_has_a_label_and_a_shorter_chip() {
        for s in SortState::ALL {
            assert!(!s.label().is_empty());
            assert!(!s.chip().is_empty());
            assert!(
                s.chip().chars().count() <= s.label().chars().count(),
                "{} is not shorter than {}",
                s.chip(),
                s.label()
            );
            // A chip carries no direction in brackets; that is what it drops.
            assert!(!s.chip().contains('('), "{}", s.chip());
        }
    }
}
