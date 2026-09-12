//! The grid's control bar, a [`crate::ui::strip`]: `[ Exit ]`, `[ Options ]`,
//! then paging across the rest. [`Zones`] places the three and [`hit`] answers
//! from the same placement [`draw`] draws at.
//!
//! Subjects and sorting are two sections of [`crate::ui::options`] rather than
//! two slots of their own: a bar of four named zones leaves a page turn a
//! sliver wide on the narrow panels.

use crate::eink::fb::Framebuffer;
use crate::ui::scale::Scale;
use crate::ui::strip;
use crate::ui::text::TextRenderer;

/// Bar height on a panel `fb_xres` wide, from [`strip::H`].
pub fn strip_h(fb_xres: u32) -> u32 {
    strip::h(fb_xres)
}

const EXIT_ZONE_W: u32 = 200;
/// Right of [`EXIT_ZONE_W`]. The wider of the two: its label carries the
/// count of subjects the grid is filtered by.
const OPTIONS_ZONE_W: u32 = 300;

/// What paging keeps at the right of the bar whatever the named zones ask for:
/// two halves a thumb can tell apart.
const NAV_MIN_W: u32 = 240;
/// The least the named zones shrink to. Below this `Exit` stops being
/// tappable, and a user who cannot leave is worse off than one who cannot page.
const FIXED_MIN_W: u32 = 260;

/// Text inset from a zone's edge, on a zone wide enough for it.
const LABEL_INSET: u32 = 32;

/// The slot labels. An action is bracketed; paging only moves through the
/// pages it reports and is not.
const EXIT: &str = "[ Exit ]";
const PREV: &str = "← Prev";
const NEXT: &str = "Next →";

/// Label sizes the bar drops through, largest first, until every label fits
/// the zone it is drawn in.
const LABEL_PX: &[f32] = &[32.0, 28.0, 24.0, 20.0, 17.0];

/// Reference panel width for the layouts checked against one.
pub const NARROWEST_PANEL_W: u32 = 600;

/// Where the toolbar's zones start. The named two hold their own widths while
/// that leaves [`NAV_MIN_W`] for paging, and shrink together, in proportion,
/// where it does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Zones {
    /// Left edge of `Options`. `Exit` runs from 0 to here.
    pub options: u32,
    /// Left edge of the page-nav region.
    pub nav: u32,
    /// Where `Prev` gives way to `Next`.
    pub nav_mid: u32,
}

impl Zones {
    pub fn compute(fb_xres: u32) -> Self {
        let s = Scale::of_width(fb_xres);
        let (exit_w, options_w) = (s.px(EXIT_ZONE_W), s.px(OPTIONS_ZONE_W));
        let fixed_w = (exit_w + options_w).max(1);
        let room = fb_xres
            .saturating_sub(s.px(NAV_MIN_W))
            .clamp(s.px(FIXED_MIN_W).min(fixed_w), fixed_w);
        let options = exit_w * room / fixed_w;
        // `min`: a panel narrower than [`FIXED_MIN_W`] plus [`NAV_MIN_W`] has
        // no nav region, and `hit` answers `None` past the named zones.
        let nav = (options + options_w * room / fixed_w).min(fb_xres);
        Self {
            options,
            nav,
            nav_mid: (nav + fb_xres) / 2,
        }
    }

    /// Width of the zone starting at `left` and ending at `right`.
    fn width(left: u32, right: u32) -> u32 {
        right.saturating_sub(left)
    }
}

/// The inset a `zone_w`-wide zone can afford, so a narrow zone still shows its
/// label rather than pushing it out.
fn inset_for(zone_w: u32, s: Scale) -> u32 {
    (zone_w / 6).min(s.px(LABEL_INSET))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagerHit {
    Exit,
    Options,
    /// The nav region's left and right half.
    Prev,
    Next,
}

pub fn n_pages(books: usize, page_size: usize) -> usize {
    // Outer `.max(1)`: an empty grid holds one page. Inner: no divide by zero.
    books.div_ceil(page_size.max(1)).max(1)
}

pub fn strip_top(fb_xres: u32, fb_yres: u32) -> u32 {
    strip::top(fb_xres, fb_yres)
}

/// The zone at `(tx, ty)`. Integer geometry, no framebuffer.
pub fn hit(tx: u32, ty: u32, fb_xres: u32, fb_yres: u32, total_pages: usize) -> Option<PagerHit> {
    if ty < strip_top(fb_xres, fb_yres) {
        return None;
    }
    let z = Zones::compute(fb_xres);
    if tx < z.options {
        return Some(PagerHit::Exit);
    }
    if tx < z.nav {
        return Some(PagerHit::Options);
    }
    if total_pages <= 1 {
        return None;
    }
    // Splits `nav`..`fb_xres`, which `Zones` keeps at [`NAV_MIN_W`] wherever
    // the panel affords it.
    if tx < z.nav_mid {
        Some(PagerHit::Prev)
    } else {
        Some(PagerHit::Next)
    }
}

/// The largest [`LABEL_PX`] at which every label fits the zone it is drawn in,
/// or the smallest of them. The labels are passed as `(text, zone width)`.
fn label_px(renderer: &mut TextRenderer, labels: &[(&str, u32)], s: Scale) -> f32 {
    for px in LABEL_PX {
        let scaled = s.font(*px);
        let fits = renderer.at_px(scaled, |r| {
            labels
                .iter()
                .all(|(text, zone_w)| r.measure_width(text) + inset_for(*zone_w, s) * 2 <= *zone_w)
        });
        if fits {
            return scaled;
        }
    }
    s.font(LABEL_PX[LABEL_PX.len() - 1])
}

pub fn draw(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    page: usize,
    total_pages: usize,
    filter_count: usize,
) {
    let xres = fb.var.xres;
    let s = Scale::of_width(xres);
    let strip_y = strip::base(fb);
    let z = Zones::compute(xres);
    // The count of subjects `options` filters the grid by.
    let options_label = match filter_count {
        0 => "[ Options ]".to_string(),
        n => format!("[ Options ({n}) ]"),
    };
    let counter = format!("{} / {}", page + 1, total_pages);

    // One size for the bar, so every label sits on the same baseline at the
    // same weight. The nav labels share their region, hence the halves.
    let nav_half = Zones::width(z.nav, xres) / 2;
    let px = label_px(
        renderer,
        &[
            (EXIT, Zones::width(0, z.options)),
            (&options_label, Zones::width(z.options, z.nav)),
            (PREV, nav_half),
            (NEXT, nav_half),
        ],
        s,
    );

    renderer.at_px(px, |r| {
        // Inside `at_px`, so the metrics the baseline is centred on are the
        // ones this size actually draws with.
        let baseline = strip::baseline(xres, strip_y, r);
        strip::label(fb, r, 0, z.options, baseline, EXIT, false);
        strip::label(
            fb,
            r,
            z.options,
            Zones::width(z.options, z.nav),
            baseline,
            &options_label,
            false,
        );

        if total_pages <= 1 {
            return;
        }
        // Paging keeps the open region the slots do not divide: each label sits
        // inside the half `hit` answers for it, a `Next` drawn left of
        // `nav_mid` reading as `Prev` under the finger.
        let inset = inset_for(nav_half, s);
        if page > 0 {
            r.draw(fb, (z.nav + inset) as i32, baseline, PREV, false);
        }
        let next_w = r.measure_width(NEXT);
        let next_x = xres
            .saturating_sub(inset)
            .saturating_sub(next_w)
            .max(z.nav_mid + inset);
        if page + 1 < total_pages {
            r.draw(fb, next_x as i32, baseline, NEXT, false);
        }
        // Centred in the nav region, and dropped where either label reaches it.
        let counter_w = r.measure_width(&counter);
        let counter_x = z.nav + Zones::width(z.nav, xres) / 2 - counter_w / 2;
        let clear_left = page == 0 || counter_x > z.nav + inset + r.measure_width(PREV);
        let clear_right = page + 1 >= total_pages || counter_x + counter_w < next_x;
        if clear_left && clear_right {
            r.draw(fb, counter_x as i32, baseline, &counter, false);
        }
    });

    // Separators last: a label that ran long is cut by its own zone edge rather
    // than reading as part of the next one. One to the left of each *live*
    // zone, so a single page leaves no empty compartment behind.
    for (edge, live) in [(z.options, true), (z.nav, total_pages > 1)] {
        if live {
            strip::separator(fb, edge);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XRES: u32 = NARROWEST_PANEL_W;
    const YRES: u32 = 800;

    /// Every panel width a shipped device's X server reports.
    const PANELS: &[u32] = &[600, 758, 1072, 1236, 1264, 1860];

    /// A tap at `x`, vertically centred in the bar.
    fn tap(x: u32, pages: usize) -> Option<PagerHit> {
        hit(x, YRES - strip_h(XRES) / 2, XRES, YRES, pages)
    }

    #[test]
    fn every_fixed_zone_is_reachable_on_the_narrowest_panel() {
        let z = Zones::compute(XRES);
        assert_eq!(tap(2, 1), Some(PagerHit::Exit));
        assert_eq!(tap(z.options + 2, 1), Some(PagerHit::Options));
        assert_eq!(tap(z.nav - 2, 1), Some(PagerHit::Options));
    }

    #[test]
    fn nav_splits_its_own_region_rather_than_the_screen() {
        let z = Zones::compute(XRES);
        // The screen midpoint sits inside the fixed zones at this width.
        assert_eq!(tap(z.nav + 2, 3), Some(PagerHit::Prev));
        assert_eq!(tap(XRES - 2, 3), Some(PagerHit::Next));
    }

    /// The named zones stay ordered and leave the nav region room, at every
    /// width the fleet reports.
    #[test]
    fn the_named_zones_always_leave_room_to_page() {
        for &xres in PANELS {
            let z = Zones::compute(xres);
            assert!(z.options > 0, "{xres}: no Exit zone");
            assert!(z.options < z.nav, "{xres}: options/nav collapsed");
            assert!(
                z.nav < z.nav_mid && z.nav_mid < xres,
                "{xres}: no nav halves"
            );
        }
    }

    /// Two named zones instead of four leave the page turn a thumb's width at
    /// every panel in the fleet.
    #[test]
    fn paging_keeps_its_own_region_on_every_panel() {
        for &xres in PANELS {
            let z = Zones::compute(xres);
            let nav_w = xres - z.nav;
            assert!(
                nav_w >= Scale::of_width(xres).px(NAV_MIN_W),
                "{xres}: {nav_w}px to page in"
            );
        }
    }

    /// Above the bar is the grid's, and a single page has no nav.
    #[test]
    fn the_bar_owns_only_its_own_rows() {
        assert_eq!(hit(0, 0, XRES, YRES, 4), None);
        let ty = strip_top(XRES, YRES);
        assert_eq!(hit(XRES - 1, ty, XRES, YRES, 1), None, "one page, no nav");
        assert_eq!(hit(0, ty, XRES, YRES, 1), Some(PagerHit::Exit));
    }
}
