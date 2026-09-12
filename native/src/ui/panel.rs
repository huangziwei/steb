//! A settings page: section headings, and the values under each one drawn as
//! chips.
//!
//! **A settings page is not a list.** Every value a setting can take is on the
//! panel at once, in its own tap target, so reading the page and changing it
//! are the same gesture.
//!
//! - **Geometry comes from the font.** A chip line is a multiple of the line
//!   height with a tap-target floor, so a larger face gives larger targets
//!   instead of a broken layout.
//! - **Chips flow.** A field of twenty subjects takes as many lines as it
//!   needs, and [`Layout::compute`] fills a page with as many of those lines
//!   as finish above the strip. One field may therefore span pages: a
//!   [`Cursor`] is an item *and* a line inside it. Standard Ebooks' subject
//!   vocabulary is read off its own markup rather than compiled in, so the
//!   longest field on the page is one this app does not set the length of,
//!   and a field that outgrows the panel pages rather than losing its tail.
//! - **Hit-testing over a measured layout.** Every chip's rect is resolved
//!   once in [`Layout::compute`] and both [`render`] and [`hit`] read the same
//!   ones, so a finger can never land on a chip other than the one under it.
//!
//! [`crate::ui::options`] builds the [`Item`]s and owns what a tap means.

use crate::eink::fb::{Framebuffer, MxcfbRect};
use crate::ui::scale::Scale;
use crate::ui::strip;
use crate::ui::text::TextRenderer;
use crate::ui::{BLACK, QUIET, WHITE};

/// Left inset for the title, the headings, the chips and the notes. One edge
/// for the whole page.
pub const MARGIN_X: u32 = 60;

/// Blank space either side of a chip's text.
const CHIP_PAD: u32 = 24;
/// Between one chip and the next, across and down.
const CHIP_GAP: u32 = 20;
/// Chip height as a percentage of the line it sits on.
const CHIP_H_PCT: u32 = 74;
/// Border of an unfilled chip.
const CHIP_BORDER: u32 = 2;
/// The rule under a [`Item::Heading`].
const RULE_H: u32 = 3;

/// Height of one line of chips, as a percentage of the text line height, and
/// the floor it never goes under: a chip is [`CHIP_H_PCT`] of it and has to
/// stay a finger target on a ~300 dpi panel.
const CHIP_LINE_PCT: u32 = 135;
const CHIP_LINE_MIN: u32 = 96;

/// Row heights of the two lines that are not chips, as a percentage of the
/// text line height, and the air above the first row.
const HEADING_PCT: u32 = 175;
const NOTE_PCT: u32 = 125;
const AIR_PCT: u32 = 70;

/// One value a row can take, and its own tap target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    pub label: String,
    /// What the setting is currently on. Drawn filled.
    pub on: bool,
}

impl Chip {
    pub fn new(label: impl Into<String>, on: bool) -> Self {
        Chip {
            label: label.into(),
            on,
        }
    }
}

/// One line of the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// A section heading with a rule under it. Names what follows; not
    /// tappable.
    Heading(String),
    /// The values under the heading above, flowing across as many lines as
    /// they need.
    Chips(Vec<Chip>),
    /// Quiet text under the field it explains, one line per `\n`-delimited
    /// line. Not tappable.
    Note(String),
}

impl Item {
    /// Lines of text stacked inside this item's row. Chips answer 1: their own
    /// line count comes from [`flow`], which needs the panel's width.
    fn lines(&self) -> u32 {
        match self {
            Item::Note(text) => text.lines().count().max(1) as u32,
            _ => 1,
        }
    }

    /// Whether this item draws anything in [`QUIET`].
    ///
    /// A `WAVEFORM_MODE_DU` region is two-level and snaps mid-grey to black or
    /// white, so a caller refreshing one row alone has to take the whole panel
    /// with a `WAVEFORM_MODE_GC16` when this is true.
    pub fn has_quiet(&self) -> bool {
        matches!(self, Item::Note(_))
    }
}

/// Where a page starts: an item, and the line inside it. Anything but a chip
/// field is drawn whole, so `line` is 0 for all of them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursor {
    pub item: usize,
    pub line: usize,
}

/// One chip's rect on the panel, as [`render`] draws it and [`hit`] reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChipBox {
    pub left: u32,
    pub top: u32,
    pub width: u32,
    pub height: u32,
}

impl ChipBox {
    fn holds(&self, x: u32, y: u32) -> bool {
        x >= self.left && x < self.left + self.width && y >= self.top && y < self.top + self.height
    }
}

/// One item's slice of the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    /// Which [`Item`] this draws.
    item: usize,
    top: u32,
    height: u32,
    /// For a chip field: the chips drawn on this page, by index into the
    /// item's own list. Empty for a heading or a note.
    chips: Vec<(usize, ChipBox)>,
}

/// Vertical geometry of one page, derived from the faces actually drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Body line height.
    lh: u32,
    title_top: u32,
    status_top: u32,
    rows_top: u32,
    rows: Vec<Row>,
    strip_top: u32,
    /// Where this page starts, and where the next one does.
    from: Cursor,
    next: Cursor,
}

impl Layout {
    /// The page starting at `from`.
    ///
    /// `lh` and `title_lh` are the line heights of the two faces actually
    /// drawn; spacing derived from the wrong one overlaps the title and the
    /// status line. `measure` is the same face `lh` came from.
    pub fn compute(
        lh: u32,
        title_lh: u32,
        xres: u32,
        yres: u32,
        items: &[Item],
        from: Cursor,
        mut measure: impl FnMut(&str) -> u32,
    ) -> Self {
        let s = Scale::of_width(xres);
        let lh = lh.max(1);
        let title_lh = title_lh.max(1);
        let title_top = title_lh / 2;
        let status_top = title_top + title_lh;
        let rows_top = status_top + lh + lh * AIR_PCT / 100;
        let strip_top = yres.saturating_sub(strip::h(xres));
        let chip_line = chip_line_h(lh, s);

        let mut rows: Vec<Row> = Vec::new();
        let mut cursor = Cursor {
            item: from.item.min(items.len()),
            line: from.line,
        };
        let mut y = rows_top;

        while let Some(item) = items.get(cursor.item) {
            // The first row of a page stands whether or not it fits: a page
            // that could hold nothing would never advance.
            let first = rows.is_empty();
            let room = strip_top.saturating_sub(y.min(strip_top));

            match item {
                Item::Chips(chips) => {
                    let lines = flow(chips, xres, &mut measure);
                    let want = lines.len().saturating_sub(cursor.line);
                    let fits = (room / chip_line.max(1)) as usize;
                    let take = want.min(fits.max(usize::from(first)));
                    if take == 0 {
                        break;
                    }
                    let height = take as u32 * chip_line;
                    let mut placed = Vec::new();
                    for (n, line) in lines[cursor.line..cursor.line + take].iter().enumerate() {
                        let top = y + n as u32 * chip_line;
                        for (at, left, width) in line {
                            placed.push((
                                *at,
                                ChipBox {
                                    left: *left,
                                    top: top + (chip_line - chip_line * CHIP_H_PCT / 100) / 2,
                                    width: *width,
                                    height: chip_line * CHIP_H_PCT / 100,
                                },
                            ));
                        }
                    }
                    rows.push(Row {
                        item: cursor.item,
                        top: y,
                        height,
                        chips: placed,
                    });
                    y += height;
                    // The field ran past the foot of the page: the next page
                    // opens on the line this one stopped at.
                    if cursor.line + take < lines.len() {
                        cursor.line += take;
                        break;
                    }
                    cursor = Cursor {
                        item: cursor.item + 1,
                        line: 0,
                    };
                }
                other => {
                    let height = match other {
                        Item::Heading(_) => lh * HEADING_PCT / 100,
                        _ => lh * NOTE_PCT / 100 * other.lines(),
                    };
                    if height > room && !first {
                        break;
                    }
                    rows.push(Row {
                        item: cursor.item,
                        top: y,
                        height,
                        chips: Vec::new(),
                    });
                    y += height;
                    cursor = Cursor {
                        item: cursor.item + 1,
                        line: 0,
                    };
                }
            }
        }

        Layout {
            lh,
            title_top,
            status_top,
            rows_top,
            rows,
            strip_top,
            from,
            next: cursor,
        }
    }

    /// Where the page after this one starts.
    pub fn next(&self) -> Cursor {
        self.next
    }

    /// True on the bottom strip.
    pub fn on_strip(&self, y: u32) -> bool {
        y >= self.strip_top
    }

    /// One item's rect, for a single-row refresh. `None` for an item this page
    /// does not draw.
    pub fn row_rect(&self, item: usize, xres: u32) -> Option<MxcfbRect> {
        let row = self.rows.iter().find(|r| r.item == item)?;
        Some(MxcfbRect {
            top: row.top,
            left: 0,
            width: xres,
            height: row.height,
        })
    }
}

/// [`Layout::compute`] for every page of `items`, answering where each starts.
///
/// Always at least one page, so a panel too short to hold anything still has
/// somewhere to draw its strip.
pub fn pages(
    lh: u32,
    title_lh: u32,
    xres: u32,
    yres: u32,
    items: &[Item],
    mut measure: impl FnMut(&str) -> u32,
) -> Vec<Cursor> {
    let mut starts = vec![Cursor::default()];
    loop {
        let at = *starts.last().expect("seeded above");
        let next = Layout::compute(lh, title_lh, xres, yres, items, at, &mut measure).next();
        if next.item >= items.len() || next == at {
            return starts;
        }
        starts.push(next);
    }
}

/// Height of one line of chips.
fn chip_line_h(lh: u32, s: Scale) -> u32 {
    (lh * CHIP_LINE_PCT / 100).max(s.px(CHIP_LINE_MIN))
}

/// The chips laid out across the page, as lines of `(index, x, width)`.
///
/// **The single source for drawing and for [`hit`]**: a chip is only as wide
/// as its own text, so anything measuring them a second time would put a
/// finger on a different one. A chip too wide for a whole line keeps the line
/// to itself, cut to the width there is rather than running off the page.
fn flow(
    chips: &[Chip],
    xres: u32,
    measure: &mut impl FnMut(&str) -> u32,
) -> Vec<Vec<(usize, u32, u32)>> {
    let s = Scale::of_width(xres);
    let (pad, gap, margin) = (s.px(CHIP_PAD), s.px(CHIP_GAP), s.px(MARGIN_X));
    let left = margin;
    let right = xres.saturating_sub(margin).max(left + 1);
    let widest = right - left;

    let mut lines: Vec<Vec<(usize, u32, u32)>> = Vec::new();
    let mut line: Vec<(usize, u32, u32)> = Vec::new();
    let mut x = left;
    for (at, chip) in chips.iter().enumerate() {
        let width = measure(&chip.label)
            .saturating_add(pad * 2)
            .min(widest)
            .max(1);
        if !line.is_empty() && x + width > right {
            lines.push(std::mem::take(&mut line));
            x = left;
        }
        line.push((at, x, width));
        x += width + gap;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// What a tap on the page landed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tap {
    /// Which item, and which of its chips.
    Chip {
        item: usize,
        chip: usize,
    },
    Prev,
    Done,
    Next,
}

/// The chip or strip slot at `(x, y)`.
///
/// `None` for a heading, a note, and the space between and past the chips.
/// None of them is a control, and a settings page where the gaps do something
/// is one you cannot rest a hand on.
pub fn hit(layout: &Layout, xres: u32, x: u32, y: u32) -> Option<Tap> {
    if layout.on_strip(y) {
        let third = xres / 3;
        return Some(if x < third {
            Tap::Prev
        } else if x < third * 2 {
            Tap::Done
        } else {
            Tap::Next
        });
    }
    let row = layout.rows.iter().find(|r| r.holds(y))?;
    row.chips
        .iter()
        .find(|(_, box_)| box_.holds(x, y))
        .map(|(chip, _)| Tap::Chip {
            item: row.item,
            chip: *chip,
        })
}

impl Row {
    fn holds(&self, y: u32) -> bool {
        y >= self.top && y < self.top + self.height
    }
}

/// Everything on the page that is not one of its rows.
#[derive(Debug, Clone, Copy)]
pub struct Chrome<'a> {
    /// The heading the page is known by.
    pub title: &'a str,
    /// The quiet line under it, or `""` for none.
    pub status: &'a str,
    /// The size the title is set at, already scaled to this panel.
    pub title_px: f32,
    /// Which page of how many, for the strip's paging labels.
    pub page: usize,
    pub pages: usize,
}

/// The whole page: title, status line, rows, strip.
pub fn render(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    layout: &Layout,
    chrome: Chrome<'_>,
    items: &[Item],
) {
    let xres = fb.var.xres;
    let margin = Scale::of_width(xres).px(MARGIN_X);
    fb.fill_rect(0, 0, xres, fb.var.yres, WHITE);

    renderer.at_px(chrome.title_px, |r| {
        let baseline = (layout.title_top + r.line_height() * 78 / 100) as i32;
        r.draw_bold(fb, margin as i32, baseline, chrome.title, BLACK);
    });
    if !chrome.status.is_empty() {
        let baseline = (layout.status_top + layout.lh * 78 / 100) as i32;
        renderer.draw_ink(fb, margin as i32, baseline, chrome.status, QUIET);
    }

    for at in 0..layout.rows.len() {
        draw_at(fb, renderer, layout, items, at);
    }
    draw_strip(fb, renderer, chrome.page, chrome.pages);
}

/// Redraw one item in place, for a chip that changed. The rest of the page is
/// left alone: a full-panel refresh to invert one chip is half a second of ink.
pub fn draw_row(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    layout: &Layout,
    items: &[Item],
    item: usize,
) {
    if let Some(at) = layout.rows.iter().position(|r| r.item == item) {
        draw_at(fb, renderer, layout, items, at);
    }
}

fn draw_at(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    layout: &Layout,
    items: &[Item],
    at: usize,
) {
    let xres = fb.var.xres;
    let margin = Scale::of_width(xres).px(MARGIN_X);
    let row = &layout.rows[at];
    let Some(item) = items.get(row.item) else {
        return;
    };
    fb.fill_rect(row.top, 0, xres, row.height, WHITE);

    match item {
        // The text sits at the foot of its row with the rule directly under
        // it, so the empty half above reads as the gap between sections.
        Item::Heading(text) => {
            let foot = row.top + row.height.saturating_sub(Scale::of_width(xres).px(RULE_H));
            let baseline = foot.saturating_sub(layout.lh / 5) as i32;
            renderer.draw_bold(fb, margin as i32, baseline, text, BLACK);
            fb.fill_rect(
                foot,
                margin,
                xres.saturating_sub(margin * 2),
                Scale::of_width(xres).px(RULE_H),
                BLACK,
            );
        }
        // No rule of its own: the chips are visibly bounded already, and a
        // line under every setting buries the structure of the page.
        Item::Chips(chips) => {
            for (chip, box_) in &row.chips {
                if let Some(chip) = chips.get(*chip) {
                    draw_chip(fb, renderer, *box_, chip);
                }
            }
        }
        Item::Note(text) => {
            let lines = item.lines();
            let per = row.height / lines.max(1);
            for (n, line) in text.lines().enumerate() {
                let top = row.top + n as u32 * per;
                let baseline = (top + per / 2) as i32 + (layout.lh as i32 * 36 / 100);
                renderer.draw_ink(fb, margin as i32, baseline, line, QUIET);
            }
        }
    }
}

/// One chip: filled when it is what the setting is on, outlined when it is
/// merely available.
///
/// **Filled, not ticked.** `ui::text` cuts glyph coverage to one bit, so a
/// tick is a smudge at this size; an inverted block is unambiguous, and it is
/// the idiom the rest of the app already marks state with.
fn draw_chip(fb: &mut Framebuffer, renderer: &mut TextRenderer, rect: ChipBox, chip: &Chip) {
    let (ground, ink) = match chip.on {
        true => (BLACK, WHITE),
        false => (WHITE, BLACK),
    };
    fb.fill_rect(rect.top, rect.left, rect.width, rect.height, ground);
    if !chip.on {
        let t = Scale::of_width(fb.var.xres).px(CHIP_BORDER);
        fb.fill_rect(rect.top, rect.left, rect.width, t, ink);
        fb.fill_rect(rect.top + rect.height - t, rect.left, rect.width, t, ink);
        fb.fill_rect(rect.top, rect.left, t, rect.height, ink);
        fb.fill_rect(rect.top, rect.left + rect.width - t, t, rect.height, ink);
    }

    let w = renderer.measure_width(&chip.label);
    let x = rect.left as i32 + ((rect.width as i32 - w as i32) / 2).max(0);
    let lh = renderer.line_height();
    let baseline = (rect.top + rect.height / 2) as i32 + (lh as i32 * 36 / 100);
    renderer.draw_ink(fb, x, baseline, &chip.label, ink);
}

/// Bottom strip: `< Prev` | `[ Done ]` | `Next >`, with the paging labels
/// drawn only where there is somewhere to go.
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

/// The whole panel, for a `WAVEFORM_MODE_GC16` refresh.
pub fn full_rect(fb: &Framebuffer) -> MxcfbRect {
    MxcfbRect {
        top: 0,
        left: 0,
        width: fb.var.xres,
        height: fb.var.yres,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed-width face: every character is 20 px. Lets the geometry be
    /// reasoned about arithmetically, with no font on the machine.
    fn measure(s: &str) -> u32 {
        s.chars().count() as u32 * 20
    }

    /// A design-pixel panel, where `Scale::px` is the identity.
    const XRES: u32 = 1264;
    const YRES: u32 = 1680;
    const LH: u32 = 40;
    const TITLE_LH: u32 = 55;

    fn chips(labels: &[&str]) -> Item {
        Item::Chips(labels.iter().map(|l| Chip::new(*l, false)).collect())
    }

    fn page(items: &[Item], at: Cursor) -> Layout {
        Layout::compute(LH, TITLE_LH, XRES, YRES, items, at, measure)
    }

    /// The same panel with the rows cut to four chip lines: what a field
    /// longer than the page it is drawn on meets.
    const SHORT_YRES: u32 = 700;

    fn short(items: &[Item], at: Cursor) -> Layout {
        Layout::compute(LH, TITLE_LH, XRES, SHORT_YRES, items, at, measure)
    }

    #[test]
    fn chips_wrap_at_the_right_margin_and_never_cross_it() {
        let subjects = chips(&[
            "Adventure",
            "Autobiography",
            "Biography",
            "Childrens",
            "Comedy",
            "Drama",
            "Fantasy",
        ]);
        let Item::Chips(list) = &subjects else {
            unreachable!()
        };
        let lines = flow(list, XRES, &mut measure);
        assert!(lines.len() > 1, "seven chips do not fit one line here");
        for line in &lines {
            let (_, left, width) = *line.last().unwrap();
            assert!(left + width <= XRES - MARGIN_X, "a chip crossed the margin");
            assert_eq!(line[0].1, MARGIN_X, "every line opens at the margin");
        }
        // Every chip is placed exactly once, in order.
        let placed: Vec<usize> = lines.iter().flatten().map(|(at, _, _)| *at).collect();
        assert_eq!(placed, (0..list.len()).collect::<Vec<_>>());
    }

    /// A label wider than the page keeps its line and is cut to it, rather
    /// than being dropped or drawn off the edge.
    #[test]
    fn a_chip_too_wide_for_the_page_still_gets_a_line() {
        let long = "a".repeat(200);
        let Item::Chips(list) = chips(&[&long, "short"]) else {
            unreachable!()
        };
        let lines = flow(&list, XRES, &mut measure);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0][0].2, XRES - MARGIN_X * 2);
        assert!(lines[0][0].1 + lines[0][0].2 <= XRES - MARGIN_X);
    }

    #[test]
    fn a_tap_lands_on_the_chip_under_it_and_nowhere_else() {
        let items = vec![Item::Heading("Sort".into()), chips(&["Default", "Newest"])];
        let layout = page(&items, Cursor::default());
        let first = layout.rows[1].chips[0].1;
        let second = layout.rows[1].chips[1].1;

        assert_eq!(
            hit(&layout, XRES, first.left + 2, first.top + 2),
            Some(Tap::Chip { item: 1, chip: 0 })
        );
        assert_eq!(
            hit(&layout, XRES, second.left + 2, second.top + 2),
            Some(Tap::Chip { item: 1, chip: 1 })
        );
        // The gap between two chips is not either of them.
        let gap = first.left + first.width + 2;
        assert!(gap < second.left);
        assert_eq!(hit(&layout, XRES, gap, first.top + 2), None);
        // Neither is the heading, nor the blank right of the last chip.
        assert_eq!(hit(&layout, XRES, MARGIN_X, layout.rows[0].top + 2), None);
        assert_eq!(hit(&layout, XRES, XRES - 2, first.top + 2), None);
    }

    #[test]
    fn the_strip_is_thirds_whatever_is_above_it() {
        let layout = page(&[Item::Heading("About".into())], Cursor::default());
        let y = YRES - 1;
        assert_eq!(hit(&layout, XRES, 2, y), Some(Tap::Prev));
        assert_eq!(hit(&layout, XRES, XRES / 2, y), Some(Tap::Done));
        assert_eq!(hit(&layout, XRES, XRES - 2, y), Some(Tap::Next));
        assert!(layout.on_strip(y));
        assert!(!layout.on_strip(0));
    }

    /// A subject vocabulary longer than the panel it is drawn on is paged
    /// rather than cut: the page it spills off opens on the line it stopped
    /// at, and every line is drawn exactly once across the pages.
    #[test]
    fn one_field_spans_pages_without_losing_or_repeating_a_line() {
        let labels: Vec<String> = (0..60).map(|n| format!("subject-{n:02}")).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let items = vec![Item::Heading("Subjects".into()), chips(&refs)];

        let starts = pages(LH, TITLE_LH, XRES, SHORT_YRES, &items, measure);
        assert!(starts.len() > 1, "60 chips fit one page here");

        let mut seen: Vec<usize> = Vec::new();
        for (at, start) in starts.iter().enumerate() {
            let layout = short(&items, *start);
            assert!(!layout.rows.is_empty(), "page {at} draws nothing");
            for row in &layout.rows {
                if row.item == 1 {
                    seen.extend(row.chips.iter().map(|(c, _)| *c));
                }
            }
        }
        assert_eq!(seen, (0..labels.len()).collect::<Vec<_>>());
    }

    /// Paging always advances, even on a panel with room for nothing.
    #[test]
    fn a_page_that_can_hold_nothing_still_moves_on() {
        let items = vec![
            chips(&["one", "two"]),
            Item::Heading("Next".into()),
            Item::Note("a note".into()),
        ];
        // A panel whose strip leaves no rows at all.
        let short = Layout::compute(LH, TITLE_LH, XRES, 1, &items, Cursor::default(), measure);
        assert_eq!(short.rows.len(), 1, "the first row stands regardless");
        assert!(short.next() != Cursor::default());

        let starts = pages(LH, TITLE_LH, XRES, 1, &items, measure);
        assert_eq!(starts.len(), items.len());
    }

    #[test]
    fn a_page_ends_where_the_next_one_begins() {
        let items = vec![
            Item::Heading("Subjects".into()),
            chips(&["Adventure", "Comedy", "Drama"]),
            Item::Note("3 selected".into()),
            Item::Heading("Sort".into()),
            chips(&["Default", "Newest"]),
        ];
        let starts = pages(LH, TITLE_LH, XRES, YRES, &items, measure);
        // Five short items fit one portrait page.
        assert_eq!(starts, vec![Cursor::default()]);

        let layout = page(&items, Cursor::default());
        assert_eq!(layout.rows.len(), items.len());
        assert_eq!(layout.next().item, items.len());
        // Rows stack with no gap and none of them reaches the strip.
        for pair in layout.rows.windows(2) {
            assert_eq!(pair[0].top + pair[0].height, pair[1].top);
        }
        let last = layout.rows.last().unwrap();
        assert!(last.top + last.height <= layout.strip_top);
        assert!(layout.rows[0].top >= layout.rows_top);
    }

    #[test]
    fn only_a_drawn_item_has_a_rect_to_refresh() {
        let items = vec![Item::Heading("Sort".into()), chips(&["Default"])];
        let layout = page(&items, Cursor::default());
        let rect = layout.row_rect(1, XRES).unwrap();
        assert_eq!(rect.left, 0);
        assert_eq!(rect.width, XRES);
        assert_eq!(rect.height, layout.rows[1].height);
        assert!(layout.row_rect(9, XRES).is_none());
    }

    /// A note is the one thing on the page drawn in [`QUIET`], which a
    /// two-level DU refresh would snap.
    #[test]
    fn only_a_note_is_drawn_in_a_shade_du_cannot_hold() {
        assert!(Item::Note("x".into()).has_quiet());
        assert!(!chips(&["a"]).has_quiet());
        assert!(!Item::Heading("x".into()).has_quiet());
        assert_eq!(Item::Note("one\ntwo".into()).lines(), 2);
        assert_eq!(Item::Note(String::new()).lines(), 1);
    }
}
