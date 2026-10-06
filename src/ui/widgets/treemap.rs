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
// Category colour palette
// ---------------------------------------------------------------------------

/// Return the terminal colour associated with a [`FileCategory`].
///
/// The palette is designed to be distinguishable in both light and dark
/// terminals. Binary and unknown categories use low-contrast colours to
/// de-emphasise them visually.
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
        // FileCategory::Other and any future variants → white
        _ => Color::White,
    }
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
    /// Content category, determines the cell's background colour.
    pub category: FileCategory,
    /// Whether this entry is a directory.
    pub is_directory: bool,
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

            let color = category_color(item.category);
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
        // 75/25 split — the larger item (Code=Blue) should occupy ≥60% of cells.
        let widget = TreemapWidget {
            items: vec![
                TreemapItem {
                    label: "large".into(),
                    size: 75,
                    category: FileCategory::Code,
                    is_directory: false,
                },
                TreemapItem {
                    label: "small".into(),
                    size: 25,
                    category: FileCategory::Image,
                    is_directory: false,
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
        // Put one item in a 20-wide area; the long label should be truncated to "...".
        let widget = TreemapWidget {
            items: vec![TreemapItem {
                label: "very_long_filename.rs".into(),
                size: 100,
                category: FileCategory::Code,
                is_directory: false,
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
        // A 2-wide cell must show no text label.
        let widget = TreemapWidget {
            items: vec![TreemapItem {
                label: "ab".into(),
                size: 100,
                category: FileCategory::Code,
                is_directory: false,
            }],
        };
        let mut state = TreemapState::default();
        let buf = render(widget, &mut state, 2, 5);
        let content: String = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        // No non-space characters should appear (the label is hidden).
        let non_space: String = content.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            non_space.is_empty(),
            "expected no label in 2-wide cell, got: {content:?}"
        );
    }

    #[test]
    fn treemap_colors_by_category() {
        // Code item → Blue; Image item → Green; they should produce different bg colours.
        let widget = TreemapWidget {
            items: vec![
                TreemapItem {
                    label: "code_file".into(),
                    size: 50,
                    category: FileCategory::Code,
                    is_directory: false,
                },
                TreemapItem {
                    label: "image_file".into(),
                    size: 50,
                    category: FileCategory::Image,
                    is_directory: false,
                },
            ],
        };
        let mut state = TreemapState::default();
        let buf = render(widget, &mut state, 40, 20);
        let blue_count = buf.content().iter().filter(|c| c.bg == Color::Blue).count();
        let green_count = buf
            .content()
            .iter()
            .filter(|c| c.bg == Color::Green)
            .count();
        assert!(blue_count > 0, "expected blue cells for Code category");
        assert!(green_count > 0, "expected green cells for Image category");
    }
}
