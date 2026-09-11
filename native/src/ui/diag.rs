//! Boot-failure Diagnostics screen: the error, a class-specific hint, and
//! **Retry** / **Exit** tap zones. `main.rs` opens the X window, framebuffer,
//! touch and renderer ahead of the first network call, for this screen.

use crate::eink::fb::{Framebuffer, MxcfbRect, WAVEFORM_MODE_GC16};
use crate::eink::input::{Input, InputEvent};
use crate::eink::touch::TouchEvent;
use crate::se::http::Error as HttpError;
use crate::ui::scale::Scale;
use crate::ui::strip;
use crate::ui::text::TextRenderer;

/// What a tap on the Diagnostics screen resolved to.
pub enum Action {
    /// Re-run the request.
    Retry,
    /// Leave the picker, back to the home screen.
    Exit,
}

/// Left inset for the info block, bounding the wrapped Last and Hint rows.
const MARGIN_X_PX: u32 = 60;

/// Side margin on a panel `fb_xres` wide.
fn margin_x(fb_xres: u32) -> u32 {
    Scale::of_width(fb_xres).px(MARGIN_X_PX)
}

/// A tap mapped to a button. Above the button row is dead space; the row
/// splits left `Exit`, right `Retry`, matching `pager`'s leftmost `Exit`. Pure
/// integer geometry, like `pager::hit`.
pub fn hit(tx: u32, ty: u32, xres: u32, yres: u32) -> Option<Action> {
    if ty < strip::top(xres, yres) {
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

/// The screen as [`run`] draws it, without a device behind it.
/// `crate::bin::preview` reads it back as a PNG.
pub fn render_screen(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    err: &HttpError,
) -> anyhow::Result<()> {
    draw(fb, renderer, err)
}

/// The info block and button row over a white panel, presented in one
/// full-screen GC16 so no DU ghosting survives.
fn draw(fb: &mut Framebuffer, renderer: &mut TextRenderer, err: &HttpError) -> anyhow::Result<()> {
    fb.fill_rect(0, 0, fb.var.xres, fb.var.yres, 0xFF);

    let lh = renderer.line_height().max(1);
    let margin = margin_x(fb.var.xres);
    let left = margin as i32;
    let max_w = fb.var.xres.saturating_sub(margin * 2);
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

/// `[ Exit ]` left, `[ Retry ]` right. Labels are bracketed ASCII, carrying no
/// glyph-coverage risk.
fn draw_buttons(fb: &mut Framebuffer, renderer: &mut TextRenderer) {
    let xres = fb.var.xres;
    let mid = xres / 2;
    let top = strip::base(fb);
    let baseline = strip::baseline(xres, top, renderer);
    strip::label(fb, renderer, 0, mid, baseline, "[ Exit ]", false);
    strip::label(fb, renderer, mid, xres - mid, baseline, "[ Retry ]", false);
    strip::separator(fb, mid);
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
        match input.next_event()? {
            // Act on finger-up, like `pager`.
            InputEvent::Touch(TouchEvent::Up { x, y }) => {
                if let Some(action) = hit(x, y, fb.var.xres, fb.var.yres) {
                    return Ok(action);
                }
            }
            // Finger-down: no press feedback here.
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
