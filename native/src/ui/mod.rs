//! On-screen UI: text rasterization + list layout. Every pixel constant here
//! is a design pixel at [`scale::DESIGN_DPI`], which the module drawing with
//! it maps through [`scale::Scale`] for the panel in front of it.

pub mod diag;
pub mod filter;
pub mod filtermenu;
pub mod grid;
pub mod pager;
pub mod scale;
pub mod search;
pub mod searchbar;
pub mod sort;
pub mod sortmenu;
pub mod strip;
pub mod text;
pub mod toast;

/// Shades everything here draws in. `eink::fb` stores one byte per channel and
/// `ui::text` stamps a glyph in one of these.
pub const BLACK: u8 = 0x00;
pub const WHITE: u8 = 0xFF;
