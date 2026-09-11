//! Subject-filter overlay: a paged checklist over the `<select name="tags[]">`
//! vocabulary, carrying no counts. A blocking sub-loop shaped like
//! [`crate::ui::sortmenu`]; GC16 on open, page and rotate, a DU on a toggle.

use crate::eink::fb::{Framebuffer, MxcfbRect, WAVEFORM_MODE_DU, WAVEFORM_MODE_GC16};
use crate::eink::input::{Input, InputEvent};
use crate::eink::touch::TouchEvent;
use crate::ui::filter::Filters;
use crate::ui::scale::Scale;
use crate::ui::sort::SortState;
use crate::ui::sortmenu;
use crate::ui::strip;
use crate::ui::text::TextRenderer;

const MARGIN_X_PX: u32 = 60;
/// Row-height floor, so a tap target stays a fingertip at any font size.
const ROW_H_MIN: u32 = 96;
/// Gap over the section title.
const TITLE_GAP: u32 = 8;

/// Side margin on a panel `fb_xres` wide.
fn margin_x(fb_xres: u32) -> u32 {
    Scale::of_width(fb_xres).px(MARGIN_X_PX)
}

enum Tap {
    /// Index into the visible page's rows.
    Row(usize),
    Prev,
    Next,
    Done,
}

struct Layout {
    rows_top: u32,
    row_h: u32,
    strip_top: u32,
    /// The panel's density, for the gap over the title.
    scale: Scale,
    /// Rows that fit on one page, always at least one.
    per_page: usize,
}

impl Layout {
    fn compute(renderer: &TextRenderer, xres: u32, yres: u32) -> Self {
        let scale = Scale::of_width(xres);
        let lh = renderer.line_height().max(1);
        let rows_top = lh * 3;
        // Generous tap targets — a [`ROW_H_MIN`] floor regardless of font
        // size, matching the sort picker so the two menus feel like one thing.
        let row_h = lh.saturating_mul(2).max(scale.px(ROW_H_MIN));
        let strip_top = strip::top(xres, yres);
        let per_page = ((strip_top.saturating_sub(rows_top)) / row_h).max(1) as usize;
        Layout {
            rows_top,
            row_h,
            strip_top,
            scale,
            per_page,
        }
    }

    /// Bottom strip is thirds: `< Prev` | `[ Done ]` | `Next >`.
    fn hit(&self, tx: u32, ty: u32, xres: u32, n_rows: usize) -> Option<Tap> {
        if ty >= self.strip_top {
            let third = xres / 3;
            return Some(if tx < third {
                Tap::Prev
            } else if tx < third * 2 {
                Tap::Done
            } else {
                Tap::Next
            });
        }
        if ty < self.rows_top {
            return None;
        }
        let row = ((ty - self.rows_top) / self.row_h) as usize;
        (row < n_rows).then_some(Tap::Row(row))
    }

    fn row_rect(&self, slot: usize, xres: u32) -> MxcfbRect {
        MxcfbRect {
            top: self.rows_top + slot as u32 * self.row_h,
            left: 0,
            width: xres,
            height: self.row_h,
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

fn n_pages(total: usize, per_page: usize) -> usize {
    total.div_ceil(per_page.max(1)).max(1)
}

/// One row: a checkbox and the tag. ASCII marks — no glyph-coverage risk on a
/// firmware carrying an unknown font set.
fn row_text(filters: &Filters, tag: &str) -> String {
    let mark = if filters.is_selected(tag) {
        "[x] "
    } else {
        "[ ] "
    };
    format!("{mark}{tag}")
}

/// The slice of tags visible on `page`.
fn page_rows(tags: &[String], page: usize, per_page: usize) -> &[String] {
    let start = (page * per_page).min(tags.len());
    let end = (start + per_page).min(tags.len());
    &tags[start..end]
}

fn draw_row(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    layout: &Layout,
    slot: usize,
    text: &str,
) {
    let row_top = layout.rows_top + slot as u32 * layout.row_h;
    fb.fill_rect(row_top, 0, fb.var.xres, layout.row_h, 0xFF);
    let baseline = (row_top + layout.row_h * 60 / 100) as i32;
    renderer.draw(fb, margin_x(fb.var.xres) as i32, baseline, text, false);
}

fn render(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    filters: &Filters,
    tags: &[String],
    page: usize,
    layout: &Layout,
) {
    let xres = fb.var.xres;
    fb.fill_rect(0, 0, xres, fb.var.yres, 0xFF);

    let pages = n_pages(tags.len(), layout.per_page);
    let title = if filters.is_empty() {
        "Subjects".to_string()
    } else {
        format!("Subjects  ({} selected)", filters.count())
    };
    let title = if pages > 1 {
        format!("{title}   {}/{}", page + 1, pages)
    } else {
        title
    };
    let tw = renderer.measure_width(&title);
    let tx = ((xres as i32 - tw as i32) / 2).max(0);
    let title_y = layout.rows_top.saturating_sub(layout.scale.px(TITLE_GAP));
    renderer.draw(fb, tx, title_y as i32, &title, false);

    for (slot, tag) in page_rows(tags, page, layout.per_page).iter().enumerate() {
        draw_row(fb, renderer, layout, slot, &row_text(filters, tag));
    }

    draw_strip(fb, renderer, page, pages);
}

/// Bottom strip: `< Prev` | `[ Done ]` | `Next >`, with the paging labels drawn
/// only when there is somewhere to go.
fn draw_strip(fb: &mut Framebuffer, renderer: &mut TextRenderer, page: usize, pages: usize) {
    let xres = fb.var.xres;
    let third = xres / 3;
    let top = strip::base(fb);
    let baseline = strip::baseline(xres, top, renderer);

    if page > 0 {
        strip::label(fb, renderer, 0, third, baseline, "< Prev", false);
    }
    strip::label(fb, renderer, third, third, baseline, "[ Done ]", false);
    if page + 1 < pages {
        strip::label(
            fb,
            renderer,
            third * 2,
            xres - third * 2,
            baseline,
            "Next >",
            false,
        );
    }
    strip::separator(fb, third);
    strip::separator(fb, third * 2);
}

/// The menu as [`run`] first draws it, without a device behind it.
/// `crate::bin::preview` reads it back as a PNG.
pub fn render_screen(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    tags: &[String],
    filters: &Filters,
    page: usize,
) {
    let layout = Layout::compute(renderer, fb.var.xres, fb.var.yres);
    let page = page.min(n_pages(tags.len(), layout.per_page).saturating_sub(1));
    render(fb, renderer, filters, tags, page, &layout);
}

/// Run the subject filter. Mutates `filters` in place; the caller snapshots it
/// beforehand to decide whether the view needs refetching. Sorting is a
/// separate overlay reached from the pager strip — see [`run_sort`].
pub fn run(
    fb: &mut Framebuffer,
    input: &mut Input,
    renderer: &mut TextRenderer,
    tags: &[String],
    filters: &mut Filters,
) -> anyhow::Result<()> {
    let mut layout = Layout::compute(renderer, fb.var.xres, fb.var.yres);
    let mut page = 0usize;
    render(fb, renderer, filters, tags, page, &layout);
    fb.send_update(full_rect(fb), WAVEFORM_MODE_GC16)?;

    loop {
        let pages = n_pages(tags.len(), layout.per_page);
        let visible = page_rows(tags, page, layout.per_page).len();

        match input.next_event()? {
            InputEvent::Touch(TouchEvent::Up { x, y }) => {
                match layout.hit(x, y, fb.var.xres, visible) {
                    Some(Tap::Row(slot)) => {
                        let tag = page_rows(tags, page, layout.per_page)[slot].clone();
                        filters.toggle(&tag);
                        // One row, one DU.
                        draw_row(fb, renderer, &layout, slot, &row_text(filters, &tag));
                        let rect = layout.row_rect(slot, fb.var.xres);
                        fb.send_update(rect, WAVEFORM_MODE_DU)?;
                    }
                    Some(Tap::Prev) if page > 0 => {
                        page -= 1;
                        render(fb, renderer, filters, tags, page, &layout);
                        fb.send_update(full_rect(fb), WAVEFORM_MODE_GC16)?;
                    }
                    Some(Tap::Next) if page + 1 < pages => {
                        page += 1;
                        render(fb, renderer, filters, tags, page, &layout);
                        fb.send_update(full_rect(fb), WAVEFORM_MODE_GC16)?;
                    }
                    Some(Tap::Done) => return Ok(()),
                    _ => {}
                }
            }
            InputEvent::Touch(TouchEvent::Down { .. }) => {}
            InputEvent::Touch(TouchEvent::Screenshot) => {
                let _ = crate::eink::screenshot::capture(fb);
            }
            // The bezel buttons page this list too, on the devices that have
            // them — same gesture as the grid behind it.
            InputEvent::Page(dir) => {
                let next = match dir {
                    crate::eink::buttons::PageButton::Next if page + 1 < pages => Some(page + 1),
                    crate::eink::buttons::PageButton::Prev if page > 0 => Some(page - 1),
                    _ => None,
                };
                if let Some(p) = next {
                    page = p;
                    render(fb, renderer, filters, tags, page, &layout);
                    fb.send_update(full_rect(fb), WAVEFORM_MODE_GC16)?;
                }
            }
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
                    page = page.min(n_pages(tags.len(), layout.per_page).saturating_sub(1));
                    render(fb, renderer, filters, tags, page, &layout);
                    fb.send_update(full_rect(fb), WAVEFORM_MODE_GC16)?;
                }
            }
        }
    }
}

/// Opens [`sortmenu::run`], then repaints this menu underneath it.
pub fn run_sort(
    fb: &mut Framebuffer,
    input: &mut Input,
    renderer: &mut TextRenderer,
    sort: &mut SortState,
    has_query: bool,
) -> anyhow::Result<()> {
    *sort = sortmenu::run(fb, input, renderer, *sort, has_query)?;
    Ok(())
}
