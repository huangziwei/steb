//! The search bar: one widget at one geometry, drawn in the grid view and in
//! the [`crate::ui::search`] overlay alike.

use crate::eink::fb::Framebuffer;
use crate::ui::grid;
use crate::ui::scale::Scale;
use crate::ui::text::TextRenderer;

/// Geometry, shared by every view drawing the bar. Design pixels; see
/// [`crate::ui::scale`].
const TOP_PX: u32 = 16;
const HEIGHT_PX: u32 = 88;
const MARGIN_X_PX: u32 = 40;
/// Right-hand zone clearing the query, live under `query_active`.
const CLEAR_W_PX: u32 = 150;
/// Magnifier radius, the gap after it, and the `✕` radius.
const GLYPH_R: u32 = 18;
const GLYPH_GAP: u32 = 24;
const CLEAR_R: u32 = 15;
/// Pill stroke thickness.
const STROKE: u32 = 3;
/// Gap between the typed text and the caret after it.
const CARET_GAP: u32 = 8;
/// Gap between the bar and the top of the grid.
const GRID_GAP: u32 = 16;

/// Bar top on a panel `fb_xres` wide.
pub fn top(fb_xres: u32) -> u32 {
    Scale::of_width(fb_xres).px(TOP_PX)
}

/// Bar height on a panel `fb_xres` wide.
pub fn height(fb_xres: u32) -> u32 {
    Scale::of_width(fb_xres).px(HEIGHT_PX)
}

/// Headroom the bar takes above the grid: its own top, height and the gap
/// under it. `ui::grid` lays out against what is left below this.
pub fn margin(fb_xres: u32) -> u32 {
    top(fb_xres) + height(fb_xres) + Scale::of_width(fb_xres).px(GRID_GAP)
}

/// Side margin on a panel `fb_xres` wide.
pub fn margin_x(fb_xres: u32) -> u32 {
    Scale::of_width(fb_xres).px(MARGIN_X_PX)
}

/// Clear-zone width on a panel `fb_xres` wide, never more than a third of the
/// field: a fixed zone swallows a narrow panel's whole query.
fn clear_w(fb_xres: u32) -> u32 {
    Scale::of_width(fb_xres)
        .px(CLEAR_W_PX)
        .min(field_w(fb_xres) / 3)
}

/// Search-field pill width: the full span between the side margins.
pub fn field_w(xres: u32) -> u32 {
    xres.saturating_sub(margin_x(xres) * 2)
}

/// A tap on the bar.
pub enum Tap {
    /// The field, opening [`crate::ui::search`].
    Open,
    /// The `✕` zone, clearing the query.
    Clear,
}

/// Hit-tests the bar. `query_active` enables the `✕` zone.
pub fn hit(tx: u32, ty: u32, xres: u32, query_active: bool) -> Option<Tap> {
    let (top, height) = (top(xres), height(xres));
    if !(top..top + height).contains(&ty) {
        return None;
    }
    let x = margin_x(xres);
    let w = field_w(xres);
    if !(x..x + w).contains(&tx) {
        return None;
    }
    if query_active && tx >= x + w - clear_w(xres) {
        return Some(Tap::Clear);
    }
    Some(Tap::Open)
}

/// A rounded pill, a magnifier glyph, the placeholder or query, and an `✕`
/// under a set query.
pub fn draw(fb: &mut Framebuffer, renderer: &mut TextRenderer, query: &str) {
    render(fb, renderer, query, "", false);
}

/// [`draw`] with what an IME is composing drawn after `query`, underlined, and
/// a caret at the end of the two. The overlay in [`crate::ui::search`] draws
/// through this; nothing matches a query against the `preedit`.
pub fn draw_composing(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    query: &str,
    preedit: &str,
) {
    render(fb, renderer, query, preedit, true);
}

/// The bar itself. `caret` marks the field the keyboard is typing into; the
/// grid's copy of the bar is not one.
fn render(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    query: &str,
    preedit: &str,
    caret: bool,
) {
    let xres = fb.var.xres;
    let s = Scale::of_width(xres);
    let (top, height) = (top(xres), height(xres));
    let x = margin_x(xres);
    let w = field_w(xres);
    let cy = (top + height / 2) as i32;
    let baseline = (top + height * 62 / 100) as i32;

    // Pill frame, magnifier inside the left rounded end.
    grid::stroke_round_rect(
        fb,
        x as i32,
        top as i32,
        w,
        height,
        height / 2,
        s.px(STROKE),
        0x00,
    );
    let mr = s.px(GLYPH_R);
    let mcx = (x + height / 2 + s.px(6)) as i32;
    grid::draw_magnifier(fb, mcx, cy, mr, 0x00);
    let text_x = mcx + mr as i32 + s.i(GLYPH_GAP as i32);

    if query.trim().is_empty() && preedit.is_empty() {
        // The two fields SE matches a query against.
        renderer.draw(fb, text_x, baseline, "Search title or author", false);
        return;
    }
    // Query text, tail-first past the field width, and the clear button.
    let clear_w = clear_w(xres);
    let right_limit = (x + w).saturating_sub(clear_w) as i32;
    let avail = (right_limit - text_x).max(0) as u32;
    let shown = clamp_tail(renderer, &format!("{query}{preedit}"), avail);
    let shown_w = renderer.measure_width(&shown) as i32;
    renderer.draw(fb, text_x, baseline, &shown, false);
    if !preedit.is_empty() {
        // The composing tail carries a rule under it, as the framework's own
        // fields draw one.
        let under = renderer.measure_width(preedit).min(shown_w as u32);
        let rule = s.px(STROKE);
        fb.fill_rect(
            (baseline + rule as i32 * 2).max(0) as u32,
            (text_x + shown_w - under as i32).max(0) as u32,
            under,
            rule,
            0x00,
        );
    }
    if caret {
        let caret_h = height / 2;
        fb.fill_rect(
            (cy - caret_h as i32 / 2).max(0) as u32,
            (text_x + shown_w + s.i(CARET_GAP as i32)).max(0) as u32,
            s.px(STROKE),
            caret_h,
            0x00,
        );
    }
    let clear_cx = (x + w).saturating_sub(clear_w / 2) as i32;
    grid::draw_x(fb, clear_cx, cy, s.i(CLEAR_R as i32), 0x00);
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
