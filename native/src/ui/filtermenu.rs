//! Subject-filter overlay — a paged checklist of Standard Ebooks' subject tags.
//!
//! Subject is the only filter dimension, so this menu is a single flat
//! checklist rather than a facet list drilling into value lists.
//!
//! Two properties worth stating, because both look like omissions:
//!
//! - **No counts.** Filtering happens on SE's server, and a listing page does
//!   not say how many books carry a tag — so a number here would either be
//!   invented or cost a request per tag.
//! - **The vocabulary is not ours.** It arrives parsed from the listing page's
//!   own `<select name="tags[]">`, so a subject SE adds shows up here with no
//!   release on our side.
//!
//! Same blocking sub-loop shape as [`crate::ui::sortmenu`]: a pure `hit`
//! geometry fn, a `render` that paints the panel, and a `run` that owns input
//! until Done. Full GC16 on open, page and rotate; a single-row DU on a toggle
//! so ticking a box doesn't flash the screen.

use crate::eink::fb::{Framebuffer, MxcfbRect, WAVEFORM_MODE_DU, WAVEFORM_MODE_GC16};
use crate::eink::input::{Input, InputEvent};
use crate::eink::touch::TouchEvent;
use crate::orientation::Orientation;
use crate::ui::filter::Filters;
use crate::ui::sort::SortState;
use crate::ui::sortmenu;
use crate::ui::text::TextRenderer;

/// Bottom strip height — matches `ui/sortmenu.rs` and `ui/diag.rs`.
const STRIP_H: u32 = 120;
const MARGIN_X: u32 = 60;

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
    /// Rows that fit on one page, always at least one.
    per_page: usize,
}

impl Layout {
    fn compute(renderer: &TextRenderer, yres: u32) -> Self {
        let lh = renderer.line_height().max(1);
        let rows_top = lh * 3;
        // Generous tap targets — 96px floor regardless of font size, matching
        // the sort picker so the two menus feel like one thing.
        let row_h = lh.saturating_mul(2).max(96);
        let strip_top = yres.saturating_sub(STRIP_H);
        let per_page = ((strip_top.saturating_sub(rows_top)) / row_h).max(1) as usize;
        Layout {
            rows_top,
            row_h,
            strip_top,
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
/// firmware whose font set we do not control.
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
    renderer.draw(fb, MARGIN_X as i32, baseline, text, false);
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
    renderer.draw(fb, tx, (layout.rows_top - 8) as i32, &title, false);

    for (slot, tag) in page_rows(tags, page, layout.per_page).iter().enumerate() {
        draw_row(fb, renderer, layout, slot, &row_text(filters, tag));
    }

    draw_strip(fb, renderer, layout, page, pages);
}

/// Bottom strip: `< Prev` | `[ Done ]` | `Next >`, with the paging labels drawn
/// only when there is somewhere to go.
fn draw_strip(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    layout: &Layout,
    page: usize,
    pages: usize,
) {
    let xres = fb.var.xres;
    let top = layout.strip_top;
    let third = xres / 3;
    fb.fill_rect(top, 0, xres, 2, 0x00);
    fb.fill_rect(top + 2, 0, xres, STRIP_H - 2, 0xFF);

    let baseline = (top + STRIP_H * 60 / 100) as i32;
    let centered = |label: &str, slot: u32, renderer: &mut TextRenderer, fb: &mut Framebuffer| {
        let w = renderer.measure_width(label);
        let x = (slot * third) as i32 + ((third as i32 - w as i32) / 2).max(0);
        renderer.draw(fb, x, baseline, label, false);
    };

    if page > 0 {
        centered("< Prev", 0, renderer, fb);
    }
    centered("[ Done ]", 1, renderer, fb);
    if page + 1 < pages {
        centered("Next >", 2, renderer, fb);
    }
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
    orient: &mut Orientation,
) -> anyhow::Result<()> {
    let mut layout = Layout::compute(renderer, fb.var.yres);
    let mut page = 0usize;
    render(fb, renderer, filters, tags, page, &layout);
    fb.send_update(full_rect(fb), WAVEFORM_MODE_GC16)?;

    loop {
        let pages = n_pages(tags.len(), layout.per_page);
        let visible = page_rows(tags, page, layout.per_page).len();

        match input.next()? {
            InputEvent::Touch(TouchEvent::Up { x, y }) => {
                match layout.hit(x, y, fb.var.xres, visible) {
                    Some(Tap::Row(slot)) => {
                        let tag = page_rows(tags, page, layout.per_page)[slot].clone();
                        filters.toggle(&tag);
                        // Only the checkbox changed — repaint that one row with
                        // a fast DU rather than flashing the whole panel.
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
            InputEvent::Tick => {
                let o = Orientation::detect();
                if o != *orient {
                    *orient = o;
                    input.set_orientation(o);
                    layout = Layout::compute(renderer, fb.var.yres);
                    page = page.min(n_pages(tags.len(), layout.per_page) - 1);
                    render(fb, renderer, filters, tags, page, &layout);
                    fb.send_update(full_rect(fb), WAVEFORM_MODE_GC16)?;
                }
            }
        }
    }
}

/// Open the sort picker, then repaint this menu underneath it.
///
/// Kept here so the caller has one "open the menus" entry point rather than
/// having to know the two are siblings.
pub fn run_sort(
    fb: &mut Framebuffer,
    input: &mut Input,
    renderer: &mut TextRenderer,
    sort: &mut SortState,
    has_query: bool,
    orient: &mut Orientation,
) -> anyhow::Result<()> {
    *sort = sortmenu::run(fb, input, renderer, *sort, has_query, orient)?;
    Ok(())
}
