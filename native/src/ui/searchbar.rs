//! The search bar: one widget at one geometry, drawn in the grid view and the
//! `keyboard` overlay alike.

use crate::eink::fb::Framebuffer;
use crate::ui::grid;
use crate::ui::text::TextRenderer;

/// Geometry, shared by every view drawing the bar.
pub const TOP: u32 = 16;
pub const HEIGHT: u32 = 88;
pub const MARGIN_X: u32 = 40;
/// Right-hand zone clearing the query, live under `query_active`.
pub const CLEAR_W: u32 = 150;

/// Search-field pill width: the full span between the side margins.
pub fn field_w(xres: u32) -> u32 {
    xres.saturating_sub(MARGIN_X * 2)
}

/// A tap on the bar.
pub enum Tap {
    /// The field, opening `keyboard`.
    Open,
    /// The `✕` zone, clearing the query.
    Clear,
}

/// Hit-tests the bar. `query_active` enables the `✕` zone.
pub fn hit(tx: u32, ty: u32, xres: u32, query_active: bool) -> Option<Tap> {
    if !(TOP..TOP + HEIGHT).contains(&ty) {
        return None;
    }
    let x = MARGIN_X;
    let w = field_w(xres);
    if !(x..x + w).contains(&tx) {
        return None;
    }
    if query_active && tx >= x + w - CLEAR_W {
        return Some(Tap::Clear);
    }
    Some(Tap::Open)
}

/// A rounded pill, a magnifier glyph, the placeholder or query, and an `✕`
/// under a set query.
pub fn draw(fb: &mut Framebuffer, renderer: &mut TextRenderer, query: &str) {
    let xres = fb.var.xres;
    let x = MARGIN_X;
    let w = field_w(xres);
    let cy = (TOP + HEIGHT / 2) as i32;
    let baseline = (TOP + HEIGHT * 62 / 100) as i32;

    // Pill frame, magnifier inside the left rounded end.
    grid::stroke_round_rect(fb, x as i32, TOP as i32, w, HEIGHT, HEIGHT / 2, 3, 0x00);
    let mr = 18u32;
    let mcx = (x + HEIGHT / 2 + 6) as i32;
    grid::draw_magnifier(fb, mcx, cy, mr, 0x00);
    let text_x = mcx + mr as i32 + 24;

    if query.trim().is_empty() {
        // The two fields SE matches a query against.
        renderer.draw(fb, text_x, baseline, "Search title or author", false);
        return;
    }
    // Query text, tail-first past the field width, and the clear button.
    let right_limit = (x + w).saturating_sub(CLEAR_W) as i32;
    let avail = (right_limit - text_x).max(0) as u32;
    let shown = clamp_tail(renderer, query, avail);
    renderer.draw(fb, text_x, baseline, &shown, false);
    let clear_cx = (x + w).saturating_sub(CLEAR_W / 2) as i32;
    grid::draw_x(fb, clear_cx, cy, 15, 0x00);
}

/// The trailing substring of `s` fitting `max_width`.
fn clamp_tail(renderer: &mut TextRenderer, s: &str, max_width: u32) -> String {
    if renderer.measure_width(s) <= max_width {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut start = 0;
    while start < chars.len() {
        let tail: String = chars[start..].iter().collect();
        if renderer.measure_width(&tail) <= max_width {
            return tail;
        }
        start += 1;
    }
    String::new()
}
