//! Squarified treemap widget for visualising disk usage.
//!
//! Renders a [`DirNode`] tree recursively: each directory's children are
//! squarified within the directory's allocated area. Files become colored
//! leaf cells; directories recurse until the cell area is too small to
//! subdivide further.

use std::{ffi::OsStr, time::SystemTime};

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, BorderType, Borders, StatefulWidget, Widget as _},
};
use streemap::Rect as SRect;

use crate::{
    types::FileCategory,
    ui::{
        colors::{category_color, contrast_text_color, is_color_enabled, no_color_fallback},
        pixel_grid::PixelGrid,
        tree::DirNode,
    },
};

// ---------------------------------------------------------------------------
// Extension palette (retained for extension_legend.rs — Task 5 removes this)
// ---------------------------------------------------------------------------

/// Perceptually distinct 256-colour palette for per-extension treemap cells.
///
/// Retained until [`extension_legend.rs`] is updated in Task 5.
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
/// Retained for [`extension_legend.rs`] until Task 5 updates that import.
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
// CellLayout
// ---------------------------------------------------------------------------

/// Per-cell geometry and metadata captured during render for spatial navigation.
///
/// Consumed by Task 5 for cursor-based navigation within the treemap.
/// Derives `Debug` and `Clone`; `Default` is implemented manually because
/// [`SystemTime`] does not implement [`Default`].
#[derive(Debug, Clone)]
pub struct CellLayout {
    /// Absolute terminal-cell coordinates and size of this cell.
    pub rect: Rect,
    /// File or directory name.
    pub name: String,
    /// File extension, if any.
    pub extension: Option<String>,
    /// `true` for directory leaf cells (too small to recurse).
    pub is_dir: bool,
    /// Total size in bytes.
    pub size: u64,
    /// Last modification time.
    pub mtime: SystemTime,
    /// Path components from the treemap root to this cell.
    pub path: Vec<String>,
}

impl Default for CellLayout {
    fn default() -> Self {
        Self {
            rect: Rect::default(),
            name: String::new(),
            extension: None,
            is_dir: false,
            size: 0,
            mtime: SystemTime::UNIX_EPOCH,
            path: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// TreemapLayout
// ---------------------------------------------------------------------------

/// Spatial layout produced by the most recent render, for navigation (Task 5).
#[derive(Debug, Clone, Default)]
pub struct TreemapLayout {
    /// All leaf cells in the last render, in paint order.
    pub cells: Vec<CellLayout>,
}

// ---------------------------------------------------------------------------
// TreemapState
// ---------------------------------------------------------------------------

/// Mutable state for the treemap widget.
///
/// Tracks which node is highlighted (selected in the directory tree panel)
/// and the spatial layout produced by the last render.
#[derive(Debug, Default, Clone)]
pub struct TreemapState {
    /// Path components (from treemap root) of the highlighted node.
    pub highlighted_path: Option<Vec<String>>,
    /// Spatial layout from the last render (populated by [`TreemapWidget::render`]).
    pub layout: TreemapLayout,
}

// ---------------------------------------------------------------------------
// TreemapWidget
// ---------------------------------------------------------------------------

/// Recursive squarified treemap widget.
///
/// Renders a [`DirNode`] tree by squarifying each directory's children
/// within its allocated area. Files become `HalfBlock` pixel-grid cells with
/// edge darkening and optional filename labels.
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

        let bg = Color::Reset;
        let mut grid = PixelGrid::new(area.width, area.height, bg);
        let mut cell_layouts: Vec<CellLayout> = Vec::new();

        paint_recursive(
            self.root,
            area,
            area,
            &mut grid,
            bg,
            state.highlighted_path.as_deref(),
            &[],
            &mut cell_layouts,
        );

        grid.flush_to_buffer(buf, area);

        // Overlay filename labels on cells that are wide and tall enough.
        for layout in &cell_layouts {
            let rect = layout.rect;
            if rect.width < 8 || rect.height < 2 {
                continue;
            }
            let color = file_color(layout.extension.as_deref());
            let text_color = contrast_text_color(color);
            let label = truncate_label(&layout.name, usize::from(rect.width));
            let label_y = rect.y + rect.height / 2;
            buf.set_string(
                rect.x,
                label_y,
                &label,
                Style::default().fg(text_color).bg(color),
            );
        }

        // Overlay double-border highlight for the exactly-matched cell.
        let highlight = state.highlighted_path.as_deref();
        for layout in &cell_layouts {
            let is_exact = highlight.is_some_and(|hp| hp == layout.path.as_slice());
            if is_exact {
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(Color::White))
                    .render(layout.rect, buf);
            }
        }

        state.layout = TreemapLayout {
            cells: cell_layouts,
        };
    }
}

// ---------------------------------------------------------------------------
// Colour helpers
// ---------------------------------------------------------------------------

/// Return the Okabe-Ito category colour for a file extension.
///
/// Falls back to grayscale when the `NO_COLOR` environment variable is set.
fn file_color(ext: Option<&str>) -> Color {
    let category = FileCategory::from_extension(ext.map(OsStr::new));
    if is_color_enabled() {
        category_color(category)
    } else {
        no_color_fallback(category)
    }
}

// ---------------------------------------------------------------------------
// Recursive paint
// ---------------------------------------------------------------------------

/// Squarify and paint `node`'s children into `grid`.
///
/// - `node_area`: absolute buffer-space [`Rect`] allocated to this node.
/// - `root_area`: the overall treemap area, used as the [`PixelGrid`] origin.
#[allow(clippy::too_many_arguments)]
fn paint_recursive(
    node: &DirNode,
    node_area: Rect,
    root_area: Rect,
    grid: &mut PixelGrid,
    bg: Color,
    highlight: Option<&[String]>,
    current_path: &[String],
    cell_layouts: &mut Vec<CellLayout>,
) {
    let children: Vec<&DirNode> = node.children.iter().filter(|c| c.size > 0).collect();
    if children.is_empty() || node_area.width == 0 || node_area.height == 0 {
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
        w: f32::from(node_area.width),
        h: f32::from(node_area.height),
    };

    streemap::squarify(
        container,
        &mut layout,
        #[allow(clippy::cast_precision_loss)]
        |(child, _r)| child.size as f32,
        |(_child, r), new_r| *r = new_r,
    );

    for (child, f32_rect) in &layout {
        let cell_rect = f32_rect_to_ratatui(*f32_rect, node_area);
        if cell_rect.width == 0 || cell_rect.height == 0 {
            continue;
        }

        let mut child_path = current_path.to_vec();
        child_path.push(child.name.clone());

        if child.is_dir {
            paint_dir_cell(
                child,
                cell_rect,
                root_area,
                grid,
                bg,
                highlight,
                &child_path,
                cell_layouts,
            );
        } else {
            paint_file_cell(child, cell_rect, root_area, grid, &child_path, cell_layouts);
        }
    }
}

/// Paint a directory cell: recurse if large enough, otherwise fill with dominant color.
#[allow(clippy::too_many_arguments)]
fn paint_dir_cell(
    child: &DirNode,
    cell_rect: Rect,
    root_area: Rect,
    grid: &mut PixelGrid,
    bg: Color,
    highlight: Option<&[String]>,
    child_path: &[String],
    cell_layouts: &mut Vec<CellLayout>,
) {
    let cell_area = u32::from(cell_rect.width) * u32::from(cell_rect.height);
    if cell_area >= 2 {
        // Large enough to recurse: indent one column when the cell is wide/tall enough.
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
        paint_recursive(
            child,
            inner,
            root_area,
            grid,
            bg,
            highlight,
            child_path,
            cell_layouts,
        );
    } else {
        // Too small to recurse: fill with the dominant child colour.
        let color = dominant_color(child);
        let (px, py, pw, ph) = to_pixel_coords(cell_rect, root_area);
        grid.fill_rect(px, py, pw, ph, color);
        grid.darken_edges(px, py, pw, ph);
        cell_layouts.push(CellLayout {
            rect: cell_rect,
            name: child.name.clone(),
            extension: child.extension.clone(),
            is_dir: true,
            size: child.size,
            mtime: child.mtime,
            path: child_path.to_vec(),
        });
    }
}

/// Paint a file leaf cell: fill `PixelGrid`, darken edges, record layout.
fn paint_file_cell(
    child: &DirNode,
    cell_rect: Rect,
    root_area: Rect,
    grid: &mut PixelGrid,
    child_path: &[String],
    cell_layouts: &mut Vec<CellLayout>,
) {
    let color = file_color(child.extension.as_deref());
    let (px, py, pw, ph) = to_pixel_coords(cell_rect, root_area);
    grid.fill_rect(px, py, pw, ph, color);
    grid.darken_edges(px, py, pw, ph);
    cell_layouts.push(CellLayout {
        rect: cell_rect,
        name: child.name.clone(),
        extension: child.extension.clone(),
        is_dir: false,
        size: child.size,
        mtime: child.mtime,
        path: child_path.to_vec(),
    });
}

// ---------------------------------------------------------------------------
// Coordinate helpers
// ---------------------------------------------------------------------------

/// Convert a terminal cell rect (absolute buffer coords) to pixel coords for the `PixelGrid`.
///
/// The `PixelGrid` origin is at `root_area`'s top-left corner.
/// Pixel y and pixel height are doubled: two pixel rows per terminal row.
const fn to_pixel_coords(rect: Rect, root_area: Rect) -> (u16, u16, u16, u16) {
    let px = rect.x.saturating_sub(root_area.x);
    let py = rect.y.saturating_sub(root_area.y).saturating_mul(2);
    let pw = rect.width;
    let ph = rect.height.saturating_mul(2);
    (px, py, pw, ph)
}

/// Return the Okabe-Ito colour of the largest file in a subtree.
///
/// Iteratively chases the largest child to avoid stack overflow on deep trees.
fn dominant_color(node: &DirNode) -> Color {
    let mut current = node;
    loop {
        if !current.is_dir {
            return file_color(current.extension.as_deref());
        }
        match current.children.iter().max_by_key(|c| c.size) {
            Some(child) => current = child,
            None => return file_color(None),
        }
    }
}

/// Truncate `name` to fit in `max_chars` terminal columns, appending `…` if needed.
fn truncate_label(name: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if name.chars().count() <= max_chars {
        return name.to_string();
    }
    let truncated: String = name.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{truncated}…")
}

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
    use crate::types::FileCategory;

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

    // --- New HalfBlock rendering tests ---

    #[test]
    fn treemap_renders_half_block_characters() {
        let root = make_dir("root", 100, vec![make_file("a.rs", 100, Some("rs"))]);
        let buf = render_treemap(&root, 40, 10);
        let has_half = buf
            .content()
            .iter()
            .any(|c| c.symbol() == "▀" || c.symbol() == "█");
        assert!(has_half, "expected half-block characters in output");
    }

    #[test]
    fn treemap_uses_category_colors_not_extension_hash() {
        let root = make_dir("root", 100, vec![make_file("a.rs", 100, Some("rs"))]);
        let buf = render_treemap(&root, 40, 10);
        let code_color = crate::ui::colors::category_color(FileCategory::Code);
        let has_code_color = buf
            .content()
            .iter()
            .any(|c| c.fg == code_color || c.bg == code_color);
        assert!(has_code_color, "expected Okabe-Ito blue for .rs file");
    }

    #[test]
    fn treemap_edge_pixels_are_darker_than_interior() {
        let root = make_dir("root", 100, vec![make_file("a.rs", 100, Some("rs"))]);
        let buf = render_treemap(&root, 40, 10);
        let code_color = crate::ui::colors::category_color(FileCategory::Code);
        // At least some cells should have colors that differ from the base category color
        // (the edge-darkened variants).
        let has_darkened = buf.content().iter().any(|c| {
            let fg_is_variant = c.fg != code_color && c.fg != Color::Reset;
            let bg_is_variant = c.bg != code_color && c.bg != Color::Reset;
            (fg_is_variant || bg_is_variant) && c.symbol() == "▀"
        });
        assert!(has_darkened, "expected edge-darkened colors");
    }

    #[test]
    fn treemap_labels_on_large_cells() {
        let root = make_dir("root", 100, vec![make_file("bigfile.rs", 100, Some("rs"))]);
        let buf = render_treemap(&root, 40, 10);
        let content: String = buf
            .content()
            .iter()
            .map(|c| c.symbol().to_string())
            .collect();
        assert!(
            content.contains("bigfile.rs"),
            "expected filename label on large cell"
        );
    }

    #[test]
    fn treemap_no_label_on_tiny_cells() {
        let files: Vec<DirNode> = (0..50)
            .map(|i| make_file(&format!("f{i}.rs"), 2, Some("rs")))
            .collect();
        let root = make_dir("root", 100, files);
        let buf = render_treemap(&root, 40, 10);
        let content: String = buf
            .content()
            .iter()
            .map(|c| c.symbol().to_string())
            .collect();
        assert!(
            !content.contains("f0.rs"),
            "tiny cells should not have labels"
        );
    }

    // --- Updated existing tests (category colors instead of extension_color) ---

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
                    make_file("b.zip", 40, Some("zip")),
                ],
            )],
        );
        let buf = render_treemap(&root, 40, 10);
        let code_color = crate::ui::colors::category_color(FileCategory::Code);
        let archive_color = crate::ui::colors::category_color(FileCategory::Archive);
        let has_code = buf
            .content()
            .iter()
            .any(|c| c.fg == code_color || c.bg == code_color);
        let has_archive = buf
            .content()
            .iter()
            .any(|c| c.fg == archive_color || c.bg == archive_color);
        assert!(has_code, "expected Code-colored cells for .rs file");
        assert!(has_archive, "expected Archive-colored cells for .zip file");
    }

    #[test]
    fn treemap_proportional_areas() {
        let root = make_dir(
            "root",
            100,
            vec![
                make_file("big.rs", 75, Some("rs")),
                make_file("small.zip", 25, Some("zip")),
            ],
        );
        let buf = render_treemap(&root, 80, 24);
        let total = 80_usize * 24;
        let code_color = crate::ui::colors::category_color(FileCategory::Code);
        // Interior cells of the big file get fg=code_color (full-block █ rendering).
        let code_count = buf.content().iter().filter(|c| c.fg == code_color).count();
        assert!(
            code_count * 100 / total >= 60,
            "expected ≥60% for 75% file, got {code_count}/{total}"
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
        let code_color = crate::ui::colors::category_color(FileCategory::Code);
        let has_code = buf
            .content()
            .iter()
            .any(|c| c.fg == code_color || c.bg == code_color);
        assert!(has_code, "nested file should produce Code-colored cells");
    }

    #[test]
    fn treemap_pruning_small_cells() {
        let files: Vec<DirNode> = (0..100)
            .map(|i| make_file(&format!("f{i}.txt"), 1, Some("txt")))
            .collect();
        let root = make_dir("root", 100, files);
        let buf = render_treemap(&root, 20, 5);
        // With HalfBlock rendering, colored cells use fg for full-block (█) chars.
        let colored = buf.content().iter().filter(|c| c.symbol() != " ").count();
        assert!(
            colored > 0,
            "pruned cells should still render non-space chars"
        );
    }

    #[test]
    fn treemap_deep_nesting_no_overflow() {
        let mut node = make_file("leaf.rs", 100, Some("rs"));
        for i in 0..150 {
            node = make_dir(&format!("d{i}"), 100, vec![node]);
        }
        let buf = render_treemap(&node, 40, 10);
        let has_content = buf.content().iter().any(|c| c.symbol() != " ");
        assert!(
            has_content,
            "deep tree should render without stack overflow"
        );
    }

    #[test]
    fn treemap_dominant_file_shows_siblings() {
        let root = make_dir(
            "root",
            100,
            vec![
                make_file("huge.zip", 99, Some("zip")),
                make_file("tiny.rs", 1, Some("rs")),
            ],
        );
        let buf = render_treemap(&root, 100, 10);
        let archive_color = crate::ui::colors::category_color(FileCategory::Archive);
        // Interior cells of huge.zip get fg=archive_color.
        let has_archive = buf.content().iter().any(|c| c.fg == archive_color);
        assert!(has_archive, "dominant file should be visible");
        // tiny.rs produces at least one non-space, non-archive-colored cell.
        let has_distinct = buf
            .content()
            .iter()
            .any(|c| c.symbol() != " " && c.fg != archive_color && c.fg != Color::Reset);
        assert!(
            has_distinct,
            "tiny sibling should produce at least 1 colored cell distinct from archive_color"
        );
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
            ..Default::default()
        };
        terminal
            .draw(|f| {
                let widget = TreemapWidget { root: &root };
                f.render_stateful_widget(widget, f.area(), &mut state);
            })
            .expect("draw");
        let buf = terminal.backend().buffer().clone();
        // Highlighted cell receives a double-line border.
        let has_border = buf
            .content()
            .iter()
            .any(|c| c.symbol() == "╔" || c.symbol() == "║" || c.symbol() == "═");
        assert!(
            has_border,
            "highlighted path should have a double-line border"
        );
    }

    #[test]
    fn treemap_empty_root() {
        let root = make_dir("root", 0, Vec::new());
        let buf = render_treemap(&root, 40, 10);
        let non_space = buf.content().iter().filter(|c| c.symbol() != " ").count();
        assert_eq!(non_space, 0, "empty root should render nothing");
    }

    #[test]
    fn cell_layout_default_is_sensible() {
        let layout = CellLayout::default();
        assert_eq!(layout.name, "");
        assert_eq!(layout.size, 0);
        assert!(!layout.is_dir);
    }

    #[test]
    fn treemap_layout_stored_in_state() {
        let root = make_dir("root", 100, vec![make_file("a.rs", 100, Some("rs"))]);
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = TreemapState::default();
        terminal
            .draw(|f| {
                let widget = TreemapWidget { root: &root };
                f.render_stateful_widget(widget, f.area(), &mut state);
            })
            .expect("draw");
        assert!(
            !state.layout.cells.is_empty(),
            "layout should be populated after render"
        );
        let cell = &state.layout.cells[0];
        assert_eq!(cell.name, "a.rs");
        assert!(!cell.is_dir);
    }
}
