//! Cover grid: [`Layout`], cell hit-test, image blit. Each cell is [`CELL_W`]
//! wide and a cover aspect-fits under `image`'s bilinear `Triangle` filter. A
//! missing cover draws a placeholder rect carrying the title.

use anyhow::Result;
use image::{DynamicImage, ImageReader, imageops::FilterType};
use std::io::Cursor;

use crate::eink::fb::Framebuffer;
use crate::font::Script;
use crate::ui::text::TextRenderer;

/// The text in a tile's name band and the convention it is set in. The pair
/// travels together: a title without its language draws Chinese in Japanese
/// shapes ([`crate::font`]).
#[derive(Clone, Copy)]
pub struct Label<'a> {
    pub text: &'a str,
    pub script: Script,
}

/// Cell width, fixed. The fleet runs ~300 ppi (KOA2 2102px/7", Scribe
/// 3100px/10.2"), making a pixel size a physical size across it. [`Layout`]
/// adapts the row and column count.
pub const CELL_W: u32 = 360;
/// Tallest a cell gets, which is the height the 7" devices settle at.
pub const CELL_H_MAX: u32 = 440;
/// Shortest a cell gets. ~3% of cover height buys a whole extra row on a tall
/// panel.
pub const CELL_H_MIN: u32 = 420;
pub const COL_GAP: u32 = 32;
pub const ROW_GAP: u32 = 20;

/// The grid as it fits *this* panel: how many cells, how tall, and where the
/// block sits. Computed once at startup from the framebuffer geometry.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub cols: usize,
    pub rows: usize,
    /// Actual cell height, between [`CELL_H_MIN`] and [`CELL_H_MAX`].
    pub cell_h: u32,
    /// Origin of the cell block, centred horizontally.
    pub left: i32,
    pub top: i32,
}

impl Layout {
    /// Rows fitted at [`CELL_H_MIN`], then given the spare height back up to
    /// [`CELL_H_MAX`]. A plain divide by the maximum loses a row on the Scribe:
    /// 2210px is four 440px rows with 390px stranded, against five at 426px.
    pub fn compute(fb_xres: u32, fb_yres: u32, top_margin: u32, strip_h: u32) -> Self {
        let cols = ((fb_xres + COL_GAP) / (CELL_W + COL_GAP)).max(1) as usize;
        let avail = fb_yres.saturating_sub(top_margin + strip_h);
        let rows = ((avail + ROW_GAP) / (CELL_H_MIN + ROW_GAP)).max(1) as usize;
        let cell_h = (avail.saturating_sub((rows as u32 - 1) * ROW_GAP) / rows as u32)
            .clamp(CELL_H_MIN, CELL_H_MAX);

        let grid_w = cols as u32 * CELL_W + (cols as u32 - 1) * COL_GAP;

        // Centred between the search bar and the pager strip. `cell_h` clamps
        // to [`CELL_H_MAX`], leaving ~136px of slack on a 1696px Colorsoft.
        let content_h = rows as u32 * cell_h + (rows as u32 - 1) * ROW_GAP;
        let slack = avail.saturating_sub(content_h);

        Self {
            cols,
            rows,
            cell_h,
            left: ((fb_xres as i32) - grid_w as i32) / 2,
            top: (top_margin + slack / 2) as i32,
        }
    }

    /// Cells per page.
    pub fn page_size(&self) -> usize {
        self.cols * self.rows
    }

    /// Screen origin of the `idx`-th cell on the current page.
    pub fn cell_xy(&self, idx: usize) -> (i32, i32) {
        let col = idx % self.cols;
        let row = idx / self.cols;
        (
            self.left + col as i32 * (CELL_W + COL_GAP) as i32,
            self.top + row as i32 * (self.cell_h + ROW_GAP) as i32,
        )
    }

    /// Which cell a tap landed on, or `None` for the gaps and the margins.
    pub fn cell_at_tap(&self, tx: u32, ty: u32, n_books: usize) -> Option<usize> {
        if (tx as i32) < self.left || (ty as i32) < self.top {
            return None;
        }
        let local_x = (tx as i32 - self.left) as u32;
        let local_y = (ty as i32 - self.top) as u32;
        let stride_x = CELL_W + COL_GAP;
        let stride_y = self.cell_h + ROW_GAP;
        let col = (local_x / stride_x) as usize;
        let row = (local_y / stride_y) as usize;
        if col >= self.cols || row >= self.rows {
            return None;
        }
        // A tap in the gap between two cells resolves to neither.
        if local_x % stride_x >= CELL_W || local_y % stride_y >= self.cell_h {
            return None;
        }
        let idx = row * self.cols + col;
        if idx < n_books { Some(idx) } else { None }
    }
}

// ---- Series-collection tile geometry (see `draw_series_cell`) ----
/// Bottom band of a series tile, reserved for the series name (book covers
/// don't draw titles, but a collection must — its art is just the lead cover).
pub const NAME_BAND_H: u32 = 64;
/// Top strip above the lead cover, holding the two stacked book-edge bars.
const BAR_STRIP_H: u32 = 22;
/// Thickness of each book-edge bar.
const BAR_H: u32 = 6;
/// Inset of the count badge from the lead cover's bottom-left corner.
const BADGE_MARGIN: u32 = 8;
/// Padding inside the count badge around its number.
const BADGE_PAD: u32 = 12;

// ---- Downloaded marker (see `draw_downloaded_badge`) ----
/// Diameter of [`draw_downloaded_badge`]'s check disc.
const CHECK_D: i32 = 44;
/// Inset of that disc from the cover's top-right corner.
const CHECK_MARGIN: i32 = 10;
/// Shade of the disc, a step lighter than the solid-black chrome: the count
/// badge and the arm cue.
const CHECK_SHADE: u8 = 0x55;

/// Decode a JPEG/PNG byte buffer and resize to fit inside `CELL_W × CELL_H_MAX`,
/// preserving aspect. Returns the resized image in its source color (the cover
/// thumbnail is a color JPEG; [`blit_fit`] samples its RGB).
pub fn decode_resize(bytes: &[u8]) -> Result<DynamicImage> {
    let img = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()?
        .decode()?;
    Ok(img.resize(CELL_W, CELL_H_MAX, FilterType::Triangle))
}

/// The aspect-fit placement of an `iw × ih` image inside the box, the rect
/// [`blit_fit`] paints. `scale` clamps at 1.0. Returns `(ox, oy, dw, dh)`, which
/// a caller pins chrome to.
pub fn fit_rect(
    box_x: i32,
    box_y: i32,
    box_w: u32,
    box_h: u32,
    iw: u32,
    ih: u32,
) -> (i32, i32, u32, u32) {
    if iw == 0 || ih == 0 || box_w == 0 || box_h == 0 {
        return (box_x, box_y, 0, 0);
    }
    let scale = (box_w as f32 / iw as f32)
        .min(box_h as f32 / ih as f32)
        .min(1.0);
    let dw = ((iw as f32 * scale).round() as u32).max(1);
    let dh = ((ih as f32 * scale).round() as u32).max(1);
    let ox = box_x + (box_w as i32 - dw as i32) / 2;
    let oy = box_y + (box_h as i32 - dh as i32) / 2;
    (ox, oy, dw, dh)
}

/// `img` aspect-fit into `(box_x, box_y, box_w × box_h)` by [`fit_rect`],
/// returning the painted rect. A [`decode_resize`] cover copies 1:1; a smaller
/// box takes a nearest-neighbor downscale.
pub fn blit_fit(
    fb: &mut Framebuffer,
    box_x: i32,
    box_y: i32,
    box_w: u32,
    box_h: u32,
    img: &DynamicImage,
) -> (i32, i32, u32, u32) {
    let rgb = img.to_rgb8();
    let (iw, ih) = (rgb.width(), rgb.height());
    let rect = fit_rect(box_x, box_y, box_w, box_h, iw, ih);
    let (ox, oy, dw, dh) = rect;
    if dw == 0 || dh == 0 {
        return rect;
    }
    let scale = dw as f32 / iw as f32;
    let raw = rgb.as_raw();
    for dy in 0..dh {
        let sy = ((dy as f32 / scale) as u32).min(ih - 1);
        let src_row = (sy * iw) as usize;
        for dx in 0..dw {
            let sx = ((dx as f32 / scale) as u32).min(iw - 1);
            let p = (src_row + sx as usize) * 3;
            fb.put_pixel_rgb(
                ox + dx as i32,
                oy + dy as i32,
                [raw[p], raw[p + 1], raw[p + 2]],
            );
        }
    }
    rect
}

/// A 6px black border framing the pressed cell. Transient, and the loudest
/// thing the grid draws; [`draw_downloaded_badge`] carries standing state.
pub fn outline_cell(fb: &mut Framebuffer, cell_x: i32, cell_y: i32, cell_h: u32) {
    outline_rect(fb, cell_x, cell_y, CELL_W, cell_h, 6, 0x00);
}

/// A gray check disc top-right of `cover`, [`draw_book_cell`]'s painted rect,
/// marking a book in [`crate::DOWNLOAD_DIR`]. SE covers carry a title plate
/// along the bottom edge. A zero-size rect no-ops.
pub fn draw_downloaded_badge(fb: &mut Framebuffer, cover: (i32, i32, u32, u32)) {
    let (ox, oy, w, h) = cover;
    if w == 0 || h == 0 {
        return;
    }
    let r = CHECK_D / 2;
    let cx = ox + w as i32 - CHECK_MARGIN - r;
    let cy = oy + CHECK_MARGIN + r;
    fill_disc(fb, cx, cy, r, CHECK_SHADE);
    draw_check_glyph(fb, cx, cy, r, 0xFF);
}

/// Fill the disc of radius `r` centered at `(cx, cy)` in `shade`. Out-of-range
/// pixels no-op in [`Framebuffer::put_pixel`], so a disc may straddle an edge.
fn fill_disc(fb: &mut Framebuffer, cx: i32, cy: i32, r: i32, shade: u8) {
    let rr = r * r;
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy <= rr {
                fb.put_pixel(cx + dx, cy + dy, shade);
            }
        }
    }
}

/// A **check** glyph — short down-stroke, long up-stroke — centered at
/// `(cx, cy)` inside a disc of radius `r`. One distance test over the glyph's
/// box joins the elbow cleanly.
fn draw_check_glyph(fb: &mut Framebuffer, cx: i32, cy: i32, r: i32, shade: u8) {
    let (fx, fy, s) = (cx as f32, cy as f32, r as f32);
    let half = (s * 0.15).max(1.0); // half stroke thickness
    // Elbow low and slightly left of center, so the long arm has room to rise.
    let elbow = (fx - s * 0.12, fy + s * 0.40);
    let short = (fx - s * 0.58, fy - s * 0.02);
    let long = (fx + s * 0.58, fy - s * 0.44);
    for y in (cy - r)..=(cy + r) {
        for x in (cx - r)..=(cx + r) {
            let p = (x as f32 + 0.5, y as f32 + 0.5);
            if dist_to_seg(p, elbow, short).min(dist_to_seg(p, elbow, long)) <= half {
                fb.put_pixel(x, y, shade);
            }
        }
    }
}

/// Distance from `p` to the segment `a`–`b` (float screen coords). A degenerate
/// segment collapses to the distance from `a`.
fn dist_to_seg(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let (wx, wy) = (p.0 - a.0, p.1 - a.1);
    let len2 = vx * vx + vy * vy;
    let t = if len2 <= f32::EPSILON {
        0.0
    } else {
        ((wx * vx + wy * vy) / len2).clamp(0.0, 1.0)
    };
    let (dx, dy) = (wx - t * vx, wy - t * vy);
    (dx * dx + dy * dy).sqrt()
}

/// The armed cue past `crate::ARM_THRESHOLD`: a dark badge with a light
/// download glyph over the cover, drawn on [`outline_cell`] in one refresh.
/// `(cell_x, cell_y)` is [`Layout::cell_xy`]'s origin; off-screen no-ops.
pub fn draw_arm_cue(fb: &mut Framebuffer, cell_x: i32, cell_y: i32, cell_h: u32) {
    if cell_x < 0 || cell_y < 0 {
        return;
    }
    // Center the badge on the cover region (the cell minus the bottom name band).
    let cover_h = cell_h - NAME_BAND_H;
    let cx = cell_x + CELL_W as i32 / 2;
    let cy = cell_y + cover_h as i32 / 2;
    const BADGE: u32 = 140;
    let half = BADGE as i32 / 2;
    fb.fill_rect(
        (cy - half).max(cell_y) as u32,
        (cx - half).max(cell_x) as u32,
        BADGE,
        BADGE,
        0x00,
    );
    draw_download_glyph(fb, cx, cy, BADGE as i32 / 4, 0xFF);
}

/// Draw a `thickness`-px outline rectangle (the four edges of `w × h` at
/// `(x, y)`) in `shade`. Used by [`outline_cell`] for the press frame.
/// Negative origin / zero size no-ops.
pub fn outline_rect(
    fb: &mut Framebuffer,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    thickness: u32,
    shade: u8,
) {
    if x < 0 || y < 0 || w == 0 || h == 0 {
        return;
    }
    let (xu, yu) = (x as u32, y as u32);
    let t = thickness.min(w).min(h);
    fb.fill_rect(yu, xu, w, t, shade); // top
    fb.fill_rect(yu + h - t, xu, w, t, shade); // bottom
    fb.fill_rect(yu, xu, t, h, shade); // left
    fb.fill_rect(yu, xu + w - t, t, h, shade); // right
}

/// A **rounded-rectangle** border (`thickness` px, corner `radius`) in `shade`.
/// `radius == h/2` gives a full pill. The straight edges are `fill_rect`s
/// between four quarter-circle corner arcs.
#[allow(clippy::too_many_arguments)] // positional geometry; a struct just moves the list
pub fn stroke_round_rect(
    fb: &mut Framebuffer,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    radius: u32,
    thickness: u32,
    shade: u8,
) {
    if x < 0 || y < 0 || w == 0 || h == 0 {
        return;
    }
    let (xu, yu) = (x as u32, y as u32);
    let r = radius.min(w / 2).min(h / 2);
    let t = thickness.min(w).min(h).max(1);
    let mid_w = w.saturating_sub(2 * r);
    let mid_h = h.saturating_sub(2 * r);
    if mid_w > 0 {
        fb.fill_rect(yu, xu + r, mid_w, t, shade); // top
        fb.fill_rect(yu + h - t, xu + r, mid_w, t, shade); // bottom
    }
    if mid_h > 0 {
        fb.fill_rect(yu + r, xu, t, mid_h, shade); // left
        fb.fill_rect(yu + r, xu + w - t, t, mid_h, shade); // right
    }
    let (cl, cr) = (x + r as i32, x + w as i32 - 1 - r as i32);
    let (ct, cb) = (y + r as i32, y + h as i32 - 1 - r as i32);
    corner_arc(fb, cl, ct, r, t, shade, -1, -1);
    corner_arc(fb, cr, ct, r, t, shade, 1, -1);
    corner_arc(fb, cl, cb, r, t, shade, -1, 1);
    corner_arc(fb, cr, cb, r, t, shade, 1, 1);
}

/// One quarter-circle arc: pixels in the `r×r` corner box at distance `[r-t, r]`
/// from `(cx, cy)`. `(sx, sy) ∈ {-1, 1}` selects the quadrant.
#[allow(clippy::too_many_arguments)] // positional geometry; a struct just moves the list
fn corner_arc(fb: &mut Framebuffer, cx: i32, cy: i32, r: u32, t: u32, shade: u8, sx: i32, sy: i32) {
    let rf = r as f32;
    let inner = r.saturating_sub(t) as f32;
    for dy in 0..=r as i32 {
        for dx in 0..=r as i32 {
            let dist = ((dx * dx + dy * dy) as f32).sqrt();
            if dist >= inner && dist <= rf {
                fb.put_pixel(cx + sx * dx, cy + sy * dy, shade);
            }
        }
    }
}

/// Draw a magnifier glyph (a ring + a lower-right diagonal handle) centered at
/// `(cx, cy)` with lens radius `r`. No face in the chain carries 🔍.
pub fn draw_magnifier(fb: &mut Framebuffer, cx: i32, cy: i32, r: u32, shade: u8) {
    const T: u32 = 3;
    let rf = r as f32;
    let inner = r.saturating_sub(T) as f32;
    for dy in -(r as i32)..=r as i32 {
        for dx in -(r as i32)..=r as i32 {
            let dist = ((dx * dx + dy * dy) as f32).sqrt();
            if dist >= inner && dist <= rf {
                fb.put_pixel(cx + dx, cy + dy, shade);
            }
        }
    }
    // Handle: a short thick diagonal off the lens's lower-right.
    let start = rf * 0.78;
    let len = (rf * 0.95) as i32;
    for i in 0..len {
        let d = start + i as f32;
        let (hx, hy) = (cx + (d * 0.707) as i32, cy + (d * 0.707) as i32);
        for by in 0..T as i32 {
            for bx in 0..T as i32 {
                fb.put_pixel(hx + bx, hy + by, shade);
            }
        }
    }
}

/// Draw an `✕` (two diagonals) centered at `(cx, cy)`, half-extent `size` — the
/// search field's clear button.
pub fn draw_x(fb: &mut Framebuffer, cx: i32, cy: i32, size: i32, shade: u8) {
    for i in -size..=size {
        for k in 0..2 {
            fb.put_pixel(cx + i, cy + i + k, shade);
            fb.put_pixel(cx + i, cy - i + k, shade);
        }
    }
}

/// A **sync** glyph — two arced arrows round a circle — centered at `(cx, cy)`
/// with ring radius `r`, each arc gap capped by a tangential arrowhead. No face
/// in the chain carries 🔄.
pub fn draw_sync_glyph(fb: &mut Framebuffer, cx: i32, cy: i32, r: i32, shade: u8) {
    const T: i32 = 5;
    let rf = r as f32;
    let inner = (r - T).max(1) as f32;
    // Ring pixels minus two gaps (screen coords, +y down): near 10° and near
    // 190°, leaving two arcs spanning roughly [38°,162°] and [218°,342°].
    for dy in -r..=r {
        for dx in -r..=r {
            let dist = ((dx * dx + dy * dy) as f32).sqrt();
            if dist < inner || dist > rf {
                continue;
            }
            let mut a = (dy as f32).atan2(dx as f32).to_degrees();
            if a < 0.0 {
                a += 360.0;
            }
            if !(38.0..=162.0).contains(&a) && !(218.0..=342.0).contains(&a) {
                continue;
            }
            fb.put_pixel(cx + dx, cy + dy, shade);
        }
    }
    // An arrowhead on each arc's low end, along the clockwise tangent: the
    // pair reads as rotation.
    sync_arrowhead(fb, cx, cy, rf, 38.0, shade);
    sync_arrowhead(fb, cx, cy, rf, 218.0, shade);
}

/// Small filled triangle capping a [`draw_sync_glyph`] arc at angle `deg`,
/// pointing along the clockwise tangent (screen `+y` down).
fn sync_arrowhead(fb: &mut Framebuffer, cx: i32, cy: i32, r: f32, deg: f32, shade: u8) {
    let a = deg.to_radians();
    let (c, s) = (a.cos(), a.sin());
    let (px, py) = (cx as f32 + r * c, cy as f32 + r * s);
    let (tx, ty) = (s, -c); // clockwise tangent
    let tip = (px + tx * 13.0, py + ty * 13.0);
    let b1 = (px + c * 9.0, py + s * 9.0);
    let b2 = (px - c * 9.0, py - s * 9.0);
    fill_tri(fb, tip, b1, b2, shade);
}

/// Fill the triangle `(a, b, c)` (float screen coords) via a barycentric test
/// over its bounding box. Small glyph triangles only — not a general rasterizer.
fn fill_tri(fb: &mut Framebuffer, a: (f32, f32), b: (f32, f32), c: (f32, f32), shade: u8) {
    let area = (b.0 - a.0) * (c.1 - a.1) - (c.0 - a.0) * (b.1 - a.1);
    if area.abs() < 1e-3 {
        return;
    }
    let minx = a.0.min(b.0).min(c.0).floor() as i32;
    let maxx = a.0.max(b.0).max(c.0).ceil() as i32;
    let miny = a.1.min(b.1).min(c.1).floor() as i32;
    let maxy = a.1.max(b.1).max(c.1).ceil() as i32;
    for y in miny..=maxy {
        for x in minx..=maxx {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = ((b.0 - px) * (c.1 - py) - (c.0 - px) * (b.1 - py)) / area;
            let w1 = ((c.0 - px) * (a.1 - py) - (a.0 - px) * (c.1 - py)) / area;
            if w0 >= 0.0 && w1 >= 0.0 && w0 + w1 <= 1.0 {
                fb.put_pixel(x, y, shade);
            }
        }
    }
}

/// Draw a **download** glyph — a vertical stem, a solid down-arrowhead, and a
/// tray line beneath — centered at `(cx, cy)`, scale `s`. No face in the chain
/// carries ⤓. [`draw_arm_cue`] paints it over a held cell.
pub fn draw_download_glyph(fb: &mut Framebuffer, cx: i32, cy: i32, s: i32, shade: u8) {
    const T: i32 = 5;
    let stem_h = s + s / 4;
    let head_h = s * 3 / 4;
    let tray_gap = s / 2;
    // Center the whole glyph (stem + head + gap + tray) on `cy`.
    let total = stem_h + head_h + tray_gap + T;
    let stem_top = cy - total / 2;
    let x0 = (cx - T / 2).max(0) as u32;
    fb.fill_rect(stem_top.max(0) as u32, x0, T as u32, stem_h as u32, shade);
    // Arrowhead: a downward filled triangle, flat top at the stem's neck.
    let head_top = stem_top + stem_h;
    for row in 0..=head_h {
        let wpx = ((head_h - row) * 2 + 1) as u32;
        let left = (cx - (head_h - row)).max(0) as u32;
        fb.fill_rect((head_top + row).max(0) as u32, left, wpx, 1, shade);
    }
    // Tray: a short horizontal base under the arrow.
    let base_y = head_top + head_h + tray_gap;
    fb.fill_rect(
        base_y.max(0) as u32,
        (cx - s).max(0) as u32,
        (s * 2) as u32,
        T as u32,
        shade,
    );
}

/// Draw a **key** glyph — a ring bow on the left, a horizontal shaft, and two
/// teeth dropping off its tip — centered at `(cx, cy)`, scale `s`. No face in
/// the chain carries 🔑.
pub fn draw_key_glyph(fb: &mut Framebuffer, cx: i32, cy: i32, s: i32, shade: u8) {
    const T: i32 = 5;
    // Bow: a ring on the left, a hair inside where the shaft meets it.
    let bow_cx = cx - s / 2;
    let bow_r = s * 3 / 5;
    let inner = (bow_r - T).max(1) as f32;
    let rf = bow_r as f32;
    for dy in -bow_r..=bow_r {
        for dx in -bow_r..=bow_r {
            let dist = ((dx * dx + dy * dy) as f32).sqrt();
            if dist >= inner && dist <= rf {
                fb.put_pixel(bow_cx + dx, cy + dy, shade);
            }
        }
    }
    // Shaft: a horizontal bar from the bow's right edge to the key's tip.
    let shaft_x0 = bow_cx + bow_r - T / 2;
    let tip_x = cx + s;
    let shaft_w = (tip_x - shaft_x0).max(1);
    let shaft_y = cy - T / 2;
    fb.fill_rect(
        shaft_y.max(0) as u32,
        shaft_x0.max(0) as u32,
        shaft_w as u32,
        T as u32,
        shade,
    );
    // Teeth: two short prongs hanging below the shaft near the tip.
    let tooth_h = s / 2;
    for off in [0, s * 2 / 5] {
        let tx = tip_x - off - T;
        fb.fill_rect(
            (shaft_y + T).max(0) as u32,
            tx.max(0) as u32,
            T as u32,
            tooth_h as u32,
            shade,
        );
    }
}

/// The cell cleared to white, the cover aspect-fit between `top_inset` and the
/// name band, `label` in that band. Returns the painted cover rect. `top_inset`
/// is 0 for a book and `BAR_STRIP_H` for a series; off-screen no-ops.
#[allow(clippy::too_many_arguments)]
fn draw_cover_tile(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    cell_x: i32,
    cell_y: i32,
    cell_h: u32,
    top_inset: u32,
    cover: Option<&DynamicImage>,
    label: Label,
) -> (i32, i32, u32, u32) {
    if cell_x < 0 || cell_y < 0 {
        return (cell_x, cell_y, 0, 0);
    }
    fb.fill_rect(cell_y as u32, cell_x as u32, CELL_W, cell_h, 0xFF);

    // Cover region: full cell width (edge-to-edge, no inset card or frame),
    // between the optional top inset and the bottom name band. The cover
    // aspect-fits exactly like a standalone book cover.
    let region_y = cell_y + top_inset as i32;
    let region_h = cell_h - NAME_BAND_H - top_inset;
    let rect = match cover {
        Some(img) => blit_fit(fb, cell_x, region_y, CELL_W, region_h, img),
        None => {
            // No cover yet: a light fill spanning the region width.
            fb.fill_rect(region_y as u32, cell_x as u32, CELL_W, region_h, 0xDD);
            (cell_x, region_y, CELL_W, region_h)
        }
    };

    // Name band: a 2px separator then the label, centered and clamped to one
    // ellipsized line so a long title can't overrun the cell.
    let band_top = cell_y as u32 + (cell_h - NAME_BAND_H);
    fb.fill_rect(band_top, cell_x as u32, CELL_W, 2, 0x00);
    const PAD: u32 = 16;
    let width = CELL_W.saturating_sub(PAD * 2);
    let lines = renderer.wrap_and_clamp_in(label.script, label.text, width, 1);
    if let Some(line) = lines.first() {
        let lw = renderer.measure_width_in(label.script, line);
        let lx = cell_x + ((CELL_W as i32 - lw as i32) / 2).max(0);
        let baseline = band_top as i32 + (NAME_BAND_H * 62 / 100) as i32;
        renderer.draw_in(label.script, fb, lx, baseline, line, false);
    }
    rect
}

/// A book tile: a full-width cover over a name band, a series tile minus the
/// stack bars and count badge. Clears its own cell. Returns the letterboxed
/// artwork rect, which [`draw_downloaded_badge`] pins its corner to.
pub fn draw_book_cell(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    cell_x: i32,
    cell_y: i32,
    cell_h: u32,
    cover: Option<&DynamicImage>,
    title: Label,
) -> (i32, i32, u32, u32) {
    draw_cover_tile(fb, renderer, cell_x, cell_y, cell_h, 0, cover, title)
}

/// A series tile: [`draw_cover_tile`] with the series name in the band, two
/// **book-edge bars** above the cover (narrower and lighter as they recede),
/// and a dark **count badge** bottom-left. Clears its own cell.
#[allow(clippy::too_many_arguments)]
pub fn draw_series_cell(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    cell_x: i32,
    cell_y: i32,
    cell_h: u32,
    cover: Option<&DynamicImage>,
    count: usize,
    name: Label,
) {
    // A series reserves [`BAR_STRIP_H`] above the cover for its stack bars.
    // The cover itself matches a book's, lining the two up in the grid.
    let (cov_x, cov_y, cov_w, cov_h) = draw_cover_tile(
        fb,
        renderer,
        cell_x,
        cell_y,
        cell_h,
        BAR_STRIP_H,
        cover,
        name,
    );
    if cov_w == 0 {
        return; // off-screen cell
    }

    // Stack hint: two book-edge bars centered above the cover, the nearer (lower,
    // wider) bar darker than the farther (higher, narrower) one.
    let cx = cov_x + cov_w as i32 / 2;
    let bar_lo_w = cov_w * 86 / 100;
    let bar_hi_w = cov_w * 66 / 100;
    fb.fill_rect(
        (cov_y - (BAR_H as i32 + 4)).max(cell_y) as u32,
        (cx - bar_lo_w as i32 / 2).max(cell_x) as u32,
        bar_lo_w,
        BAR_H,
        0x66,
    );
    fb.fill_rect(
        (cov_y - (BAR_H as i32 * 2 + 6)).max(cell_y) as u32,
        (cx - bar_hi_w as i32 / 2).max(cell_x) as u32,
        bar_hi_w,
        BAR_H,
        0x99,
    );

    // Count badge: solid black rect + white (inverted) number, bottom-left of the
    // cover. Sized to the number so 1- and 2-digit counts both fit.
    let badge_text = count.to_string();
    let lh = renderer.line_height().max(1);
    let tw = renderer.measure_width(&badge_text);
    let badge_w = tw + BADGE_PAD * 2;
    let badge_h = lh + BADGE_PAD;
    let badge_x = cov_x + BADGE_MARGIN as i32;
    let badge_y = cov_y + cov_h as i32 - badge_h as i32 - BADGE_MARGIN as i32;
    fb.fill_rect(
        badge_y.max(cell_y) as u32,
        badge_x.max(cell_x) as u32,
        badge_w,
        badge_h,
        0x00,
    );
    let text_x = badge_x + ((badge_w as i32 - tw as i32) / 2).max(0);
    let text_baseline = badge_y + (badge_h * 70 / 100) as i32;
    renderer.draw(fb, text_x, text_baseline, &badge_text, true);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 7" panels take 3×3 at the full cell height. Both geometries are
    /// checked: a Colorsoft reports 1272×1696, and a rule holding for
    /// 1264×1680 alone holds for no real device.
    #[test]
    fn seven_inch_panels_are_unchanged() {
        for (w, h, expect_left) in [(1264u32, 1680u32, 60i32), (1272, 1696, 64)] {
            let l = Layout::compute(w, h, 190, 80);
            assert_eq!((l.cols, l.rows), (3, 3), "{w}x{h}");
            assert_eq!(l.cell_h, CELL_H_MAX, "{w}x{h}: cell height must not shrink");
            assert_eq!(l.page_size(), 9, "{w}x{h}");
            assert_eq!(l.left, expect_left, "{w}x{h}: grid stays centred");
        }
    }

    /// The Scribe's extra area buys rows, not bigger covers.
    #[test]
    fn scribe_gains_rows_and_columns() {
        let l = Layout::compute(1860, 2480, 190, 80);
        assert_eq!((l.cols, l.rows), (4, 5));
        assert_eq!(l.page_size(), 20);
        assert_eq!(l.left, 162);
        // Fit-then-expand: five rows exist below [`CELL_H_MAX`], over the floor.
        assert!(
            (CELL_H_MIN..CELL_H_MAX).contains(&l.cell_h),
            "cell_h {} outside [{CELL_H_MIN}, {CELL_H_MAX})",
            l.cell_h
        );
        // Everything has to actually fit between the header and the strip.
        let used = l.rows as u32 * l.cell_h + (l.rows as u32 - 1) * ROW_GAP;
        assert!(
            used <= 2480 - 190 - 80,
            "{used} overflows the usable height"
        );
    }

    /// A panel under one full cell yields a usable grid, past a divide-by-zero
    /// and an empty page.
    #[test]
    fn degenerate_panel_still_yields_one_cell() {
        let l = Layout::compute(100, 100, 190, 80);
        assert_eq!((l.cols, l.rows), (1, 1));
        assert_eq!(l.page_size(), 1);
    }

    #[test]
    fn taps_in_gaps_and_margins_miss() {
        let l = Layout::compute(1860, 2480, 190, 80);
        let (x, y) = l.cell_xy(0);
        assert_eq!(l.cell_at_tap(x as u32 + 5, y as u32 + 5, 20), Some(0));
        // The column gap between cell 0 and cell 1.
        let gap_x = x as u32 + CELL_W + COL_GAP / 2;
        assert_eq!(l.cell_at_tap(gap_x, y as u32 + 5, 20), None);
        // Left of the grid entirely.
        assert_eq!(l.cell_at_tap(4, y as u32 + 5, 20), None);
        // Past the last row — must not wrap onto a phantom cell.
        assert_eq!(l.cell_at_tap(x as u32 + 5, 2470, 20), None);
    }

    #[test]
    fn the_grid_is_centred_between_the_search_bar_and_the_strip() {
        // Colorsoft geometry, as the device actually reports it.
        let (xres, yres, top_margin, strip_h) = (1272, 1696, 120, 80);
        let l = Layout::compute(xres, yres, top_margin, strip_h);

        let avail = yres - top_margin - strip_h;
        let content_h = l.rows as u32 * l.cell_h + (l.rows as u32 - 1) * ROW_GAP;
        let above = l.top as u32 - top_margin;
        let below = avail - content_h - above;

        assert!(
            content_h < avail,
            "cell_h is clamped, so there is slack to distribute"
        );
        assert!(
            l.top > top_margin as i32,
            "top-anchored again: the grid crowds the search bar"
        );
        // Even split, give or take the odd pixel.
        assert!(above.abs_diff(below) <= 1, "above={above} below={below}");
    }
}
