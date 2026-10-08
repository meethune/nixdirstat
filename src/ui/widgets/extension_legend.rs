//! Extension legend widget — a scrollable list of file extensions
//! sorted by total size, with color swatches matching the treemap palette.

use std::ffi::OsStr;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Widget,
};

use crate::types::{FileCategory, format_size};
use crate::ui::colors::{category_color, is_color_enabled, no_color_fallback};
use crate::ui::tree::ExtensionStat;

/// Scrollable extension legend showing per-extension size and percentage.
///
/// Each row displays: extension name, 2-cell color swatch, formatted size,
/// and percentage of total. The color swatch uses the Okabe-Ito category
/// palette, with `NO_COLOR` grayscale fallback.
pub struct ExtensionLegendWidget<'a> {
    /// Extension statistics, sorted by `total_size` descending.
    pub stats: &'a [ExtensionStat],
    /// Sum of all file sizes (denominator for percentage).
    pub total_size: u64,
    /// Number of rows to skip (for scrolling).
    pub scroll_offset: usize,
}

impl Widget for ExtensionLegendWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() || self.stats.is_empty() {
            return;
        }

        let visible_rows = usize::from(area.height);

        for (i, stat) in self
            .stats
            .iter()
            .skip(self.scroll_offset)
            .take(visible_rows)
            .enumerate()
        {
            let y = area.y.saturating_add(u16::try_from(i).unwrap_or(u16::MAX));
            if y >= area.bottom() {
                break;
            }

            let ext_label = stat.extension.as_deref().map_or_else(
                || rust_i18n::t!("widgets.extension-legend.none").to_string(),
                |e| format!(".{e}"),
            );

            let pct = if self.total_size > 0 {
                #[allow(clippy::cast_precision_loss)]
                // u64→f64: display is 1 decimal place, so mantissa precision loss is invisible
                let p = (stat.total_size as f64 / self.total_size as f64) * 100.0;
                format!("{p:5.1}%")
            } else {
                String::from("    -%")
            };

            let cat = FileCategory::from_extension(stat.extension.as_deref().map(OsStr::new));
            let color = if is_color_enabled() {
                category_color(cat)
            } else {
                no_color_fallback(cat)
            };
            let size_str = format_size(stat.total_size);

            // Fixed suffix: "██ 999.9 MiB  99.9%" = ~20 chars.
            let suffix_width = 20_usize;
            let ext_width = usize::from(area.width).saturating_sub(suffix_width).max(5);

            let spans = vec![
                Span::styled(
                    crate::ui::pad_display_width(&ext_label, ext_width),
                    Style::default().fg(Color::White),
                ),
                Span::styled("██ ", Style::default().fg(color)),
                Span::styled(format!("{size_str:>9}"), Style::default().fg(Color::White)),
                Span::styled(format!(" {pct}"), Style::default().fg(Color::DarkGray)),
            ];

            let line = Line::from(spans);
            let line_width = area.width;
            buf.set_line(area.x, y, &line, line_width);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    fn make_stat(ext: Option<&str>, total_size: u64) -> ExtensionStat {
        ExtensionStat {
            extension: ext.map(String::from),
            count: 1,
            total_size,
        }
    }

    fn render_legend(stats: &[ExtensionStat], total: u64, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| {
                let widget = ExtensionLegendWidget {
                    stats,
                    total_size: total,
                    scroll_offset: 0,
                };
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
    fn legend_renders_extension_names() {
        let stats = [make_stat(Some("py"), 1024), make_stat(Some("rs"), 512)];
        let content = render_legend(&stats, 1536, 40, 5);
        assert!(content.contains("py"), "missing .py: {content:?}");
        assert!(content.contains("rs"), "missing .rs: {content:?}");
    }

    #[test]
    fn legend_renders_sizes() {
        let stats = [make_stat(Some("bin"), 1_048_576)];
        let content = render_legend(&stats, 1_048_576, 40, 5);
        assert!(content.contains("MiB"), "missing size: {content:?}");
    }

    #[test]
    fn legend_renders_percentages() {
        let stats = [make_stat(Some("rs"), 500)];
        let content = render_legend(&stats, 1000, 40, 5);
        assert!(content.contains("50"), "missing 50%: {content:?}");
    }

    #[test]
    fn legend_renders_color_swatches() {
        let stats = [make_stat(Some("py"), 1024)];
        let backend = TestBackend::new(40, 5);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| {
                let widget = ExtensionLegendWidget {
                    stats: &stats,
                    total_size: 1024,
                    scroll_offset: 0,
                };
                f.render_widget(widget, f.area());
            })
            .expect("draw");
        let buf = terminal.backend().buffer().clone();
        let py_category = FileCategory::from_extension(Some(OsStr::new("py")));
        let py_color = category_color(py_category);
        let has_color = buf.content().iter().any(|c| c.fg == py_color);
        assert!(has_color, "expected swatch with .py color");
    }

    #[test]
    fn legend_empty_stats() {
        let content = render_legend(&[], 0, 40, 5);
        let non_space: String = content.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(non_space.is_empty(), "expected empty render: {content:?}");
    }

    #[test]
    fn legend_sorted_order() {
        let stats = [make_stat(Some("big"), 1000), make_stat(Some("small"), 100)];
        let content = render_legend(&stats, 1100, 40, 5);
        let big_pos = content.find("big").unwrap_or(usize::MAX);
        let small_pos = content.find("small").unwrap_or(usize::MAX);
        assert!(
            big_pos < small_pos,
            "largest should be first: big@{big_pos} small@{small_pos}"
        );
    }

    #[test]
    fn legend_uses_category_colors() {
        let stats = [make_stat(Some("rs"), 1024)];
        let backend = TestBackend::new(40, 5);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| {
                let widget = ExtensionLegendWidget {
                    stats: &stats,
                    total_size: 1024,
                    scroll_offset: 0,
                };
                f.render_widget(widget, f.area());
            })
            .expect("draw");
        let buf = terminal.backend().buffer().clone();
        let code_color = crate::ui::colors::category_color(crate::types::FileCategory::Code);
        let has_color = buf.content().iter().any(|c| c.fg == code_color);
        assert!(has_color, "expected Okabe-Ito blue swatch for .rs");
    }
}
