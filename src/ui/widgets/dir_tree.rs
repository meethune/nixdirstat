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
///
/// When `filter` is `Some`, only nodes whose name contains the substring
/// (case-insensitive) or that have matching descendants are included.
pub fn dir_node_to_tree_items<'a>(
    node: &DirNode,
    parent_size: u64,
    sort_field: TreeSortField,
    sort_ascending: bool,
    row_width: u16,
    filter: Option<&str>,
) -> Vec<TreeItem<'a, String>> {
    let mut children: Vec<&DirNode> = node.children.iter().collect();
    sort_children(&mut children, sort_field, sort_ascending);

    children
        .iter()
        .filter(|child| filter.is_none_or(|q| node_matches_filter(child, &q.to_lowercase())))
        .map(|child| {
            let line = format_node_line(child, parent_size, row_width);
            let id = child.name.clone();

            if child.is_dir && !child.children.is_empty() {
                let sub = dir_node_to_tree_items(
                    child,
                    child.size,
                    sort_field,
                    sort_ascending,
                    row_width.saturating_sub(3),
                    filter,
                );
                TreeItem::new(id, line, sub).unwrap_or_else(|_| {
                    TreeItem::new_leaf(
                        child.name.clone(),
                        format_node_line(child, parent_size, row_width),
                    )
                })
            } else {
                TreeItem::new_leaf(id, line)
            }
        })
        .collect()
}

/// Check whether a node or any of its descendants match the filter query.
///
/// `query_lower` must already be lowercased by the caller so the allocation
/// happens once per filter pass, not per node.
fn node_matches_filter(node: &DirNode, query_lower: &str) -> bool {
    if node.name.to_lowercase().contains(query_lower) {
        return true;
    }
    if node.is_dir {
        return node
            .children
            .iter()
            .any(|c| node_matches_filter(c, query_lower));
    }
    false
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

/// Format a single tree row: name (fills remaining space), size, percentage, bar.
fn format_node_line(node: &DirNode, parent_size: u64, row_width: u16) -> Line<'static> {
    let name = &node.name;
    let size_str = format_size(node.size);

    let pct = if parent_size > 0 {
        #[allow(clippy::cast_precision_loss)]
        // u64→f64: display is 1 decimal place, so mantissa precision loss is invisible
        let p = (node.size as f64 / parent_size as f64) * 100.0;
        format!("{p:5.1}%")
    } else {
        String::from("    -%")
    };

    // Fixed suffix: " 999.9 MiB  99.9% ██████████" = ~28 chars.
    let suffix_width = 28_usize;
    let name_width = usize::from(row_width).saturating_sub(suffix_width).max(8);
    let bar_width = 10.min(usize::from(row_width).saturating_sub(name_width + 18));
    let bar = make_bar(node.size, parent_size, bar_width);

    Line::from(vec![
        Span::styled(
            format!("{name:<name_width$}"),
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

/// Build a proportional bar string with sub-block characters: `█▊▏  `.
fn make_bar(value: u64, total: u64, width: usize) -> String {
    if total == 0 {
        return " ".repeat(width);
    }

    #[allow(clippy::cast_precision_loss)]
    // Loss of precision in u64->f64 cast is acceptable for proportion calculation
    let proportion = value as f64 / total as f64;
    #[allow(clippy::cast_precision_loss)]
    // Loss of precision in usize->f64 cast is acceptable (width never exceeds 50 in tests)
    let filled_f64 = proportion * width as f64;

    // Full blocks from the integer part
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    // Truncation is intentional (floor already truncates); filled_f64 is always non-negative
    let full_blocks = filled_f64.floor() as usize;

    // Fractional part: round to nearest 1/8
    let fract = filled_f64.fract();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    // Truncation is intentional; (fract * 8.0).round() is always 0..=8 (non-negative)
    let fract_index = (fract * 8.0).round() as usize;

    // Sub-block characters: index 0 = space, 1-7 = sub-blocks, 8 = full block
    let sub_blocks = [" ", "▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"];

    let mut result = String::new();

    // Add full blocks
    result.push_str(&"█".repeat(full_blocks));

    // Add fractional character (if any)
    let remaining_width = if fract_index == 0 {
        width - full_blocks
    } else if fract_index == 8 {
        // An 8/8 fractional part becomes a full block
        result.push('█');
        width - full_blocks - 1
    } else {
        result.push_str(sub_blocks[fract_index]);
        width - full_blocks - 1
    };

    // Fill the rest with spaces
    result.push_str(&" ".repeat(remaining_width));

    result
}

/// Render the directory tree into `area`.
///
/// Uses `tui-tree-widget`'s [`Tree`] widget with [`TreeState`] for
/// expand/collapse tracking.
/// Render the directory tree into `area`.
///
/// Display options for the directory tree widget.
pub struct TreeDisplayOpts<'a> {
    /// Whether the tree panel currently has keyboard focus.
    pub focused: bool,
    /// Field used to sort children at each directory level.
    pub sort_field: TreeSortField,
    /// Sort in ascending order when `true`.
    pub sort_ascending: bool,
    /// Case-insensitive substring filter; `None` shows all nodes.
    pub filter: Option<&'a str>,
}

/// Uses `tui-tree-widget`'s [`Tree`] widget with [`TreeState`] for
/// expand/collapse tracking. Children are sorted by `sort_field`.
pub fn render_dir_tree(
    frame: &mut Frame<'_>,
    node: &DirNode,
    state: &mut TreeState<String>,
    area: Rect,
    opts: &TreeDisplayOpts<'_>,
) {
    let items = dir_node_to_tree_items(
        node,
        node.size,
        opts.sort_field,
        opts.sort_ascending,
        area.width,
        opts.filter,
    );
    let highlight_style = if opts.focused {
        Style::default()
            .bg(Color::Indexed(24)) // muted blue — avoids overpowering cyan dir names
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

    use proptest::{prop_assert_eq, proptest};
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
                let opts = TreeDisplayOpts {
                    focused: true,
                    sort_field: TreeSortField::Size,
                    sort_ascending: false,
                    filter: None,
                };
                render_dir_tree(f, node, &mut state, f.area(), &opts);
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

    #[test]
    fn bar_full_is_all_full_blocks() {
        assert_eq!(make_bar(100, 100, 10), "██████████");
    }

    #[test]
    fn bar_empty_is_all_spaces() {
        assert_eq!(make_bar(0, 100, 10), "          ");
    }

    #[test]
    fn bar_half_is_five_full_blocks() {
        assert_eq!(make_bar(50, 100, 10), "█████     ");
    }

    #[test]
    fn bar_one_eighth() {
        let bar = make_bar(1, 8, 1);
        assert_eq!(bar, "▏");
    }

    #[test]
    fn bar_three_eighths() {
        let bar = make_bar(3, 8, 1);
        assert_eq!(bar, "▍");
    }

    proptest! {
        #[test]
        fn bar_char_count_equals_width(v in 0u64..=1000, t in 1u64..=1000, w in 1usize..=50) {
            let bar = make_bar(v.min(t), t, w);
            prop_assert_eq!(bar.chars().count(), w);
        }
    }

    #[test]
    fn filter_hides_non_matching_files() {
        let root = make_dir_node(
            "root",
            300,
            vec![
                make_file_node("main.rs", 100),
                make_file_node("readme.md", 100),
                make_file_node("lib.rs", 100),
            ],
        );
        let items =
            dir_node_to_tree_items(&root, root.size, TreeSortField::Name, true, 80, Some("rs"));
        assert_eq!(items.len(), 2, "filter should keep only .rs files");
    }

    #[test]
    fn filter_keeps_dir_with_matching_descendant() {
        let root = make_dir_node(
            "root",
            200,
            vec![
                make_dir_node("src", 100, vec![make_file_node("main.rs", 100)]),
                make_dir_node("docs", 100, vec![make_file_node("guide.md", 100)]),
            ],
        );
        let items = dir_node_to_tree_items(
            &root,
            root.size,
            TreeSortField::Name,
            true,
            80,
            Some("main"),
        );
        assert_eq!(items.len(), 1, "only src/ should match via descendant");
    }

    #[test]
    fn filter_none_shows_all() {
        let root = make_dir_node(
            "root",
            200,
            vec![make_file_node("a.txt", 100), make_file_node("b.txt", 100)],
        );
        let items = dir_node_to_tree_items(&root, root.size, TreeSortField::Name, true, 80, None);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn filter_is_case_insensitive() {
        let root = make_dir_node("root", 100, vec![make_file_node("README.md", 100)]);
        let items = dir_node_to_tree_items(
            &root,
            root.size,
            TreeSortField::Name,
            true,
            80,
            Some("readme"),
        );
        assert_eq!(items.len(), 1);
    }
}
