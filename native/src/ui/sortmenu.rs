//! Sort-picker overlay: a full-screen list over a `[ Done ]` strip, shaped like
//! [`crate::ui::diag`]. GC16 on open and on rotation, a list-region DU on a
//! selection change. A `Tick` re-detects orientation here.

use crate::eink::fb::{Framebuffer, MxcfbRect, WAVEFORM_MODE_DU, WAVEFORM_MODE_GC16};
use crate::eink::input::{Input, InputEvent};
use crate::eink::touch::TouchEvent;
use crate::ui::scale::Scale;
use crate::ui::sort::SortState;
use crate::ui::strip;
use crate::ui::text::TextRenderer;

/// Left inset for the title and row labels.
const MARGIN_X_PX: u32 = 60;
/// Row-height floor, so a tap target stays a fingertip at any font size.
const ROW_H_MIN: u32 = 96;

/// Side margin on a panel `fb_xres` wide.
fn margin_x(fb_xres: u32) -> u32 {
    Scale::of_width(fb_xres).px(MARGIN_X_PX)
}

/// What a tap resolved to. SE bakes direction into each option it offers
/// ("Author name (a → z)"), leaving no direction row.
enum Tap {
    /// Index into the caller's row list.
    Row(usize),
    Done,
}

/// Precomputed vertical geometry. Stable across KOA2's Up/Down (both portrait,
/// same `xres`/`yres`), but recomputed on rotation anyway in case a future
/// device reports different dims.
struct Layout {
    lh: u32,
    rows_top: u32,
    row_h: u32,
    strip_top: u32,
}

impl Layout {
    fn compute(renderer: &TextRenderer, xres: u32, yres: u32) -> Self {
        let scale = Scale::of_width(xres);
        let lh = renderer.line_height().max(1);
        Layout {
            lh,
            rows_top: lh * 3,
            // Generous tap targets — a [`ROW_H_MIN`] floor regardless of font size.
            row_h: (lh * 2).max(scale.px(ROW_H_MIN)),
            strip_top: strip::top(xres, yres),
        }
    }

    /// Map a finger-up to an action. The Done strip spans the full bottom width;
    /// above it, rows are `row_h` tall starting at `rows_top`.
    fn hit(&self, ty: u32, n_rows: usize) -> Option<Tap> {
        if ty >= self.strip_top {
            return Some(Tap::Done);
        }
        if ty < self.rows_top {
            return None;
        }
        let row = ((ty - self.rows_top) / self.row_h) as usize;
        (row < n_rows).then_some(Tap::Row(row))
    }

    /// The list region, refreshed with DU on an in-menu change so a tap doesn't
    /// flash the whole screen.
    fn rows_rect(&self, xres: u32) -> MxcfbRect {
        MxcfbRect {
            top: self.rows_top,
            left: 0,
            width: xres,
            height: self.strip_top.saturating_sub(self.rows_top),
        }
    }
}

fn full_rect(fb: &Framebuffer) -> MxcfbRect {
    MxcfbRect {
        top: 0,
        left: 0,
        width: fb.var.xres,
        height: fb.var.yres,
    }
}

/// Paint the whole panel into the framebuffer (no refresh — caller decides the
/// rect + waveform). White background; the selected key row is inverted
/// (black fill, white text) so the highlight needs no glyph coverage.
fn render(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    state: SortState,
    rows: &[SortState],
    layout: &Layout,
) {
    let xres = fb.var.xres;
    fb.fill_rect(0, 0, xres, fb.var.yres, 0xFF);

    // Centered title in the top gap above the rows.
    let title = "Sort by";
    let tw = renderer.measure_width(title);
    let tx = ((xres as i32 - tw as i32) / 2).max(0);
    renderer.draw(fb, tx, (layout.lh * 2) as i32, title, false);

    for (i, row) in rows.iter().enumerate() {
        let row_top = layout.rows_top + i as u32 * layout.row_h;
        let selected = state == *row;
        if selected {
            fb.fill_rect(row_top, 0, xres, layout.row_h, 0x00);
        }
        let baseline = (row_top + layout.row_h * 60 / 100) as i32;
        renderer.draw(fb, margin_x(xres) as i32, baseline, row.label(), selected);
    }

    draw_done(fb, renderer);
}

/// Full-width `[ Done ]` bar at the bottom.
fn draw_done(fb: &mut Framebuffer, renderer: &mut TextRenderer) {
    let xres = fb.var.xres;
    let top = strip::base(fb);
    let baseline = strip::baseline(xres, top, renderer);
    strip::label(fb, renderer, 0, xres, baseline, "[ Done ]", false);
}

/// The picker as [`run`] first draws it, without a device behind it.
/// `crate::bin::preview` reads it back as a PNG.
pub fn render_screen(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    state: SortState,
    has_query: bool,
) {
    let rows: Vec<SortState> = SortState::ALL
        .into_iter()
        .filter(|s| s.available(has_query))
        .collect();
    let layout = Layout::compute(renderer, fb.var.xres, fb.var.yres);
    render(fb, renderer, state, &rows, &layout);
}

/// The sort picker seeded with `initial`, blocking until Done and returning the
/// chosen `SortState`. `has_query` gates the Relevance row, a sort SE accepts
/// alongside a query alone.
pub fn run(
    fb: &mut Framebuffer,
    input: &mut Input,
    renderer: &mut TextRenderer,
    initial: SortState,
    has_query: bool,
) -> anyhow::Result<SortState> {
    let rows: Vec<SortState> = SortState::ALL
        .into_iter()
        .filter(|s| s.available(has_query))
        .collect();

    let mut state = initial;
    let mut layout = Layout::compute(renderer, fb.var.xres, fb.var.yres);
    render(fb, renderer, state, &rows, &layout);
    fb.send_update(full_rect(fb), WAVEFORM_MODE_GC16)?;

    loop {
        match input.next_event()? {
            InputEvent::Touch(TouchEvent::Up { y, .. }) => match layout.hit(y, rows.len()) {
                Some(Tap::Row(i)) if state != rows[i] => {
                    state = rows[i];
                    render(fb, renderer, state, &rows, &layout);
                    let rect = layout.rows_rect(fb.var.xres);
                    fb.send_update(rect, WAVEFORM_MODE_DU)?;
                }
                Some(Tap::Done) => return Ok(state),
                // The selected row, or no hit.
                Some(Tap::Row(_)) | None => {}
            },
            InputEvent::Touch(TouchEvent::Down { .. }) => {}
            InputEvent::Touch(TouchEvent::Screenshot) => {
                let _ = crate::eink::screenshot::capture(fb);
            }
            // Seven rows fit on one screen — nothing to page.
            InputEvent::Page(_) => {}
            // The one place this overlay drains the X queue and re-reads the
            // framework orientation; `crate::eink::input` throttles the read.
            InputEvent::Tick => {
                let pump = fb.pump_events();
                if let Some(covered) = pump.covered {
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
                    layout = Layout::compute(renderer, fb.var.xres, fb.var.yres);
                    render(fb, renderer, state, &rows, &layout);
                    fb.send_update(full_rect(fb), WAVEFORM_MODE_GC16)?;
                }
            }
        }
    }
}
