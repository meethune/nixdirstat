//! Half-block pixel grid for high-fidelity treemap rendering.
//!
//! Provides a [`PixelGrid`] that maps two terminal rows per logical pixel row,
//! enabling double-height resolution via Unicode half-block characters.
//!
//! # Overview
//!
//! - [`PixelGrid::new`] — create a grid sized in terminal cell dimensions.
//! - [`PixelGrid::fill_rect`] — paint a rectangle in pixel coordinates.
//! - [`PixelGrid::darken_edges`] — apply a fixed vignette to a pixel rectangle.
//! - [`PixelGrid::darken_edges_adaptive`] — apply an adaptive vignette with variable ring counts.
//! - [`PixelGrid::flush_to_buffer`] — render the grid into a ratatui [`Buffer`].

use ratatui::{
    buffer::{Buffer, Cell},
    layout::Rect,
    style::Color,
};

use crate::ui::colors::darken;

/// Configuration for [`PixelGrid::darken_edges_adaptive`].
#[derive(Debug, Clone, Copy)]
pub struct VignetteConfig {
    /// Number of darkening rings from the outside in.
    pub outer_rings: u16,
    /// Number of additional darkening rings continuing inward.
    pub inner_rings: u16,
    /// Minimum pixel dimension required before any darkening is applied.
    pub min_size: u16,
}

/// A pixel grid that provides 2× vertical resolution via Unicode half-block
/// characters.
///
/// Each terminal cell is split into two vertical pixels: the top pixel maps to
/// the foreground of `▀` (U+2580 UPPER HALF BLOCK) and the bottom pixel maps
/// to the background.  When both pixels in a cell are identical the full-block
/// character `█` (U+2588) is used instead.  When both pixels equal the
/// background colour a plain space is emitted.
///
/// # Coordinate systems
///
/// - **Terminal cell space** — used for `width`/`height` in [`new`] and for
///   the `area` parameter of [`flush_to_buffer`].
/// - **Pixel space** — used by [`fill_rect`] and [`darken_edges`].
///   x ∈ `[0, pixel_width)`, y ∈ `[0, pixel_height)`.
///
/// [`new`]: PixelGrid::new
/// [`flush_to_buffer`]: PixelGrid::flush_to_buffer
/// [`fill_rect`]: PixelGrid::fill_rect
/// [`darken_edges`]: PixelGrid::darken_edges
pub struct PixelGrid {
    /// Terminal cell width (= pixel width).
    width: u16,
    /// Terminal cell height (pixel height = `height * 2`).
    height: u16,
    /// Background colour used for empty pixels.
    bg: Color,
    /// Flat row-major pixel buffer; length = `width * (height * 2)`.
    pixels: Vec<Color>,
}

impl PixelGrid {
    /// Create a new pixel grid with the given terminal cell dimensions.
    ///
    /// The internal pixel buffer has `width × (height × 2)` elements, all
    /// initialised to `bg`.
    pub fn new(width: u16, height: u16, bg: Color) -> Self {
        let pixel_count = usize::from(width) * usize::from(height) * 2;
        Self {
            width,
            height,
            bg,
            pixels: vec![bg; pixel_count],
        }
    }

    /// Reset every pixel to `bg` and record the new background colour.
    pub fn clear(&mut self, bg: Color) {
        self.bg = bg;
        self.pixels.fill(bg);
    }

    /// Return the pixel-space width (equal to the terminal cell width).
    pub const fn pixel_width(&self) -> u16 {
        self.width
    }

    /// Return the pixel-space height (twice the terminal cell height).
    pub const fn pixel_height(&self) -> u16 {
        self.height.saturating_mul(2)
    }

    /// Clamp a pixel rectangle to the grid bounds.
    ///
    /// Returns `None` for zero-dimension or fully out-of-bounds rectangles.
    /// Otherwise returns `(x, y, x_end, y_end)` with endpoints clamped to the
    /// grid dimensions.
    fn clamp_rect(&self, x: u16, y: u16, w: u16, h: u16) -> Option<(u16, u16, u16, u16)> {
        if w == 0 || h == 0 {
            return None;
        }
        let pw = self.pixel_width();
        let ph = self.pixel_height();
        if x >= pw || y >= ph {
            return None;
        }
        let x_end = x.saturating_add(w).min(pw);
        let y_end = y.saturating_add(h).min(ph);
        Some((x, y, x_end, y_end))
    }

    /// Paint a rectangle in pixel coordinates with `color`.
    ///
    /// - `(x, y)` is the top-left pixel corner.
    /// - `(w, h)` is the width and height in pixels.
    ///
    /// Zero-dimension rectangles are silently ignored.  Out-of-bounds
    /// coordinates are clamped to the grid; the method never panics.
    pub fn fill_rect(&mut self, x: u16, y: u16, w: u16, h: u16, color: Color) {
        let Some((x, y, x_end, y_end)) = self.clamp_rect(x, y, w, h) else {
            return;
        };
        for py in y..y_end {
            let row_base = usize::from(py) * usize::from(self.width);
            for px in x..x_end {
                self.pixels[row_base + usize::from(px)] = color;
            }
        }
    }

    /// Apply a darkening vignette to a pixel rectangle.
    ///
    /// - The outermost pixel ring is darkened by 30 %.
    /// - When the clamped rectangle is at least 6 pixels wide **and** 6 pixels
    ///   tall, the next inner ring is additionally darkened by 15 %.
    ///
    /// Coordinates are in pixel space and are clamped to the grid bounds.
    /// Zero-dimension rectangles are silently ignored.
    pub fn darken_edges(&mut self, x: u16, y: u16, w: u16, h: u16) {
        let Some((x, y, x_end, y_end)) = self.clamp_rect(x, y, w, h) else {
            return;
        };
        self.apply_ring(x, y, x_end, y_end, 0.30);

        // Inner ring: darken 15 % when the actual clamped region is wide and tall enough.
        let actual_w = x_end - x;
        let actual_h = y_end - y;
        if actual_w >= 6 && actual_h >= 6 {
            // Checked: x < pw <= u16::MAX so x + 1 <= u16::MAX.
            // x_end >= x + 6 so x_end - 1 >= 5; similarly for y.
            self.apply_ring(x + 1, y + 1, x_end - 1, y_end - 1, 0.15);
        }
    }

    /// Apply an adaptive darkening vignette with configurable ring counts.
    ///
    /// Applies darkening to a pixel rectangle with independent control over the
    /// number of outer and inner rings. The method has a minimum size gate: if
    /// `min(w, h) < cfg.min_size`, no darkening is applied.
    ///
    /// # Parameters
    ///
    /// - `x`, `y`, `w`, `h` — rectangle in pixel space (clamped to grid bounds).
    /// - `cfg` — vignette configuration (ring counts and minimum size gate).
    ///
    /// Darkening amounts per ring index: `[0.30, 0.15, 0.08, ...]`.
    pub fn darken_edges_adaptive(&mut self, x: u16, y: u16, w: u16, h: u16, cfg: VignetteConfig) {
        let Some((mut x0, mut y0, mut xe, mut ye)) = self.clamp_rect(x, y, w, h) else {
            return;
        };

        // Gate on minimum size.
        let actual_w = xe - x0;
        let actual_h = ye - y0;
        if actual_w < cfg.min_size || actual_h < cfg.min_size {
            return;
        }

        // Darkening amounts indexed by ring number.
        let amounts = [0.30, 0.15, 0.08];
        let total_rings = cfg.outer_rings.saturating_add(cfg.inner_rings);

        // Apply rings from outside inward.
        for ring_idx in 0..total_rings {
            if x0 >= xe || y0 >= ye {
                break;
            }
            let amount = amounts.get(usize::from(ring_idx)).copied().unwrap_or(0.08);
            self.apply_ring(x0, y0, xe, ye, amount);

            // Shrink inward.
            x0 = x0.saturating_add(1);
            y0 = y0.saturating_add(1);
            xe = xe.saturating_sub(1);
            ye = ye.saturating_sub(1);
        }
    }

    /// Paint a rectangle with cushion shading — a smooth 2D gradient that is
    /// brightest at the centre and darkest at the edges, creating a 3D "pillow"
    /// effect.
    ///
    /// `intensity` controls how strong the shading is: 0.0 = flat (no shading),
    /// 1.0 = maximum shading (edges go fully dark). Values around 0.4–0.6
    /// produce the classic WinDirStat/SequoiaView look.
    ///
    /// Coordinates are in pixel space and are clamped to the grid bounds.
    pub fn fill_rect_cushion(
        &mut self,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        color: Color,
        intensity: f32,
    ) {
        let Some((x, y, x_end, y_end)) = self.clamp_rect(x, y, w, h) else {
            return;
        };
        let Color::Rgb(cr, cg, cb) = color else {
            self.fill_rect(x, y, x_end - x, y_end - y, color);
            return;
        };

        let actual_w = x_end - x;
        let actual_h = y_end - y;
        if actual_w == 0 || actual_h == 0 {
            return;
        }

        let half_w = f32::from(actual_w) / 2.0;
        let half_h = f32::from(actual_h) / 2.0;

        for py in y..y_end {
            let row_base = usize::from(py) * usize::from(self.width);
            let dy = (f32::from(py - y) + 0.5 - half_h) / half_h;
            let dy2 = dy * dy;
            for px in x..x_end {
                let dx = (f32::from(px - x) + 0.5 - half_w) / half_w;
                // Squared distance from centre, normalised to [0, 1] at corners.
                let dist2 = dx.mul_add(dx, dy2).min(1.0);
                // Cosine-like falloff: bright centre, dark edges.
                let factor = intensity.mul_add(-dist2, 1.0);
                // Verified false positive: factor ∈ [1-intensity, 1] and channels ∈ [0,255],
                // so products are in [0, 255]. Clamping ensures no sign loss.
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                let pr = (f32::from(cr) * factor).clamp(0.0, 255.0) as u8;
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                let pg = (f32::from(cg) * factor).clamp(0.0, 255.0) as u8;
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                let pb = (f32::from(cb) * factor).clamp(0.0, 255.0) as u8;
                self.pixels[row_base + usize::from(px)] = Color::Rgb(pr, pg, pb);
            }
        }
    }

    /// Render the pixel grid into a ratatui [`Buffer`] within `area`.
    ///
    /// For each terminal cell in `area` (clamped to the grid dimensions):
    /// - **space** (`" "`): both pixels equal the background colour.
    /// - **full block** (`"█"`): top and bottom pixels are equal and non-background.
    /// - **upper half block** (`"▀"`): top and bottom pixels differ; fg = top, bg = bottom.
    ///
    /// Cells outside the grid bounds are left unchanged.
    pub fn flush_to_buffer(&self, buf: &mut Buffer, area: Rect) {
        let cols = area.width.min(self.width);
        let rows = area.height.min(self.height);
        for cy in 0..rows {
            for cx in 0..cols {
                self.flush_one_cell(buf, area, cx, cy);
            }
        }
    }

    /// Write a single half-block cell at terminal position `(cx, cy)` within `area`.
    fn flush_one_cell(&self, buf: &mut Buffer, area: Rect, cx: u16, cy: u16) {
        let buf_x = area.x.saturating_add(cx);
        let buf_y = area.y.saturating_add(cy);
        let row_stride = usize::from(self.width);
        let top_row = usize::from(cy) * 2;
        let bot_row = top_row + 1;
        let col = usize::from(cx);
        let top = self.pixels[top_row * row_stride + col];
        let bot = self.pixels[bot_row * row_stride + col];
        let Some(cell) = buf.cell_mut((buf_x, buf_y)) else {
            return;
        };
        set_half_block_cell(cell, top, bot, self.bg);
    }

    /// Darken every pixel on the perimeter of `(x, y, x_end, y_end)` by `amount`.
    ///
    /// Iterates the top row, bottom row, and the left/right columns of the
    /// middle rows directly, avoiding redundant per-pixel conditionals.
    fn apply_ring(&mut self, x: u16, y: u16, x_end: u16, y_end: u16, amount: f32) {
        if x_end <= x || y_end <= y {
            return;
        }
        let w = usize::from(self.width);

        // Top row.
        let top_base = usize::from(y) * w;
        for px in x..x_end {
            let i = top_base + usize::from(px);
            self.pixels[i] = darken(self.pixels[i], amount);
        }

        if y_end <= y + 1 {
            return;
        }

        // Bottom row (only when height > 1).
        let bot_base = usize::from(y_end - 1) * w;
        for px in x..x_end {
            let i = bot_base + usize::from(px);
            self.pixels[i] = darken(self.pixels[i], amount);
        }

        // Left and right columns of the interior rows.
        for py in (y + 1)..(y_end - 1) {
            let base = usize::from(py) * w;
            let left_i = base + usize::from(x);
            self.pixels[left_i] = darken(self.pixels[left_i], amount);
            if x_end > x + 1 {
                let right_i = base + usize::from(x_end - 1);
                self.pixels[right_i] = darken(self.pixels[right_i], amount);
            }
        }
    }
}

/// Write a half-block character into a ratatui [`Cell`] based on top/bottom pixel colours.
///
/// - Both pixels equal `bg` → space, bg = `bg`.
/// - Both pixels equal each other (non-bg) → `█`, fg = colour, bg = `bg`.
/// - Pixels differ → `▀`, fg = top, bg = bottom.
fn set_half_block_cell(cell: &mut Cell, top: Color, bot: Color, bg: Color) {
    if top == bg && bot == bg {
        cell.set_symbol(" ");
        cell.set_bg(bg);
    } else if top == bot {
        cell.set_symbol("█");
        cell.set_fg(top);
        cell.set_bg(bg);
    } else {
        cell.set_symbol("▀");
        cell.set_fg(top);
        cell.set_bg(bot);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ratatui::{buffer::Buffer, layout::Rect, style::Color};

    use super::*;

    /// Access a pixel by (x, y) coordinates for test assertions.
    fn get_pixel(grid: &PixelGrid, x: u16, y: u16) -> Color {
        let idx = usize::from(y) * usize::from(grid.width) + usize::from(x);
        grid.pixels[idx]
    }

    // --- fill_rect ---

    #[test]
    fn fill_rect_sets_pixels() {
        let mut grid = PixelGrid::new(10, 4, Color::Reset);
        grid.fill_rect(2, 1, 3, 2, Color::Rgb(255, 0, 0));
        // Filled pixel should be red.
        assert_eq!(get_pixel(&grid, 2, 1), Color::Rgb(255, 0, 0));
        // Untouched pixel should still be bg.
        assert_eq!(get_pixel(&grid, 0, 0), Color::Reset);
    }

    #[test]
    fn fill_rect_zero_dimension_is_noop() {
        let mut grid = PixelGrid::new(10, 4, Color::Reset);
        grid.fill_rect(0, 0, 0, 5, Color::Rgb(255, 0, 0));
        grid.fill_rect(0, 0, 5, 0, Color::Rgb(255, 0, 0));
        // No pixels should have changed.
        assert_eq!(get_pixel(&grid, 0, 0), Color::Reset);
    }

    #[test]
    fn overlapping_rects_last_wins() {
        let mut grid = PixelGrid::new(10, 4, Color::Reset);
        grid.fill_rect(0, 0, 5, 4, Color::Rgb(255, 0, 0));
        grid.fill_rect(2, 1, 3, 2, Color::Rgb(0, 0, 255));
        // Overlap pixel should be blue (last write wins).
        assert_eq!(get_pixel(&grid, 3, 2), Color::Rgb(0, 0, 255));
    }

    #[test]
    fn fill_rect_oob_clamps_silently() {
        let mut grid = PixelGrid::new(5, 3, Color::Reset);
        // pixel_width=5, pixel_height=6; rect starts near edge and extends far out.
        grid.fill_rect(3, 4, 10, 10, Color::Rgb(1, 2, 3));
        // In-bounds pixel inside the clamped region should be filled.
        assert_eq!(get_pixel(&grid, 4, 5), Color::Rgb(1, 2, 3));
        // Pixel well outside the clamped region should remain bg.
        assert_eq!(get_pixel(&grid, 0, 0), Color::Reset);
    }

    // --- flush_to_buffer ---

    #[test]
    fn flush_same_colors_produces_full_block() {
        let mut grid = PixelGrid::new(1, 2, Color::Reset);
        let red = Color::Rgb(255, 0, 0);
        grid.fill_rect(0, 0, 1, 2, red);
        let area = Rect::new(0, 0, 1, 1);
        let mut buf = Buffer::empty(area);
        grid.flush_to_buffer(&mut buf, area);
        let cell = &buf[(0, 0)];
        assert_eq!(cell.symbol(), "█");
        assert_eq!(cell.fg, red);
        // Full-block cells use the PixelGrid background colour for bg.
        assert_eq!(cell.bg, Color::Reset);
    }

    #[test]
    fn flush_different_colors_produces_upper_half() {
        let mut grid = PixelGrid::new(1, 2, Color::Reset);
        let red = Color::Rgb(255, 0, 0);
        let blue = Color::Rgb(0, 0, 255);
        grid.fill_rect(0, 0, 1, 1, red);
        grid.fill_rect(0, 1, 1, 1, blue);
        let area = Rect::new(0, 0, 1, 1);
        let mut buf = Buffer::empty(area);
        grid.flush_to_buffer(&mut buf, area);
        let cell = &buf[(0, 0)];
        assert_eq!(cell.symbol(), "▀");
        assert_eq!(cell.fg, red);
        assert_eq!(cell.bg, blue);
    }

    #[test]
    fn flush_both_bg_produces_space() {
        let bg = Color::Reset;
        let grid = PixelGrid::new(1, 2, bg);
        let area = Rect::new(0, 0, 1, 1);
        let mut buf = Buffer::empty(area);
        grid.flush_to_buffer(&mut buf, area);
        assert_eq!(buf[(0, 0)].symbol(), " ");
    }

    // --- darken_edges ---

    #[test]
    fn darken_edges_darkens_border_preserves_interior() {
        let red = Color::Rgb(200, 100, 50);
        let mut grid = PixelGrid::new(6, 8, Color::Reset);
        // pixel_width=6, pixel_height=16; fill the top 8 pixel rows.
        grid.fill_rect(0, 0, 6, 8, red);
        grid.darken_edges(0, 0, 6, 8);
        // Interior pixel (3, 4) should be unchanged.
        assert_eq!(get_pixel(&grid, 3, 4), red);
        // Corner pixel (0, 0) is on the outer ring → darkened by 30 %.
        // darken(Rgb(200, 100, 50), 0.30) = Rgb(140, 70, 35).
        assert_eq!(get_pixel(&grid, 0, 0), Color::Rgb(140, 70, 35));
    }

    #[test]
    fn darken_edges_zero_dimension_is_noop() {
        let red = Color::Rgb(200, 100, 50);
        let mut grid = PixelGrid::new(10, 5, Color::Reset);
        grid.fill_rect(0, 0, 10, 10, red);
        let before = get_pixel(&grid, 0, 0);
        grid.darken_edges(0, 0, 0, 5);
        grid.darken_edges(0, 0, 5, 0);
        assert_eq!(get_pixel(&grid, 0, 0), before);
    }

    // --- darken_edges_adaptive ---

    #[test]
    fn darken_edges_adaptive_three_outer_rings() {
        let red = Color::Rgb(200, 100, 50);
        let mut grid = PixelGrid::new(20, 10, Color::Reset);
        grid.fill_rect(0, 0, 20, 20, red);
        grid.darken_edges_adaptive(
            0,
            0,
            20,
            20,
            VignetteConfig {
                outer_rings: 3,
                inner_rings: 0,
                min_size: 6,
            },
        );
        // Ring 0 (outermost): 30% darker
        assert_eq!(get_pixel(&grid, 0, 0), Color::Rgb(140, 70, 35));
        // Ring 1: 15% darker than original
        assert_eq!(get_pixel(&grid, 1, 1), darken(red, 0.15));
        // Ring 2: 8% darker than original
        assert_eq!(get_pixel(&grid, 2, 2), darken(red, 0.08));
        // Interior: unchanged
        assert_eq!(get_pixel(&grid, 5, 5), red);
    }

    #[test]
    fn darken_edges_adaptive_below_min_size_is_noop() {
        let red = Color::Rgb(200, 100, 50);
        let mut grid = PixelGrid::new(5, 3, Color::Reset);
        grid.fill_rect(0, 0, 4, 4, red);
        grid.darken_edges_adaptive(
            0,
            0,
            4,
            4,
            VignetteConfig {
                outer_rings: 1,
                inner_rings: 0,
                min_size: 6,
            },
        );
        // 4 < min_size 6 → no darkening applied
        assert_eq!(get_pixel(&grid, 0, 0), red);
    }

    #[test]
    fn darken_edges_adaptive_with_inner_rings() {
        let red = Color::Rgb(200, 100, 50);
        let mut grid = PixelGrid::new(20, 10, Color::Reset);
        grid.fill_rect(0, 0, 20, 20, red);
        grid.darken_edges_adaptive(
            0,
            0,
            20,
            20,
            VignetteConfig {
                outer_rings: 1,
                inner_rings: 2,
                min_size: 6,
            },
        );
        // Outer ring: 30%
        assert_eq!(get_pixel(&grid, 0, 0), darken(red, 0.30));
        // Inner ring 1: 15%
        assert_eq!(get_pixel(&grid, 1, 1), darken(red, 0.15));
        // Inner ring 2: 8%
        assert_eq!(get_pixel(&grid, 2, 2), darken(red, 0.08));
    }

    // --- proptest ---

    proptest! {
        #[test]
        fn fill_rect_only_affects_interior(
            gw in 1u16..=50,
            gh in 1u16..=25,
            rx in 0u16..=49,
            ry in 0u16..=49,
            rw in 0u16..=50,
            rh in 0u16..=50,
            r in 0u8..=255,
            g in 0u8..=255,
            b in 0u8..=255,
        ) {
            let bg = Color::Reset;
            let color = Color::Rgb(r, g, b);
            let mut grid = PixelGrid::new(gw, gh, bg);
            grid.fill_rect(rx, ry, rw, rh, color);

            let pw = grid.pixel_width();
            let ph = grid.pixel_height();

            // Compute the actual clamped fill bounds (mirrors fill_rect logic).
            let filled = if rw == 0 || rh == 0 || rx >= pw || ry >= ph {
                None
            } else {
                let x_end = rx.saturating_add(rw).min(pw);
                let y_end = ry.saturating_add(rh).min(ph);
                Some((rx, ry, x_end, y_end))
            };

            for py in 0..ph {
                for px in 0..pw {
                    let pixel = get_pixel(&grid, px, py);
                    let inside = filled.is_some_and(|(x0, y0, xe, ye)| {
                        px >= x0 && px < xe && py >= y0 && py < ye
                    });
                    if !inside {
                        prop_assert_eq!(
                            pixel,
                            bg,
                            "pixel ({}, {}) outside the filled rect should remain bg",
                            px,
                            py
                        );
                    }
                }
            }
        }
    }
}
