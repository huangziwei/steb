//! The selected subject tags. SE filters server-side, so a selection reaches
//! it as `tags[]` parameters and no predicate lives here.

use std::collections::BTreeSet;

/// SE's "all tags" `<option>`, meaning no filter. Never sent, never stored.
pub const ALL: &str = "all";

/// The current selection. Empty is unfiltered, the state the app opens in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filters {
    tags: BTreeSet<String>,
}

impl Filters {
    /// Active tags, as the `pager` strip shows on its Filter slot.
    pub fn count(&self) -> usize {
        self.tags.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }

    pub fn is_selected(&self, tag: &str) -> bool {
        self.tags.contains(tag)
    }

    /// Toggles one tag. [`ALL`] clears the selection.
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

    /// The URL values, in a stable order.
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
