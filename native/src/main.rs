//! Steb — search Standard Ebooks from the Kindle and download the `.azw3`. One
//! paginated cover grid; `keyboard`, `filtermenu` and `sortmenu` open as
//! blocking sub-loops. A held cover downloads, then `convert` writes a `.kfx`.

// `eink` and `ui` carry helpers the run loop leaves uncalled.
#![allow(dead_code)]

mod cache;
mod convert;
mod cover_cache;
mod eink;
mod font;
mod net;
mod orientation;
mod se;
mod ui;
mod wrap;

use std::path::Path;
use std::time::{Duration, Instant};

use eink::buttons::{Buttons, PageButton};
use eink::fb::{Framebuffer, MxcfbRect, WAVEFORM_MODE_DU, WAVEFORM_MODE_GC16};
use eink::input::{Input, InputEvent};
use eink::touch::{SwipeDir, Touch, TouchEvent, classify_swipe};
use image::DynamicImage;

use se::listing::Hit;
use se::url::{Endpoint, Listing};
use ui::filter::Filters;
use ui::sort::SortState;
use ui::text::TextRenderer;
use ui::{diag, filtermenu, grid, keyboard, pager, searchbar, toast};

/// Extension bundle root, holding `crate::cache`.
const BUNDLE_DIR: &str = "/mnt/us/extensions/steb";
/// Where books land, a subfolder of `documents/`.
const DOWNLOAD_DIR: &str = "/mnt/us/documents/standardebooks";
/// Kindle's own cover thumbnails, taking a `ThumbnailHref::file_name` verbatim.
const THUMBNAILS_DIR: &str = "/mnt/us/system/thumbnails";
/// Where `bin/steb.sh` funnels this process's stderr.
const LOG_PATH: &str = "/mnt/us/logs/steb.log";

const FONT_PX: f32 = 32.0;
/// Headroom above the grid for the search bar.
const TOP_MARGIN: u32 = searchbar::TOP + searchbar::HEIGHT + 16;

/// How long a cover must be held, within [`ARM_SLOP_PX`], before its download
/// arms and fires. A tap downloads nothing.
const ARM_THRESHOLD: Duration = Duration::from_millis(1000);
/// Max drift in user-visible px, either axis, across a hold.
const ARM_SLOP_PX: u32 = 40;
/// How long the armed cue holds the panel before the download overlay.
const ARM_DWELL: Duration = Duration::from_millis(250);
/// How long the hint after a too-short tap holds the panel.
const TOAST_LINGER: Duration = Duration::from_millis(1200);

/// Refresh rect for one grid cell.
fn cell_rect(cell_x: i32, cell_y: i32, cell_h: u32) -> MxcfbRect {
    MxcfbRect {
        top: cell_y.max(0) as u32,
        left: cell_x.max(0) as u32,
        width: grid::CELL_W,
        height: cell_h,
    }
}

/// A cover outlined under a finger. A release before [`ARM_THRESHOLD`] draws
/// the hint; a hold past it fires the download.
struct Armed {
    /// Slot on the current page.
    slot: usize,
    /// Index into `view.hits`.
    idx: usize,
    down_at: Instant,
    /// Where the finger landed, against [`ARM_SLOP_PX`].
    at: (u32, u32),
}

/// One line to stderr, which `bin/steb.sh` redirects into [`LOG_PATH`].
/// Never [`LOG_PATH`] directly: that doubles every line.
fn log(msg: impl AsRef<str>) {
    eprintln!("[{}] {}", now(), msg.as_ref());
}

fn now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "?".into())
}

/// What the grid draws. `hits` accumulates across SE pages; `pager` pages that
/// vector at `layout.page_size()`.
struct View {
    query: String,
    filters: Filters,
    sort: SortState,
    hits: Vec<Hit>,
    covers: Vec<Option<DynamicImage>>,
    /// SE pages pulled.
    fetched: u32,
    /// SE's own `rel="next"` control.
    more: bool,
    /// Subject vocabulary, as parsed from the last listing page.
    tags: Vec<String>,
    /// `se::download::existing_files` over [`DOWNLOAD_DIR`].
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

    /// Clears the fields a query, filter or sort change invalidates.
    /// `crate::cache` is keyed by book and untouched.
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

/// `attempt`, guarded by [`net::is_offline`], carrying the request's own error.
fn online<T>(attempt: impl FnOnce() -> se::http::Result<T>) -> se::http::Result<T> {
    if net::is_offline() {
        return Err(se::http::Error::Unreachable("Wi-Fi is off".into()));
    }
    attempt()
}

/// `attempt` under [`diag`], retried until it succeeds or Exit returns `Ok(None)`.
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

/// The next SE page into `view`, where `view.more` holds.
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

/// One conditional GET against [`Endpoint::Feed`], answering 304 with no body
/// against an unchanged catalogue.
fn refresh_from_feed(client: &se::http::Client, catalogue: &mut cache::Catalogue) {
    let known = catalogue.feed.clone();
    match online(|| client.text_if_modified(&Endpoint::Feed, &known)) {
        Ok(se::http::Fresh::Unchanged) => log("feed: 304, catalogue unchanged"),
        Ok(se::http::Fresh::Changed { body, validators }) => {
            let entries = se::feed::parse(&body);
            let state = catalogue.freshness(&entries);
            log(format!("feed: {} entries, {state:?}", entries.len()));
            catalogue.feed = validators;
        }
        // Non-fatal: `catalogue` holds what the last launch stored.
        Err(e) => log(format!("feed check failed (non-fatal): {e}")),
    }
}

fn label_of(hit: &Hit) -> grid::Label<'_> {
    grid::Label {
        text: &hit.title,
        script: font::Script::Unknown,
    }
}

/// One page of the grid, plus `searchbar` and the `pager` strip.
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
        // Zero hits under a query names the query and points at `keyboard`.
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
        // [`is_downloaded`] draws a corner check.
        if is_downloaded(view, idx) {
            grid::draw_downloaded_badge(fb, rect);
        }
    }

    pager::draw(fb, renderer, page, total_pages, view.filters.count());
}

/// Is this book in [`DOWNLOAD_DIR`]? Matched on the stem, which the extension
/// swap in `convert` leaves intact.
fn is_downloaded(view: &View, idx: usize) -> bool {
    let Some(hit) = view.hits.get(idx) else {
        return false;
    };
    let stem = hit.path.as_key().replace('/', "_");
    view.downloaded.iter().any(|f| f.starts_with(&stem))
}

/// Covers for the visible page, each cell refreshed as its image arrives.
/// `cover_cache` is keyed by the content hash in the cover URL.
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
    // Asked once per page: an uncached cover with no radio spends the
    // resolver's full timeout. A cached cover skips the network.
    let offline = net::is_offline();

    for idx in start..end {
        if view.covers.get(idx).is_some_and(|c| c.is_some()) {
            continue;
        }
        let href = view.hits[idx].cover.clone();
        let name = href.cache_name();

        let bytes = match cover_cache::load(&covers_dir, &name) {
            Some(b) => b,
            None if offline => continue,
            None => match client.bytes(&Endpoint::Cover(href)) {
                Ok(b) => {
                    let _ = cover_cache::store(&covers_dir, &name, &b);
                    b
                }
                // A missing cover leaves the placeholder.
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

/// One book: its page for the `.azw3` href, the file, the Kindle thumbnail,
/// then [`to_kfx`].
fn download(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    client: &se::http::Client,
    conv: Option<&convert::Converter>,
    hit: &Hit,
) -> anyhow::Result<String> {
    let (rect, _) = toast::draw_download(fb, renderer, &hit.title, "Fetching…");
    fb.send_update(rect, WAVEFORM_MODE_GC16)?;

    let html = match online(|| client.text(&Endpoint::Book(hit.path.clone()))) {
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
    let azw3 = match se::download::commit(Path::new(DOWNLOAD_DIR), &file_name, &bytes) {
        Ok(path) => {
            log(format!("downloaded {}", path.display()));
            path
        }
        Err(e) => return Ok(format!("Failed: {e}")),
    };

    // `ThumbnailHref::file_name` is written verbatim. A failure leaves the
    // grey placeholder.
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

    let file_name = match conv {
        Some(c) => to_kfx(fb, renderer, c, &hit.title, &azw3)?.unwrap_or(file_name),
        None => file_name,
    };

    Ok(finish(&file_name))
}

/// `conv.convert(azw3)` under a banner, returning the `.kfx` file name.
/// A [`convert::Error`] goes to [`log`] and returns `None`, leaving `azw3`.
fn to_kfx(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    conv: &convert::Converter,
    title: &str,
    azw3: &Path,
) -> anyhow::Result<Option<String>> {
    let rect = toast::draw_download_done(fb, renderer, &format!("{title}\nConverting to KFX…"));
    fb.send_update(rect, WAVEFORM_MODE_GC16)?;

    match conv.convert(azw3) {
        Ok(kfx) => {
            log(format!("converted {}", kfx.display()));
            Ok(kfx.file_name().map(|n| n.to_string_lossy().into_owned()))
        }
        Err(e) => {
            log(format!("convert {}: {e}", azw3.display()));
            Ok(None)
        }
    }
}

fn finish(file_name: &str) -> String {
    // [`se::download::request_reindex`] puts the file in the library.
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
    // [`diag`] needs a surface and input. The `Buttons` grab precedes both: an
    // ungrabbed press repaints the home library over this window.
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

    // `converter` is `None` where bokai is not installed at `convert::BIN_PATH`.
    let converter = convert::locate();
    match &converter {
        Some(c) => log(format!("converter: {}", c.exe().display())),
        None => log(format!("converter: none at {}", convert::BIN_PATH)),
    }

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

    // Bare `/ebooks`, the one request blocking startup.
    if with_diag(&mut fb, &mut input, &mut renderer, || {
        online(|| fetch_next_page(&client, &mut view, &mut catalogue))
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

    /// More results as `page` nears the end of `view.hits`.
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

    // Shared by the strip, the bezel buttons and a swipe.
    macro_rules! next_page {
        () => {{
            ensure_page!(page + 1);
            if page + 1 < pager::n_pages(view.hits.len(), layout.page_size()) {
                page += 1;
                repaint!();
            }
        }};
    }
    macro_rules! prev_page {
        () => {{
            if page > 0 {
                page -= 1;
                repaint!();
            }
        }};
    }

    /// One cell in place, past `repaint!` and its full-panel flash.
    macro_rules! redraw_cell {
        ($slot:expr) => {{
            let slot = $slot;
            let idx = page * layout.page_size() + slot;
            let (cx, cy) = layout.cell_xy(slot);
            if idx < view.hits.len() && cx >= 0 && cy >= 0 {
                let rect = grid::draw_book_cell(
                    &mut fb,
                    &mut renderer,
                    cx,
                    cy,
                    layout.cell_h,
                    view.covers.get(idx).and_then(|c| c.as_ref()),
                    label_of(&view.hits[idx]),
                );
                if is_downloaded(&view, idx) {
                    grid::draw_downloaded_badge(&mut fb, rect);
                }
                fb.send_update(cell_rect(cx, cy, layout.cell_h), WAVEFORM_MODE_DU)?;
            }
        }};
    }

    let mut armed: Option<Armed> = None;
    // Set on every `Down`, taken on every `Up`.
    let mut down_pos: Option<(u32, u32)> = None;

    loop {
        // A deadline at the arm instant. Finger micro-jitter keeps `poll` busy
        // through a hold, and no `Tick` arrives on its own.
        let deadline = armed.as_ref().map(|a| a.down_at + ARM_THRESHOLD);
        match input.next_deadline(deadline)? {
            InputEvent::Touch(TouchEvent::Up { x, y }) => {
                // A horizontal swipe flips the page, checked ahead of the press
                // the `Down` armed. `take()` ends this stroke either way.
                if let Some(dir) = down_pos
                    .take()
                    .and_then(|(x0, y0)| classify_swipe(x0, y0, x, y, fb.var.xres))
                {
                    // At a page boundary the turn is a no-op and the press
                    // outline wants clearing by hand.
                    let stale = armed.take().map(|a| a.slot);
                    log(format!("swipe {dir:?} on page {page}"));
                    let before = page;
                    match dir {
                        SwipeDir::Next => next_page!(),
                        SwipeDir::Prev => prev_page!(),
                    }
                    if page == before
                        && let Some(slot) = stale
                    {
                        redraw_cell!(slot);
                    }
                    continue;
                }

                // A `Tick` arm clears `armed`. A cover armed here was released
                // inside [`ARM_THRESHOLD`].
                if let Some(a) = armed.take() {
                    log(format!(
                        "short tap ({:?}), showing hint",
                        a.down_at.elapsed()
                    ));
                    let dirty = toast::draw(&mut fb, &mut renderer, "Hold cover to download");
                    fb.send_update(dirty, WAVEFORM_MODE_GC16)?;
                    std::thread::sleep(TOAST_LINGER);
                    // Clears the toast and the press outline together.
                    repaint!();
                    continue;
                }
                // `searchbar` sits above the grid.
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
                        pager::PagerHit::Next => next_page!(),
                        pager::PagerHit::Prev => prev_page!(),
                    }
                    continue;
                }

                // A lift on a cover does nothing: the download fires from `Tick`.
            }
            InputEvent::Touch(TouchEvent::Down { x, y }) => {
                // Every stroke's landing point, margins and strip included.
                down_pos = Some((x, y));
                // An outline under the press, ahead of [`ARM_THRESHOLD`].
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
                PageButton::Next => next_page!(),
                PageButton::Prev => prev_page!(),
            },
            InputEvent::Tick => {
                // The arm deadline, or an idle poll reaching the orientation check.
                if armed
                    .as_ref()
                    .is_some_and(|a| a.down_at.elapsed() >= ARM_THRESHOLD)
                {
                    let a = armed.take().expect("checked above");
                    // Drift past [`ARM_SLOP_PX`] is a drag. One cell repaints:
                    // its `Up` is often a page-turn swipe.
                    let (px, py) = input.touch_pos();
                    if px.abs_diff(a.at.0) > ARM_SLOP_PX || py.abs_diff(a.at.1) > ARM_SLOP_PX {
                        log(format!(
                            "arm cancelled: drifted to ({px},{py}) from ({},{})",
                            a.at.0, a.at.1
                        ));
                        redraw_cell!(a.slot);
                        continue;
                    }
                    let Some(hit) = view.hits.get(a.idx).cloned() else {
                        repaint!();
                        continue;
                    };
                    // The armed cue holds for [`ARM_DWELL`] under the overlay.
                    let (cx, cy) = layout.cell_xy(a.slot);
                    if cx >= 0 && cy >= 0 {
                        grid::draw_arm_cue(&mut fb, cx, cy, layout.cell_h);
                        fb.send_update(cell_rect(cx, cy, layout.cell_h), WAVEFORM_MODE_DU)?;
                        std::thread::sleep(ARM_DWELL);
                    }
                    // Fires under the finger. The lift finds `armed` taken.
                    log(format!(
                        "arm fired ({:?}) on {}",
                        a.down_at.elapsed(),
                        hit.title
                    ));
                    let msg = download(&mut fb, &mut renderer, &client, converter.as_ref(), &hit)?;
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
