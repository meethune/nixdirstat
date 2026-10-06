//! Sortable file-table widget.
//!
//! Renders a [`Table`] of directory children with columns for name, size,
//! type, modification date, and owner. A sort indicator (▲/▼) is shown on
//! the active sort column, and the selected row is highlighted.

use std::time::SystemTime;

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Row, StatefulWidget, Table, TableState, Widget},
};

use crate::types::{FileEntry, SortDirection, SortField, format_size};

// ---------------------------------------------------------------------------
// FileTableWidget
// ---------------------------------------------------------------------------

/// Sortable table widget for displaying directory children.
///
/// Renders as a [`Table`] with five columns (Name, Size, Type, Modified,
/// Owner). The header row includes a ▲/▼ indicator on the active sort column.
/// The selected row is highlighted using a reversed-video style.
pub struct FileTableWidget<'a> {
    /// Entries to display (already sorted by the caller).
    pub entries: &'a [FileEntry],
    /// Which column the entries are currently sorted by.
    pub sort_field: SortField,
    /// Current sort direction.
    pub sort_direction: SortDirection,
    /// Index of the selected entry (0-based).
    pub selected_index: usize,
}

impl Widget for FileTableWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut table_state = TableState::default();
        if !self.entries.is_empty() {
            table_state.select(Some(self.selected_index));
        }
        StatefulWidget::render(self.build_table(), area, buf, &mut table_state);
    }
}

impl<'a> FileTableWidget<'a> {
    /// Build the underlying [`Table`] widget from the current widget state.
    fn build_table(&self) -> Table<'a> {
        let sort_indicator = match self.sort_direction {
            SortDirection::Ascending => "▲",
            // SortDirection::Descending and any future variants → descending indicator.
            _ => "▼",
        };

        // Column names; append the sort indicator to the active column.
        let name_header = if self.sort_field == SortField::Name {
            format!("Name {sort_indicator}")
        } else {
            String::from("Name")
        };
        let size_header = if self.sort_field == SortField::Size {
            format!("Size {sort_indicator}")
        } else {
            String::from("Size")
        };
        let type_header = if self.sort_field == SortField::Type {
            format!("Type {sort_indicator}")
        } else {
            String::from("Type")
        };
        let modified_header = if self.sort_field == SortField::Modified {
            format!("Modified {sort_indicator}")
        } else {
            String::from("Modified")
        };

        let header_style = Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);

        let header = Row::new(vec![
            Cell::from(Line::from(Span::styled(name_header, header_style))),
            Cell::from(Line::from(Span::styled(size_header, header_style))),
            Cell::from(Line::from(Span::styled(type_header, header_style))),
            Cell::from(Line::from(Span::styled(modified_header, header_style))),
            Cell::from(Line::from(Span::styled("Owner", header_style))),
        ])
        .height(1);

        let rows: Vec<Row<'_>> = self.entries.iter().map(entry_to_row).collect();

        let widths = [
            Constraint::Percentage(38),
            Constraint::Percentage(14),
            Constraint::Percentage(12),
            Constraint::Percentage(22),
            Constraint::Percentage(14),
        ];

        Table::new(rows, widths).header(header).row_highlight_style(
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
    }
}

/// Convert a [`FileEntry`] into a table [`Row`].
fn entry_to_row(entry: &FileEntry) -> Row<'_> {
    let name = entry.path.file_name().map_or_else(
        || entry.path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );

    let size = format_size(entry.size);
    let type_str = entry.file_type.to_string();
    let modified = format_mtime(entry.mtime);
    let owner = entry.uid.to_string();

    Row::new(vec![name, size, type_str, modified, owner])
}

/// Format a [`SystemTime`] as an approximate `YYYY-MM-DD` string.
///
/// Uses a rough calendar approximation (30-day months, no leap years) which
/// is sufficient for display purposes without pulling in a date library.
fn format_mtime(t: SystemTime) -> String {
    let Ok(duration) = t.duration_since(SystemTime::UNIX_EPOCH) else {
        return String::from("---");
    };
    let secs = duration.as_secs();
    let days = secs / 86_400;
    let year = 1970_u64 + days / 365;
    let doy = days % 365;
    let month = doy / 30 + 1;
    let day = doy % 30 + 1;
    format!("{year:04}-{month:02}-{day:02}")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::SystemTime;

    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::types::{FileCategory, FileType};

    fn make_entry(name: &str, size: u64) -> FileEntry {
        FileEntry {
            path: PathBuf::from("/test").join(name),
            size,
            allocated_size: size,
            file_type: FileType::Regular,
            category: FileCategory::Code,
            inode: 0,
            device: 0,
            nlink: 1,
            uid: 1000,
            gid: 1000,
            mtime: SystemTime::UNIX_EPOCH,
            mode: 0o644,
        }
    }

    fn render_table(
        entries: &[FileEntry],
        sort_field: SortField,
        sort_dir: SortDirection,
        selected: usize,
    ) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| {
                let widget = FileTableWidget {
                    entries,
                    sort_field,
                    sort_direction: sort_dir,
                    selected_index: selected,
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
    fn file_table_renders_header() {
        let entries = [make_entry("foo.rs", 1024)];
        let content = render_table(&entries, SortField::Size, SortDirection::Descending, 0);
        assert!(content.contains("Name"), "missing 'Name' in: {content:?}");
        assert!(content.contains("Size"), "missing 'Size' in: {content:?}");
        assert!(content.contains("Type"), "missing 'Type' in: {content:?}");
    }

    #[test]
    fn file_table_sorts_by_size_desc() {
        // Entries pre-sorted by the caller: largest first.
        let entries = [make_entry("big.rs", 9999), make_entry("small.rs", 100)];
        let content = render_table(&entries, SortField::Size, SortDirection::Descending, 0);
        let big_pos = content.find("big").unwrap_or(usize::MAX);
        let small_pos = content.find("small").unwrap_or(usize::MAX);
        assert!(
            big_pos < small_pos,
            "largest entry ('big') should appear before 'small'; got big@{big_pos} small@{small_pos}"
        );
    }

    #[test]
    fn file_table_highlights_selected_row() {
        // Render with selection on row 0; re-render with selection on row 1.
        // Check that the buffer differs (selected row has a different style).
        let entries = [make_entry("first.rs", 500), make_entry("second.rs", 200)];

        let backend0 = TestBackend::new(80, 24);
        let mut t0 = Terminal::new(backend0).expect("terminal");
        t0.draw(|f| {
            f.render_widget(
                FileTableWidget {
                    entries: &entries,
                    sort_field: SortField::Size,
                    sort_direction: SortDirection::Descending,
                    selected_index: 0,
                },
                f.area(),
            );
        })
        .expect("draw");

        let backend1 = TestBackend::new(80, 24);
        let mut t1 = Terminal::new(backend1).expect("terminal");
        t1.draw(|f| {
            f.render_widget(
                FileTableWidget {
                    entries: &entries,
                    sort_field: SortField::Size,
                    sort_direction: SortDirection::Descending,
                    selected_index: 1,
                },
                f.area(),
            );
        })
        .expect("draw");

        // The two buffers must differ — the selected row changes.
        let b0 = t0.backend().buffer().clone();
        let b1 = t1.backend().buffer().clone();
        let differs = b0
            .content()
            .iter()
            .zip(b1.content().iter())
            .any(|(a, b)| a.bg != b.bg || a.fg != b.fg || a.modifier != b.modifier);
        assert!(
            differs,
            "selected row should have different style from unselected"
        );
    }
}
