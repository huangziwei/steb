//! Steb — search Standard Ebooks from the Kindle and download the `.azw3`.
//!
//! One screen: a paginated cover grid, with the keyboard, subject filter and
//! sort picker as blocking overlay sub-loops. Hold a cover and it downloads —
//! there is no detail page.
//!
//! The device layer is Linux-only (`libc::ioctl`'s request argument differs
//! between BSD and Linux), so `eink/` and most of `ui/` are declared here
//! rather than in `lib.rs`. Everything pure — the standardebooks.org client and
//! the catalogue cache — lives in the library so `cargo test` runs on a host.

// The `eink/` and `ui/` modules carry a few helpers the run loop does not
// currently call — swipe classification, some input polling. They are kept so
// the modules stay whole and usable, and cost a few hundred bytes in a
// stripped binary.
#![allow(dead_code)]

mod cache;
mod cover_cache;
mod eink;
mod font;
mod orientation;
mod se;
mod ui;
mod wrap;

use std::path::Path;
use std::time::{Duration, Instant};

use eink::buttons::{Buttons, PageButton};
use eink::fb::{Framebuffer, MxcfbRect, WAVEFORM_MODE_DU, WAVEFORM_MODE_GC16};
use eink::input::{Input, InputEvent};
use eink::touch::{Touch, TouchEvent};
use image::DynamicImage;

use se::listing::Hit;
use se::url::{Endpoint, Listing};
use ui::filter::Filters;
use ui::sort::SortState;
use ui::text::TextRenderer;
use ui::{diag, filtermenu, grid, keyboard, pager, searchbar, toast};

/// On-device extension bundle root — the cache lives under it.
const BUNDLE_DIR: &str = "/mnt/us/extensions/steb";
/// Where books land. A subfolder of `documents/` so Steb's downloads stay
/// distinguishable from everything else in the library; the framework indexes
/// it recursively either way.
const DOWNLOAD_DIR: &str = "/mnt/us/documents/standardebooks";
/// Kindle's own cover thumbnails. Standard Ebooks names its thumbnail file
/// after the azw3's ASIN, which is exactly the name the framework looks for, so
/// the file drops in verbatim.
const THUMBNAILS_DIR: &str = "/mnt/us/system/thumbnails";
/// Where `bin/steb.sh` funnels this process's stderr. The wrapper creates the
/// directory before the first write, so nothing here has to.
const LOG_PATH: &str = "/mnt/us/logs/steb.log";

const FONT_PX: f32 = 32.0;
/// Headroom above the grid for the search bar.
const TOP_MARGIN: u32 = searchbar::TOP + searchbar::HEIGHT + 16;

/// How long a cover must be held — without drifting more than [`ARM_SLOP_PX`]
/// from where the finger landed — before its download arms and fires.
///
/// A plain tap used to download, which misfires constantly: the grid is a wall
/// of covers with nothing else to touch, so every stray brush while reading the
/// titles started a fetch. The armed cue paints on the cell the instant the
/// threshold passes and the download starts with it, so the gesture is "hold
/// until the cover flips", not "hold, then release at the right moment" — an
/// over-long hold costs nothing and a too-short one is a visible non-event
/// rather than a silent misclick.
const ARM_THRESHOLD: Duration = Duration::from_millis(1000);
/// Max drift (either axis, user-visible px) from the landing point that still
/// counts as a hold rather than a drag.
const ARM_SLOP_PX: u32 = 40;
/// How long the armed cue stays up before the download overlay paints over it,
/// so the "held long enough" signal is actually seen.
const ARM_DWELL: Duration = Duration::from_millis(250);
/// How long the hint after a too-short tap stays up.
const TOAST_LINGER: Duration = Duration::from_millis(1200);

/// Refresh rect for one grid cell, for the partial updates the press outline and
/// the armed cue send.
fn cell_rect(cell_x: i32, cell_y: i32, cell_h: u32) -> MxcfbRect {
    MxcfbRect {
        top: cell_y.max(0) as u32,
        left: cell_x.max(0) as u32,
        width: grid::CELL_W,
        height: cell_h,
    }
}

/// A cover outlined under a finger, waiting to see whether the press becomes a
/// hold. Release before [`ARM_THRESHOLD`] is a too-short tap and only shows the
/// hint; holding past it auto-fires the download from the arm deadline.
struct Armed {
    /// Slot on the current page, for redrawing that cell.
    slot: usize,
    /// Index into `view.hits`.
    idx: usize,
    down_at: Instant,
    /// Where the finger landed, so drift past [`ARM_SLOP_PX`] can cancel.
    at: (u32, u32),
}

/// Write one line to stderr, which `bin/steb.sh` redirects into the log.
///
/// Deliberately *only* stderr, never [`LOG_PATH`] directly: the wrapper
/// already redirects `2>>` to that file, so writing it here too would put every
/// line in twice.
fn log(msg: impl AsRef<str>) {
    eprintln!("[{}] {}", now(), msg.as_ref());
}

fn now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "?".into())
}

/// What the grid is currently showing.
///
/// `hits` accumulates across Standard Ebooks pages: SE pages at 12–48 while the
/// device grid pages at `layout.page_size()`, and the two must not be
/// conflated. Hits append here, `pager` pages *this* vector, and the next SE
/// page is pulled only when the user nears the end of it.
///
/// Grid page size is whatever the panel yields — never assume a figure. A
/// Colorsoft reports 1272×1696, not the 1264×1680 its spec sheet implies, which
/// is exactly why `grid::Layout::compute` derives everything from
/// `fb.var.{xres,yres}` at runtime.
struct View {
    query: String,
    filters: Filters,
    sort: SortState,
    hits: Vec<Hit>,
    covers: Vec<Option<DynamicImage>>,
    /// SE pages pulled so far.
    fetched: u32,
    /// Whether Standard Ebooks says another page exists. Driven by its own
    /// `rel="next"` control rather than a page count, so paging does not depend
    /// on how many links the nav happens to render.
    more: bool,
    /// Subject vocabulary, as parsed from the last listing page.
    tags: Vec<String>,
    /// Filenames already in the download directory, so taken books read as taken.
    downloaded: Vec<String>,
}

impl View {
    fn listing(&self, page: u32) -> Listing {
        Listing {
            query: (!self.query.trim().is_empty()).then(|| self.query.clone()),
            page,
            sort: self.sort.0,
            tags: self.filters.as_params(),
        }
    }

    fn has_query(&self) -> bool {
        !self.query.trim().is_empty()
    }

    /// Reset everything derived from a query/filter/sort change. The catalogue
    /// cache is untouched — it is keyed by book, not by what we last searched.
    fn clear_results(&mut self) {
        self.hits.clear();
        self.covers.clear();
        self.fetched = 0;
        self.more = true;
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

/// Run a network call, showing the Diagnostics screen on failure until the user
/// either succeeds via Retry or taps Exit (`Ok(None)`).
///
/// This is why all device setup happens before the first request: `diag` needs
/// a surface to draw on and `input` to take taps.
fn with_diag<T>(
    fb: &mut Framebuffer,
    input: &mut Input,
    renderer: &mut TextRenderer,
    mut attempt: impl FnMut() -> se::http::Result<T>,
) -> anyhow::Result<Option<T>> {
    loop {
        match attempt() {
            Ok(v) => return Ok(Some(v)),
            Err(e) => {
                log(format!("network: {e}"));
                match diag::run(fb, input, renderer, &e)? {
                    diag::Action::Retry => continue,
                    diag::Action::Exit => return Ok(None),
                }
            }
        }
    }
}

/// Pull the next Standard Ebooks page into `view`, if there is one.
fn fetch_next_page(
    client: &se::http::Client,
    view: &mut View,
    catalogue: &mut cache::Catalogue,
) -> se::http::Result<()> {
    if view.fetched > 0 && !view.more {
        return Ok(());
    }
    let page = view.fetched + 1;
    let html = client.text(&Endpoint::Listing(view.listing(page)))?;
    let parsed = match se::listing::parse(&html) {
        Ok(p) => p,
        Err(e) => {
            log(format!("listing parse failed: {e}"));
            return Ok(());
        }
    };

    catalogue.merge(&parsed.hits);
    if !parsed.tags.is_empty() {
        view.tags = parsed.tags;
    }
    view.more = parsed.has_next;
    view.covers
        .resize_with(view.covers.len() + parsed.hits.len(), || None);
    view.hits.extend(parsed.hits);
    view.fetched = page;
    Ok(())
}

/// Launch freshness check: one conditional GET against the public new-releases
/// feed. When Standard Ebooks has published nothing since last launch this
/// returns a 304 with no body and costs essentially nothing — which is the
/// whole point, since the cache means we would otherwise never need to ask.
fn refresh_from_feed(client: &se::http::Client, catalogue: &mut cache::Catalogue) {
    let known = catalogue.feed.clone();
    match client.text_if_modified(&Endpoint::Feed, &known) {
        Ok(se::http::Fresh::Unchanged) => log("feed: 304, catalogue unchanged"),
        Ok(se::http::Fresh::Changed { body, validators }) => {
            let entries = se::feed::parse(&body);
            let state = catalogue.freshness(&entries);
            log(format!("feed: {} entries, {state:?}", entries.len()));
            catalogue.feed = validators;
        }
        // Never fatal: a failed freshness check just means we show what we
        // already have, which is the entire reason the cache exists.
        Err(e) => log(format!("feed check failed (non-fatal): {e}")),
    }
}

fn label_of(hit: &Hit) -> grid::Label<'_> {
    grid::Label {
        text: &hit.title,
        script: font::Script::Unknown,
    }
}

/// Paint one page of the grid plus the search bar and pager strip.
#[allow(clippy::too_many_arguments)]
fn draw_page(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    view: &View,
    layout: grid::Layout,
    page: usize,
    total_pages: usize,
) {
    fb.fill_rect(0, 0, fb.var.xres, fb.var.yres, 0xFF);
    searchbar::draw(fb, renderer, &view.query);

    let start = page * layout.page_size();
    let end = (start + layout.page_size()).min(view.hits.len());

    if view.hits.is_empty() {
        // Zero results is routine, not an error: Standard Ebooks has no typo
        // tolerance, and typing on a device keyboard makes mistakes cheap. Say
        // what was searched and point back at the keyboard, rather than showing
        // an empty grid that looks broken.
        let lh = renderer.line_height().max(1);
        let msg = if view.has_query() {
            format!("No books match \u{201c}{}\u{201d}", view.query)
        } else {
            "No books to show".to_string()
        };
        let hint = "Tap the search bar to edit";
        for (i, line) in [msg.as_str(), hint].iter().enumerate() {
            let w = renderer.measure_width(line);
            let x = ((fb.var.xres as i32 - w as i32) / 2).max(0);
            let y = (fb.var.yres / 3 + i as u32 * lh * 2) as i32;
            renderer.draw(fb, x, y, line, false);
        }
    }

    for idx in start..end {
        let (cx, cy) = layout.cell_xy(idx - start);
        let cover = view.covers.get(idx).and_then(|c| c.as_ref());
        let rect = grid::draw_book_cell(
            fb,
            renderer,
            cx,
            cy,
            layout.cell_h,
            cover,
            label_of(&view.hits[idx]),
        );
        // A book already in the library gets a corner check, so a second tap
        // reads as redundant rather than silently re-downloading.
        if is_downloaded(view, idx) {
            grid::draw_downloaded_badge(fb, rect);
        }
    }

    pager::draw(fb, renderer, page, total_pages, view.filters.count());
}

/// Is this book already in `documents/standardebooks/`?
///
/// Matched on Standard Ebooks' own filename, which is stable per book — the
/// same string we would write if the user tapped it.
fn is_downloaded(view: &View, idx: usize) -> bool {
    let Some(hit) = view.hits.get(idx) else {
        return false;
    };
    let stem = hit.path.as_key().replace('/', "_");
    view.downloaded.iter().any(|f| f.starts_with(&stem))
}

/// Fetch covers for the visible page, filling each cell as its image arrives.
///
/// Cells are painted as placeholders first and refreshed one at a time, so a
/// slow link never blocks the grid from appearing. A cover is fetched at most
/// once ever: the on-disk cache is keyed by the content hash in Standard
/// Ebooks' own URL.
fn fill_covers(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    client: &se::http::Client,
    view: &mut View,
    layout: grid::Layout,
    page: usize,
) -> anyhow::Result<()> {
    let covers_dir = cache::covers_dir(Path::new(BUNDLE_DIR));
    let start = page * layout.page_size();
    let end = (start + layout.page_size()).min(view.hits.len());

    for idx in start..end {
        if view.covers.get(idx).is_some_and(|c| c.is_some()) {
            continue;
        }
        let href = view.hits[idx].cover.clone();
        let name = href.cache_name();

        let bytes = match cover_cache::load(&covers_dir, &name) {
            Some(b) => b,
            None => match client.bytes(&Endpoint::Cover(href)) {
                Ok(b) => {
                    let _ = cover_cache::store(&covers_dir, &name, &b);
                    b
                }
                // A missing cover costs a placeholder, never the grid.
                Err(e) => {
                    log(format!("cover {name}: {e}"));
                    continue;
                }
            },
        };

        let Ok(img) = grid::decode_resize(&bytes) else {
            log(format!("cover {name}: decode failed"));
            continue;
        };
        view.covers[idx] = Some(img);

        let (cx, cy) = layout.cell_xy(idx - start);
        let rect = grid::draw_book_cell(
            fb,
            renderer,
            cx,
            cy,
            layout.cell_h,
            view.covers[idx].as_ref(),
            label_of(&view.hits[idx]),
        );
        if is_downloaded(view, idx) {
            grid::draw_downloaded_badge(fb, rect);
        }
        fb.send_update(
            MxcfbRect {
                top: cy.max(0) as u32,
                left: cx.max(0) as u32,
                width: grid::CELL_W,
                height: layout.cell_h,
            },
            WAVEFORM_MODE_DU,
        )?;
    }
    Ok(())
}

/// Download one book: its page for the `.azw3` href, the file itself, then the
/// Kindle thumbnail so it gets a real cover on the home screen.
fn download(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    client: &se::http::Client,
    hit: &Hit,
) -> anyhow::Result<String> {
    let (rect, _) = toast::draw_download(fb, renderer, &hit.title, "Fetching…");
    fb.send_update(rect, WAVEFORM_MODE_GC16)?;

    let html = match client.text(&Endpoint::Book(hit.path.clone())) {
        Ok(h) => h,
        Err(e) => return Ok(format!("Failed: {e}")),
    };
    let page = match se::book::parse(&html) {
        Ok(p) => p,
        Err(e) => return Ok(format!("Failed: {e}")),
    };

    let rect = toast::draw_progress(fb, renderer, &hit.title, 0, 1);
    fb.send_update(rect, WAVEFORM_MODE_DU)?;

    let bytes = match client.bytes(&Endpoint::Download(page.azw3.clone())) {
        Ok(b) => b,
        Err(e) => return Ok(format!("Failed: {e}")),
    };

    let file_name = page.azw3.file_name().to_string();
    match se::download::commit(Path::new(DOWNLOAD_DIR), &file_name, &bytes) {
        Ok(path) => log(format!("downloaded {}", path.display())),
        Err(e) => return Ok(format!("Failed: {e}")),
    }

    // Best-effort cover for the home screen. SE's filename already carries the
    // ASIN the framework expects, so it is written verbatim. A failure here
    // costs a grey placeholder in the library, never the book.
    if let Some(thumb) = page.thumbnail {
        match client.bytes(&Endpoint::Thumbnail(thumb.clone())) {
            Ok(b) => {
                let dest = Path::new(THUMBNAILS_DIR).join(thumb.file_name());
                if let Err(e) = std::fs::write(&dest, b) {
                    log(format!("thumbnail write {}: {e}", dest.display()));
                }
            }
            Err(e) => log(format!("thumbnail: {e}")),
        }
    }

    Ok(finish(&file_name))
}

fn finish(file_name: &str) -> String {
    // Without this the file sits on disk and never appears in the library.
    se::download::request_reindex();
    format!("Downloaded {file_name}")
}

fn main() {
    if let Err(e) = run() {
        log(format!("fatal: {e:#}"));
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    log(format!("steb {} starting", env!("CARGO_PKG_VERSION")));

    // ---- Device setup, before any network call -------------------------
    // The Diagnostics screen needs a surface and input to be usable, so every
    // piece of device state is up before the first request. The bezel grab in
    // particular must precede it, or a button press leaks to the framework and
    // repaints the home library over whatever we are showing.
    let mut renderer = TextRenderer::load(FONT_PX)?;
    log(format!("fonts: {}", renderer.chain_description()));

    let mut orient = orientation::Orientation::detect();
    let mut fb = Framebuffer::open()?;
    let touch = Touch::open(orient, fb.var.xres, fb.var.yres)?;
    let buttons = match Buttons::open() {
        Ok(Some(b)) => {
            log("buttons: grabbed gpio-keys");
            Some(b)
        }
        Ok(None) => {
            log("buttons: none — touch-only");
            None
        }
        Err(e) => {
            log(format!("buttons: {e:#} — touch-only"));
            None
        }
    };
    let mut input = Input::new(touch, buttons);
    input.set_orientation(orient);

    // ---- Cache + first fetch -------------------------------------------
    let cat_path = cache::catalogue_path(Path::new(BUNDLE_DIR));
    let mut catalogue = cache::load(&cat_path);
    log(format!("catalogue: {} books cached", catalogue.len()));

    let client = se::http::Client::new();
    refresh_from_feed(&client, &mut catalogue);

    let mut view = View {
        query: String::new(),
        filters: Filters::default(),
        sort: SortState::default(),
        hits: Vec::new(),
        covers: Vec::new(),
        fetched: 0,
        more: true,
        tags: Vec::new(),
        downloaded: se::download::existing_files(Path::new(DOWNLOAD_DIR)),
    };

    // Opens on bare /ebooks — the latest releases — so the first screen is a
    // full grid of covers with no user input at all. This is the request the
    // Diagnostics screen exists for, and the only one that blocks startup.
    if with_diag(&mut fb, &mut input, &mut renderer, || {
        fetch_next_page(&client, &mut view, &mut catalogue)
    })?
    .is_none()
    {
        return Ok(());
    }

    let _ = cache::store(&cat_path, &catalogue);

    // ---- Grid loop ------------------------------------------------------
    let mut layout = grid::Layout::compute(fb.var.xres, fb.var.yres, TOP_MARGIN, pager::STRIP_H);
    let mut page = 0usize;
    let mut total_pages = pager::n_pages(view.hits.len(), layout.page_size());

    draw_page(&mut fb, &mut renderer, &view, layout, page, total_pages);
    fb.send_update(full_rect(&fb), WAVEFORM_MODE_GC16)?;
    fill_covers(&mut fb, &mut renderer, &client, &mut view, layout, page)?;

    macro_rules! repaint {
        () => {{
            total_pages = pager::n_pages(view.hits.len(), layout.page_size()).max(1);
            page = page.min(total_pages.saturating_sub(1));
            draw_page(&mut fb, &mut renderer, &view, layout, page, total_pages);
            fb.send_update(full_rect(&fb), WAVEFORM_MODE_GC16)?;
            fill_covers(&mut fb, &mut renderer, &client, &mut view, layout, page)?;
        }};
    }

    /// Pull more results when the user reaches the end of what we have.
    macro_rules! ensure_page {
        ($p:expr) => {{
            let needed = ($p + 1) * layout.page_size();
            while view.hits.len() < needed && view.more {
                if let Err(e) = fetch_next_page(&client, &mut view, &mut catalogue) {
                    log(format!("paging fetch: {e}"));
                    break;
                }
            }
            let _ = cache::store(&cat_path, &catalogue);
        }};
    }

    // The cover under the finger, if any.
    let mut armed: Option<Armed> = None;

    loop {
        // While a cover is held, wake the loop at the arm instant so the cue can
        // flip and the download fire on its own. A `Tick` otherwise never
        // arrives mid-hold: finger micro-jitter keeps `poll` busy.
        let deadline = armed.as_ref().map(|a| a.down_at + ARM_THRESHOLD);
        match input.next_deadline(deadline)? {
            InputEvent::Touch(TouchEvent::Up { x, y }) => {
                // A hold long enough to act already fired from the `Tick` arm and
                // cleared this, so a cover still armed here was released early.
                if let Some(a) = armed.take() {
                    log(format!(
                        "short tap ({:?}), showing hint",
                        a.down_at.elapsed()
                    ));
                    let dirty = toast::draw(&mut fb, &mut renderer, "Hold cover to download");
                    fb.send_update(dirty, WAVEFORM_MODE_GC16)?;
                    std::thread::sleep(TOAST_LINGER);
                    // Clears both the toast and the outline the press left.
                    repaint!();
                    continue;
                }
                // Search bar first — it sits above the grid.
                if let Some(tap) = searchbar::hit(x, y, fb.var.xres, !view.query.is_empty()) {
                    match tap {
                        searchbar::Tap::Clear => {
                            view.query.clear();
                            view.clear_results();
                            ensure_page!(0);
                            page = 0;
                            repaint!();
                        }
                        searchbar::Tap::Open => {
                            let q = keyboard::run(
                                &mut fb,
                                &mut input,
                                &mut renderer,
                                &view.query,
                                &mut orient,
                            )?;
                            if q != view.query {
                                view.query = q;
                                view.clear_results();
                                ensure_page!(0);
                                page = 0;
                            }
                            repaint!();
                        }
                    }
                    continue;
                }

                // Pager strip.
                if let Some(hit) = pager::hit(x, y, fb.var.xres, fb.var.yres, total_pages) {
                    match hit {
                        pager::PagerHit::Exit => return Ok(()),
                        pager::PagerHit::Filter => {
                            let before = (view.filters.clone(), view.sort);
                            let tags = view.tags.clone();
                            filtermenu::run(
                                &mut fb,
                                &mut input,
                                &mut renderer,
                                &tags,
                                &mut view.filters,
                                &mut orient,
                            )?;
                            if (view.filters.clone(), view.sort) != before {
                                view.clear_results();
                                ensure_page!(0);
                                page = 0;
                            }
                            repaint!();
                        }
                        pager::PagerHit::Sort => {
                            let has_query = view.has_query();
                            filtermenu::run_sort(
                                &mut fb,
                                &mut input,
                                &mut renderer,
                                &mut view.sort,
                                has_query,
                                &mut orient,
                            )?;
                            view.clear_results();
                            ensure_page!(0);
                            page = 0;
                            repaint!();
                        }
                        pager::PagerHit::Next => {
                            ensure_page!(page + 1);
                            let pages = pager::n_pages(view.hits.len(), layout.page_size());
                            if page + 1 < pages {
                                page += 1;
                                repaint!();
                            }
                        }
                        pager::PagerHit::Prev if page > 0 => {
                            page -= 1;
                            repaint!();
                        }
                        _ => {}
                    }
                    continue;
                }

                // A lift on a cover does nothing: downloading is the hold, armed
                // on `Down` and fired from the arm deadline in `Tick`.
            }
            InputEvent::Touch(TouchEvent::Down { x, y }) => {
                // Outline the cover so the press is acknowledged immediately;
                // whether it downloads is decided by how long the finger stays.
                if searchbar::hit(x, y, fb.var.xres, !view.query.is_empty()).is_none()
                    && pager::hit(x, y, fb.var.xres, fb.var.yres, total_pages).is_none()
                    && let Some(slot) = layout.cell_at_tap(x, y, view.hits.len())
                {
                    let idx = page * layout.page_size() + slot;
                    let (cx, cy) = layout.cell_xy(slot);
                    if idx < view.hits.len() && cx >= 0 && cy >= 0 {
                        grid::outline_cell(&mut fb, cx, cy, layout.cell_h);
                        fb.send_update(cell_rect(cx, cy, layout.cell_h), WAVEFORM_MODE_DU)?;
                        armed = Some(Armed {
                            slot,
                            idx,
                            down_at: Instant::now(),
                            at: (x, y),
                        });
                    }
                }
            }
            InputEvent::Touch(TouchEvent::Screenshot) => {
                let _ = eink::screenshot::capture(&mut fb);
            }
            InputEvent::Page(dir) => match dir {
                PageButton::Next => {
                    ensure_page!(page + 1);
                    let pages = pager::n_pages(view.hits.len(), layout.page_size());
                    if page + 1 < pages {
                        page += 1;
                        repaint!();
                    }
                }
                PageButton::Prev if page > 0 => {
                    page -= 1;
                    repaint!();
                }
                _ => {}
            },
            InputEvent::Tick => {
                // Either the arm deadline came up while a cover was held, or it
                // is an ordinary idle poll and only the orientation needs a look.
                if armed
                    .as_ref()
                    .is_some_and(|a| a.down_at.elapsed() >= ARM_THRESHOLD)
                {
                    let a = armed.take().expect("checked above");
                    // A finger that wandered off its landing point is dragging,
                    // not holding — cancel and clear the outline.
                    let (px, py) = input.touch_pos();
                    if px.abs_diff(a.at.0) > ARM_SLOP_PX || py.abs_diff(a.at.1) > ARM_SLOP_PX {
                        log(format!(
                            "arm cancelled: drifted to ({px},{py}) from ({},{})",
                            a.at.0, a.at.1
                        ));
                        repaint!();
                        continue;
                    }
                    let Some(hit) = view.hits.get(a.idx).cloned() else {
                        repaint!();
                        continue;
                    };
                    // Flip the cover to the armed cue and let it show before the
                    // download overlay covers it, so the signal is actually seen.
                    let (cx, cy) = layout.cell_xy(a.slot);
                    if cx >= 0 && cy >= 0 {
                        grid::draw_arm_cue(&mut fb, cx, cy, layout.cell_h);
                        fb.send_update(cell_rect(cx, cy, layout.cell_h), WAVEFORM_MODE_DU)?;
                        std::thread::sleep(ARM_DWELL);
                    }
                    // Fire while the finger is still down. Its eventual lift is
                    // inert: `armed` is already taken, and a lift on the grid
                    // does nothing.
                    log(format!(
                        "arm fired ({:?}) on {}",
                        a.down_at.elapsed(),
                        hit.title
                    ));
                    let msg = download(&mut fb, &mut renderer, &client, &hit)?;
                    log(&msg);
                    let rect = toast::draw_download_done(&mut fb, &mut renderer, &msg);
                    fb.send_update(rect, WAVEFORM_MODE_GC16)?;
                    std::thread::sleep(Duration::from_millis(900));

                    view.downloaded = se::download::existing_files(Path::new(DOWNLOAD_DIR));
                    repaint!();
                    continue;
                }
                let o = orientation::Orientation::detect();
                if o != orient {
                    orient = o;
                    input.set_orientation(o);
                    layout =
                        grid::Layout::compute(fb.var.xres, fb.var.yres, TOP_MARGIN, pager::STRIP_H);
                    repaint!();
                }
            }
        }
    }
}
