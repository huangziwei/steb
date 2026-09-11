//! Every screen drawn to a PNG, with no Kindle behind it. `--list` names the
//! shots, `--panel` picks the panel sizes, one PNG per pair. `STEB_FONTS` must
//! point at a copy of the device's `/usr/java/lib/fonts`.

mod fixture;

use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use steb_native::eink::fb::Framebuffer;
use steb_native::font;
use steb_native::se::http::Error as HttpError;
use steb_native::ui::filter::Filters;
use steb_native::ui::scale::Scale;
use steb_native::ui::sort::SortState;
use steb_native::ui::text::TextRenderer;
use steb_native::ui::{diag, filtermenu, grid, keyboard, pager, searchbar, sortmenu, toast};

/// `main::FONT_PX`: the body size as a design pixel, which `ui::scale` maps to
/// each panel the way the app does.
const FONT_PX: f32 = 32.0;
/// `main::GRID_GAP`.
const GRID_GAP: u32 = 16;

/// Where the PNGs land under `--out`.
const OUT: &str = "artifacts/preview";

/// The panels, by the name `--panel` takes. Every width the X server reports
/// on a shipped device; the narrow two are where a layout runs out of room.
const PANELS: &[(&str, u32, u32)] = &[
    ("koa2", 1264, 1680),
    ("scribe", 1860, 2480),
    ("colorsoft", 1272, 1696),
    ("pw3", 1072, 1448),
    ("pw2", 758, 1024),
    ("basic", 600, 800),
];

/// What `--shot` takes, in the order `--list` prints them.
const SHOTS: &[&str] = &[
    "grid:full",
    "grid:one-page",
    "grid:no-covers",
    "grid:all-downloaded",
    "grid:armed",
    "toolbar:paged",
    "toolbar:one-page",
    "toolbar:filtered",
    "searchbar:empty",
    "searchbar:query",
    "keyboard:empty",
    "keyboard:typed",
    "filtermenu",
    "filtermenu:selected",
    "sortmenu",
    "sortmenu:with-query",
    "toast:hint",
    "toast:downloading",
    "toast:progress",
    "toast:done",
    "diag:offline",
];

/// `main::top_margin`.
fn top_margin(fb_xres: u32) -> u32 {
    searchbar::top(fb_xres) + searchbar::height(fb_xres) + Scale::of_width(fb_xres).px(GRID_GAP)
}

/// `main::layout_for`.
fn layout_for(fb: &Framebuffer) -> grid::Layout {
    grid::Layout::compute(
        fb.var.xres,
        fb.var.yres,
        top_margin(fb.var.xres),
        pager::strip_h(fb.var.xres),
    )
}

struct Args {
    shots: Vec<String>,
    panels: Vec<(String, u32, u32)>,
    out: PathBuf,
    list: bool,
}

fn parse_args() -> Result<Args> {
    let mut shots: Vec<String> = Vec::new();
    let mut panels: Vec<(String, u32, u32)> = Vec::new();
    let mut out = PathBuf::from(OUT);
    let mut list = false;
    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--list" => list = true,
            "--shot" => {
                let name = argv.next().context("--shot wants a name")?;
                if !SHOTS.contains(&name.as_str()) {
                    bail!("no shot named {name} — `--list` names them all");
                }
                shots.push(name);
            }
            "--panel" => {
                let name = argv.next().context("--panel wants a name")?;
                let found = PANELS
                    .iter()
                    .find(|(n, _, _)| *n == name)
                    .with_context(|| format!("no panel named {name}"))?;
                panels.push((found.0.to_string(), found.1, found.2));
            }
            "--out" => out = PathBuf::from(argv.next().context("--out wants a directory")?),
            other => bail!("unrecognized argument {other}"),
        }
    }
    if shots.is_empty() {
        shots = SHOTS.iter().map(|s| s.to_string()).collect();
    }
    if panels.is_empty() {
        panels = PANELS
            .iter()
            .map(|(n, w, h)| (n.to_string(), *w, *h))
            .collect();
    }
    Ok(Args {
        shots,
        panels,
        out,
        list,
    })
}

fn main() -> Result<()> {
    let args = parse_args()?;
    if args.list {
        for shot in SHOTS {
            println!("{shot}");
        }
        return Ok(());
    }
    // A host has none of the device's faces, and a chain of nothing draws
    // nothing. Say which variable fixes it rather than failing on `.notdef`.
    if std::env::var(font::FONT_DIR_ENV).is_err() && font::discover().is_empty() {
        bail!(
            "no fonts found — point {} at a copy of the Kindle's /usr/java/lib/fonts",
            font::FONT_DIR_ENV
        );
    }
    std::fs::create_dir_all(&args.out).with_context(|| format!("create {}", args.out.display()))?;

    for (panel, w, h) in &args.panels {
        let mut renderer = TextRenderer::load(Scale::of_width(*w).font(FONT_PX))?;
        for shot in &args.shots {
            let mut fb = Framebuffer::offscreen(*w, *h);
            draw(&mut fb, &mut renderer, shot)?;
            let file = args
                .out
                .join(format!("{}-{panel}.png", shot.replace(':', "-")));
            fb.capture_png(&file)?;
            println!("{}", file.display());
        }
    }
    Ok(())
}

/// One `shot` into `fb`'s backing. Nothing is presented: `send_update` has no
/// surface under an offscreen [`Framebuffer`].
fn draw(fb: &mut Framebuffer, renderer: &mut TextRenderer, shot: &str) -> Result<()> {
    let layout = layout_for(fb);
    let (xres, yres) = (fb.var.xres, fb.var.yres);
    fb.fill_rect(0, 0, xres, yres, 0xFF);

    match shot {
        "grid:full" => grid_screen(fb, renderer, layout, true, 0, 3),
        "grid:one-page" => {
            let n = layout.page_size();
            grid_page(fb, renderer, layout, &fixture::hits(n), true, 0, 0);
            searchbar::draw(fb, renderer, "");
            pager::draw(fb, renderer, 0, 1, 0);
        }
        "grid:no-covers" => grid_screen(fb, renderer, layout, false, 0, 0),
        "grid:all-downloaded" => grid_screen(fb, renderer, layout, true, 0, layout.page_size()),
        "grid:armed" => {
            grid_screen(fb, renderer, layout, true, 0, 0);
            let (cx, cy) = layout.cell_xy(layout.page_size() / 2);
            grid::draw_arm_cue(fb, layout, cx, cy);
        }
        "toolbar:paged" => pager::draw(fb, renderer, 2, 7, 0),
        "toolbar:one-page" => pager::draw(fb, renderer, 0, 1, 0),
        "toolbar:filtered" => pager::draw(fb, renderer, 0, 4, 3),
        "searchbar:empty" => searchbar::draw(fb, renderer, ""),
        "searchbar:query" => searchbar::draw(fb, renderer, "the time machine"),
        "keyboard:empty" => keyboard::render_screen(fb, renderer, ""),
        "keyboard:typed" => keyboard::render_screen(fb, renderer, "middlemarch"),
        "filtermenu" => {
            let tags = tags();
            filtermenu::render_screen(fb, renderer, &tags, &Filters::default(), 0);
        }
        "filtermenu:selected" => {
            let tags = tags();
            let mut filters = Filters::default();
            for tag in ["Adventure", "Gothic", "Poetry"] {
                filters.toggle(tag);
            }
            filtermenu::render_screen(fb, renderer, &tags, &filters, 0);
        }
        "sortmenu" => sortmenu::render_screen(fb, renderer, SortState::default(), false),
        "sortmenu:with-query" => sortmenu::render_screen(fb, renderer, SortState::default(), true),
        "toast:hint" => {
            grid_screen(fb, renderer, layout, true, 0, 0);
            toast::draw(fb, renderer, "Hold cover to download");
        }
        "toast:downloading" => {
            toast::draw_download(fb, renderer, "Middlemarch", "Fetching…");
        }
        "toast:progress" => {
            toast::draw_progress(fb, renderer, "Middlemarch", 3, 8);
        }
        "toast:done" => {
            toast::draw_download_done(fb, renderer, "Middlemarch\nSaved as middlemarch.kfx");
        }
        "diag:offline" => {
            diag::render_screen(fb, renderer, &HttpError::Unreachable("Wi-Fi is off".into()))?;
        }
        other => bail!("no shot named {other}"),
    }
    Ok(())
}

fn tags() -> Vec<String> {
    fixture::TAGS.iter().map(|t| t.to_string()).collect()
}

/// The whole grid view: search bar, a page of cells, the toolbar.
fn grid_screen(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    layout: grid::Layout,
    covers: bool,
    page: usize,
    downloaded: usize,
) {
    // Three pages' worth, so the toolbar has somewhere to page to.
    let hits = fixture::hits(layout.page_size() * 3);
    grid_page(fb, renderer, layout, &hits, covers, page, downloaded);
    searchbar::draw(fb, renderer, "");
    pager::draw(fb, renderer, page, 3, 0);
}

/// One page of cells. `downloaded` cells carry the corner check.
fn grid_page(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    layout: grid::Layout,
    hits: &[steb_native::se::listing::Hit],
    covers: bool,
    page: usize,
    downloaded: usize,
) {
    let start = page * layout.page_size();
    let end = (start + layout.page_size()).min(hits.len());
    for (slot, hit) in hits[start..end].iter().enumerate() {
        let (cx, cy) = layout.cell_xy(slot);
        let art = covers.then(|| fixture::cover(start + slot, layout.cell_w, layout.cell_h));
        let rect = grid::draw_book_cell(
            fb,
            renderer,
            layout,
            cx,
            cy,
            art.as_ref(),
            grid::Label {
                text: &hit.title,
                script: font::Script::Unknown,
            },
        );
        if slot < downloaded {
            grid::draw_downloaded_badge(fb, layout.scale, rect);
        }
    }
}
