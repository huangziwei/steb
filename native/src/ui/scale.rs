//! Physical sizing. Every pixel constant in `ui` is a *design pixel* written
//! for a [`DESIGN_DPI`] panel, which [`Scale::px`] turns into the device pixels
//! drawing it at that size. Density comes off the width; nothing names a model.

/// The density every `ui` constant is written at: 300 ppi, the Voyage and
/// everything after it.
pub const DESIGN_DPI: u32 = 300;

/// Panel density by framebuffer width, for the two widths that are not
/// [`DESIGN_DPI`]. The nominal 768 px Paperwhite is not one: the X server
/// reports its 758 px framebuffer.
const PANELS: &[(u32, u32)] = &[(600, 167), (758, 212)];

/// What one design pixel is worth on the panel being drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scale {
    dpi: u32,
}

impl Scale {
    /// The density [`PANELS`] gives `fb_xres`, else [`DESIGN_DPI`].
    pub fn of_width(fb_xres: u32) -> Self {
        let dpi = PANELS
            .iter()
            .find(|(width, _)| *width == fb_xres)
            .map(|(_, dpi)| *dpi)
            .unwrap_or(DESIGN_DPI);
        Self { dpi }
    }

    pub fn dpi(&self) -> u32 {
        self.dpi
    }

    /// `design` in device pixels. A non-zero constant never scales to 0: a
    /// hairline that rounds away leaves the shape it divides unreadable.
    pub fn px(&self, design: u32) -> u32 {
        match design {
            0 => 0,
            _ => (design * self.dpi / DESIGN_DPI).max(1),
        }
    }

    /// [`Scale::px`] for a signed constant, keeping its sign.
    pub fn i(&self, design: i32) -> i32 {
        let scaled = self.px(design.unsigned_abs()) as i32;
        match design < 0 {
            true => -scaled,
            false => scaled,
        }
    }

    /// [`Scale::px`] for a font size, which keeps its fraction.
    pub fn font(&self, design: f32) -> f32 {
        design * self.dpi as f32 / DESIGN_DPI as f32
    }
}

#[cfg(test)]
mod tests {
    use super::{DESIGN_DPI, Scale};

    /// The two panels that are not [`DESIGN_DPI`], and the ones that are.
    #[test]
    fn the_shipped_widths_carry_their_own_density() {
        assert_eq!(Scale::of_width(600).dpi(), 167);
        assert_eq!(Scale::of_width(758).dpi(), 212);
        for wide in [1072, 1236, 1264, 1272, 1860] {
            assert_eq!(Scale::of_width(wide).dpi(), DESIGN_DPI, "{wide}");
        }
        // An unreported width draws at the density most devices have.
        assert_eq!(Scale::of_width(999).dpi(), DESIGN_DPI);
    }

    /// A design pixel is itself at [`DESIGN_DPI`] and smaller below it.
    #[test]
    fn a_design_pixel_shrinks_with_the_panel() {
        let oasis = Scale::of_width(1264);
        assert_eq!(oasis.px(360), 360);
        assert_eq!(oasis.font(32.0), 32.0);

        let pw2 = Scale::of_width(758);
        assert_eq!(pw2.px(360), 360 * 212 / 300);
        assert_eq!(pw2.px(80), 56);

        let basic = Scale::of_width(600);
        assert_eq!(basic.px(360), 360 * 167 / 300);
    }

    /// A constant that would round to nothing keeps one pixel, and a zero
    /// stays zero.
    #[test]
    fn a_hairline_survives_the_smallest_panel() {
        let basic = Scale::of_width(600);
        assert_eq!(basic.px(1), 1);
        assert_eq!(basic.px(2), 1);
        assert_eq!(basic.px(0), 0);
    }

    /// [`Scale::i`] keeps the sign.
    #[test]
    fn a_signed_constant_scales_both_ways() {
        let pw2 = Scale::of_width(758);
        assert_eq!(pw2.i(44), pw2.px(44) as i32);
        assert_eq!(pw2.i(-44), -(pw2.px(44) as i32));
        assert_eq!(pw2.i(0), 0);
        assert_eq!(pw2.i(-1), -1);
    }

    /// Type set at a density is the same size on the page at every other.
    #[test]
    fn a_body_line_is_one_size_on_every_panel() {
        // Within a fiftieth of an inch of 32 px at 300 ppi, on every panel.
        let want = 32.0 / DESIGN_DPI as f32;
        for width in [600, 758, 1072, 1236, 1264, 1860] {
            let scale = Scale::of_width(width);
            let inches = scale.font(32.0) / scale.dpi() as f32;
            assert!((inches - want).abs() < 0.02, "{width}: {inches}″");
        }
    }
}
