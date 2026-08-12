//! Which subject tags the user has selected.
//!
//! Standard Ebooks filters by subject tag, server-side, and the vocabulary
//! comes from the listing page itself rather than from anything computed here.
//!
//! That is why there is no `matches()` predicate. Nothing is filtered on
//! device: selections become `tags[]` parameters and SE returns the narrowed
//! listing.

use std::collections::BTreeSet;

/// SE's "all tags" sentinel. Present as an `<option>` in the page's tag select,
/// but it means "no filter", so it is never sent and never stored.
pub const ALL: &str = "all";

/// The user's current selection. Empty means unfiltered, which is the state the
/// app opens in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filters {
    tags: BTreeSet<String>,
}

impl Filters {
    /// How many tags are active — the count the pager strip shows on its Filter
    /// slot so the user can see a filter is on without opening the menu.
    pub fn count(&self) -> usize {
        self.tags.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }

    pub fn is_selected(&self, tag: &str) -> bool {
        self.tags.contains(tag)
    }

    /// Toggle one tag. Selecting SE's `all` sentinel clears instead, which is
    /// what it means on the site.
    pub fn toggle(&mut self, tag: &str) {
        if tag == ALL {
            self.clear();
            return;
        }
        if !self.tags.remove(tag) {
            self.tags.insert(tag.to_string());
        }
    }

    pub fn clear(&mut self) {
        self.tags.clear();
    }

    /// The values to put in the URL, in a stable order so two identical
    /// selections always build the same URL.
    pub fn as_params(&self) -> Vec<String> {
        self.tags.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_unfiltered() {
        let f = Filters::default();
        assert!(f.is_empty());
        assert_eq!(f.count(), 0);
        assert!(f.as_params().is_empty());
    }

    #[test]
    fn toggling_adds_then_removes() {
        let mut f = Filters::default();
        f.toggle("fantasy");
        assert!(f.is_selected("fantasy"));
        assert_eq!(f.count(), 1);
        f.toggle("fantasy");
        assert!(!f.is_selected("fantasy"));
        assert!(f.is_empty());
    }

    #[test]
    fn the_all_sentinel_clears_rather_than_being_selected() {
        let mut f = Filters::default();
        f.toggle("horror");
        f.toggle("poetry");
        assert_eq!(f.count(), 2);
        f.toggle(ALL);
        assert!(f.is_empty(), "`all` means no filter, not a tag named all");
        assert!(!f.is_selected(ALL));
    }

    #[test]
    fn params_are_stable_regardless_of_selection_order() {
        let mut a = Filters::default();
        a.toggle("poetry");
        a.toggle("comedy");
        let mut b = Filters::default();
        b.toggle("comedy");
        b.toggle("poetry");
        assert_eq!(a.as_params(), b.as_params());
    }
}
