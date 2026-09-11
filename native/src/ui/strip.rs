//! The bottom control bar, shared by every screen that ends on one: a rule, a
//! white body, and slots divided by [`separator`]. An action reads
//! `[ Bracketed ]`; a page count does not. [`H`] is also the grid's headroom.

use crate::eink::fb::Framebuffer;
use crate::ui::scale::Scale;
use crate::ui::text::TextRenderer;
use crate::ui::{BLACK, WHITE};

/// Bar height.
pub const H: u32 = 80;
/// The rule along the top, and a separator's thickness.
pub const RULE: u32 = 2;
/// Inset of a separator from the bar's top and bottom.
const SEP_INSET: u32 = 12;

/// [`H`] on a panel `fb_xres` wide.
pub fn h(fb_xres: u32) -> u32 {
    Scale::of_width(fb_xres).px(H)
}

/// The bar's top edge.
pub fn top(fb_xres: u32, fb_yres: u32) -> u32 {
    fb_yres.saturating_sub(h(fb_xres))
}

/// The rule and the white body, answering the top edge the caller draws from.
pub fn base(fb: &mut Framebuffer) -> u32 {
    let top = top(fb.var.xres, fb.var.yres);
    base_at(fb, top)
}

/// [`base`] at a top edge the caller names, for a bar standing clear of the
/// screen's foot: the search overlay's sits on the on-screen keyboard.
pub fn base_at(fb: &mut Framebuffer, top: u32) -> u32 {
    let xres = fb.var.xres;
    let rule = Scale::of_width(xres).px(RULE);
    fb.fill_rect(top, 0, xres, rule, BLACK);
    fb.fill_rect(top + rule, 0, xres, h(xres).saturating_sub(rule), WHITE);
    top
}

/// The baseline every label on the bar sits on: the one leaving equal white
/// above the ink and below it. [`TextRenderer::centred_baseline`] reads the
/// size the renderer is set to, so a caller inside `at_px` gets that size's.
pub fn baseline(fb_xres: u32, top: u32, renderer: &TextRenderer) -> i32 {
    top as i32 + renderer.centred_baseline(h(fb_xres))
}

/// A separator at `x`, dividing the slot left of it from the one right of it.
/// An `x` on either edge draws nothing: a bar does not open or close on a rule.
pub fn separator(fb: &mut Framebuffer, x: u32) {
    let top = top(fb.var.xres, fb.var.yres);
    separator_at(fb, top, x);
}

/// [`separator`] on the bar [`base_at`] drew at `top`.
pub fn separator_at(fb: &mut Framebuffer, top: u32, x: u32) {
    let xres = fb.var.xres;
    let s = Scale::of_width(xres);
    let (rule, inset) = (s.px(RULE), s.px(SEP_INSET));
    if x < rule || x >= xres {
        return;
    }
    fb.fill_rect(
        top + inset,
        x - rule,
        rule,
        h(xres).saturating_sub(inset * 2),
        BLACK,
    );
}

/// `text` centred in the slot `[x, x + w)`. A label wider than its slot starts
/// at the slot's left edge rather than spilling into the one before it.
pub fn label(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    x: u32,
    w: u32,
    baseline: i32,
    text: &str,
    inverted: bool,
) {
    let tw = renderer.measure_width(text);
    let tx = x + w.saturating_sub(tw) / 2;
    renderer.draw(fb, tx as i32, baseline, text, inverted);
}

#[cfg(test)]
mod tests {
    use super::*;

    const XRES: u32 = 1264;
    const YRES: u32 = 1680;

    /// The bar sits at the bottom and is [`H`] tall.
    #[test]
    fn the_bar_ends_the_panel() {
        assert_eq!(h(XRES), H);
        assert_eq!(top(XRES, YRES), YRES - H);
        assert_eq!(top(XRES, YRES) + h(XRES), YRES);
    }

    /// A 212 dpi panel takes a shorter bar, and the top follows it.
    #[test]
    fn the_bar_scales_with_the_panel() {
        let pw2 = 758;
        assert_eq!(h(pw2), Scale::of_width(pw2).px(H));
        assert!(h(pw2) < H);
        assert_eq!(top(pw2, 1024) + h(pw2), 1024);
    }

    /// A centred baseline leaves equal white above the ink and below it. The
    /// figures stand in for a face's metrics, which are not available here.
    #[test]
    fn a_centred_baseline_balances_the_white() {
        let (box_h, ascent, descent) = (80.0f32, 26.0f32, -6.0f32);
        let b = ((box_h - (ascent - descent)) / 2.0 + ascent).round();
        assert_eq!(b, 50.0);
        assert_eq!(b - ascent, box_h - (b - descent));
    }
}
