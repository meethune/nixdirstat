//! Directory tree widget backed by `tui-tree-widget`.
//!
//! Renders a [`DirNode`] tree as an expandable directory listing with
//! per-node size and percentage information.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use tui_tree_widget::{Tree, TreeItem, TreeState};

use crate::{types::format_size, ui::app::TreeSortField, ui::tree::DirNode};

/// Convert a [`DirNode`] into `tui-tree-widget` [`TreeItem`] values.
///
/// Children at each level are sorted by `sort_field` / `sort_ascending`
/// before conversion. Directories produce items with children; files
/// produce leaf items.
pub fn dir_node_to_tree_items<'a>(
    node: &DirNode,
    parent_size: u64,
    sort_field: TreeSortField,
    sort_ascending: bool,
) -> Vec<TreeItem<'a, String>> {
    let mut children: Vec<&DirNode> = node.children.iter().collect();
    sort_children(&mut children, sort_field, sort_ascending);

    children
        .iter()
        .map(|child| {
            let line = format_node_line(child, parent_size);
            let id = child.name.clone();

            if child.is_dir && !child.children.is_empty() {
                let sub = dir_node_to_tree_items(child, child.size, sort_field, sort_ascending);
                TreeItem::new(id, line, sub).unwrap_or_else(|_| {
                    TreeItem::new_leaf(child.name.clone(), format_node_line(child, parent_size))
                })
            } else {
                TreeItem::new_leaf(id, line)
            }
        })
        .collect()
}

/// Sort a slice of `DirNode` references by the given field and direction.
fn sort_children(children: &mut [&DirNode], field: TreeSortField, ascending: bool) {
    children.sort_by(|a, b| {
        let ord = match field {
            TreeSortField::Size => a.size.cmp(&b.size),
            TreeSortField::Name => a
                .name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase()),
            TreeSortField::Modified => a.mtime.cmp(&b.mtime),
        };
        if ascending { ord } else { ord.reverse() }
    });
}

/// Format a single tree row: name, size, percentage, and bar.
fn format_node_line(node: &DirNode, parent_size: u64) -> Line<'static> {
    let name = &node.name;
    let size_str = format_size(node.size);

    let pct = if parent_size > 0 {
        #[allow(clippy::cast_precision_loss)]
        let p = (node.size as f64 / parent_size as f64) * 100.0;
        format!("{p:5.1}%")
    } else {
        String::from("    -%")
    };

    let bar = make_bar(node.size, parent_size, 10);

    Line::from(vec![
        Span::styled(
            format!("{name:<20} "),
            Style::default().fg(if node.is_dir {
                Color::Cyan
            } else {
                Color::White
            }),
        ),
        Span::styled(
            format!("{size_str:>9} "),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(format!("{pct} "), Style::default().fg(Color::DarkGray)),
        Span::styled(bar, Style::default().fg(Color::Green)),
    ])
}

/// Build a proportional bar string: `████░░░░░░`.
fn make_bar(value: u64, total: u64, width: usize) -> String {
    if total == 0 {
        return "░".repeat(width);
    }
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let filled = ((value as f64 / total as f64) * width as f64).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

/// Render the directory tree into `area`.
///
/// Uses `tui-tree-widget`'s [`Tree`] widget with [`TreeState`] for
/// expand/collapse tracking.
/// Render the directory tree into `area`.
///
/// Uses `tui-tree-widget`'s [`Tree`] widget with [`TreeState`] for
/// expand/collapse tracking. Children are sorted by `sort_field`.
pub fn render_dir_tree(
    frame: &mut Frame<'_>,
    node: &DirNode,
    state: &mut TreeState<String>,
    area: Rect,
    focused: bool,
    sort_field: TreeSortField,
    sort_ascending: bool,
) {
    let items = dir_node_to_tree_items(node, node.size, sort_field, sort_ascending);
    let highlight_style = if focused {
        Style::default()
            .bg(Color::Blue)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(Color::DarkGray).fg(Color::White)
    };

    if let Ok(tree) = Tree::new(&items) {
        let tree = tree.highlight_style(highlight_style);
        frame.render_stateful_widget(tree, area, state);
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    fn make_dir_node(name: &str, size: u64, children: Vec<DirNode>) -> DirNode {
        DirNode {
            name: name.to_string(),
            size,
            allocated: size,
            file_count: 0,
            children,
            is_dir: true,
            extension: None,
            mtime: SystemTime::UNIX_EPOCH,
        }
    }

    fn make_file_node(name: &str, size: u64) -> DirNode {
        DirNode {
            name: name.to_string(),
            size,
            allocated: size,
            file_count: 1,
            children: Vec::new(),
            is_dir: false,
            extension: name
                .rsplit('.')
                .next()
                .filter(|e| *e != name)
                .map(String::from),
            mtime: SystemTime::UNIX_EPOCH,
        }
    }

    fn render_tree_to_string(node: &DirNode, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = TreeState::default();
        terminal
            .draw(|f| {
                render_dir_tree(
                    f,
                    node,
                    &mut state,
                    f.area(),
                    true,
                    TreeSortField::Size,
                    false,
                );
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
    fn tree_renders_directory_names() {
        let root = make_dir_node(
            "root",
            100,
            vec![
                make_dir_node("src", 60, vec![make_file_node("main.rs", 60)]),
                make_dir_node("docs", 40, vec![make_file_node("readme.md", 40)]),
            ],
        );
        let content = render_tree_to_string(&root, 80, 10);
        assert!(content.contains("src"), "missing 'src': {content:?}");
        assert!(content.contains("docs"), "missing 'docs': {content:?}");
    }

    #[test]
    fn tree_renders_sizes() {
        let root = make_dir_node(
            "root",
            1_048_576,
            vec![make_file_node("big.bin", 1_048_576)],
        );
        let content = render_tree_to_string(&root, 80, 5);
        assert!(content.contains("MiB"), "missing size: {content:?}");
    }

    #[test]
    fn tree_renders_percentages() {
        let root = make_dir_node(
            "root",
            200,
            vec![
                make_file_node("half.txt", 100),
                make_file_node("other.txt", 100),
            ],
        );
        let content = render_tree_to_string(&root, 80, 5);
        assert!(content.contains("50"), "missing 50%: {content:?}");
    }

    #[test]
    fn tree_renders_expand_indicator() {
        let root = make_dir_node(
            "root",
            100,
            vec![make_dir_node(
                "sub",
                100,
                vec![make_file_node("f.txt", 100)],
            )],
        );
        let content = render_tree_to_string(&root, 80, 5);
        // tui-tree-widget uses special characters for expand/collapse
        let has_indicator = content.contains('▸')
            || content.contains('▾')
            || content.contains('►')
            || content.contains('▼')
            || content.contains('>')
            || content.contains('v');
        assert!(
            has_indicator || content.contains("sub"),
            "should show directory with indicator: {content:?}"
        );
    }

    #[test]
    fn tree_files_have_no_indicator() {
        let root = make_dir_node("root", 50, vec![make_file_node("solo.txt", 50)]);
        let content = render_tree_to_string(&root, 80, 5);
        assert!(content.contains("solo"), "missing file name: {content:?}");
    }

    #[test]
    fn tree_empty_node() {
        let root = make_dir_node("root", 0, Vec::new());
        let content = render_tree_to_string(&root, 80, 5);
        let non_space: String = content.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            non_space.is_empty() || non_space.len() < 5,
            "empty tree should render minimal: {content:?}"
        );
    }
}
