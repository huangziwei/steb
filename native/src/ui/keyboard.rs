//! On-screen keyboard: a QWERTY grid with `Del` at the number row's right end,
//! `[ Search ]` far right and `[ Back ]` leftmost. A blocking sub-loop shaped
//! like [`crate::ui::filtermenu`]; a single-band DU on a keystroke.

use crate::eink::fb::{Framebuffer, MxcfbRect, WAVEFORM_MODE_DU, WAVEFORM_MODE_GC16};
use crate::eink::input::{Input, InputEvent};
use crate::eink::touch::TouchEvent;
use crate::ui::grid::outline_rect;
use crate::ui::scale::Scale;
use crate::ui::searchbar;
use crate::ui::strip;
use crate::ui::text::TextRenderer;

/// Letter/digit rows. Row 0 carries `Del` in an eleventh cell at its right end,
/// appended by [`layout`].
const ROWS: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl", "zxcvbnm"];

/// Gap between key faces, and the panel margin. Design pixels; see
/// [`crate::ui::scale`].
const GAP: i32 = 8;
const MARGIN: i32 = 20;
/// Gap under the search bar, above the key block's prompt line.
const BAND_GAP: u32 = 24;
/// Key-face outline thickness.
const FACE_T: u32 = 2;

/// Width of a command slot, matching [`crate::ui::pager`]'s so `[ Back ]`
/// lands where `[ Exit ]` does.
const ZONE_W_PX: u32 = 200;

/// Bar height on a panel `fb_xres` wide, from [`strip::H`].
fn strip_h(fb_xres: u32) -> u32 {
    strip::h(fb_xres)
}

#[derive(Clone, Copy)]
enum Key {
    Char(char),
    Space,
    Backspace,
    Clear,
    /// Leave without searching, returning the query the overlay opened with, so
    /// an accidental open costs nothing. Distinct from [`Key::Done`], which is
    /// this keyboard's Enter.
    Back,
    /// Submit: run the search on what has been typed.
    Done,
}

/// How a key is drawn. Both kinds hit-test the same way.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Style {
    /// A key in the grid: an outlined face inset from its cell.
    Face,
    /// A slot in the bottom strip: a label only, with the strip drawing the
    /// dividers around it.
    Zone,
}

/// `x`/`y`/`w`/`h` is the **cell**, the whole tappable area. A `Face` key draws
/// inset by half a gap, leaving the gutters inside a cell.
struct KeyButton {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    key: Key,
    label: String,
    style: Style,
}

/// Top of the key grid — the band above it holds the title, query, and count.
/// Bottom of the top band (the shared search bar + the match count below it). A
/// keystroke refreshes only `[0, band_bottom]`, leaving the keyboard untouched.
fn band_bottom(fb_xres: u32, lh: u32) -> u32 {
    searchbar::top(fb_xres)
        + searchbar::height(fb_xres)
        + lh
        + Scale::of_width(fb_xres).px(BAND_GAP)
}

fn strip_top(fb_xres: u32, yres: u32) -> u32 {
    strip::top(fb_xres, yres)
}

/// Keyboard metrics `(unit, unit_digits, key_h, keys_top)`. Letter rows divide
/// the span into ten columns, the digit row into eleven for `Del`. Key faces
/// are square, and the four-row block anchors above the command strip.
fn metrics(xres: u32, yres: u32) -> (i32, i32, i32, i32) {
    let s = Scale::of_width(xres);
    let (gap, margin) = (s.i(GAP), s.i(MARGIN));
    let span = (xres as i32 - 2 * margin).max(1);
    let unit = (span / 10).max(1);
    let unit_digits = (span / 11).max(1);
    let key_h = (unit - gap).max(1);
    let block_h = 4 * key_h + 3 * gap;
    let keys_top = (yres as i32 - strip_h(xres) as i32 - margin - block_h).max(margin);
    (unit, unit_digits, key_h, keys_top)
}

/// Lay out every key: the four letter/digit rows, then the four strip slots.
fn layout(xres: u32, yres: u32) -> Vec<KeyButton> {
    let s = Scale::of_width(xres);
    let (unit, unit_digits, key_h, top) = metrics(xres, yres);
    let stride = key_h + s.i(GAP);
    let mut out = Vec::new();

    for (r, row) in ROWS.iter().enumerate() {
        let digits = r == 0;
        let u = if digits { unit_digits } else { unit };
        // The digit row reserves one extra column for `Del`.
        let n = row.chars().count() as i32 + i32::from(digits);
        let start_x = (xres as i32 - n * u) / 2;
        let y = top + r as i32 * stride;
        for (i, c) in row.chars().enumerate() {
            out.push(KeyButton {
                x: start_x + i as i32 * u,
                y,
                w: u as u32,
                h: stride as u32,
                key: Key::Char(c),
                label: c.to_string(),
                style: Style::Face,
            });
        }
        if digits {
            out.push(KeyButton {
                x: start_x + (n - 1) * u,
                y,
                w: u as u32,
                h: stride as u32,
                key: Key::Backspace,
                label: "Del".to_string(),
                style: Style::Face,
            });
        }
    }

    // `[ Back ]` takes the leftmost slot, the one `crate::ui::pager` gives
    // `Exit`. `[ Search ]` sits far right, the wide `space` between it and
    // `Clear`.
    let sy = strip_top(xres, yres) as i32;
    let side = s.px(ZONE_W_PX).min(xres / 5);
    for (x, w, key, label) in [
        (0, side, Key::Back, "[ Back ]"),
        (side as i32, side, Key::Clear, "Clear"),
        (
            (side * 2) as i32,
            xres.saturating_sub(side * 3),
            Key::Space,
            "space",
        ),
        (
            xres.saturating_sub(side) as i32,
            side,
            Key::Done,
            "[ Search ]",
        ),
    ] {
        out.push(KeyButton {
            x,
            y: sy,
            w,
            h: strip_h(xres),
            key,
            label: label.to_string(),
            style: Style::Zone,
        });
    }
    out
}

fn full_rect(fb: &Framebuffer) -> MxcfbRect {
    MxcfbRect {
        top: 0,
        left: 0,
        width: fb.var.xres,
        height: fb.var.yres,
    }
}

/// The query band at the top, its own rect for a per-keystroke DU.
fn band_rect(fb: &Framebuffer, lh: u32) -> MxcfbRect {
    MxcfbRect {
        top: 0,
        left: 0,
        width: fb.var.xres,
        height: band_bottom(fb.var.xres, lh),
    }
}

/// The drawn face of a key: a grid cell inset by half a gap, a bar slot inset
/// past the bar's rules. Those rules sit inside the slot rects, so a full-cell
/// fill would paint over them.
fn face(kb: &KeyButton, fb_xres: u32) -> (i32, i32, u32, u32) {
    let s = Scale::of_width(fb_xres);
    match kb.style {
        Style::Zone => {
            let rule = s.px(strip::RULE);
            (
                kb.x + rule as i32,
                kb.y + rule as i32,
                kb.w.saturating_sub(rule).max(1),
                kb.h.saturating_sub(rule).max(1),
            )
        }
        Style::Face => {
            let gap = s.i(GAP);
            (
                kb.x + gap / 2,
                kb.y + gap / 2,
                kb.w.saturating_sub(gap as u32).max(1),
                kb.h.saturating_sub(gap as u32).max(1),
            )
        }
    }
}

fn key_rect(kb: &KeyButton, fb_xres: u32) -> MxcfbRect {
    let (x, y, w, h) = face(kb, fb_xres);
    MxcfbRect {
        top: y.max(0) as u32,
        left: x.max(0) as u32,
        width: w,
        height: h,
    }
}

/// `crate::ui::searchbar` at the grid view's own position and size, over a
/// static prompt. The caller white-fills the band first. SE offers no
/// autocomplete endpoint; the search runs once, on `[ Search ]`.
fn draw_band(fb: &mut Framebuffer, renderer: &mut TextRenderer, query: &str, lh: u32) {
    let xres = fb.var.xres;
    searchbar::draw(fb, renderer, query);

    // Centered prompt directly below the bar.
    let count = if query.trim().is_empty() {
        "Type to search Standard Ebooks".to_string()
    } else {
        "Tap Search to run it".to_string()
    };
    let cw = renderer.measure_width(&count);
    let cy = (searchbar::top(xres) + searchbar::height(xres) + lh) as i32;
    renderer.draw(
        fb,
        ((xres as i32 - cw as i32) / 2).max(0),
        cy,
        &count,
        false,
    );
}

/// One key. `pressed` inverts it, filled black with a white label: the whole
/// acknowledgement a tap gets under the finger, ahead of the band refresh at
/// the far end of the screen.
fn draw_key(fb: &mut Framebuffer, renderer: &mut TextRenderer, kb: &KeyButton, pressed: bool) {
    let (x, y, w, h) = face(kb, fb.var.xres);
    let (top, left) = (y.max(0) as u32, x.max(0) as u32);
    fb.fill_rect(top, left, w, h, if pressed { 0x00 } else { 0xFF });
    if !pressed && kb.style == Style::Face {
        outline_rect(
            fb,
            x,
            y,
            w,
            h,
            Scale::of_width(fb.var.xres).px(FACE_T),
            0x00,
        );
    }
    let lw = renderer.measure_width(&kb.label);
    let tx = x + ((w as i32 - lw as i32) / 2).max(0);
    let baseline = y + (h * 62 / 100) as i32;
    renderer.draw(fb, tx, baseline, &kb.label, pressed);
}

/// The strip's chrome: the rule above it and the slot separators, drawn the same
/// way `ui/filtermenu.rs` and `ui/pager.rs` draw theirs. [`face`] insets a slot
/// past these, so pressing one leaves them intact.
fn draw_strip_chrome(fb: &mut Framebuffer, keys: &[KeyButton]) {
    let (xres, yres) = (fb.var.xres, fb.var.yres);
    let rule = Scale::of_width(xres).px(strip::RULE);
    fb.fill_rect(strip::top(xres, yres), 0, xres, rule, 0x00);
    for kb in keys.iter().filter(|k| k.style == Style::Zone && k.x > 0) {
        strip::separator(fb, kb.x as u32);
    }
}

fn render_all(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    keys: &[KeyButton],
    query: &str,
    lh: u32,
) {
    fb.fill_rect(0, 0, fb.var.xres, fb.var.yres, 0xFF);
    draw_band(fb, renderer, query, lh);
    for kb in keys {
        draw_key(fb, renderer, kb, false);
    }
    draw_strip_chrome(fb, keys);
}

/// Index of the key under a touch. Cells tile their row, landing a gutter tap
/// on a neighbouring key.
fn hit_index(keys: &[KeyButton], tx: u32, ty: u32) -> Option<usize> {
    let (tx, ty) = (tx as i32, ty as i32);
    keys.iter()
        .position(|k| tx >= k.x && tx < k.x + k.w as i32 && ty >= k.y && ty < k.y + k.h as i32)
}

fn hit(keys: &[KeyButton], tx: u32, ty: u32) -> Option<Key> {
    hit_index(keys, tx, ty).map(|i| keys[i].key)
}

/// The keyboard as [`run`] first draws it, without a device behind it: the
/// whole screen into `fb`'s backing, nothing presented. `crate::bin::preview`
/// reads it back as a PNG.
pub fn render_screen(fb: &mut Framebuffer, renderer: &mut TextRenderer, query: &str) {
    let lh = renderer.line_height().max(1);
    let keys = layout(fb.var.xres, fb.var.yres);
    render_all(fb, renderer, &keys, query, lh);
}

/// Runs the keyboard, returning the typed query on `[ Search ]` and `initial`
/// on `[ Back ]`. The caller acts on a difference. `initial` pre-fills the box,
/// carrying the current search into a re-open.
pub fn run(
    fb: &mut Framebuffer,
    input: &mut Input,
    renderer: &mut TextRenderer,
    initial: &str,
) -> anyhow::Result<String> {
    let lh = renderer.line_height().max(1);
    let mut query = initial.to_string();
    let mut keys = layout(fb.var.xres, fb.var.yres);
    // The key currently held down, so it can be un-inverted on release.
    let mut pressed: Option<usize> = None;

    render_all(fb, renderer, &keys, &query, lh);
    fb.send_update(full_rect(fb), WAVEFORM_MODE_GC16)?;

    // Refresh just the query+count band after a keystroke (fast DU, no flash).
    macro_rules! refresh_band {
        () => {{
            fb.fill_rect(0, 0, fb.var.xres, band_bottom(fb.var.xres, lh), 0xFF);
            draw_band(fb, renderer, &query, lh);
            fb.send_update(band_rect(fb, lh), WAVEFORM_MODE_DU)?;
        }};
    }

    loop {
        match input.next_event()? {
            InputEvent::Touch(TouchEvent::Up { x, y }) => {
                if let Some(i) = pressed.take() {
                    draw_key(fb, renderer, &keys[i], false);
                    fb.send_update(key_rect(&keys[i], fb.var.xres), WAVEFORM_MODE_DU)?;
                }
                // `searchbar` is live in the overlay: its `✕` clears, a field
                // tap no-ops. Past it, a key resolves.
                if let Some(tap) = searchbar::hit(x, y, fb.var.xres, !query.is_empty()) {
                    if matches!(tap, searchbar::Tap::Clear) {
                        query.clear();
                        refresh_band!();
                    }
                } else {
                    match hit(&keys, x, y) {
                        Some(Key::Char(c)) => {
                            query.push(c);
                            refresh_band!();
                        }
                        Some(Key::Space) => {
                            query.push(' ');
                            refresh_band!();
                        }
                        Some(Key::Backspace) => {
                            query.pop();
                            refresh_band!();
                        }
                        Some(Key::Clear) => {
                            query.clear();
                            refresh_band!();
                        }
                        Some(Key::Done) => return Ok(query),
                        // Hand back what the overlay opened with. The caller
                        // compares against its current query, so an unchanged
                        // return is a no-op and nothing is searched.
                        Some(Key::Back) => return Ok(initial.to_string()),
                        None => {}
                    }
                }
            }
            InputEvent::Touch(TouchEvent::Down { x, y }) => {
                if searchbar::hit(x, y, fb.var.xres, !query.is_empty()).is_none()
                    && let Some(i) = hit_index(&keys, x, y)
                {
                    draw_key(fb, renderer, &keys[i], true);
                    fb.send_update(key_rect(&keys[i], fb.var.xres), WAVEFORM_MODE_DU)?;
                    pressed = Some(i);
                }
            }
            InputEvent::Touch(TouchEvent::Screenshot) => {
                let _ = crate::eink::screenshot::capture(fb);
            }
            InputEvent::Page(_) => {}
            // The one place this overlay drains the X queue and re-reads the
            // framework orientation; `crate::eink::input` throttles the read.
            InputEvent::Tick => {
                let pump = fb.pump_events();
                if let Some(covered) = pump.covered {
                    pressed = None;
                    input.set_covered(covered);
                }
                input.retake();
                let turned = input.follow_orientation();
                if turned || pump.resized.is_some() {
                    input.set_size(fb.var.xres, fb.var.yres);
                }
                if pump.covered == Some(true) {
                    continue;
                }
                if turned || pump.resized.is_some() || pump.repaint || pump.covered.is_some() {
                    keys = layout(fb.var.xres, fb.var.yres);
                    pressed = None;
                    render_all(fb, renderer, &keys, &query, lh);
                    fb.send_update(full_rect(fb), WAVEFORM_MODE_GC16)?;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XRES: u32 = 1264;
    const YRES: u32 = 1680;

    fn find(keys: &[KeyButton], c: char) -> &KeyButton {
        keys.iter()
            .find(|k| matches!(k.key, Key::Char(x) if x == c))
            .expect("key present")
    }

    #[test]
    fn layout_covers_every_letter_and_digit() {
        let keys = layout(XRES, YRES);
        // 11 (digits + Del) + 10 + 9 + 7 faces, plus the four strip slots.
        assert_eq!(keys.len(), 37 + 4);
        for c in "abcdefghijklmnopqrstuvwxyz0123456789".chars() {
            assert!(
                keys.iter().any(|k| matches!(k.key, Key::Char(x) if x == c)),
                "missing key {c}"
            );
        }
    }

    #[test]
    fn del_sits_at_the_right_end_of_the_number_row() {
        let keys = layout(XRES, YRES);
        let zero = find(&keys, '0');
        let del = keys
            .iter()
            .find(|k| matches!(k.key, Key::Backspace))
            .expect("Del present");
        assert_eq!(del.y, zero.y, "Del shares the number row");
        assert!(del.x > zero.x, "Del sits right of 0");
        assert_eq!(del.style, Style::Face);
        // The digit row spans the same width as a letter row, within a column.
        let q = find(&keys, 'q');
        let p = find(&keys, 'p');
        let one = find(&keys, '1');
        let letter_span = p.x + p.w as i32 - q.x;
        let digit_span = del.x + del.w as i32 - one.x;
        assert!(
            (letter_span - digit_span).abs() <= del.w as i32,
            "rows span the same width: letters {letter_span}, digits {digit_span}"
        );
    }

    #[test]
    fn back_takes_the_leftmost_slot_and_search_the_rightmost() {
        let keys = layout(XRES, YRES);
        let back = keys
            .iter()
            .find(|k| matches!(k.key, Key::Back))
            .expect("Back present");
        assert_eq!(back.x, 0, "Back is flush to the left edge, as Exit is");
        assert_eq!(back.y, strip_top(XRES, YRES) as i32);
        assert_eq!(back.style, Style::Zone);
        // Left to right: Back, Clear, space, Search — the submit at the far
        // right, and the space bar between Clear and Search so a mis-tap cannot
        // wipe the query and submit in one slip.
        let row = YRES - 10;
        assert!(matches!(hit(&keys, 10, row), Some(Key::Back)));
        assert!(matches!(hit(&keys, ZONE_W_PX + 10, row), Some(Key::Clear)));
        assert!(matches!(hit(&keys, XRES / 2, row), Some(Key::Space)));
        assert!(matches!(hit(&keys, XRES - 10, row), Some(Key::Done)));
    }

    #[test]
    fn pressing_a_strip_slot_cannot_erase_the_chrome() {
        // A face inside its cell: a full-cell fill paints over the top rule
        // and the vertical rule at its left edge, and the restore draws no
        // chrome back.
        let keys = layout(XRES, YRES);
        let strip = strip_top(XRES, YRES) as i32;
        for kb in keys.iter().filter(|k| k.style == Style::Zone) {
            let (fx, fy, _, _) = face(kb, XRES);
            assert!(
                fy >= strip + strip::RULE as i32,
                "{} face starts at y={fy}, inside the top rule at {strip}",
                kb.label
            );
            if kb.x > 0 {
                assert!(
                    fx >= kb.x + strip::RULE as i32,
                    "{} face starts at x={fx}, inside its own rule at {}",
                    kb.label,
                    kb.x
                );
            }
        }
    }

    #[test]
    fn back_abandons_the_edit() {
        // `Back` hands back the query the overlay opened with, which the
        // caller's difference guard reads as a no-op. The wiring is under
        // test; `run` takes a framebuffer.
        let keys = layout(XRES, YRES);
        assert!(matches!(hit(&keys, 10, YRES - 10), Some(Key::Back),));
        assert!(
            keys.iter().filter(|k| matches!(k.key, Key::Done)).count() == 1,
            "exactly one submit key"
        );
    }

    #[test]
    fn a_tap_in_a_gutter_still_lands_on_a_key() {
        let keys = layout(XRES, YRES);
        let a = find(&keys, 'a');
        let s = find(&keys, 's');
        // The seam between two neighbouring keys belongs to one of them.
        let seam = (a.x + a.w as i32) as u32;
        assert!(seam <= s.x as u32 + 1, "cells tile the row");
        let mid = a.y + a.h as i32 / 2;
        assert!(hit(&keys, seam.saturating_sub(1), mid as u32).is_some());
        assert!(hit(&keys, seam, mid as u32).is_some());
        // The drawn face is inset, so it is narrower than the cell it fills.
        let (_, _, fw, _) = face(a, XRES);
        assert!(fw < a.w, "face {fw} is inset within cell {}", a.w);
    }

    #[test]
    fn letter_key_faces_are_square() {
        let keys = layout(XRES, YRES);
        let (_, _, fw, fh) = face(find(&keys, 'a'), XRES);
        assert_eq!(fw, fh, "a letter face is {fw}x{fh}, not square");
    }

    #[test]
    fn hit_finds_the_tapped_key() {
        let keys = layout(XRES, YRES);
        let k0 = &keys[0];
        let cx = (k0.x + k0.w as i32 / 2) as u32;
        let cy = (k0.y + k0.h as i32 / 2) as u32;
        assert!(matches!(hit(&keys, cx, cy), Some(Key::Char('1'))));
        // Above the key block there is nothing to tap.
        assert!(hit(&keys, 0, 0).is_none());
    }
}
