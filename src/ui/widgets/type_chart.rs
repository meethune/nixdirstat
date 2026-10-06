//! File-type bar chart widget.
//!
//! Renders a [`BarChart`] showing disk usage broken down by [`FileCategory`].
//! Each bar uses the same colour palette as the treemap widget and shows the
//! formatted size as its value label.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::Line,
    widgets::{Bar, BarChart, BarGroup, Widget},
};

use crate::types::{TypeStat, format_size};

use super::treemap::category_color;

// ---------------------------------------------------------------------------
// TypeChartWidget
// ---------------------------------------------------------------------------

/// Bar chart widget that displays per-category disk usage.
///
/// Each bar is labelled with the category name and shows the total allocated
/// size as a formatted string (e.g. `"1.5 GiB"`).
pub struct TypeChartWidget<'a> {
    /// Category statistics to chart, sorted by the caller (typically largest first).
    pub type_stats: &'a [TypeStat],
}

impl Widget for TypeChartWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() || self.type_stats.is_empty() {
            return;
        }

        let bars: Vec<Bar<'_>> = self
            .type_stats
            .iter()
            .map(|ts| {
                // Scale value to KiB for the bar height; show the formatted size as text.
                let value_kib = ts.total_size / 1_024;
                Bar::default()
                    .value(value_kib)
                    .text_value(format_size(ts.total_size))
                    .label(Line::from(ts.category.to_string()))
                    .style(Style::default().fg(category_color(ts.category)))
            })
            .collect();

        let bar_group = BarGroup::default().bars(&bars);

        // bar_count fits in u16: FileCategory has < 20 variants.
        #[allow(clippy::cast_possible_truncation)]
        let bar_count = (bars.len().max(1)) as u16;
        let available = area.width.saturating_sub(bar_count.saturating_sub(1));
        let bar_width = (available / bar_count).clamp(3, 9);

        let chart = BarChart::default()
            .data(bar_group)
            .bar_width(bar_width)
            .bar_gap(1);

        Widget::render(chart, area, buf);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::types::FileCategory;

    fn make_stat(category: FileCategory, total_size: u64) -> TypeStat {
        TypeStat {
            category,
            count: 1,
            total_size,
            total_allocated: total_size,
        }
    }

    fn render_chart(stats: &[TypeStat], width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| {
                let widget = TypeChartWidget { type_stats: stats };
                f.render_widget(widget, f.area());
            })
            .expect("draw");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    #[test]
    fn type_chart_renders_categories() {
        let stats = [
            make_stat(FileCategory::Code, 2 * 1024 * 1024),
            make_stat(FileCategory::Image, 1024 * 1024),
        ];
        let content = render_chart(&stats, 80, 24);
        assert!(
            content.contains("Code"),
            "expected 'Code' category label in chart: {content:?}"
        );
        assert!(
            content.contains("Image"),
            "expected 'Image' category label in chart: {content:?}"
        );
    }

    #[test]
    fn type_chart_shows_sizes() {
        let stats = [make_stat(FileCategory::Code, 1024 * 1024)];
        let content = render_chart(&stats, 80, 24);
        // format_size(1024*1024) == "1.0 MiB"; check partial match since bars may truncate.
        assert!(
            content.contains("MiB") || content.contains("KiB"),
            "expected formatted size in chart: {content:?}"
        );
    }
}
