//! Steb's modules, for `cargo test` on the host and for the `preview` binary;
//! `main.rs` declares the same files again as its own crate root. All of it
//! builds off a Kindle, `Framebuffer::offscreen` standing in for the X server.

pub mod cache;
pub mod convert;
pub mod cover_cache;
pub mod eink;
pub mod font;
pub mod keyboard;
pub mod lipc;
pub mod net;
pub mod orientation;
pub mod se;
pub mod ui;
pub mod wrap;
