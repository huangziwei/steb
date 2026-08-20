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

    /// SE's own wording.
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

    /// Whether this row is offered under `has_query`. `Relevance` is
    /// search-only.
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
        // Every other row is unconditional.
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
