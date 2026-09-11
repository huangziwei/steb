//! The search overlay: the shared search bar standing on the device's own
//! on-screen keyboard. Keys arrive as X keysyms on [`crate::eink::fb::Pump`];
//! what a candidate engine eats and commits arrives over [`crate::lipc`].

use anyhow::Result;

use crate::eink::fb::{Framebuffer, MxcfbRect, WAVEFORM_MODE_DU, WAVEFORM_MODE_GC16};
use crate::eink::input::{Input, InputEvent};
use crate::eink::keysym::{Typed, of_keysym};
use crate::eink::touch::TouchEvent;
use crate::lipc::Service;
use crate::ui::WHITE;
use crate::ui::scale::Scale;
use crate::ui::searchbar;
use crate::ui::strip;
use crate::ui::text::TextRenderer;

/// Gap closing the band, under the prompt line. Design pixels; see
/// [`crate::ui::scale`].
const BAND_GAP: u32 = 24;

/// The two slots on the bar.
const BACK: &str = "[ Back ]";
const SEARCH: &str = "[ Search ]";

/// The line under the bar, on an empty field and on a typed one.
const PROMPT_EMPTY: &str = "Type to search Standard Ebooks";
const PROMPT_TYPED: &str = "Press Enter, or tap Search";

/// A tap on the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tap {
    /// Leave without searching, returning the query the overlay opened with.
    Back,
    /// Submit: run the search on what has been typed.
    Search,
}

/// What the keyboard types into: the query, and what a candidate engine is
/// still composing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Query {
    text: String,
    /// Drawn after `text` and matched on by nothing.
    preedit: String,
}

impl Query {
    fn of(text: &str) -> Self {
        Self {
            text: text.to_string(),
            preedit: String::new(),
        }
    }

    /// Takes `said` onto `text`.
    fn typed(&mut self, said: char) {
        self.text.push(said);
    }

    /// Takes the last character off `text`, answering whether there was one.
    fn backspace(&mut self) -> bool {
        self.text.pop().is_some()
    }

    /// Takes `said` onto `text`, clearing `preedit`.
    fn commit(&mut self, said: &str) {
        self.preedit.clear();
        self.text.push_str(said);
    }

    /// Takes `count` characters off the end of `text`.
    fn delete(&mut self, count: usize) {
        for _ in 0..count {
            self.text.pop();
        }
    }

    /// `keyboardCommit` carries the text; `keyboardSetPreeditString`
    /// `position:str`; `keyboardDelete` `before:after`; `keyboardReplace`
    /// `before:after:str`.
    fn set(&mut self, property: &str, value: &str) -> bool {
        match property {
            "keyboardCommit" => self.commit(value),
            "keyboardSetPreeditString" => {
                let (_, said) = value.split_once(':').unwrap_or(("", value));
                said.clone_into(&mut self.preedit);
            }
            "keyboardDelete" => {
                let (before, _) = value.split_once(':').unwrap_or((value, ""));
                self.delete(before.parse().unwrap_or(0));
            }
            "keyboardReplace" => {
                let mut parts = value.splitn(3, ':');
                let before = parts.next().unwrap_or_default().parse().unwrap_or(0);
                let said = parts.nth(1).unwrap_or_default().to_string();
                self.delete(before);
                self.commit(&said);
            }
            _ => return false,
        }
        true
    }
}

/// Where the overlay's own chrome sits, against a keyboard that takes the foot
/// of the screen at a height only the device states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Layout {
    /// Top edge of the keyboard. Everything drawn stands above it and every
    /// touch below it belongs to the framework.
    keyboard_top: u32,
    /// Top edge of the [`BACK`] / [`SEARCH`] bar.
    strip_top: u32,
    /// Bottom of the band the search bar and the prompt take, refreshed on its
    /// own after a keystroke.
    band_bottom: u32,
}

impl Layout {
    /// `line_h` is [`TextRenderer::line_height`], the prompt's own row.
    fn compute(line_h: u32, fb_xres: u32, fb_yres: u32) -> Self {
        let keyboard_h = crate::keyboard::height(fb_yres as i32).clamp(0, fb_yres as i32) as u32;
        let keyboard_top = fb_yres.saturating_sub(keyboard_h);
        Self {
            keyboard_top,
            strip_top: keyboard_top.saturating_sub(strip::h(fb_xres)),
            band_bottom: searchbar::top(fb_xres)
                + searchbar::height(fb_xres)
                + line_h.max(1)
                + Scale::of_width(fb_xres).px(BAND_GAP),
        }
    }

    /// The slot at `(tx, ty)`: `None` above the bar, and on the keyboard.
    fn hit(&self, tx: u32, ty: u32, fb_xres: u32) -> Option<Tap> {
        if !(self.strip_top..self.keyboard_top).contains(&ty) {
            return None;
        }
        match tx < fb_xres / 2 {
            true => Some(Tap::Back),
            false => Some(Tap::Search),
        }
    }
}

/// What a batch of keysyms did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    Nothing,
    /// The band needs redrawing.
    Moved,
    Search,
    Back,
}

/// Takes `keysyms` into `query`. `Escape` and `Enter` drop the rest of them.
fn typed(query: &mut Query, keysyms: &[u32]) -> Act {
    let mut act = Act::Nothing;
    for keysym in keysyms {
        let Some(said) = of_keysym(*keysym) else {
            continue;
        };
        match said {
            Typed::Char(said) => {
                query.typed(said);
                act = Act::Moved;
            }
            Typed::Backspace => {
                if query.backspace() {
                    act = Act::Moved;
                }
            }
            Typed::Enter => return Act::Search,
            Typed::Escape => return Act::Back,
        }
    }
    act
}

/// Takes what the keyboard set on this app's lipc service into `query`,
/// answering whether it moved.
fn committed(service: Option<&mut Service>, query: &mut Query) -> bool {
    let Some(service) = service else {
        return false;
    };
    let mut moved = false;
    for set in service.drain() {
        moved |= query.set(&set.property, &set.value);
    }
    moved
}

fn full_rect(fb: &Framebuffer) -> MxcfbRect {
    MxcfbRect {
        top: 0,
        left: 0,
        width: fb.var.xres,
        height: fb.var.yres,
    }
}

/// The query band at the top, its own rect for a per-keystroke DU.
fn band_rect(fb: &Framebuffer, layout: &Layout) -> MxcfbRect {
    MxcfbRect {
        top: 0,
        left: 0,
        width: fb.var.xres,
        height: layout.band_bottom,
    }
}

/// The search bar over a prompt. The caller white-fills the band first. SE
/// offers no autocomplete endpoint; the search runs once, on submit.
fn draw_band(fb: &mut Framebuffer, renderer: &mut TextRenderer, query: &Query) {
    let xres = fb.var.xres;
    searchbar::draw_composing(fb, renderer, &query.text, &query.preedit);

    let prompt = match query.text.trim().is_empty() && query.preedit.is_empty() {
        true => PROMPT_EMPTY,
        false => PROMPT_TYPED,
    };
    let w = renderer.measure_width(prompt);
    let y = (searchbar::top(xres) + searchbar::height(xres) + renderer.line_height().max(1)) as i32;
    renderer.draw(fb, ((xres as i32 - w as i32) / 2).max(0), y, prompt, false);
}

/// The bar, standing on the keyboard rather than on the screen's foot.
fn draw_strip(fb: &mut Framebuffer, renderer: &mut TextRenderer, layout: &Layout) {
    let xres = fb.var.xres;
    let half = xres / 2;
    let top = strip::base_at(fb, layout.strip_top);
    let baseline = strip::baseline(xres, top, renderer);
    strip::label(fb, renderer, 0, half, baseline, BACK, false);
    strip::label(fb, renderer, half, xres - half, baseline, SEARCH, false);
    strip::separator_at(fb, top, half);
}

/// The whole overlay. The keyboard's own rows are another window's and are
/// left white underneath it.
fn render(fb: &mut Framebuffer, renderer: &mut TextRenderer, query: &Query, layout: &Layout) {
    fb.fill_rect(0, 0, fb.var.xres, fb.var.yres, WHITE);
    draw_band(fb, renderer, query);
    draw_strip(fb, renderer, layout);
}

/// The overlay as [`run`] first draws it, without a device behind it.
/// `crate::bin::preview` reads it back as a PNG.
pub fn render_screen(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    query: &str,
    preedit: &str,
) {
    let layout = Layout::compute(renderer.line_height(), fb.var.xres, fb.var.yres);
    let query = Query {
        text: query.to_string(),
        preedit: preedit.to_string(),
    };
    render(fb, renderer, &query, &layout);
}

/// Runs the search overlay, returning the typed query on submit and `initial`
/// on `[ Back ]`. The caller acts on a difference. `initial` pre-fills the
/// field, carrying the current search into a re-open.
pub fn run(
    fb: &mut Framebuffer,
    input: &mut Input,
    renderer: &mut TextRenderer,
    initial: &str,
) -> Result<String> {
    // Every key the candidate engine does not eat arrives over X, which needs
    // none of this: a service that will not open is Latin typing.
    let mut service = match Service::open(crate::keyboard::CLIENT) {
        Ok(service) => {
            eprintln!("lipc: {} is open", service.name());
            Some(service)
        }
        Err(err) => {
            eprintln!("?? lipc: {err:#} — Latin typing only");
            None
        }
    };
    // `EVIOCGRAB` is exclusive against the keyboard's own window.
    input.set_keyboard(true);
    crate::keyboard::open();
    let out = drive(fb, input, renderer, &mut service, initial);
    crate::keyboard::close();
    // The lipc socket closes with `service`; a descriptor left in `watched`
    // outlives it.
    input.watch([None, None]);
    input.set_keyboard(false);
    input.retake();
    out
}

fn drive(
    fb: &mut Framebuffer,
    input: &mut Input,
    renderer: &mut TextRenderer,
    service: &mut Option<Service>,
    initial: &str,
) -> Result<String> {
    let mut query = Query::of(initial);
    let mut layout = Layout::compute(renderer.line_height(), fb.var.xres, fb.var.yres);

    render(fb, renderer, &query, &layout);
    fb.send_update(full_rect(fb), WAVEFORM_MODE_GC16)?;

    // DU, not GC16: a keystroke must not flash the panel.
    macro_rules! refresh_band {
        () => {{
            fb.fill_rect(0, 0, fb.var.xres, layout.band_bottom, WHITE);
            draw_band(fb, renderer, &query);
            fb.send_update(band_rect(fb, &layout), WAVEFORM_MODE_DU)?;
        }};
    }

    loop {
        // A `KeyPress` arrives on the X connection and a commit on the lipc
        // socket, neither of them an input device.
        input.watch([fb.raw_fd(), service.as_ref().map(|s| s.raw_fd())]);
        match input.next_event()? {
            InputEvent::Touch(TouchEvent::Up { x, y }) => {
                // The keyboard's own rows belong to the framework's window.
                if y >= layout.keyboard_top {
                    continue;
                }
                if let Some(tap) = searchbar::hit(x, y, fb.var.xres, !query.text.is_empty()) {
                    if matches!(tap, searchbar::Tap::Clear) {
                        query = Query::default();
                        refresh_band!();
                    }
                    continue;
                }
                match layout.hit(x, y, fb.var.xres) {
                    Some(Tap::Search) => return Ok(query.text),
                    Some(Tap::Back) => return Ok(initial.to_string()),
                    None => {}
                }
            }
            InputEvent::Touch(TouchEvent::Down { .. }) => {}
            InputEvent::Touch(TouchEvent::Screenshot) => {
                let _ = crate::eink::screenshot::capture(fb);
            }
            InputEvent::Page(_) => {}
            InputEvent::Tick => {
                let pump = fb.pump_events();
                if let Some(covered) = pump.covered {
                    input.set_covered(covered);
                }
                input.retake();
                let moved = match typed(&mut query, &pump.typed) {
                    Act::Search => return Ok(query.text),
                    Act::Back => return Ok(initial.to_string()),
                    Act::Moved => true,
                    Act::Nothing => false,
                } | committed(service.as_mut(), &mut query);
                let turned = input.follow_orientation();
                if turned || pump.resized.is_some() {
                    input.set_size(fb.var.xres, fb.var.yres);
                    layout = Layout::compute(renderer.line_height(), fb.var.xres, fb.var.yres);
                }
                if pump.covered == Some(true) {
                    continue;
                }
                if turned || pump.resized.is_some() || pump.repaint || pump.covered.is_some() {
                    render(fb, renderer, &query, &layout);
                    fb.send_update(full_rect(fb), WAVEFORM_MODE_GC16)?;
                } else if moved {
                    refresh_band!();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XRES: u32 = 1264;
    const YRES: u32 = 1680;

    /// Every panel width a shipped device's X server reports, and its height.
    const PANELS: &[(u32, u32)] = &[
        (600, 800),
        (758, 1024),
        (1072, 1448),
        (1236, 1648),
        (1264, 1680),
        (1860, 2480),
    ];

    /// A body line at the panel's own density, standing in for the metrics a
    /// font would answer with.
    fn layout(xres: u32, yres: u32) -> Layout {
        Layout::compute(Scale::of_width(xres).px(40), xres, yres)
    }

    /// The bar stands clear of the keyboard on every panel, and the band it
    /// refreshes stands clear of the bar.
    #[test]
    fn the_bar_stands_on_the_keyboard() {
        for &(xres, yres) in PANELS {
            let l = layout(xres, yres);
            assert!(l.keyboard_top < yres, "{xres}: no keyboard to stand on");
            assert_eq!(
                l.strip_top + strip::h(xres),
                l.keyboard_top,
                "{xres}: the bar and the keyboard do not meet"
            );
            assert!(
                l.band_bottom < l.strip_top,
                "{xres}: the band runs into the bar"
            );
        }
    }

    /// Left half leaves, right half searches, and the keyboard's own rows are
    /// nobody's.
    #[test]
    fn the_bar_splits_in_halves() {
        let l = layout(XRES, YRES);
        let row = l.strip_top + strip::h(XRES) / 2;
        assert_eq!(l.hit(2, row, XRES), Some(Tap::Back));
        assert_eq!(l.hit(XRES - 2, row, XRES), Some(Tap::Search));
        assert_eq!(l.hit(XRES / 2, row, XRES), Some(Tap::Search));
        assert_eq!(l.hit(XRES / 2, l.keyboard_top, XRES), None, "the keyboard");
        assert_eq!(l.hit(XRES / 2, 0, XRES), None, "the band");
    }

    #[test]
    fn a_run_of_keys_lands_in_order() {
        let mut query = Query::default();
        // 'a', 'b', BackSpace, 'c'.
        assert_eq!(typed(&mut query, &[0x61, 0x62, 0xFF08, 0x63]), Act::Moved);
        assert_eq!(query.text, "ac");
        assert_eq!(typed(&mut query, &[0xFFE1]), Act::Nothing);
        assert_eq!(query.text, "ac");
    }

    /// `Enter` and `Escape` drop what follows them in the same batch.
    #[test]
    fn a_submit_ends_the_batch() {
        let mut query = Query::of("ab");
        assert_eq!(typed(&mut query, &[0xFF0D, 0x63]), Act::Search);
        assert_eq!(query.text, "ab");
        assert_eq!(typed(&mut query, &[0xFF1B, 0x63]), Act::Back);
        assert_eq!(query.text, "ab");
    }

    /// A backspace on an empty query moves nothing.
    #[test]
    fn an_empty_query_has_nothing_to_take_back() {
        let mut query = Query::default();
        assert_eq!(typed(&mut query, &[0xFF08]), Act::Nothing);
        assert_eq!(query.text, "");
    }

    /// The four properties the keyboard sets on this app's own service.
    #[test]
    fn the_candidate_engine_commits_over_the_service() {
        let mut query = Query::default();
        assert!(query.set("keyboardSetPreeditString", "0:ni"));
        assert_eq!((query.text.as_str(), query.preedit.as_str()), ("", "ni"));
        assert!(query.set("keyboardCommit", "你"));
        assert_eq!((query.text.as_str(), query.preedit.as_str()), ("你", ""));
        assert!(query.set("keyboardDelete", "1:0"));
        assert_eq!(query.text, "");
        assert!(
            !query.set("keyboardGetSurround", ""),
            "not a set this reads"
        );
    }

    /// `keyboardReplace` takes `before` characters off and commits the third
    /// field in their place.
    #[test]
    fn a_replace_takes_back_what_it_names() {
        let mut query = Query::of("teh");
        assert!(query.set("keyboardReplace", "2:0:he"));
        assert_eq!(query.text, "the");
    }
}
