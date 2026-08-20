//! Bottom-strip toolbar: an 80px strip of tap zones, left to right `Exit`,
//! `Filter` (carrying `(N)` selected subjects), `Sort`, then `← Prev / N /
//! Next →` over the remaining width under an `n_pages` past 1.

use crate::eink::fb::Framebuffer;
use crate::ui::text::TextRenderer;

pub const STRIP_H: u32 = 80;

const EXIT_ZONE_W: u32 = 200;
/// Filter zone sits immediately right of Exit, same fixed-width pattern.
const FILTER_ZONE_W: u32 = 220;
/// Sort zone sits right of Filter, same pattern. The page nav (Prev/mid/Next)
/// gets whatever width is left.
const SORT_ZONE_W: u32 = 200;
/// Left edge of the Sort zone (right after Exit + Filter).
const SORT_LEFT: u32 = EXIT_ZONE_W + FILTER_ZONE_W;
/// Left edge of the page-nav region (after Exit + Filter + Sort).
const NAV_LEFT: u32 = EXIT_ZONE_W + FILTER_ZONE_W + SORT_ZONE_W;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagerHit {
    Exit,
    Filter,
    /// Opens the sort picker.
    Sort,
    /// Page back or forward: the nav region's left or right half, from [`hit`].
    Prev,
    Next,
}

pub fn n_pages(books: usize, page_size: usize) -> usize {
    // `.max(1)` keeps an empty library on a single (empty) page; the inner one
    // also guards the divide against a degenerate layout.
    books.div_ceil(page_size.max(1)).max(1)
}

pub fn strip_top(fb_yres: u32) -> u32 {
    fb_yres.saturating_sub(STRIP_H)
}

pub fn hit(tx: u32, ty: u32, fb_xres: u32, fb_yres: u32, total_pages: usize) -> Option<PagerHit> {
    if ty < strip_top(fb_yres) {
        return None;
    }
    // Exit, Filter and Sort take the three leftmost fixed slices; the rest of
    // the strip is the page-nav zone, live only when there's somewhere to go.
    if tx < EXIT_ZONE_W {
        return Some(PagerHit::Exit);
    }
    if tx < SORT_LEFT {
        return Some(PagerHit::Filter);
    }
    if tx < NAV_LEFT {
        return Some(PagerHit::Sort);
    }
    if total_pages <= 1 {
        return None;
    }
    // Split the NAV REGION (NAV_LEFT..xres), never the screen: the screen's
    // midpoint can land left of NAV_LEFT, which leaves Prev a sliver and sends
    // every nav tap to Next.
    let nav_mid = (NAV_LEFT + fb_xres) / 2;
    if tx < nav_mid {
        Some(PagerHit::Prev)
    } else {
        Some(PagerHit::Next)
    }
}

pub fn draw(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    page: usize,
    total_pages: usize,
    filter_count: usize,
) {
    let strip_y = strip_top(fb.var.yres);
    // 2px black divider, white strip body below.
    fb.fill_rect(strip_y, 0, fb.var.xres, 2, 0x00);
    fb.fill_rect(strip_y + 2, 0, fb.var.xres, STRIP_H - 2, 0xFF);

    let baseline = (strip_y + STRIP_H * 70 / 100) as i32;

    // Exit on the left. Always visible.
    renderer.draw(fb, 40, baseline, "Exit", false);
    // Vertical separator after exit zone.
    fb.fill_rect(strip_y + 12, EXIT_ZONE_W - 2, 2, STRIP_H - 24, 0x00);

    // Filter zone, right of Exit. `(N)` when N subjects are selected, so a
    // filtered state is obvious without opening the menu.
    let filter_label = if filter_count > 0 {
        format!("Filter ({filter_count})")
    } else {
        "Filter".to_string()
    };
    renderer.draw(fb, EXIT_ZONE_W as i32 + 40, baseline, &filter_label, false);
    fb.fill_rect(strip_y + 12, SORT_LEFT - 2, 2, STRIP_H - 24, 0x00);

    // Sort zone, right of Filter.
    renderer.draw(fb, SORT_LEFT as i32 + 40, baseline, "Sort", false);
    fb.fill_rect(strip_y + 12, NAV_LEFT - 2, 2, STRIP_H - 24, 0x00);

    if total_pages <= 1 {
        return;
    }

    let label_prev = "← Prev";
    let label_next = "Next →";
    let label_mid = format!("{} / {}", page + 1, total_pages);

    // Prev = left half of the nav region, Next = right half (see `hit`). Show
    // each label only when that direction exists, so a dead edge reads as dead.
    if page > 0 {
        renderer.draw(fb, NAV_LEFT as i32 + 40, baseline, label_prev, false);
    }
    // Center "N / M" in the nav region (`NAV_LEFT`..xres, NOT the whole screen —
    // screen-centering shoved it left against the Sync separator once Sync
    // widened the fixed zones).
    let mid_w = renderer.measure_width(&label_mid);
    let mid_x = (NAV_LEFT as i32 + fb.var.xres as i32) / 2 - mid_w as i32 / 2;
    renderer.draw(fb, mid_x, baseline, &label_mid, false);
    if page + 1 < total_pages {
        let next_w = renderer.measure_width(label_next);
        let next_x = fb.var.xres as i32 - 80 - next_w as i32;
        renderer.draw(fb, next_x, baseline, label_next, false);
    }
}
