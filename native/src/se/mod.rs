//! The standardebooks.org client.
//!
//! Split by page shape rather than by feature, because that is how the site is
//! organised and it keeps each parser next to the markup it understands:
//!
//! - [`url`] — the closed set of URLs we may fetch, and the honeypot rule
//! - [`http`] — the only place that touches the network
//! - [`listing`] — `/ebooks`, with or without a query (one parser, both modes)
//! - [`book`] — a book's page, read for its `.azw3` and cover thumbnail
//! - [`download`] — fetch, verify, and commit an azw3 to the library
//! - [`feed`] — the public new-releases feed, our delta channel

pub mod book;
pub mod download;
pub mod feed;
pub mod http;
pub mod listing;
pub mod url;
