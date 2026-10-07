//! Explorer view: the main interactive view shown after a scan completes.
//!
//! Layout (three panels):
//!
//! ```text
//! ┌─ /path/to/root ────────────────────────────────────────┐
//! │┌─ Directory Tree ──────┐┌─ Extensions ────────────────┐│
//! ││ ▸ src       450 KiB   ││ .rs  ██  300 KiB   30.0%    ││
//! ││ ▸ docs      200 KiB   ││ .py  ██  150 KiB   15.0%    ││
//! │└───────────────────────┘└─────────────────────────────┘│
//! │┌─ Disk Usage ─────────────────────────────────────────┐│
//! ││ [recursive squarified treemap]                        ││
//! │└──────────────────────────────────────────────────────┘│
//! └────────────────────────────────────────────────────────┘
//! ```

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::ui::{
    app::{ExplorerState, PanelFocus},
    tree::find_node,
    widgets::{
        dir_tree::{TreeDisplayOpts, render_dir_tree},
        extension_legend::ExtensionLegendWidget,
        treemap::TreemapWidget,
    },
};

const MIN_TERMINAL_WIDTH: u16 = 40;
const MIN_TERMINAL_HEIGHT: u16 = 12;
const HELP_POPUP_WIDTH: u16 = 52;
const HELP_POPUP_HEIGHT: u16 = 32;
const INFO_POPUP_WIDTH: u16 = 60;
const WARNINGS_POPUP_WIDTH: u16 = 72;
const WARNINGS_POPUP_HEIGHT: u16 = 20;
const PREVIEW_POPUP_WIDTH: u16 = 80;
const PREVIEW_MIN_HEIGHT: u16 = 5;
const POPUP_MARGIN: u16 = 4;

/// Render the full explorer view into `area`.
pub fn render_explorer(frame: &mut Frame<'_>, state: &mut ExplorerState, area: Rect) {
    if area.width < MIN_TERMINAL_WIDTH || area.height < MIN_TERMINAL_HEIGHT {
        frame.render_widget(Paragraph::new("Terminal too small (need 40×12)"), area);
        return;
    }

    let inner = render_outer_block(frame, state, area);
    let focus = state.focus();

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(inner);

    render_top_panels(frame, state, vertical[0], focus);
    render_treemap_section(frame, state, vertical[1], focus);

    if state.show_info() {
        render_info_popup(frame, state, inner);
    }
    if state.show_help() {
        render_help_overlay(frame, inner);
    }
    if state.show_warnings() {
        render_warnings_popup(frame, state, inner);
    }
    if state.show_preview() {
        render_preview_popup(frame, state, inner);
    }
    if let Some(msg) = state.error_message() {
        let error_area = Rect {
            x: inner.x,
            y: inner.bottom().saturating_sub(1),
            width: inner.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(msg.to_owned()).style(Style::default().fg(Color::White).bg(Color::Red)),
            error_area,
        );
    }
}

/// Render the outer border block and return the inner area.
///
/// The outer block title shows only the scan root path.  The inner breadcrumb
/// bar (rendered by [`render_breadcrumb_bar`]) shows the full drilled-down
/// path with per-segment styling, eliminating the redundancy that existed when
/// both showed the same `breadcrumb_path()` string.
fn render_outer_block(frame: &mut Frame<'_>, state: &ExplorerState, area: Rect) -> Rect {
    let warning_count = state.warnings().len();
    let mut title_spans = vec![Span::raw(format!(" {} ", state.scan_root().display()))];
    if warning_count > 0 {
        title_spans.push(Span::styled(
            format!(
                " {warning_count} warning{} (w) ",
                if warning_count == 1 { "" } else { "s" }
            ),
            Style::default().fg(Color::Black).bg(Color::Yellow),
        ));
    }
    if state.filesystem_changed() {
        title_spans.push(Span::styled(
            " Changed — R to refresh ",
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ));
    }
    let title = Line::from(title_spans);
    let mut outer = Block::default().borders(Borders::ALL).title(title);
    if let Some(space) = state.free_space() {
        use std::fmt::Write as _;
        let mut info = format!(
            " Free: {} / {}",
            crate::types::format_size(space.free_bytes),
            crate::types::format_size(space.total_bytes),
        );
        if space.unknown_bytes > 0 {
            let _ = write!(
                info,
                " | {} unknown",
                crate::types::format_size(space.unknown_bytes),
            );
        }
        info.push(' ');
        outer = outer.title_top(
            ratatui::text::Line::from(info).alignment(ratatui::layout::Alignment::Right),
        );
    }
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    inner
}

/// Render the tree + legend panels in `top_area`.
fn render_top_panels(
    frame: &mut Frame<'_>,
    state: &mut ExplorerState,
    top_area: Rect,
    focus: PanelFocus,
) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(top_area);

    let tree_block = Block::default()
        .borders(Borders::ALL)
        .title(" Directory Tree ")
        .border_style(Style::default().fg(panel_border_color(focus, PanelFocus::Tree)));
    let tree_inner = tree_block.inner(cols[0]);
    frame.render_widget(tree_block, cols[0]);

    let treemap_root = state.treemap_root().to_vec();
    let sort_field = state.sort_field();
    let sort_ascending = state.sort_ascending();
    let search_query = state.search_query().to_owned();
    let filter = if search_query.is_empty() {
        None
    } else {
        Some(search_query.as_str())
    };
    let (tree, tree_state) = state.tree_and_tree_state_mut();
    let tm_node = find_node(tree, &treemap_root).unwrap_or(tree);
    let total_size = tm_node.size;
    let tree_opts = TreeDisplayOpts {
        focused: focus == PanelFocus::Tree,
        sort_field,
        sort_ascending,
        filter,
    };
    render_dir_tree(frame, tm_node, tree_state, tree_inner, &tree_opts);

    if state.search_active() || !state.search_query().is_empty() {
        render_search_bar(frame, state, cols[0]);
    }

    let legend_block = Block::default()
        .borders(Borders::ALL)
        .title(" Extensions ")
        .border_style(Style::default().fg(panel_border_color(focus, PanelFocus::Legend)));
    let legend_inner = legend_block.inner(cols[1]);
    frame.render_widget(legend_block, cols[1]);
    frame.render_widget(
        ExtensionLegendWidget {
            stats: state.extension_stats(),
            total_size,
            scroll_offset: state.legend_scroll(),
        },
        legend_inner,
    );
}

/// Render the treemap section: border, breadcrumb bar, treemap content, and status bar.
fn render_treemap_section(
    frame: &mut Frame<'_>,
    state: &mut ExplorerState,
    area: Rect,
    focus: PanelFocus,
) {
    let treemap_block = Block::default()
        .borders(Borders::ALL)
        .title(" Disk Usage ")
        .border_style(Style::default().fg(panel_border_color(focus, PanelFocus::Treemap)));
    let inner = treemap_block.inner(area);
    frame.render_widget(treemap_block, area);

    let sel_cell = {
        let ts = state.treemap_state();
        ts.selected_index
            .and_then(|i| ts.layout.cells.get(i).cloned())
    };

    let (bc_area, content_area, st_area) = split_treemap_inner(inner);

    if let Some(a) = bc_area {
        render_breadcrumb_bar(frame, state, a);
    }

    let treemap_root = state.treemap_root().to_vec();
    let (tree, treemap_state) = state.tree_and_treemap_state_mut();
    let tm_node = find_node(tree, &treemap_root).unwrap_or(tree);
    frame.render_stateful_widget(TreemapWidget { root: tm_node }, content_area, treemap_state);

    if let (Some(a), Some(cell)) = (st_area, sel_cell) {
        render_treemap_status_bar(frame, &cell, state.scan_root(), state.treemap_root(), a);
    }
}

/// Split `area` into (breadcrumb, content, status) sub-areas.
///
/// When `area.height < 3` no breadcrumb or status bar is allocated.
fn split_treemap_inner(area: Rect) -> (Option<Rect>, Rect, Option<Rect>) {
    if area.height < 3 {
        return (None, area, None);
    }
    let sub = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);
    (Some(sub[0]), sub[1], Some(sub[2]))
}

/// Return the border color for `panel` based on which panel has `focus`.
const fn panel_border_color(focus: PanelFocus, panel: PanelFocus) -> Color {
    if focus as u8 == panel as u8 {
        Color::Cyan
    } else {
        Color::Indexed(240)
    }
}

/// Render the breadcrumb bar: styled path segments, final segment bold, parents dim.
fn render_breadcrumb_bar(frame: &mut Frame<'_>, state: &ExplorerState, area: Rect) {
    let path = state.breadcrumb_path();
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let mut spans: Vec<Span<'_>> = Vec::new();

    if path.starts_with('/') {
        if segments.is_empty() {
            // Root "/" alone.
            spans.push(Span::styled(
                "/",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            // Prefix "/" before the first segment.
            spans.push(Span::styled("/", Style::default().fg(Color::DarkGray)));
        }
    }

    let last_idx = segments.len().saturating_sub(1);
    for (i, seg) in segments.iter().enumerate() {
        if i == last_idx {
            spans.push(Span::styled(
                seg.to_string(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                seg.to_string(),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            ));
            spans.push(Span::styled("/", Style::default().fg(Color::DarkGray)));
        }
    }

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

/// Convert a [`std::time::SystemTime`] to a `YYYY-MM-DD` string.
///
/// Uses the Euclidean affine civil-date algorithm (correct for all dates since
/// the Unix epoch). Returns `"---"` if `mtime` predates the Unix epoch.
fn format_mtime(mtime: std::time::SystemTime) -> String {
    let Ok(dur) = mtime.duration_since(std::time::SystemTime::UNIX_EPOCH) else {
        return "---".to_string();
    };
    let days = (dur.as_secs() / 86_400).cast_signed();
    // Civil date from days since epoch (Euclidean affine algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Render the treemap status bar showing the keyboard-selected cell's info.
///
/// Format: `▸ filename  |  size  |  YYYY-MM-DD  |  /full/path`
fn render_treemap_status_bar(
    frame: &mut Frame<'_>,
    cell: &crate::ui::widgets::treemap::CellLayout,
    scan_root: &std::path::Path,
    treemap_root: &[String],
    area: Rect,
) {
    let date_str = format_mtime(cell.mtime);

    let mut full_path = std::path::PathBuf::from(scan_root);
    for segment in treemap_root {
        full_path.push(segment);
    }
    for segment in &cell.path {
        full_path.push(segment);
    }
    let path_display = full_path.display().to_string();

    let info_line = Line::from(vec![
        Span::styled(
            format!(" \u{25b8} {}", cell.name),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  |  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            crate::types::format_size(cell.size),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled("  |  ", Style::default().fg(Color::DarkGray)),
        Span::styled(date_str, Style::default().fg(Color::White)),
        Span::styled("  |  ", Style::default().fg(Color::DarkGray)),
        Span::styled(path_display, Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
    ]);

    frame.render_widget(
        Paragraph::new(info_line).style(Style::default().bg(Color::Indexed(236))),
        area,
    );
}

fn render_search_bar(frame: &mut Frame<'_>, state: &ExplorerState, tree_area: Rect) {
    let bar_area = Rect {
        x: tree_area.x,
        y: tree_area.bottom().saturating_sub(1),
        width: tree_area.width,
        height: 1,
    };
    let cursor = if state.search_active() { "▏" } else { "" };
    let query = state.search_query();
    let text = format!("/{query}{cursor}");
    let style = if state.search_active() {
        Style::default().fg(Color::Yellow).bg(Color::Black)
    } else {
        Style::default().fg(Color::DarkGray).bg(Color::Black)
    };
    frame.render_widget(Paragraph::new(text).style(style), bar_area);
}

fn render_help_overlay(frame: &mut Frame<'_>, area: Rect) {
    let help_width = HELP_POPUP_WIDTH.min(area.width.saturating_sub(POPUP_MARGIN));
    let help_height = HELP_POPUP_HEIGHT.min(area.height.saturating_sub(POPUP_MARGIN));
    let help_area = Rect {
        x: area.x + (area.width.saturating_sub(help_width)) / 2,
        y: area.y + (area.height.saturating_sub(help_height)) / 2,
        width: help_width,
        height: help_height,
    };

    frame.render_widget(Clear, help_area);

    let help_text = vec![
        Line::from(vec![Span::styled(
            " NixDirStat — Keybindings ",
            Style::default().fg(Color::Yellow),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            " Tree panel:",
            Style::default().fg(Color::Cyan),
        )]),
        Line::from(" ↑/↓ j/k        Navigate tree"),
        Line::from(" →/l/Enter      Go into directory"),
        Line::from(" ←/h/Bksp/u    Go up / collapse"),
        Line::from(" PgUp/PgDn      Page scroll"),
        Line::from(" Home/g         Jump to first"),
        Line::from(" End/G          Jump to last"),
        Line::from(" n/s/m          Sort name/size/mtime"),
        Line::from(" r              Reverse sort"),
        Line::from(" Z              Zoom to root"),
        Line::from(" i              File info popup"),
        Line::from(" v              Preview file"),
        Line::from(" /              Search / filter"),
        Line::from(""),
        Line::from(vec![Span::styled(
            " Treemap panel:",
            Style::default().fg(Color::Cyan),
        )]),
        Line::from(" ↑/↓/←/→ h/j/k/l  Navigate cells"),
        Line::from(" Enter          Drill into directory"),
        Line::from(" Backspace      Zoom out one level"),
        Line::from(" Esc            Return to tree panel"),
        Line::from(""),
        Line::from(vec![Span::styled(
            " Global:",
            Style::default().fg(Color::Cyan),
        )]),
        Line::from(" Tab            Cycle panels"),
        Line::from(" ?              Toggle this help"),
        Line::from(" w              Scan warnings"),
        Line::from(" R              Refresh (re-scan)"),
        Line::from(" q/Ctrl-C       Quit"),
    ];

    let help = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title(" Help "))
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::White).bg(Color::Black));

    frame.render_widget(help, help_area);
}

/// Render the file info popup for the currently selected node.
fn render_info_popup(frame: &mut Frame<'_>, state: &ExplorerState, area: Rect) {
    let selected = state.tree_state().selected();
    if selected.is_empty() {
        return;
    }

    let mut full_path = state.scan_root().to_path_buf();
    for component in state.treemap_root() {
        full_path.push(component);
    }
    for component in selected {
        full_path.push(component);
    }

    let mut lookup_path = state.treemap_root().to_vec();
    lookup_path.extend(selected.iter().cloned());
    let Some(node) = find_node(state.tree(), &lookup_path) else {
        return;
    };

    let mtime_str = {
        let date = format_mtime(node.mtime);
        if date == "---" {
            date
        } else {
            // Append HH:MM from the raw seconds within the day.
            let secs = node
                .mtime
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let hour = (secs % 86_400) / 3600;
            let minute = (secs % 3600) / 60;
            format!("{date} {hour:02}:{minute:02}")
        }
    };

    let kind = if node.is_dir { "Directory" } else { "File" };
    let ext_str = node
        .extension
        .as_deref()
        .map_or_else(|| "(none)".to_string(), |e| format!(".{e}"));

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Path:      ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                full_path.display().to_string(),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Size:      ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                crate::types::format_size(node.size),
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Allocated: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                crate::types::format_size(node.allocated),
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Type:      ", Style::default().fg(Color::DarkGray)),
            Span::styled(kind, Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled("  Extension: ", Style::default().fg(Color::DarkGray)),
            Span::styled(ext_str, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  Modified:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(mtime_str, Style::default().fg(Color::White)),
        ]),
    ];

    if node.is_dir {
        lines.push(Line::from(vec![
            Span::styled("  Children:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                node.children.len().to_string(),
                Style::default().fg(Color::White),
            ),
        ]));
    }

    let popup_width = INFO_POPUP_WIDTH.min(area.width.saturating_sub(POPUP_MARGIN));
    let popup_height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_add(3)
        .min(area.height.saturating_sub(POPUP_MARGIN));
    let popup_area = Rect {
        x: area.x + (area.width.saturating_sub(popup_width)) / 2,
        y: area.y + (area.height.saturating_sub(popup_height)) / 2,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup_area);
    let popup = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" File Info "))
        .style(Style::default().fg(Color::White).bg(Color::Black));
    frame.render_widget(popup, popup_area);
}

/// Render the warnings popup listing scan warnings with scrollable navigation.
fn render_warnings_popup(frame: &mut Frame<'_>, state: &mut ExplorerState, area: Rect) {
    if state.warnings().is_empty() {
        return;
    }

    let popup_width = WARNINGS_POPUP_WIDTH.min(area.width.saturating_sub(POPUP_MARGIN));
    let popup_height = WARNINGS_POPUP_HEIGHT.min(area.height.saturating_sub(POPUP_MARGIN));
    let popup_area = Rect {
        x: area.x + (area.width.saturating_sub(popup_width)) / 2,
        y: area.y + (area.height.saturating_sub(popup_height)) / 2,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup_area);

    let inner_height = popup_height.saturating_sub(2) as usize;
    state.set_warnings_viewport(inner_height);
    let warning_count = state.warnings().len();
    let scroll = state.warnings_scroll();

    let warnings = state.warnings();
    let visible = &warnings[scroll..warning_count.min(scroll + inner_height)];

    let mut lines: Vec<Line<'_>> = Vec::with_capacity(visible.len());
    for w in visible {
        let path_str = w.path.display().to_string();
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {path_str}: "),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(w.message.clone(), Style::default().fg(Color::White)),
        ]));
    }

    let end = warning_count.min(scroll + inner_height);
    let title = format!(
        " Scan Warnings ({}-{} of {}) ",
        scroll + 1,
        end,
        warning_count
    );
    let popup = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(title))
        .style(Style::default().fg(Color::White).bg(Color::Black));

    frame.render_widget(popup, popup_area);
}

fn render_preview_popup(frame: &mut Frame<'_>, state: &mut ExplorerState, area: Rect) {
    let popup_width = PREVIEW_POPUP_WIDTH.min(area.width.saturating_sub(POPUP_MARGIN));
    let popup_height = area
        .height
        .saturating_sub(POPUP_MARGIN)
        .max(PREVIEW_MIN_HEIGHT);
    let popup_area = Rect {
        x: area.x + (area.width.saturating_sub(popup_width)) / 2,
        y: area.y + (area.height.saturating_sub(popup_height)) / 2,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup_area);

    let inner_height = popup_height.saturating_sub(2) as usize;
    state.set_preview_viewport(inner_height);
    let content = state.preview_content();
    let scroll = state.preview_scroll();
    let end = content.len().min(scroll + inner_height);

    let lines: Vec<Line<'_>> = content[scroll..end]
        .iter()
        .map(|s| Line::from(s.as_str()))
        .collect();

    let title = if content.len() <= inner_height {
        format!(" {} ", state.preview_title())
    } else {
        format!(
            " {} ({}-{} of {}) ",
            state.preview_title(),
            scroll + 1,
            end,
            content.len(),
        )
    };
    let popup = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(title))
        .style(Style::default().fg(Color::White).bg(Color::Black));

    frame.render_widget(popup, popup_area);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{
        app::ExplorerState,
        tree::test_fixtures::{make_dir, make_file},
    };
    use ratatui::{Terminal, backend::TestBackend};
    use std::path::PathBuf;

    fn make_test_state() -> ExplorerState {
        let tree = make_dir(
            "root",
            vec![
                make_dir(
                    "src",
                    vec![make_file("main.rs", 1000), make_file("lib.rs", 2000)],
                ),
                make_file("readme.md", 500),
            ],
        );
        ExplorerState::new(tree, PathBuf::from("/test"))
    }

    fn render_to_string(state: &mut ExplorerState, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render_explorer(f, state, f.area()))
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
    fn explorer_renders_breadcrumb() {
        let mut state = make_test_state();
        let content = render_to_string(&mut state, 120, 40);
        assert!(
            content.contains("/test"),
            "expected breadcrumb '/test' in buffer"
        );
    }

    #[test]
    fn explorer_renders_panel_titles() {
        let mut state = make_test_state();
        let content = render_to_string(&mut state, 120, 40);
        assert!(
            content.contains("Directory Tree"),
            "expected 'Directory Tree' in buffer"
        );
        assert!(
            content.contains("Extensions"),
            "expected 'Extensions' in buffer"
        );
        assert!(
            content.contains("Disk Usage"),
            "expected 'Disk Usage' in buffer"
        );
    }

    #[test]
    fn explorer_too_small_shows_message() {
        let mut state = make_test_state();
        let content = render_to_string(&mut state, 30, 8);
        assert!(
            content.contains("too small"),
            "expected 'too small' in buffer: {content:?}"
        );
    }

    #[test]
    fn help_overlay_renders_keybindings() {
        let mut state = make_test_state();
        state.toggle_show_help();
        let content = render_to_string(&mut state, 120, 40);
        assert!(
            content.contains("Keybindings"),
            "expected 'Keybindings' in buffer"
        );
    }

    #[test]
    fn info_popup_renders_file_details() {
        let mut state = make_test_state();
        state.tree_state_mut().select(vec!["src".to_owned()]);
        state.toggle_show_info();
        let content = render_to_string(&mut state, 120, 40);
        assert!(
            content.contains("File Info"),
            "expected 'File Info' in buffer"
        );
    }

    #[test]
    fn explorer_renders_breadcrumb_bar() {
        let mut state = make_test_state();
        let content = render_to_string(&mut state, 120, 40);
        assert!(content.contains("/test"), "expected breadcrumb in output");
    }

    #[test]
    fn explorer_treemap_panel_title_changes_color_with_focus() {
        use crate::ui::app::PanelFocus;
        let mut state = make_test_state();
        state.cycle_focus(); // Tree → Treemap
        assert_eq!(state.focus(), PanelFocus::Treemap);

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render_explorer(f, &mut state, f.area()))
            .expect("draw");
        let buf = terminal.backend().buffer().clone();
        // "Disk Usage" title should be cyan when treemap is focused.
        let has_cyan_disk = buf
            .content()
            .iter()
            .any(|c| c.symbol() == "D" && c.fg == Color::Cyan);
        assert!(
            has_cyan_disk,
            "expected cyan Disk Usage title when treemap focused"
        );
    }

    #[test]
    fn explorer_tree_panel_border_gray_when_treemap_focused() {
        use crate::ui::app::PanelFocus;
        let mut state = make_test_state();
        state.cycle_focus(); // Tree → Treemap
        assert_eq!(state.focus(), PanelFocus::Treemap);

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render_explorer(f, &mut state, f.area()))
            .expect("draw");
        let buf = terminal.backend().buffer().clone();
        // Tree panel border chars (corner/line) should be gray (Indexed 240) when unfocused.
        let has_gray_border = buf
            .content()
            .iter()
            .any(|c| c.fg == Color::Indexed(240) && (c.symbol() == "─" || c.symbol() == "│"));
        assert!(
            has_gray_border,
            "expected gray border chars on unfocused panels"
        );
    }

    #[test]
    fn help_overlay_includes_treemap_keys() {
        let mut state = make_test_state();
        state.toggle_show_help();
        let content = render_to_string(&mut state, 120, 40);
        assert!(
            content.contains("Treemap"),
            "expected 'Treemap' section in help overlay"
        );
    }

    #[test]
    fn explorer_status_bar_shows_treemap_selection() {
        use crate::ui::widgets::treemap::{CellLayout, TreemapLayout};
        use std::time::SystemTime;

        let mut state = make_test_state();
        // Inject a fake layout with one cell so the status bar has data to show.
        let fake_cell = CellLayout {
            rect: ratatui::layout::Rect::new(0, 0, 20, 4),
            name: "main.rs".to_owned(),
            extension: Some("rs".to_owned()),
            is_dir: false,
            size: 1000,
            mtime: SystemTime::UNIX_EPOCH,
            path: vec!["main.rs".to_owned()],
        };
        state.treemap_state_mut().layout = TreemapLayout {
            cells: vec![fake_cell],
        };
        state.treemap_state_mut().selected_index = Some(0);

        let content = render_to_string(&mut state, 120, 40);
        assert!(
            content.contains("main.rs"),
            "expected 'main.rs' in status bar output"
        );
    }

    #[test]
    fn explorer_shows_free_and_unknown_space() {
        let mut state = make_test_state();
        state.set_free_space(Some(crate::types::SpaceInfo {
            total_bytes: 500_000_000_000,
            free_bytes: 200_000_000_000,
            unknown_bytes: 50_000_000_000,
        }));
        let content = render_to_string(&mut state, 120, 40);
        assert!(
            content.contains("Free:"),
            "expected 'Free:' in header: {content:?}"
        );
        assert!(
            content.contains("unknown"),
            "expected 'unknown' in header: {content:?}"
        );
    }

    #[test]
    fn explorer_hides_unknown_when_zero() {
        let mut state = make_test_state();
        state.set_free_space(Some(crate::types::SpaceInfo {
            total_bytes: 100_000_000_000,
            free_bytes: 50_000_000_000,
            unknown_bytes: 0,
        }));
        let content = render_to_string(&mut state, 120, 40);
        assert!(content.contains("Free:"), "expected 'Free:' in header");
        assert!(
            !content.contains("unknown"),
            "expected no 'unknown' when unknown_bytes is 0"
        );
    }
}
