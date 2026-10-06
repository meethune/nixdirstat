//! Squarified treemap widget for visualising disk usage.
//!
//! Uses the `streemap` crate's [`squarify`] algorithm to lay out items
//! proportionally in the available area. Each cell is filled with a
//! category-specific background colour and labelled with the file name.
//!
//! [`squarify`]: streemap::squarify

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Span,
    widgets::StatefulWidget,
};
use streemap::Rect as SRect;

use crate::{types::FileCategory, ui::app::TreemapState};

// ---------------------------------------------------------------------------
// Colour palettes
// ---------------------------------------------------------------------------

/// Return the terminal colour associated with a [`FileCategory`].
///
/// Used by the bar chart widget for category-level colouring.
pub(crate) const fn category_color(category: FileCategory) -> Color {
    match category {
        FileCategory::Code => Color::Blue,
        FileCategory::Image => Color::Green,
        FileCategory::Document => Color::Yellow,
        FileCategory::Archive => Color::Red,
        FileCategory::Audio => Color::Magenta,
        FileCategory::Video => Color::Cyan,
        FileCategory::Binary => Color::DarkGray,
        FileCategory::NoExtension => Color::Gray,
        _ => Color::White,
    }
}

/// Perceptually distinct 256-colour palette for per-extension treemap cells.
///
/// Chosen to be saturated and distinguishable on dark backgrounds. Each
/// extension hashes to one of these colours, so files with the same extension
/// share a colour while different extensions are visually distinct — matching
/// the WinDirStat approach.
const EXTENSION_PALETTE: [Color; 22] = [
    Color::Indexed(196), // red
    Color::Indexed(202), // orange
    Color::Indexed(208), // dark orange
    Color::Indexed(214), // gold
    Color::Indexed(220), // yellow
    Color::Indexed(226), // bright yellow
    Color::Indexed(46),  // green
    Color::Indexed(34),  // forest green
    Color::Indexed(48),  // sea green
    Color::Indexed(51),  // cyan
    Color::Indexed(39),  // sky blue
    Color::Indexed(27),  // blue
    Color::Indexed(21),  // deep blue
    Color::Indexed(57),  // indigo
    Color::Indexed(129), // purple
    Color::Indexed(165), // magenta
    Color::Indexed(205), // hot pink
    Color::Indexed(172), // brown
    Color::Indexed(136), // olive
    Color::Indexed(71),  // moss
    Color::Indexed(109), // steel blue
    Color::Indexed(174), // rose
];

/// Map a file extension to a colour from the extension palette.
///
/// Uses FNV-1a hash for fast, low-collision distribution across the palette.
/// Files with no extension get [`Color::Gray`].
pub(crate) fn extension_color(ext: Option<&str>) -> Color {
    let Some(ext) = ext else {
        return Color::Gray;
    };
    if ext.is_empty() {
        return Color::Gray;
    }
    // FNV-1a hash — fast, no allocation, good distribution for short strings.
    let mut hash: u32 = 2_166_136_261;
    for byte in ext.as_bytes() {
        hash ^= u32::from(byte.to_ascii_lowercase());
        hash = hash.wrapping_mul(16_777_619);
    }
    EXTENSION_PALETTE[hash as usize % EXTENSION_PALETTE.len()]
}

// ---------------------------------------------------------------------------
// TreemapItem
// ---------------------------------------------------------------------------

/// A single entry to be rendered in the treemap.
#[derive(Debug, Clone)]
pub struct TreemapItem {
    /// Display label (typically the file/directory name).
    pub label: String,
    /// Size in bytes — used to compute proportional area.
    pub size: u64,
    /// Content category (used for bar chart, not treemap colouring).
    pub category: FileCategory,
    /// Whether this entry is a directory.
    pub is_directory: bool,
    /// File extension (lowercase), used for per-extension colouring.
    pub extension: Option<String>,
}

// ---------------------------------------------------------------------------
// TreemapWidget
// ---------------------------------------------------------------------------

/// Squarified treemap widget.
///
/// Implements [`StatefulWidget`] with [`TreemapState`] so that the caller
/// can track and update the selected cell index across frames.
#[derive(Debug, Default)]
pub struct TreemapWidget {
    /// Items to lay out in the treemap, in descending size order for best aspect ratios.
    pub items: Vec<TreemapItem>,
}

impl StatefulWidget for TreemapWidget {
    type State = TreemapState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        if area.is_empty() || self.items.is_empty() {
            return;
        }

        // Build layout data: each entry holds (original_index, item_ref, assigned_rect).
        let const_zero = SRect {
            x: 0.0_f32,
            y: 0.0_f32,
            w: 0.0_f32,
            h: 0.0_f32,
        };
        let mut layout: Vec<(usize, &TreemapItem, SRect<f32>)> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.size > 0)
            .map(|(i, item)| (i, item, const_zero))
            .collect();

        if layout.is_empty() {
            return;
        }

        let container = SRect {
            x: 0.0_f32,
            y: 0.0_f32,
            w: f32::from(area.width),
            h: f32::from(area.height),
        };

        streemap::squarify(
            container,
            &mut layout,
            // cast_precision_loss: u64→f32 intentional — treemap only needs relative proportions.
            #[allow(clippy::cast_precision_loss)]
            |(_i, item, _r)| item.size as f32,
            |(_i, _item, r), new_r| *r = new_r,
        );

        // Render each cell.
        for (original_idx, item, f32_rect) in &layout {
            let cell_rect = f32_rect_to_ratatui(*f32_rect, area);
            if cell_rect.width == 0 || cell_rect.height == 0 {
                continue;
            }

            let color = if item.is_directory {
                category_color(item.category)
            } else {
                extension_color(item.extension.as_deref())
            };
            let is_selected = state.selected == Some(*original_idx);
            let bg_style = if is_selected {
                Style::default()
                    .bg(color)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().bg(color).fg(Color::Black)
            };

            // Fill the entire cell with the category background.
            buf.set_style(cell_rect, bg_style);

            // Render label in the top-left of the cell (only if wide enough).
            if cell_rect.width >= 3 {
                render_cell_label(buf, &item.label, cell_rect, bg_style);
            }
            // width < 3: no label rendered
        }
    }
}

// ---------------------------------------------------------------------------
// Label rendering helper
// ---------------------------------------------------------------------------

/// Render a cell label into `buf` at the top-left of `cell_rect`.
///
/// Truncates with `"..."` if the label is wider than the cell.
/// Must only be called when `cell_rect.width >= 3`.
fn render_cell_label(buf: &mut Buffer, label: &str, cell_rect: Rect, style: Style) {
    let max_chars = usize::from(cell_rect.width);
    if label.len() <= max_chars {
        let span = Span::styled(label, style);
        buf.set_span(cell_rect.x, cell_rect.y, &span, cell_rect.width);
        return;
    }
    // Truncate to `max_chars - 3` visible chars, then append "...".
    let keep = max_chars.saturating_sub(3);
    // Slice at a char boundary — file names are typically ASCII.
    let safe_end = label
        .char_indices()
        .nth(keep)
        .map_or(label.len(), |(i, _)| i);
    let truncated = format!("{}...", &label[..safe_end]);
    let span = Span::styled(truncated, style);
    buf.set_span(cell_rect.x, cell_rect.y, &span, cell_rect.width);
}

// ---------------------------------------------------------------------------
// Coordinate helpers
// ---------------------------------------------------------------------------

/// Convert a [`streemap::Rect<f32>`] (relative to top-left = 0,0) into a
/// [`ratatui::layout::Rect`] offset by `container`'s origin, clamped to container bounds.
///
/// Coordinates: `floor()` for x/y (move inward), `ceil()` for right/bottom
/// (move outward slightly) so adjacent cells share edges exactly.
fn f32_rect_to_ratatui(r: SRect<f32>, container: Rect) -> Rect {
    // cast_possible_truncation / cast_sign_loss: values are clamped to [0, u16::MAX] before cast.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let x_rel = r.x.floor().clamp(0.0, f32::from(u16::MAX)) as u16;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let y_rel = r.y.floor().clamp(0.0, f32::from(u16::MAX)) as u16;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let right_rel = (r.x + r.w).ceil().clamp(0.0, f32::from(u16::MAX)) as u16;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let bottom_rel = (r.y + r.h).ceil().clamp(0.0, f32::from(u16::MAX)) as u16;

    let x = container.x.saturating_add(x_rel).min(container.right());
    let y = container.y.saturating_add(y_rel).min(container.bottom());
    let right = container.x.saturating_add(right_rel).min(container.right());
    let bottom = container
        .y
        .saturating_add(bottom_rel)
        .min(container.bottom());

    Rect {
        x,
        y,
        width: right.saturating_sub(x),
        height: bottom.saturating_sub(y),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::ui::app::TreemapState;

    fn render(
        widget: TreemapWidget,
        state: &mut TreemapState,
        width: u16,
        height: u16,
    ) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| {
                f.render_stateful_widget(widget, f.area(), state);
            })
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    #[test]
    fn treemap_renders_proportional_cells() {
        // 75/25 split using directories (category-colored) to verify proportions.
        let widget = TreemapWidget {
            items: vec![
                TreemapItem {
                    label: "large".into(),
                    size: 75,
                    category: FileCategory::Code,
                    is_directory: true,
                    extension: None,
                },
                TreemapItem {
                    label: "small".into(),
                    size: 25,
                    category: FileCategory::Image,
                    is_directory: true,
                    extension: None,
                },
            ],
        };
        let mut state = TreemapState::default();
        let buf = render(widget, &mut state, 80, 24);

        let total = 80_usize * 24_usize;
        let blue_count = buf.content().iter().filter(|c| c.bg == Color::Blue).count();
        assert!(
            blue_count * 100 / total >= 60,
            "expected ≥60% blue cells for 75% item, got {blue_count}/{total}"
        );
    }

    #[test]
    fn treemap_truncates_long_labels() {
        let widget = TreemapWidget {
            items: vec![TreemapItem {
                label: "very_long_filename.rs".into(),
                size: 100,
                category: FileCategory::Code,
                is_directory: false,
                extension: Some("rs".into()),
            }],
        };
        let mut state = TreemapState::default();
        let buf = render(widget, &mut state, 10, 5);
        let content: String = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(
            content.contains("..."),
            "expected '...' for truncated label in narrow cell, got: {content:?}"
        );
    }

    #[test]
    fn treemap_hides_labels_in_tiny_cells() {
        let widget = TreemapWidget {
            items: vec![TreemapItem {
                label: "ab".into(),
                size: 100,
                category: FileCategory::Code,
                is_directory: false,
                extension: None,
            }],
        };
        let mut state = TreemapState::default();
        let buf = render(widget, &mut state, 2, 5);
        let content: String = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        let non_space: String = content.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            non_space.is_empty(),
            "expected no label in 2-wide cell, got: {content:?}"
        );
    }

    #[test]
    fn treemap_colors_by_extension() {
        // Files with different extensions should produce different background colours.
        let widget = TreemapWidget {
            items: vec![
                TreemapItem {
                    label: "code.rs".into(),
                    size: 50,
                    category: FileCategory::Code,
                    is_directory: false,
                    extension: Some("rs".into()),
                },
                TreemapItem {
                    label: "image.png".into(),
                    size: 50,
                    category: FileCategory::Image,
                    is_directory: false,
                    extension: Some("png".into()),
                },
            ],
        };
        let mut state = TreemapState::default();
        let buf = render(widget, &mut state, 40, 20);
        let rs_color = extension_color(Some("rs"));
        let png_color = extension_color(Some("png"));
        assert_ne!(
            rs_color, png_color,
            "different extensions should map to different colours"
        );
        let rs_count = buf.content().iter().filter(|c| c.bg == rs_color).count();
        let png_count = buf.content().iter().filter(|c| c.bg == png_color).count();
        assert!(rs_count > 0, "expected cells with .rs colour");
        assert!(png_count > 0, "expected cells with .png colour");
    }
}
