//! Library facade exposing the pure-logic modules so `cargo test` runs on the
//! host.
//!
//! The full binary is Linux-only: `eink/fb.rs` and `eink/touch.rs` call
//! `libc::ioctl`, whose request argument is `c_ulong` on BSD and `c_int` on
//! Linux, so they cannot compile on macOS. Anything touching the framebuffer,
//! touchscreen or X server therefore lives in `main.rs` only, and everything
//! pure — URL construction, HTML and Atom parsing, the catalogue cache — is
//! re-declared here so the test runner can build it without dragging the
//! device modules in.

pub mod cache;
pub mod se;

/// The two UI modules with no device dependency at all.
///
/// `filter` and `sort` hold the user's current selection and turn it into query
/// parameters — no framebuffer, no touch, no fonts. Filtering and sorting both
/// happen on Standard Ebooks' side, so neither module carries a comparator or a
/// predicate, which is what makes them testable here rather than on the device.
pub mod ui {
    #[path = "filter.rs"]
    pub mod filter;
    #[path = "sort.rs"]
    pub mod sort;
}
