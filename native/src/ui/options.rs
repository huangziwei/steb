//! The Options page: the subject filter, the sort, and what the device is
//! running. A blocking sub-loop over [`crate::ui::panel`]; GC16 on open, page
//! and rotate, a DU on the one row a tap changed.
//!
//! The two About chips are buttons, not settings: [`run`] leaves the page on
//! either, the caller fetches, and the page is reopened on whatever landed.

use crate::convert;
use crate::eink::fb::{Framebuffer, WAVEFORM_MODE_DU, WAVEFORM_MODE_GC16};
use crate::eink::input::{Input, InputEvent};
use crate::eink::touch::TouchEvent;
use crate::install;
use crate::ui::filter::{self, Filters};
use crate::ui::panel::{self, Chip, Cursor, Item, Layout, Tap};
use crate::ui::scale::Scale;
use crate::ui::sort::SortState;
use crate::ui::text::TextRenderer;

/// The page's title, and the word the grid's toolbar opens it under.
pub const TITLE: &str = "Options";

/// Title size as a design pixel; `ui::scale` maps it to the panel.
const TITLE_PX: f32 = 44.0;

/// The line under the title. What this page changes does not reach the grid
/// until it is left, which is the one thing about it that is not obvious.
const STATUS: &str = "Subjects and sorting reload the grid when you leave";

/// The chip that clears the subject filter, drawn ahead of the vocabulary.
/// SE's own `all` option, which `se::listing` strips out of the tag list.
const ALL_CHIP: &str = "All";

/// Which section a chip belongs to. Headings and notes belong to none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    /// [`ALL_CHIP`], then one chip per tag in the order given.
    Subjects,
    /// One of [`SortState::offered`].
    Sort,
    /// Two buttons, in [`FETCHABLE`] order.
    About,
}

/// Why [`run`] returned.
#[derive(Clone, Copy)]
pub enum Exit {
    /// The `[ Done ]` strip.
    Done,
    /// An About chip. `crate::install::run` is what it asks for.
    Fetch(&'static install::Source),
}

/// The About buttons, in draw order.
const FETCHABLE: [&install::Source; 2] = [&install::STEB, &install::BOKAI];

/// What the About section reports.
///
/// Read once when the page opens: bokai states its version by being run, and
/// a settings page that spawns a process per repaint is a settings page that
/// stutters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct About {
    /// This build, as `Cargo.toml` states it.
    pub steb: String,
    /// bokai's own version, where a copy that runs here is installed.
    pub bokai: Option<String>,
}

impl About {
    /// What is on the device.
    pub fn read() -> About {
        About {
            steb: install::VERSION.to_string(),
            bokai: convert::installed_version(),
        }
    }

    /// The note under the About heading: both versions on one line.
    fn note(&self) -> String {
        let bokai = match &self.bokai {
            Some(version) => format!("bokai {version}"),
            None => "bokai not installed".to_string(),
        };
        format!("{} {}   ·   {bokai}", install::STEB.name, self.steb)
    }

    /// What each [`FETCHABLE`] button says. bokai is fetched before it can be
    /// updated, and the word is the difference.
    fn label(&self, source: &install::Source) -> String {
        let have = match source.name {
            name if name == install::BOKAI.name => self.bokai.is_some(),
            _ => true,
        };
        match have {
            true => format!("Update {}", source.name),
            false => format!("Get {}", source.name),
        }
    }
}

/// The page in draw order, and which [`Section`] each of its items belongs to.
pub struct Page {
    pub items: Vec<Item>,
    sections: Vec<Option<Section>>,
}

impl Page {
    fn push(&mut self, item: Item, section: Option<Section>) {
        self.items.push(item);
        self.sections.push(section);
    }

    fn section(&self, item: usize) -> Option<Section> {
        self.sections.get(item).copied().flatten()
    }
}

/// What the page shows and what it changes.
///
/// `tags` is the subject vocabulary `se::listing` read off the last listing
/// page, which is empty until one has been fetched; the Subjects field is then
/// [`ALL_CHIP`] alone.
pub struct Settings<'a> {
    pub tags: &'a [String],
    pub filters: &'a mut Filters,
    pub sort: &'a mut SortState,
    /// Whether a query stands, which is what offers the Relevance sort.
    pub has_query: bool,
    pub about: &'a About,
}

impl Settings<'_> {
    /// The whole page in draw order, and which [`Section`] each item belongs
    /// to.
    pub fn page(&self) -> Page {
        let mut p = Page {
            items: Vec::new(),
            sections: Vec::new(),
        };

        p.push(Item::Heading("Subjects".into()), None);
        let mut subjects = vec![Chip::new(ALL_CHIP, self.filters.is_empty())];
        subjects.extend(
            self.tags
                .iter()
                .map(|tag| Chip::new(filter::display(tag), self.filters.is_selected(tag))),
        );
        p.push(Item::Chips(subjects), Some(Section::Subjects));

        p.push(Item::Heading("Sort".into()), None);
        p.push(
            Item::Chips(
                SortState::offered(self.has_query)
                    .into_iter()
                    .map(|s| Chip::new(s.chip(), s == *self.sort))
                    .collect(),
            ),
            Some(Section::Sort),
        );

        p.push(Item::Heading("About".into()), None);
        p.push(Item::Note(self.about.note()), None);
        p.push(
            Item::Chips(
                FETCHABLE
                    .iter()
                    // Never filled: both are buttons rather than settings.
                    .map(|source| Chip::new(self.about.label(source), false))
                    .collect(),
            ),
            Some(Section::About),
        );

        p
    }

    /// One tap on chip `chip` of `section`.
    ///
    /// Chips are added and removed by nothing here: a tap only ever flips
    /// which ones are on, so [`run`] may redraw the tapped row against the
    /// layout it already has.
    fn apply(&mut self, section: Section, chip: usize) {
        match section {
            // Chip 0 is [`ALL_CHIP`]; the rest run alongside `tags`.
            Section::Subjects => match chip.checked_sub(1) {
                None => self.filters.clear(),
                Some(at) => {
                    if let Some(tag) = self.tags.get(at) {
                        self.filters.toggle(tag);
                    }
                }
            },
            Section::Sort => {
                if let Some(picked) = SortState::offered(self.has_query).get(chip) {
                    *self.sort = *picked;
                }
            }
            // `run` leaves the page on these before it ever gets here.
            Section::About => {}
        }
    }
}

/// [`TITLE_PX`] on a panel `fb_xres` wide.
fn title_px(fb_xres: u32) -> f32 {
    Scale::of_width(fb_xres).font(TITLE_PX)
}

/// [`Layout::compute`] against the two faces this page draws.
fn layout_for(
    renderer: &mut TextRenderer,
    xres: u32,
    yres: u32,
    items: &[Item],
    from: Cursor,
) -> Layout {
    let lh = renderer.line_height();
    let title_lh = renderer.at_px(title_px(xres), |r| r.line_height());
    Layout::compute(lh, title_lh, xres, yres, items, from, |s| {
        renderer.measure_width(s)
    })
}

/// Where each page of `items` starts on this panel.
fn starts_for(renderer: &mut TextRenderer, xres: u32, yres: u32, items: &[Item]) -> Vec<Cursor> {
    let lh = renderer.line_height();
    let title_lh = renderer.at_px(title_px(xres), |r| r.line_height());
    panel::pages(lh, title_lh, xres, yres, items, |s| {
        renderer.measure_width(s)
    })
}

fn draw(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    layout: &Layout,
    items: &[Item],
    at: usize,
    pages: usize,
) {
    let chrome = panel::Chrome {
        title: TITLE,
        status: STATUS,
        title_px: title_px(fb.var.xres),
        page: at,
        pages,
    };
    panel::render(fb, renderer, layout, chrome, items);
}

/// The page as [`run`] first draws it, without a device behind it.
/// `crate::bin::preview` reads it back as a PNG.
pub fn render_screen(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    settings: &Settings<'_>,
    at: usize,
) {
    let p = settings.page();
    let (xres, yres) = (fb.var.xres, fb.var.yres);
    let starts = starts_for(renderer, xres, yres, &p.items);
    let at = at.min(starts.len() - 1);
    let layout = layout_for(renderer, xres, yres, &p.items, starts[at]);
    draw(fb, renderer, &layout, &p.items, at, starts.len());
}

/// Run the page. `settings` is mutated in place; the caller snapshots it
/// beforehand to decide whether the grid needs refetching.
pub fn run(
    fb: &mut Framebuffer,
    input: &mut Input,
    renderer: &mut TextRenderer,
    settings: &mut Settings<'_>,
) -> anyhow::Result<Exit> {
    let mut p = settings.page();
    let mut starts = starts_for(renderer, fb.var.xres, fb.var.yres, &p.items);
    let mut at = 0usize;
    let mut layout = layout_for(renderer, fb.var.xres, fb.var.yres, &p.items, starts[at]);
    draw(fb, renderer, &layout, &p.items, at, starts.len());
    fb.send_update(panel::full_rect(fb), WAVEFORM_MODE_GC16)?;

    loop {
        match input.next_event()? {
            InputEvent::Touch(TouchEvent::Up { x, y }) => {
                match panel::hit(&layout, fb.var.xres, x, y) {
                    Some(Tap::Done) => return Ok(Exit::Done),
                    Some(Tap::Prev) if at > 0 => {
                        at -= 1;
                    }
                    Some(Tap::Next) if at + 1 < starts.len() => {
                        at += 1;
                    }
                    Some(Tap::Chip { item, chip }) => {
                        let Some(section) = p.section(item) else {
                            continue;
                        };
                        if section == Section::About {
                            // A chip the row does not have is not a tap on it.
                            let Some(source) = FETCHABLE.get(chip) else {
                                continue;
                            };
                            return Ok(Exit::Fetch(source));
                        }
                        settings.apply(section, chip);
                        // Only the `on` flags moved, so the layout still
                        // describes the panel and one row repaints alone.
                        p = settings.page();
                        panel::draw_row(fb, renderer, &layout, &p.items, item);
                        if let Some(rect) = layout.row_rect(item, fb.var.xres) {
                            fb.send_update(rect, WAVEFORM_MODE_DU)?;
                        }
                        continue;
                    }
                    _ => continue,
                }
                layout = layout_for(renderer, fb.var.xres, fb.var.yres, &p.items, starts[at]);
                draw(fb, renderer, &layout, &p.items, at, starts.len());
                fb.send_update(panel::full_rect(fb), WAVEFORM_MODE_GC16)?;
            }
            InputEvent::Touch(TouchEvent::Down { .. }) => {}
            InputEvent::Touch(TouchEvent::Screenshot) => {
                let _ = crate::eink::screenshot::capture(fb);
            }
            // The bezel buttons page this list too, on the devices that have
            // them — same gesture as the grid behind it.
            InputEvent::Page(dir) => {
                let next = match dir {
                    crate::eink::buttons::PageButton::Next if at + 1 < starts.len() => Some(at + 1),
                    crate::eink::buttons::PageButton::Prev if at > 0 => Some(at - 1),
                    _ => None,
                };
                if let Some(page) = next {
                    at = page;
                    layout = layout_for(renderer, fb.var.xres, fb.var.yres, &p.items, starts[at]);
                    draw(fb, renderer, &layout, &p.items, at, starts.len());
                    fb.send_update(panel::full_rect(fb), WAVEFORM_MODE_GC16)?;
                }
            }
            // The one place this page drains the X queue and re-reads the
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
                    // A resize changes how many chip lines a page holds, so
                    // where each one starts moves with it.
                    starts = starts_for(renderer, fb.var.xres, fb.var.yres, &p.items);
                    at = at.min(starts.len() - 1);
                    layout = layout_for(renderer, fb.var.xres, fb.var.yres, &p.items, starts[at]);
                    draw(fb, renderer, &layout, &p.items, at, starts.len());
                    fb.send_update(panel::full_rect(fb), WAVEFORM_MODE_GC16)?;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::se::url::Sort;

    fn tags() -> Vec<String> {
        ["adventure", "science-fiction", "poetry"]
            .iter()
            .map(|t| t.to_string())
            .collect()
    }

    fn about() -> About {
        About {
            steb: "0.2.1".into(),
            bokai: Some("0.2.0".into()),
        }
    }

    /// A page over the fixture vocabulary, opened unfiltered and unsorted.
    struct Opened {
        tags: Vec<String>,
        filters: Filters,
        sort: SortState,
        about: About,
    }

    impl Opened {
        fn new() -> Opened {
            Opened {
                tags: tags(),
                filters: Filters::default(),
                sort: SortState::default(),
                about: about(),
            }
        }

        fn settings(&mut self, has_query: bool) -> Settings<'_> {
            Settings {
                tags: &self.tags,
                filters: &mut self.filters,
                sort: &mut self.sort,
                has_query,
                about: &self.about,
            }
        }

        /// One tap, and the chips it left behind.
        fn tap(&mut self, section: Section, chip: usize) -> Vec<Chip> {
            let mut settings = self.settings(false);
            settings.apply(section, chip);
            chips_of(&settings.page(), section)
        }

        fn chips(&mut self, section: Section, has_query: bool) -> Vec<Chip> {
            chips_of(&self.settings(has_query).page(), section)
        }
    }

    fn chips_of(p: &Page, section: Section) -> Vec<Chip> {
        let at = p
            .sections
            .iter()
            .position(|s| *s == Some(section))
            .expect("the page carries every section");
        match &p.items[at] {
            Item::Chips(chips) => chips.clone(),
            other => panic!("{section:?} is not a chip field: {other:?}"),
        }
    }

    #[test]
    fn the_subject_field_opens_on_all_and_names_the_vocabulary() {
        let chips = Opened::new().chips(Section::Subjects, false);
        assert_eq!(chips.len(), tags().len() + 1);
        assert_eq!(chips[0].label, ALL_CHIP);
        assert!(chips[0].on, "nothing selected reads as All");
        // A slug is drawn as words.
        assert_eq!(chips[2].label, "Science fiction");
        assert!(chips[1..].iter().all(|c| !c.on));
    }

    #[test]
    fn a_selected_subject_fills_its_own_chip_and_empties_all() {
        let mut open = Opened::new();
        let chips = open.tap(Section::Subjects, 2);

        assert!(open.filters.is_selected("science-fiction"));
        assert!(!chips[0].on, "All is off once a subject is picked");
        assert!(chips[2].on);
        assert_eq!(chips.iter().filter(|c| c.on).count(), 1);

        // The same chip again puts it back.
        open.tap(Section::Subjects, 2);
        assert!(open.filters.is_empty());
    }

    #[test]
    fn the_all_chip_clears_whatever_was_picked() {
        let mut open = Opened::new();
        for chip in [1, 2, 3] {
            open.tap(Section::Subjects, chip);
        }
        assert_eq!(open.filters.count(), 3);
        assert!(open.tap(Section::Subjects, 0)[0].on);
        assert!(open.filters.is_empty());
    }

    #[test]
    fn sorting_is_a_pick_and_relevance_is_search_only() {
        let mut open = Opened::new();

        // Browsing: the row opens with Default and no Relevance chip.
        let browsing = open.chips(Section::Sort, false);
        assert_eq!(browsing.len(), SortState::ALL.len() - 1);
        assert!(browsing[0].on);
        assert!(!browsing.iter().any(|c| c.label == "Relevance"));

        let picked = open.tap(Section::Sort, 1);
        assert_eq!(open.sort, SortState(Some(Sort::Newest)));
        assert_eq!(
            picked.iter().filter(|c| c.on).count(),
            1,
            "a pick, not a toggle"
        );
        assert!(picked[1].on);

        // Under a query the row gains Relevance, and it sits second.
        let searching = open.chips(Section::Sort, true);
        assert_eq!(searching.len(), SortState::ALL.len());
        assert_eq!(searching[1].label, "Relevance");
    }

    #[test]
    fn the_about_row_says_what_is_installed_and_what_each_button_would_do() {
        let buttons = Opened::new().chips(Section::About, false);
        assert_eq!(buttons.len(), FETCHABLE.len());
        assert_eq!(buttons[0].label, "Update Steb");
        assert_eq!(buttons[1].label, "Update bokai");
        // Neither is a setting, so neither is ever filled.
        assert!(buttons.iter().all(|c| !c.on));

        let note = about().note();
        assert!(note.contains("Steb 0.2.1"), "{note}");
        assert!(note.contains("bokai 0.2.0"), "{note}");
    }

    #[test]
    fn bokai_is_got_before_it_is_updated() {
        let mut open = Opened {
            about: About {
                steb: "0.2.1".into(),
                bokai: None,
            },
            ..Opened::new()
        };
        let buttons = open.chips(Section::About, false);
        assert_eq!(buttons[0].label, "Update Steb");
        assert_eq!(buttons[1].label, "Get bokai");
        assert!(open.about.note().contains("bokai not installed"));
    }

    /// Before the first listing lands there is no vocabulary, and the field is
    /// the one chip that needs none.
    #[test]
    fn an_empty_vocabulary_still_leaves_a_page_to_draw() {
        let mut open = Opened {
            tags: Vec::new(),
            ..Opened::new()
        };
        let p = open.settings(false).page();
        let chips = chips_of(&p, Section::Subjects);
        assert_eq!(chips.len(), 1);
        assert_eq!(chips[0].label, ALL_CHIP);
        // Three headings and a note carry no section of their own.
        assert_eq!(p.items.len(), p.sections.len());
        assert_eq!(p.sections.iter().filter(|s| s.is_none()).count(), 4);
    }

    /// Every fetchable add-on is one the installer knows how to fetch.
    #[test]
    fn the_about_buttons_name_the_installers_own_sources() {
        assert_eq!(FETCHABLE.len(), 2);
        assert_eq!(FETCHABLE[0].name, install::STEB.name);
        assert_eq!(FETCHABLE[1].name, install::BOKAI.name);
        assert_eq!(FETCHABLE[1].dest, convert::EXTENSION_DIR);
    }
}
