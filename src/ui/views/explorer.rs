//! Explorer view: the main interactive view shown after a scan completes.
//!
//! Layout (three panels):
//!
//! ```text
//! ┌────────────────────────────────────────────┐
//! │  Treemap (top half)                        │
//! ├──────────────────────┬─────────────────────┤
//! │  File Table (60%)    │  Type Chart (40%)   │
//! └──────────────────────┴─────────────────────┘
//! ```
//!
//! When the terminal is too narrow (< 20 columns) or too short (< 6 rows),
//! each panel collapses gracefully rather than panicking.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};

use crate::{
    types::{FileCategory, FileType},
    ui::{
        app::ExplorerState,
        widgets::{
            file_table::FileTableWidget,
            treemap::{TreemapItem, TreemapWidget},
            type_chart::TypeChartWidget,
        },
    },
};

// ---------------------------------------------------------------------------
// Public render function
// ---------------------------------------------------------------------------

/// Render the full explorer view into `area`.
///
/// Divides `area` into three panels: a treemap spanning the top half, a
/// sortable file table in the bottom-left (60 %), and a category bar chart
/// in the bottom-right (40 %).
pub fn render_explorer(frame: &mut Frame<'_>, state: &mut ExplorerState, area: Rect) {
    // Minimum usable size check — render a fallback message if the terminal
    // is too small to show anything meaningful.
    if area.width < 20 || area.height < 6 {
        frame.render_widget(ratatui::widgets::Paragraph::new("Terminal too small"), area);
        return;
    }

    // If an error message is present, reserve one row at the bottom for a status line.
    let has_error = state.error_message.is_some();
    let outer = if has_error {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0)])
            .split(area)
    };

    let main_area = outer[0];

    if has_error && let Some(ref msg) = state.error_message {
        let status = Paragraph::new(msg.as_str()).style(Style::default().fg(Color::Red));
        frame.render_widget(status, outer[1]);
    }

    // Vertical split: top half for treemap, bottom half for table + chart.
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_area);

    let treemap_area = vertical[0];
    let bottom_area = vertical[1];

    // Horizontal split of the bottom half: 60 % table, 40 % chart.
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(bottom_area);

    let table_area = horizontal[0];
    let chart_area = horizontal[1];

    // --- Treemap ---
    let treemap_block = Block::default().borders(Borders::ALL).title(" Disk Usage ");
    let treemap_inner = treemap_block.inner(treemap_area);
    frame.render_widget(treemap_block, treemap_area);

    let treemap_items = build_treemap_items(state);
    let treemap = TreemapWidget {
        items: treemap_items,
    };
    frame.render_stateful_widget(treemap, treemap_inner, &mut state.treemap_state);

    // --- File table ---
    let table_block = Block::default().borders(Borders::ALL).title(" Files ");
    let table_inner = table_block.inner(table_area);
    frame.render_widget(table_block, table_area);

    let file_table = FileTableWidget {
        entries: &state.entries,
        sort_field: state.sort_field,
        sort_direction: state.sort_direction,
        selected_index: state.selected_index,
    };
    frame.render_widget(file_table, table_inner);

    // --- Type chart ---
    let chart_block = Block::default().borders(Borders::ALL).title(" File Types ");
    let chart_inner = chart_block.inner(chart_area);
    frame.render_widget(chart_block, chart_area);

    let type_chart = TypeChartWidget {
        type_stats: &state.type_stats,
    };
    frame.render_widget(type_chart, chart_inner);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Rotating palette for directory cells so adjacent directories are visually
/// distinct in the treemap even when none of them have a file extension.
const DIR_COLORS: [FileCategory; 7] = [
    FileCategory::Code,
    FileCategory::Image,
    FileCategory::Document,
    FileCategory::Archive,
    FileCategory::Audio,
    FileCategory::Video,
    FileCategory::Binary,
];

/// Build the list of [`TreemapItem`] values from the current explorer state.
///
/// Directories are included with their aggregated size; only non-zero-size
/// entries are added (zero-size entries would produce invisible cells).
/// Directories receive rotating category colours so they are visually
/// distinct in the treemap.
fn build_treemap_items(state: &ExplorerState) -> Vec<TreemapItem> {
    let mut dir_idx = 0_usize;
    state
        .entries
        .iter()
        .filter(|e| e.size > 0)
        .map(|e| {
            let label = e.path.file_name().map_or_else(
                || e.path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );

            let category = if e.file_type == FileType::Directory {
                let c = DIR_COLORS[dir_idx % DIR_COLORS.len()];
                dir_idx += 1;
                c
            } else {
                e.category
            };

            let extension = e
                .path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(str::to_lowercase);

            TreemapItem {
                label,
                size: e.size,
                category,
                is_directory: e.file_type == FileType::Directory,
                extension,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::SystemTime};

    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::{
        types::{FileCategory, FileEntry, FileType, SortDirection, SortField, TypeStat},
        ui::app::{ExplorerState, TreemapState},
    };

    fn make_entry(path: &str, size: u64, category: FileCategory) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            size,
            allocated_size: size,
            file_type: FileType::Regular,
            category,
            inode: 0,
            device: 0,
            nlink: 1,
            uid: 1000,
            gid: 1000,
            mtime: SystemTime::UNIX_EPOCH,
            mode: 0o644,
        }
    }

    fn make_explorer_state() -> ExplorerState {
        let entries = vec![
            make_entry("/root/big.rs", 3072, FileCategory::Code),
            make_entry("/root/img.png", 1024, FileCategory::Image),
        ];
        let type_stats = vec![
            TypeStat {
                category: FileCategory::Code,
                count: 1,
                total_size: 3072,
                total_allocated: 3072,
            },
            TypeStat {
                category: FileCategory::Image,
                count: 1,
                total_size: 1024,
                total_allocated: 1024,
            },
        ];
        ExplorerState {
            current_path: PathBuf::from("/root"),
            breadcrumb: vec![],
            entries,
            type_stats,
            selected_index: 0,
            sort_field: SortField::Size,
            sort_direction: SortDirection::Descending,
            treemap_state: TreemapState::default(),
            error_message: None,
        }
    }

    #[test]
    fn explorer_three_panel_layout() {
        // 80×24 buffer: treemap top half, table bottom-left, chart bottom-right.
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");

        let mut state = make_explorer_state();
        terminal
            .draw(|f| {
                render_explorer(f, &mut state, f.area());
            })
            .expect("draw");

        // After rendering, the treemap should have coloured the top half.
        // We verify that at least one coloured cell appears in the top half
        // (row 0..12) and at least one text char appears in the bottom half (row 12..24).
        let buf = terminal.backend().buffer().clone();

        let top_has_color = buf.content().iter().enumerate().any(|(i, c)| {
            let row = i / 80;
            row < 12 && c.bg != ratatui::style::Color::Reset
        });

        // Bottom section should have the header row from the file table.
        let bottom_text: String = buf
            .content()
            .iter()
            .enumerate()
            .filter(|(i, _)| i / 80 >= 12)
            .map(|(_, c)| c.symbol())
            .collect();

        assert!(
            top_has_color,
            "treemap (top half) should render coloured cells"
        );
        assert!(
            bottom_text.contains("Name") || bottom_text.contains("Size"),
            "file table header should appear in bottom half: {bottom_text:?}"
        );
    }
}
