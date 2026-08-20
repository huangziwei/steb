//! Modal status overlay: a black banner centered on the panel, white text.
//! A `message` lays out one row per `\n`-delimited line. Every function here
//! returns the dirty rect.

use crate::eink::fb::{Framebuffer, MxcfbRect};
use crate::ui::text::TextRenderer;

const BANNER_HEIGHT: u32 = 140;
const BANNER_MARGIN_X: u32 = 80;
/// Padding above and below the text block, past [`BANNER_HEIGHT`].
const BANNER_PAD_Y: u32 = 20;

/// [`draw_download`]: a title, a progress line, and the Cancel button.
const DL_BANNER_HEIGHT: u32 = 300;

/// [`draw_progress`]: a title, an `n / total` count, and the bar.
const PROGRESS_BANNER_HEIGHT: u32 = 260;
/// Horizontal inset of the progress bar from the banner's side edges.
const PROGRESS_BAR_INSET: u32 = 60;
/// Progress-bar track height.
const PROGRESS_BAR_H: u32 = 44;
/// Cancel button footprint, a finger target on a ~300 DPI panel.
const CANCEL_W: u32 = 320;
const CANCEL_H: u32 = 84;

pub fn draw(fb: &mut Framebuffer, renderer: &mut TextRenderer, message: &str) -> MxcfbRect {
    // [`BANNER_HEIGHT`] as a floor, taller for a `message` past it.
    let banner_h = BANNER_HEIGHT.max(block_height(renderer, message) + BANNER_PAD_Y * 2);
    let banner_w = fb.var.xres.saturating_sub(BANNER_MARGIN_X * 2);
    let banner_x = (fb.var.xres - banner_w) / 2;
    let banner_y = fb.var.yres.saturating_sub(banner_h) / 2;

    fb.fill_rect(banner_y, banner_x, banner_w, banner_h, 0x00);
    draw_message_block(
        fb, renderer, banner_x, banner_y, banner_w, banner_h, message,
    );

    MxcfbRect {
        top: banner_y,
        left: banner_x,
        width: banner_w,
        height: banner_h,
    }
}

/// `message` at one row per line, a minimum of one row.
fn block_height(renderer: &TextRenderer, message: &str) -> u32 {
    renderer.line_height() * message.lines().count().max(1) as u32
}

/// `message` centered as a block in the banner, each row centered on its own
/// width, white-on-black.
fn draw_message_block(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    banner_x: u32,
    banner_y: u32,
    banner_w: u32,
    banner_h: u32,
    message: &str,
) {
    let lh = renderer.line_height();
    let block_top = banner_y + banner_h.saturating_sub(block_height(renderer, message)) / 2;
    for (i, line) in message.lines().enumerate() {
        let text_w = renderer.measure_width(line);
        let text_x = banner_x as i32 + ((banner_w as i32 - text_w as i32) / 2).max(0);
        // Baseline 72% down the slot, clearing ascenders and descenders.
        let baseline = (block_top + lh * i as u32 + lh * 72 / 100) as i32;
        renderer.draw(fb, text_x, baseline, line, true);
    }
}

/// A `title` line, a `progress` line, and a white Cancel button.
/// Returns the banner's dirty rect and the Cancel button's absolute hit rect.
pub fn draw_download(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    title: &str,
    progress: &str,
) -> (MxcfbRect, MxcfbRect) {
    let banner_w = fb.var.xres.saturating_sub(BANNER_MARGIN_X * 2);
    let banner_x = (fb.var.xres - banner_w) / 2;
    let banner_y = (fb.var.yres.saturating_sub(DL_BANNER_HEIGHT)) / 2;

    fb.fill_rect(banner_y, banner_x, banner_w, DL_BANNER_HEIGHT, 0x00);

    // Title + progress, white-on-black, stacked in the upper half.
    let centered = |renderer: &mut TextRenderer, s: &str| -> i32 {
        let w = renderer.measure_width(s);
        banner_x as i32 + ((banner_w as i32 - w as i32) / 2).max(0)
    };
    let tx = centered(renderer, title);
    renderer.draw(fb, tx, (banner_y + 74) as i32, title, true);
    let px = centered(renderer, progress);
    renderer.draw(fb, px, (banner_y + 150) as i32, progress, true);

    // Cancel button: filled white box with black label, near the bottom.
    let cancel_x = banner_x + (banner_w.saturating_sub(CANCEL_W)) / 2;
    let cancel_y = banner_y + DL_BANNER_HEIGHT - CANCEL_H - 34;
    fb.fill_rect(cancel_y, cancel_x, CANCEL_W, CANCEL_H, 0xFF);
    let label = "Cancel";
    let lw = renderer.measure_width(label);
    let lx = cancel_x as i32 + ((CANCEL_W as i32 - lw as i32) / 2).max(0);
    let lbaseline = (cancel_y + CANCEL_H * 66 / 100) as i32;
    renderer.draw(fb, lx, lbaseline, label, false);

    let banner_rect = MxcfbRect {
        top: banner_y,
        left: banner_x,
        width: banner_w,
        height: DL_BANNER_HEIGHT,
    };
    let cancel_rect = MxcfbRect {
        top: cancel_y,
        left: cancel_x,
        width: CANCEL_W,
        height: CANCEL_H,
    };
    (banner_rect, cancel_rect)
}

/// `message` in [`draw_download`]'s footprint — same width, height and
/// position — with no Cancel button.
pub fn draw_download_done(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    message: &str,
) -> MxcfbRect {
    let banner_w = fb.var.xres.saturating_sub(BANNER_MARGIN_X * 2);
    let banner_x = (fb.var.xres - banner_w) / 2;
    let banner_y = (fb.var.yres.saturating_sub(DL_BANNER_HEIGHT)) / 2;

    fb.fill_rect(banner_y, banner_x, banner_w, DL_BANNER_HEIGHT, 0x00);
    draw_message_block(
        fb,
        renderer,
        banner_x,
        banner_y,
        banner_w,
        DL_BANNER_HEIGHT,
        message,
    );

    MxcfbRect {
        top: banner_y,
        left: banner_x,
        width: banner_w,
        height: DL_BANNER_HEIGHT,
    }
}

/// A `title` line, an `n / total` count, and a bar filled to `done / total`.
/// `total == 0` draws an empty track.
pub fn draw_progress(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    title: &str,
    done: usize,
    total: usize,
) -> MxcfbRect {
    let banner_w = fb.var.xres.saturating_sub(BANNER_MARGIN_X * 2);
    let banner_x = (fb.var.xres - banner_w) / 2;
    let banner_y = (fb.var.yres.saturating_sub(PROGRESS_BANNER_HEIGHT)) / 2;

    fb.fill_rect(banner_y, banner_x, banner_w, PROGRESS_BANNER_HEIGHT, 0x00);

    let centered = |renderer: &mut TextRenderer, s: &str| -> i32 {
        let w = renderer.measure_width(s);
        banner_x as i32 + ((banner_w as i32 - w as i32) / 2).max(0)
    };

    // Title + count, white-on-black, stacked in the upper half.
    let tx = centered(renderer, title);
    renderer.draw(fb, tx, (banner_y + 72) as i32, title, true);
    let count = format!("{done} / {total}");
    let cx = centered(renderer, &count);
    renderer.draw(fb, cx, (banner_y + 140) as i32, &count, true);

    // Progress track: a white outline, filled white to `done / total`.
    let bar_x = banner_x + PROGRESS_BAR_INSET;
    let bar_w = banner_w.saturating_sub(PROGRESS_BAR_INSET * 2);
    let bar_y = banner_y + PROGRESS_BANNER_HEIGHT - PROGRESS_BAR_H - 40;
    const T: u32 = 3;
    fb.fill_rect(bar_y, bar_x, bar_w, T, 0xFF); // top
    fb.fill_rect(bar_y + PROGRESS_BAR_H - T, bar_x, bar_w, T, 0xFF); // bottom
    fb.fill_rect(bar_y, bar_x, T, PROGRESS_BAR_H, 0xFF); // left
    fb.fill_rect(bar_y, bar_x + bar_w - T, T, PROGRESS_BAR_H, 0xFF); // right
    if total > 0 {
        let inner_w = bar_w.saturating_sub(T * 2);
        // u64 math: inner_w·done can overflow u32 on a wide panel / many books.
        let fill_w = (inner_w as u64 * done as u64 / total as u64) as u32;
        if fill_w > 0 {
            fb.fill_rect(bar_y + T, bar_x + T, fill_w, PROGRESS_BAR_H - T * 2, 0xFF);
        }
    }

    MxcfbRect {
        top: banner_y,
        left: banner_x,
        width: banner_w,
        height: PROGRESS_BANNER_HEIGHT,
    }
}
