//! Boot-failure Diagnostics screen: the error, a class-specific hint, and
//! **Retry** / **Exit** tap zones. `main.rs` opens the X window, framebuffer,
//! touch and renderer ahead of the first network call, for this screen.

use crate::eink::fb::{Framebuffer, MxcfbRect, WAVEFORM_MODE_GC16};
use crate::eink::input::{Input, InputEvent};
use crate::eink::touch::TouchEvent;
use crate::se::http::Error as HttpError;
use crate::ui::text::TextRenderer;

/// What a tap on the Diagnostics screen resolved to.
pub enum Action {
    /// Re-run the request.
    Retry,
    /// Leave the picker, back to the home screen.
    Exit,
}

/// Bottom button row height, over `pager`'s 80px strip: two wide zones.
const BTN_H: u32 = 120;
/// Left inset for the info block, bounding the wrapped Last and Hint rows.
const MARGIN_X: u32 = 60;

fn btn_top(yres: u32) -> u32 {
    yres.saturating_sub(BTN_H)
}

/// A tap mapped to a button. Above the button row is dead space; the row
/// splits left `Exit`, right `Retry`, matching `pager`'s leftmost `Exit`. Pure
/// integer geometry, like `pager::hit`.
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

/// The `Last` and `Hint` rows for `err`, the hint from
/// [`crate::se::http::Error::hint`].
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

/// A two-zone button row: `[ Exit ]` left, `[ Retry ]` right, a 2px top divider
/// and a vertical mid divider in `pager`'s style. Labels are bracketed ASCII,
/// carrying no glyph-coverage risk.
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

/// The panel for `err`, blocking until a Retry or Exit tap. Called fresh per
/// failed attempt, carrying the latest error into the "Last" row.
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
            // past the framework, which repaints over this window.
            InputEvent::Page(_) => {}
            // Idle tick — the diag panel is transient; ignore rotation here.
            InputEvent::Tick => {}
        }
    }
}
