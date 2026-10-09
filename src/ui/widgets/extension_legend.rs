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
use crate::ui::visualization::ColorScheme;

/// Content variant for the legend panel.
///
/// Controls whether the panel shows a scrollable extension list or a
/// gradient scale for the active color scheme.
pub enum LegendContent<'a> {
    /// Extension-based legend (used when [`ColorScheme::FileType`] is active).
    Extensions {
        /// Extension statistics, sorted by `total_size` descending.
        stats: &'a [ExtensionStat],
        /// Sum of all file sizes (denominator for percentage).
        total_size: u64,
        /// Number of rows to skip (for scrolling).
        scroll_offset: usize,
    },
    /// Gradient scale for [`ColorScheme::Mtime`] or [`ColorScheme::Depth`].
    Gradient {
        /// The active color scheme (typically `Mtime` or `Depth`).
        scheme: ColorScheme,
        /// Label for the minimum end of the gradient (oldest date or shallowest depth).
        label_min: String,
        /// Label for the maximum end of the gradient (newest date or deepest depth).
        label_max: String,
    },
}

/// Legend widget that displays either extension statistics or a gradient scale.
///
/// Use [`LegendContent::Extensions`] for the file-type color scheme and
/// [`LegendContent::Gradient`] for mtime or depth color schemes.
///
/// Each extension row displays: extension name, 2-cell color swatch, formatted
/// size, and percentage of total. The gradient variant renders a vertical color
/// bar with min/max labels.
pub struct ExtensionLegendWidget<'a> {
    /// Content to render.
    pub content: LegendContent<'a>,
}

impl Widget for ExtensionLegendWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        match self.content {
            LegendContent::Extensions {
                stats,
                total_size,
                scroll_offset,
            } => render_extensions(stats, total_size, scroll_offset, area, buf),
            LegendContent::Gradient {
                scheme,
                label_min,
                label_max,
            } => render_gradient(scheme, &label_min, &label_max, area, buf),
        }
    }
}

/// Render extension statistics as a scrollable list (the existing behavior).
fn render_extensions(
    stats: &[ExtensionStat],
    total_size: u64,
    scroll_offset: usize,
    area: Rect,
    buf: &mut Buffer,
) {
    if area.is_empty() || stats.is_empty() {
        return;
    }

    let visible_rows = usize::from(area.height);

    for (i, stat) in stats
        .iter()
        .skip(scroll_offset)
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

        let pct = if total_size > 0 {
            #[allow(clippy::cast_precision_loss)]
            // u64→f64: display is 1 decimal place, so mantissa precision loss is invisible
            let p = (stat.total_size as f64 / total_size as f64) * 100.0;
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

/// Render a vertical gradient bar with `label_max` at the top and `label_min` at the bottom.
///
/// - [`ColorScheme::Mtime`]: red (recent) at top → blue (old) at bottom.
/// - [`ColorScheme::Depth`]: dark (deep) at top → light (shallow) at bottom.
/// - [`ColorScheme::FileType`]: not expected for this variant; renders nothing.
fn render_gradient(
    scheme: ColorScheme,
    label_min: &str,
    label_max: &str,
    area: Rect,
    buf: &mut Buffer,
) {
    if area.is_empty() {
        return;
    }

    let height = area.height;

    // Row 0: max label (most recent / deepest).
    let max_line = Line::from(Span::styled(label_max, Style::default().fg(Color::White)));
    buf.set_line(area.x, area.y, &max_line, area.width);

    // Last row: min label (oldest / shallowest), only when a second row exists.
    if height >= 2 {
        let min_y = area.y + height - 1;
        let min_line = Line::from(Span::styled(label_min, Style::default().fg(Color::White)));
        buf.set_line(area.x, min_y, &min_line, area.width);
    }

    // Middle rows: gradient bar (rows 1 .. height-1).
    let bar_start = 1_u16;
    let bar_end = height.saturating_sub(1);
    let bar_rows = bar_end.saturating_sub(bar_start);
    if bar_rows == 0 {
        return;
    }

    let (red_top, grn_top, blu_top, red_bot, grn_bot, blu_bot) = match scheme {
        ColorScheme::Mtime => (220_u8, 50_u8, 30_u8, 30_u8, 100_u8, 200_u8),
        ColorScheme::Depth => (60_u8, 60_u8, 60_u8, 200_u8, 200_u8, 200_u8),
        ColorScheme::FileType => return,
    };

    let bar_str = "█".repeat(usize::from(area.width));
    for row in 0..bar_rows {
        let bar_y = area.y + bar_start + row;
        let red = interp_u8(red_top, red_bot, row, bar_rows);
        let grn = interp_u8(grn_top, grn_bot, row, bar_rows);
        let blu = interp_u8(blu_top, blu_bot, row, bar_rows);
        let line = Line::from(Span::styled(
            bar_str.as_str(),
            Style::default().fg(Color::Rgb(red, grn, blu)),
        ));
        buf.set_line(area.x, bar_y, &line, area.width);
    }
}

/// Linearly interpolate between two `u8` values at step `step` of `num_steps` total steps.
///
/// Returns `from` when `step == 0` and `to` when `step == num_steps − 1`.
/// When `num_steps <= 1`, always returns `from`.
fn interp_u8(from: u8, to: u8, step: u16, num_steps: u16) -> u8 {
    if num_steps <= 1 {
        return from;
    }
    let start_val = i32::from(from);
    let end_val = i32::from(to);
    let cur = i32::from(step);
    let total = i32::from(num_steps - 1);
    let result = start_val + (end_val - start_val) * cur / total;
    // result is in [min(from,to), max(from,to)] ⊆ [0, 255]; clamp is defensive.
    u8::try_from(result.clamp(0, 255)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, layout::Rect, widgets::Widget};

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
                    content: LegendContent::Extensions {
                        stats,
                        total_size: total,
                        scroll_offset: 0,
                    },
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
                    content: LegendContent::Extensions {
                        stats: &stats,
                        total_size: 1024,
                        scroll_offset: 0,
                    },
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
                    content: LegendContent::Extensions {
                        stats: &stats,
                        total_size: 1024,
                        scroll_offset: 0,
                    },
                };
                f.render_widget(widget, f.area());
            })
            .expect("draw");
        let buf = terminal.backend().buffer().clone();
        let code_color = crate::ui::colors::category_color(crate::types::FileCategory::Code);
        let has_color = buf.content().iter().any(|c| c.fg == code_color);
        assert!(has_color, "expected Okabe-Ito blue swatch for .rs");
    }

    #[test]
    fn gradient_legend_renders_min_max_labels() {
        let content = LegendContent::Gradient {
            scheme: ColorScheme::Mtime,
            label_min: "2020-01-01".to_owned(),
            label_max: "2026-10-08".to_owned(),
        };
        let widget = ExtensionLegendWidget { content };
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
        let text: String = buf
            .content()
            .iter()
            .map(|c| c.symbol().to_string())
            .collect();
        assert!(text.contains("2020"), "expected min label");
        assert!(text.contains("2026"), "expected max label");
    }
}
