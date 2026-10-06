//! Squarified treemap widget for visualising disk usage.
//!
//! Renders a [`DirNode`] tree recursively: each directory's children are
//! squarified within the directory's allocated area. Files become colored
//! leaf cells; directories recurse until the cell area is too small to
//! subdivide further.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Span,
    widgets::StatefulWidget,
};
use streemap::Rect as SRect;

use crate::{types::FileCategory, ui::tree::DirNode};

// ---------------------------------------------------------------------------
// Colour palettes
// ---------------------------------------------------------------------------

/// Return the terminal colour associated with a [`FileCategory`].
///
/// Retained for potential future category-level views.
#[allow(dead_code)]
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
        _ => Color::White,
    }
}

/// Perceptually distinct 256-colour palette for per-extension treemap cells.
///
/// Each extension hashes to one of these colours, so files with the same
/// extension share a colour while different extensions are visually distinct.
const EXTENSION_PALETTE: [Color; 22] = [
    Color::Indexed(196), // red
    Color::Indexed(202), // orange
    Color::Indexed(208), // dark orange
    Color::Indexed(214), // gold
    Color::Indexed(220), // yellow
    Color::Indexed(226), // bright yellow
    Color::Indexed(46),  // green
    Color::Indexed(34),  // forest green
    Color::Indexed(48),  // sea green
    Color::Indexed(51),  // cyan
    Color::Indexed(39),  // sky blue
    Color::Indexed(27),  // blue
    Color::Indexed(21),  // deep blue
    Color::Indexed(57),  // indigo
    Color::Indexed(129), // purple
    Color::Indexed(165), // magenta
    Color::Indexed(205), // hot pink
    Color::Indexed(172), // brown
    Color::Indexed(136), // olive
    Color::Indexed(71),  // moss
    Color::Indexed(109), // steel blue
    Color::Indexed(174), // rose
];

/// Map a file extension to a colour from the extension palette.
///
/// Uses FNV-1a hash for fast, low-collision distribution across the palette.
/// Files with no extension get [`Color::Gray`].
pub(crate) fn extension_color(ext: Option<&str>) -> Color {
    let Some(ext) = ext else {
        return Color::Gray;
    };
    if ext.is_empty() {
        return Color::Gray;
    }
    let mut hash: u32 = 2_166_136_261;
    for byte in ext.as_bytes() {
        hash ^= u32::from(byte.to_ascii_lowercase());
        hash = hash.wrapping_mul(16_777_619);
    }
    EXTENSION_PALETTE[hash as usize % EXTENSION_PALETTE.len()]
}

// ---------------------------------------------------------------------------
// TreemapState
// ---------------------------------------------------------------------------

/// Mutable state for the treemap widget.
///
/// Tracks which node is highlighted (selected in the directory tree panel).
#[derive(Debug, Default, Clone)]
pub struct TreemapState {
    /// Path components (from treemap root) of the highlighted node.
    pub highlighted_path: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// TreemapWidget
// ---------------------------------------------------------------------------

/// Recursive squarified treemap widget.
///
/// Renders a [`DirNode`] tree by squarifying each directory's children
/// within its allocated area. Files become colored leaf cells.
pub struct TreemapWidget<'a> {
    /// The root node to render (may be a subtree for zoom).
    pub root: &'a DirNode,
}

impl StatefulWidget for TreemapWidget<'_> {
    type State = TreemapState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        if area.is_empty() || self.root.children.is_empty() {
            return;
        }
        render_recursive(self.root, area, buf, state.highlighted_path.as_deref(), &[]);
    }
}

/// Recursively render a directory's children into `area`.
fn render_recursive(
    node: &DirNode,
    area: Rect,
    buf: &mut Buffer,
    highlight: Option<&[String]>,
    current_path: &[String],
) {
    let children: Vec<&DirNode> = node.children.iter().filter(|c| c.size > 0).collect();
    if children.is_empty() || area.width == 0 || area.height == 0 {
        return;
    }

    let zero_rect = SRect {
        x: 0.0_f32,
        y: 0.0_f32,
        w: 0.0_f32,
        h: 0.0_f32,
    };
    let mut layout: Vec<(&DirNode, SRect<f32>)> =
        children.iter().map(|c| (*c, zero_rect)).collect();

    let container = SRect {
        x: 0.0_f32,
        y: 0.0_f32,
        w: f32::from(area.width),
        h: f32::from(area.height),
    };

    streemap::squarify(
        container,
        &mut layout,
        #[allow(clippy::cast_precision_loss)]
        |(child, _r)| child.size as f32,
        |(_child, r), new_r| *r = new_r,
    );

    for (child, f32_rect) in &layout {
        let cell_rect = f32_rect_to_ratatui(*f32_rect, area);
        if cell_rect.width == 0 || cell_rect.height == 0 {
            continue;
        }

        let cell_area = u32::from(cell_rect.width) * u32::from(cell_rect.height);

        let mut child_path = current_path.to_vec();
        child_path.push(child.name.clone());

        let is_highlighted =
            highlight.is_some_and(|hp| child_path.starts_with(hp) || hp.starts_with(&child_path));

        if child.is_dir {
            render_dir_cell(child, cell_rect, cell_area, buf, highlight, &child_path);
        } else {
            render_file_cell(child, cell_rect, is_highlighted, buf);
        }

        // Draw highlight border if this node matches.
        if is_highlighted && highlight.is_some_and(|hp| hp == child_path) {
            draw_highlight_border(buf, cell_rect);
        }
    }
}

/// Render a directory cell: recurse if large enough, otherwise fill with dominant color.
fn render_dir_cell(
    child: &DirNode,
    cell_rect: Rect,
    cell_area: u32,
    buf: &mut Buffer,
    highlight: Option<&[String]>,
    child_path: &[String],
) {
    if cell_area >= 2 {
        let inner = if cell_rect.width >= 6 && cell_rect.height >= 6 {
            Rect {
                x: cell_rect.x.saturating_add(1),
                y: cell_rect.y,
                width: cell_rect.width.saturating_sub(1),
                height: cell_rect.height,
            }
        } else {
            cell_rect
        };
        render_recursive(child, inner, buf, highlight, child_path);
    } else {
        let color = dominant_color(child);
        buf.set_style(cell_rect, Style::default().bg(color).fg(Color::Black));
    }
}

/// Render a file cell: colored background with optional label.
fn render_file_cell(child: &DirNode, cell_rect: Rect, is_highlighted: bool, buf: &mut Buffer) {
    let color = extension_color(child.extension.as_deref());
    let style = if is_highlighted {
        Style::default()
            .bg(color)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(color).fg(Color::Black)
    };
    buf.set_style(cell_rect, style);

    if cell_rect.width >= 3 {
        render_cell_label(buf, &child.name, cell_rect, style);
    }
}

/// Find the extension color of the largest file in a subtree.
///
/// Iteratively chases the largest child to avoid stack overflow on deep trees.
fn dominant_color(node: &DirNode) -> Color {
    let mut current = node;
    loop {
        if !current.is_dir {
            return extension_color(current.extension.as_deref());
        }
        match current.children.iter().max_by_key(|c| c.size) {
            Some(child) => current = child,
            None => return Color::Gray,
        }
    }
}

/// Draw a bright border around `rect` to highlight a selected region.
/// Set a single border cell if it is within the buffer area.
fn set_border_cell(buf: &mut Buffer, x: u16, y: u16, ch: char, style: Style) {
    if let Some(c) = buf.cell_mut((x, y)) {
        c.set_style(style);
        c.set_char(ch);
    }
}

fn draw_highlight_border(buf: &mut Buffer, rect: Rect) {
    let style = Style::default()
        .fg(Color::Black)
        .bg(Color::White)
        .add_modifier(Modifier::BOLD);
    let right = rect.right().saturating_sub(1);
    let bottom = rect.bottom().saturating_sub(1);

    // Top and bottom edges.
    for x in rect.x..rect.right() {
        set_border_cell(buf, x, rect.y, '─', style);
        if bottom > rect.y {
            set_border_cell(buf, x, bottom, '─', style);
        }
    }
    // Left and right edges.
    for y in rect.y..rect.bottom() {
        set_border_cell(buf, rect.x, y, '│', style);
        if right > rect.x {
            set_border_cell(buf, right, y, '│', style);
        }
    }
    // Corners.
    set_border_cell(buf, rect.x, rect.y, '┌', style);
    if right > rect.x {
        set_border_cell(buf, right, rect.y, '┐', style);
    }
    if bottom > rect.y {
        set_border_cell(buf, rect.x, bottom, '└', style);
    }
    if right > rect.x && bottom > rect.y {
        set_border_cell(buf, right, bottom, '┘', style);
    }
}

// ---------------------------------------------------------------------------
// Label rendering helper
// ---------------------------------------------------------------------------

/// Render a cell label into `buf` at the top-left of `cell_rect`.
///
/// Truncates with `"..."` if the label is wider than the cell.
fn render_cell_label(buf: &mut Buffer, label: &str, cell_rect: Rect, style: Style) {
    let max_chars = usize::from(cell_rect.width);
    if label.len() <= max_chars {
        let span = Span::styled(label, style);
        buf.set_span(cell_rect.x, cell_rect.y, &span, cell_rect.width);
        return;
    }
    let keep = max_chars.saturating_sub(3);
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

/// Convert a [`streemap::Rect<f32>`] into a [`ratatui::layout::Rect`]
/// offset by `container`'s origin, clamped to container bounds.
fn f32_rect_to_ratatui(r: SRect<f32>, container: Rect) -> Rect {
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
    use std::time::SystemTime;

    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    fn make_file(name: &str, size: u64, ext: Option<&str>) -> DirNode {
        DirNode {
            name: name.into(),
            size,
            allocated: size,
            file_count: 1,
            children: Vec::new(),
            is_dir: false,
            extension: ext.map(String::from),
            mtime: SystemTime::UNIX_EPOCH,
        }
    }

    fn make_dir(name: &str, size: u64, children: Vec<DirNode>) -> DirNode {
        DirNode {
            name: name.into(),
            size,
            allocated: size,
            file_count: 0,
            children,
            is_dir: true,
            extension: None,
            mtime: SystemTime::UNIX_EPOCH,
        }
    }

    fn render_treemap(root: &DirNode, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = TreemapState::default();
        terminal
            .draw(|f| {
                let widget = TreemapWidget { root };
                f.render_stateful_widget(widget, f.area(), &mut state);
            })
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    #[test]
    fn treemap_recursive_renders_files() {
        let root = make_dir(
            "root",
            100,
            vec![make_dir(
                "sub",
                100,
                vec![
                    make_file("a.rs", 60, Some("rs")),
                    make_file("b.py", 40, Some("py")),
                ],
            )],
        );
        let buf = render_treemap(&root, 40, 10);
        let rs_color = extension_color(Some("rs"));
        let py_color = extension_color(Some("py"));
        let rs_count = buf.content().iter().filter(|c| c.bg == rs_color).count();
        let py_count = buf.content().iter().filter(|c| c.bg == py_color).count();
        assert!(rs_count > 0, "expected .rs colored cells");
        assert!(py_count > 0, "expected .py colored cells");
    }

    #[test]
    fn treemap_proportional_areas() {
        let root = make_dir(
            "root",
            100,
            vec![
                make_file("big.rs", 75, Some("rs")),
                make_file("small.py", 25, Some("py")),
            ],
        );
        let buf = render_treemap(&root, 80, 24);
        let total = 80_usize * 24;
        let rs_color = extension_color(Some("rs"));
        let rs_count = buf.content().iter().filter(|c| c.bg == rs_color).count();
        assert!(
            rs_count * 100 / total >= 60,
            "expected ≥60% for 75% file, got {rs_count}/{total}"
        );
    }

    #[test]
    fn treemap_directory_recursion() {
        let root = make_dir(
            "root",
            50,
            vec![make_dir(
                "dir",
                50,
                vec![make_file("file.rs", 50, Some("rs"))],
            )],
        );
        let buf = render_treemap(&root, 40, 10);
        let rs_color = extension_color(Some("rs"));
        let has_rs = buf.content().iter().any(|c| c.bg == rs_color);
        assert!(has_rs, "nested file should produce colored cells");
    }

    #[test]
    fn treemap_pruning_small_cells() {
        let files: Vec<DirNode> = (0..100)
            .map(|i| make_file(&format!("f{i}.txt"), 1, Some("txt")))
            .collect();
        let root = make_dir("root", 100, files);
        let buf = render_treemap(&root, 20, 5);
        let colored = buf
            .content()
            .iter()
            .filter(|c| c.bg != Color::Reset)
            .count();
        assert!(colored > 0, "pruned cells should still be colored");
    }

    #[test]
    fn treemap_deep_nesting_no_overflow() {
        let mut node = make_file("leaf.rs", 100, Some("rs"));
        for i in 0..150 {
            node = make_dir(&format!("d{i}"), 100, vec![node]);
        }
        let buf = render_treemap(&node, 40, 10);
        let has_color = buf.content().iter().any(|c| c.bg != Color::Reset);
        assert!(has_color, "deep tree should render without stack overflow");
    }

    #[test]
    fn treemap_dominant_file_shows_siblings() {
        let root = make_dir(
            "root",
            100,
            vec![
                make_file("huge.bin", 99, Some("bin")),
                make_file("tiny.rs", 1, Some("rs")),
            ],
        );
        let buf = render_treemap(&root, 100, 10);
        let bin_color = extension_color(Some("bin"));
        let rs_color = extension_color(Some("rs"));
        let bin_count = buf.content().iter().filter(|c| c.bg == bin_color).count();
        let rs_count = buf.content().iter().filter(|c| c.bg == rs_color).count();
        assert!(bin_count > 0, "dominant file should be visible");
        assert!(rs_count > 0, "tiny sibling should have at least 1 cell");
    }

    #[test]
    fn treemap_highlight_path() {
        let root = make_dir(
            "root",
            100,
            vec![
                make_file("a.rs", 50, Some("rs")),
                make_file("b.py", 50, Some("py")),
            ],
        );
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = TreemapState {
            highlighted_path: Some(vec!["a.rs".into()]),
        };
        terminal
            .draw(|f| {
                let widget = TreemapWidget { root: &root };
                f.render_stateful_widget(widget, f.area(), &mut state);
            })
            .expect("draw");
        let buf = terminal.backend().buffer().clone();
        let has_bold = buf
            .content()
            .iter()
            .any(|c| c.modifier.contains(Modifier::BOLD));
        assert!(has_bold, "highlighted path should have bold cells");
    }

    #[test]
    fn treemap_empty_root() {
        let root = make_dir("root", 0, Vec::new());
        let buf = render_treemap(&root, 40, 10);
        let colored = buf
            .content()
            .iter()
            .filter(|c| c.bg != Color::Reset)
            .count();
        assert_eq!(colored, 0, "empty root should render nothing");
    }
}
