//! The modules with no `eink` dependency, built by `cargo test` on the host.
//! `main.rs` declares these again alongside the Linux-only ones.

pub mod cache;
pub mod convert;
pub mod se;

/// `filter` and `sort`, holding a selection as query parameters.
pub mod ui {
    #[path = "filter.rs"]
    pub mod filter;
    #[path = "sort.rs"]
    pub mod sort;
}
