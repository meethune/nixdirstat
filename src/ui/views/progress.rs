//! Scan progress view.
//!
//! Renders the full-screen progress display shown while a filesystem scan
//! is in progress: a title row, an animated line gauge, and live statistics.

use std::time::Duration;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{LineGauge, Paragraph},
};

use crate::ui::app::ScanProgressState;

/// Format a [`Duration`] as `M:SS` (or `H:MM:SS` for scans longer than an hour).
fn format_elapsed(elapsed: Duration) -> String {
    let total_secs = elapsed.as_secs();
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    if hours > 0 {
        format!("{hours}:{mins:02}:{secs:02}")
    } else {
        format!("{mins}:{secs:02}")
    }
}

/// Render the scan progress view into `area`.
///
/// Displays:
/// - A `"Scanning..."` title row (with a `"ROOT"` badge in the top-right corner
///   when [`ScanProgressState::is_root`] is `true`)
/// - An animated [`LineGauge`] labelled with the current file count
/// - Per-second rate, elapsed time, and the most recently scanned path
pub fn render_progress(frame: &mut Frame, state: &ScanProgressState, paused: bool, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title row
            Constraint::Length(1), // blank
            Constraint::Length(1), // gauge
            Constraint::Length(1), // blank
            Constraint::Length(1), // file count
            Constraint::Length(1), // rate
            Constraint::Length(1), // elapsed
            Constraint::Length(1), // current path
            Constraint::Min(0),    // remainder / padding
        ])
        .split(area);

    // Title row.
    let title = if paused {
        Line::from(vec![
            Span::raw(rust_i18n::t!("progress.scanning-paused-label").to_string()),
            Span::styled(
                rust_i18n::t!("progress.paused").to_string(),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw(rust_i18n::t!("progress.hint.paused").to_string()),
        ])
    } else {
        let label = format!(
            "{}{}",
            rust_i18n::t!("progress.scanning"),
            rust_i18n::t!("progress.hint.running"),
        );
        Line::from(label)
    };
    frame.render_widget(Paragraph::new(title), chunks[0]);

    // ROOT badge — rendered in the top-right corner of the title row.
    if state.is_root {
        let badge_width: u16 = 4;
        if area.width >= badge_width {
            let badge_area = Rect {
                x: area
                    .x
                    .saturating_add(area.width.saturating_sub(badge_width)),
                y: area.y,
                width: badge_width,
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(
                    rust_i18n::t!("progress.root-badge").to_string(),
                    Style::default().bg(Color::Red).fg(Color::White),
                )])),
                badge_area,
            );
        }
    }

    // Animated line gauge.  The total file count is unknown, so we animate the
    // fill ratio using elapsed time to show continuous motion.
    let ratio = if state.elapsed.as_secs_f64() > 0.0 {
        (state.elapsed.as_secs_f64() * 0.2).rem_euclid(1.0)
    } else {
        0.0
    };
    let gauge_label = rust_i18n::t!("progress.files-gauge", count = state.file_count).to_string();
    let gauge = LineGauge::default().ratio(ratio).label(gauge_label);
    frame.render_widget(gauge, chunks[2]);

    // Statistics rows.
    frame.render_widget(
        Paragraph::new(rust_i18n::t!("progress.files-label", count = state.file_count).to_string()),
        chunks[4],
    );
    frame.render_widget(
        Paragraph::new(
            rust_i18n::t!(
                "progress.rate",
                rate = format!("{:.0}", state.files_per_sec)
            )
            .to_string(),
        ),
        chunks[5],
    );
    frame.render_widget(
        Paragraph::new(
            rust_i18n::t!("progress.elapsed", time = format_elapsed(state.elapsed)).to_string(),
        ),
        chunks[6],
    );

    // Current path — truncated with a leading "..." if it does not fit.
    let path_str = state.current_path.display().to_string();
    let prefix = rust_i18n::t!("progress.path-prefix").to_string();
    let available = usize::from(area.width);
    let full = format!("{prefix}{path_str}");
    let display = if full.len() > available {
        let keep = available.saturating_sub(prefix.len() + 3);
        // Use char_indices to find a safe byte offset, avoiding panics on
        // multi-byte characters (same approach as the treemap label code).
        let char_count = path_str.chars().count();
        let skip = char_count.saturating_sub(keep);
        let start = path_str
            .char_indices()
            .nth(skip)
            .map_or(path_str.len(), |(i, _)| i);
        format!("{prefix}...{}", &path_str[start..])
    } else {
        full
    };
    frame.render_widget(Paragraph::new(display), chunks[7]);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::app::ScanProgressState;
    use ratatui::{Terminal, backend::TestBackend};
    use std::path::PathBuf;
    use std::time::Duration;

    fn make_state(
        file_count: u64,
        files_per_sec: f64,
        elapsed: Duration,
        path: &str,
        is_root: bool,
    ) -> ScanProgressState {
        ScanProgressState {
            file_count,
            files_per_sec,
            elapsed,
            current_path: PathBuf::from(path),
            is_root,
        }
    }

    fn render_to_string(state: &ScanProgressState, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render_progress(f, state, false, f.area()))
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
    fn progress_view_renders_file_count() {
        let state = make_state(42, 100.0, Duration::from_secs(5), "/tmp/test", false);
        let content = render_to_string(&state, 80, 24);
        assert!(
            content.contains("42"),
            "expected '42' in buffer: {content:?}"
        );
    }

    #[test]
    fn progress_view_renders_scan_rate() {
        let state = make_state(100, 250.0, Duration::from_secs(1), "/tmp/test", false);
        let content = render_to_string(&state, 80, 24);
        assert!(
            content.contains("files/sec"),
            "expected 'files/sec' in buffer: {content:?}"
        );
    }

    #[test]
    fn progress_view_renders_current_path() {
        let state = make_state(
            10,
            10.0,
            Duration::from_secs(1),
            "/home/user/documents",
            false,
        );
        let content = render_to_string(&state, 80, 24);
        assert!(
            content.contains("documents"),
            "expected 'documents' in buffer: {content:?}"
        );
    }

    #[test]
    fn progress_view_renders_elapsed_time() {
        let state = make_state(1, 1.0, Duration::from_secs(3), "/tmp", false);
        let content = render_to_string(&state, 80, 24);
        assert!(
            content.contains("0:03"),
            "expected '0:03' in buffer: {content:?}"
        );
    }

    #[test]
    fn progress_view_renders_at_minimum_size() {
        let state = make_state(5, 5.0, Duration::from_secs(1), "/tmp", false);
        // Must not panic with a small terminal.
        render_to_string(&state, 40, 10);
    }

    #[test]
    fn root_indicator_shown_for_root_user() {
        let state = make_state(0, 0.0, Duration::ZERO, "/", true);
        let content = render_to_string(&state, 80, 24);
        assert!(
            content.contains("ROOT"),
            "expected 'ROOT' in buffer: {content:?}"
        );
    }
}
