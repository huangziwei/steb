//! Boot-failure Diagnostics screen.
//!
//! When the first request to standardebooks.org fails at launch, Steb renders
//! this panel instead of flashing a toast and dropping back to the home screen.
//! It shows the actual error and a class-specific hint, and offers two tap
//! zones — **Retry** and **Exit** — so the user has an on-device recourse
//! (turn Wi-Fi on, then Retry) without relaunching.
//!
//! This is why `main.rs` opens the X window, framebuffer, touch and renderer
//! *before* the first network call: this screen needs a surface to draw on and
//! `input` to take taps.
//!
//! Modeled on `ui/pager.rs` (a bottom button strip + a pure `hit` geometry fn)
//! and `ui/toast.rs` (a centered panel). Page-button events are ignored here —
//! there's no paging — but because `Input` has grabbed the bezel device, a
//! press still can't leak to the framework and repaint over us.

use crate::eink::fb::{Framebuffer, MxcfbRect, WAVEFORM_MODE_GC16};
use crate::eink::input::{Input, InputEvent};
use crate::eink::touch::TouchEvent;
use crate::se::http::Error as HttpError;
use crate::ui::text::TextRenderer;

/// What the user chose on the Diagnostics screen.
pub enum Action {
    /// Re-run `list_books` — the server may now be reachable.
    Retry,
    /// Leave the picker, back to the home screen.
    Exit,
}

/// Bottom button row height. Taller than `pager`'s 80px strip — it holds
/// only two zones (no page nav), and a boot-failure screen wants a
/// generous, hard-to-miss target.
const BTN_H: u32 = 120;
/// Left inset for the info block, and the per-side margin used to bound
/// the wrapped Last/Hint rows.
const MARGIN_X: u32 = 60;

fn btn_top(yres: u32) -> u32 {
    yres.saturating_sub(BTN_H)
}

/// Map a tap to a button. Anything above the button row is dead space
/// (no action); the row splits left = Exit, right = Retry — the leave action
/// sits leftmost, matching the gallery's Exit and the filter screens. Pure
/// integer geometry so it can be reasoned about without a framebuffer (mirrors
/// `pager::hit`).
pub fn hit(tx: u32, ty: u32, xres: u32, yres: u32) -> Option<Action> {
    if ty < btn_top(yres) {
        return None;
    }
    if tx < xres / 2 {
        Some(Action::Exit)
    } else {
        Some(Action::Retry)
    }
}

/// The `Last` (what failed) and `Hint` (what to do) rows for an error.
///
/// There is exactly one server and no credential, so there are no connection
/// settings worth printing — only the error and what the user can do about it.
/// The hint comes from
/// [`crate::se::http::Error::hint`], which is written for someone holding an
/// e-reader rather than reading a terminal.
fn rows_for(err: &HttpError) -> (String, String) {
    (format!("{err}"), err.hint().to_string())
}

/// Draw a single left-aligned line at the running `y` cursor, advancing
/// `y` by one line height. Baseline ≈ 80% down the line box (above the
/// descender), matching the ratio `pager`/grid placeholders use.
fn draw_line(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    x: i32,
    y: &mut u32,
    lh: u32,
    s: &str,
) {
    let baseline = (*y + lh * 80 / 100) as i32;
    renderer.draw(fb, x, baseline, s, false);
    *y += lh;
}

/// White-fill the panel, paint the info block + button row, then a single
/// full-screen GC16 refresh so the screen lands clean (no DU ghosting
/// from whatever was there before).
fn draw(fb: &mut Framebuffer, renderer: &mut TextRenderer, err: &HttpError) -> anyhow::Result<()> {
    fb.fill_rect(0, 0, fb.var.xres, fb.var.yres, 0xFF);

    let lh = renderer.line_height().max(1);
    let left = MARGIN_X as i32;
    let max_w = fb.var.xres.saturating_sub(MARGIN_X * 2);
    let mut y = lh * 3; // a little headroom from the top edge

    draw_line(
        fb,
        renderer,
        left,
        &mut y,
        lh,
        "Can't reach Standard Ebooks",
    );
    y += lh; // blank spacer under the title

    draw_line(fb, renderer, left, &mut y, lh, "Site:   standardebooks.org");

    let (last, hint) = rows_for(err);
    // Error chains can be long — wrap to width and clamp so the panel
    // never overflows into the button row.
    let last = format!("Last:   {last}");
    for line in renderer.wrap_and_clamp(&last, max_w, 4) {
        draw_line(fb, renderer, left, &mut y, lh, &line);
    }
    let hint = format!("Hint:   {hint}");
    for line in renderer.wrap_and_clamp(&hint, max_w, 3) {
        draw_line(fb, renderer, left, &mut y, lh, &line);
    }

    draw_buttons(fb, renderer);

    fb.send_update(
        MxcfbRect {
            top: 0,
            left: 0,
            width: fb.var.xres,
            height: fb.var.yres,
        },
        WAVEFORM_MODE_GC16,
    )?;
    Ok(())
}

/// Two-zone button row at the bottom: `[ Exit ]` left half, `[ Retry ]`
/// right half, a 2px top divider + a vertical mid divider in `pager`'s
/// style. Exit (leave) is leftmost so it matches the gallery and the filter
/// screens. Labels are bracketed ASCII (no glyph-coverage risk on a screen
/// whose whole job is to be readable when things are broken).
fn draw_buttons(fb: &mut Framebuffer, renderer: &mut TextRenderer) {
    let xres = fb.var.xres;
    let top = btn_top(fb.var.yres);
    let mid = xres / 2;

    fb.fill_rect(top, 0, xres, 2, 0x00); // top divider
    fb.fill_rect(top + 2, 0, xres, BTN_H - 2, 0xFF); // white body
    fb.fill_rect(top + 12, mid.saturating_sub(1), 2, BTN_H - 24, 0x00); // mid divider

    let baseline = (top + BTN_H * 60 / 100) as i32;
    // Exit (leave) in the left half — leftmost, matching every other screen.
    let exit = "[ Exit ]";
    let ew = renderer.measure_width(exit);
    let ex = ((mid as i32 - ew as i32) / 2).max(0);
    renderer.draw(fb, ex, baseline, exit, false);

    // Retry in the right half.
    let retry = "[ Retry ]";
    let rw = renderer.measure_width(retry);
    let rx = (mid as i32 + (mid as i32 - rw as i32) / 2).max(mid as i32);
    renderer.draw(fb, rx, baseline, retry, false);
}

/// Draw the panel for `err`, then block until the user taps Retry or
/// Exit. Called fresh on every failed `list_books` attempt, so the "Last"
/// row always reflects the latest error across retries.
pub fn run(
    fb: &mut Framebuffer,
    input: &mut Input,
    renderer: &mut TextRenderer,
    err: &HttpError,
) -> anyhow::Result<Action> {
    draw(fb, renderer, err)?;
    loop {
        match input.next()? {
            // Act on finger-up, like `pager`.
            InputEvent::Touch(TouchEvent::Up { x, y }) => {
                if let Some(action) = hit(x, y, fb.var.xres, fb.var.yres) {
                    return Ok(action);
                }
            }
            // Finger-down: no press feedback for v1 (keep it minimal).
            InputEvent::Touch(TouchEvent::Down { .. }) => {}
            InputEvent::Touch(TouchEvent::Screenshot) => {
                let _ = crate::eink::screenshot::capture(fb);
            }
            // Page buttons do nothing here, but they're grabbed by `Input`
            // so they can't reach the framework and repaint over us.
            InputEvent::Page(_) => {}
            // Idle tick — the diag panel is transient; ignore rotation here.
            InputEvent::Tick => {}
        }
    }
}
