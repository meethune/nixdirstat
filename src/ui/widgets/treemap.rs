//! Treemap data types, spatial navigation, and compatibility widget shim.
//!
//! This module defines the core data structures for the squarified treemap:
//! - [`CellLayout`] — per-cell geometry captured during render for navigation.
//! - [`TreemapLayout`] — the full spatial layout from the most recent render.
//! - [`TreemapState`] — mutable state (selection, highlight, layout).
//! - [`Direction`] — spatial navigation direction enum.
//!
//! Rendering is implemented in [`crate::ui::visualization::treemap::TreemapVisualization`],
//! which conforms to the [`crate::ui::visualization::Visualization`] trait.
//!
//! [`TreemapWidget`] is a thin compatibility shim that delegates to
//! [`TreemapVisualization`] for code paths that still use the stateful-widget API.

use std::time::SystemTime;

use ratatui::{buffer::Buffer, layout::Rect, widgets::StatefulWidget};

use crate::ui::{
    tree::DirNode,
    visualization::treemap::TreemapVisualization,
    visualization::{ColorScheme, RenderParams, Visualization as _},
};

// ---------------------------------------------------------------------------
// CellLayout
// ---------------------------------------------------------------------------

/// Per-cell geometry and metadata captured during render for spatial navigation.
///
/// Consumed by the explorer shell for cursor-based navigation within the treemap.
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

impl CellLayout {
    /// Construct a [`CellLayout`] from a [`DirNode`], its allocated rect, and its path.
    pub fn from_node(child: &DirNode, rect: Rect, child_path: &[String]) -> Self {
        Self {
            rect,
            name: child.name.clone(),
            extension: child.extension.clone(),
            is_dir: child.is_dir,
            size: child.size,
            mtime: child.mtime,
            path: child_path.to_vec(),
        }
    }
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

/// Spatial layout produced by the most recent render, for navigation.
#[derive(Debug, Clone, Default)]
pub struct TreemapLayout {
    /// All leaf cells in the last render, in paint order.
    pub cells: Vec<CellLayout>,
}

// ---------------------------------------------------------------------------
// Direction
// ---------------------------------------------------------------------------

/// Spatial navigation direction for treemap cell movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Move toward the top of the screen.
    Up,
    /// Move toward the bottom of the screen.
    Down,
    /// Move toward the left of the screen.
    Left,
    /// Move toward the right of the screen.
    Right,
}

// ---------------------------------------------------------------------------
// TreemapState
// ---------------------------------------------------------------------------

/// Mutable state for the treemap widget.
///
/// Tracks which node is highlighted (selected in the directory tree panel),
/// the spatial layout produced by the last render, and the keyboard-selected
/// cell index for treemap-panel navigation.
#[derive(Debug, Default, Clone)]
pub struct TreemapState {
    /// Path components (from treemap root) of the highlighted node.
    pub highlighted_path: Option<Vec<String>>,
    /// Spatial layout from the last render.
    pub layout: TreemapLayout,
    /// Index into `layout.cells` of the keyboard-selected cell, if any.
    pub selected_index: Option<usize>,
}

impl TreemapState {
    /// Move the selection one step in `direction` using nearest-neighbour spatial search.
    ///
    /// Algorithm:
    /// 1. Compute the current cell's centre `(cx, cy)`.
    /// 2. Filter candidate cells by half-plane (e.g. Right: `candidate.cx > cx`).
    /// 3. Pick the candidate with the smallest squared Euclidean distance.
    ///
    /// Returns `true` if the selection moved, `false` if no candidate exists in
    /// that direction (edge of the treemap) or if no cell is currently selected.
    pub fn move_selection(&mut self, direction: Direction) -> bool {
        let cells = &self.layout.cells;
        let idx = match self.selected_index {
            Some(i) if i < cells.len() => i,
            _ => return false,
        };
        let cur = &cells[idx];
        let cx = i32::from(cur.rect.x + cur.rect.width / 2);
        let cy = i32::from(cur.rect.y + cur.rect.height / 2);

        let nearest = cells
            .iter()
            .enumerate()
            .filter(|(i, c)| {
                if *i == idx {
                    return false;
                }
                let ncx = i32::from(c.rect.x + c.rect.width / 2);
                let ncy = i32::from(c.rect.y + c.rect.height / 2);
                match direction {
                    Direction::Right => ncx > cx,
                    Direction::Left => ncx < cx,
                    Direction::Down => ncy > cy,
                    Direction::Up => ncy < cy,
                }
            })
            .min_by_key(|(_, c)| {
                let ncx = i32::from(c.rect.x + c.rect.width / 2);
                let ncy = i32::from(c.rect.y + c.rect.height / 2);
                // Use i64 to avoid overflow: u16 coordinates give centres up to
                // ~98 302, so dx² can reach ~9.66 × 10⁹ — exceeding i32::MAX.
                let dx = i64::from(ncx) - i64::from(cx);
                let dy = i64::from(ncy) - i64::from(cy);
                dx * dx + dy * dy
            });

        match nearest {
            Some((new_idx, _)) => {
                self.selected_index = Some(new_idx);
                true
            },
            None => false,
        }
    }
}

// ---------------------------------------------------------------------------
// TreemapWidget (compatibility shim)
// ---------------------------------------------------------------------------

/// Thin compatibility shim that delegates to [`TreemapVisualization`].
///
/// Preserves the [`StatefulWidget`] API used by the explorer shell while the
/// shell rewiring (Task 5) is pending. All rendering is performed by
/// [`TreemapVisualization::render`]; this struct adds no logic of its own.
pub struct TreemapWidget<'a> {
    /// The root node to render (may be a subtree for zoom).
    pub root: &'a DirNode,
}

impl StatefulWidget for TreemapWidget<'_> {
    type State = TreemapState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let params = RenderParams::from_area(area);
        let mut viz = TreemapVisualization::from_state(state.clone());
        viz.render(self.root, area, buf, &params, &ColorScheme::default());
        *state = viz.into_state();
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

    // --- HalfBlock rendering tests (via thin wrapper) ---

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
        let has_darkened = buf.content().iter().any(|c| {
            let fg_is_variant = c.fg != code_color && c.fg != ratatui::style::Color::Reset;
            let bg_is_variant = c.bg != code_color && c.bg != ratatui::style::Color::Reset;
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
        let has_archive = buf.content().iter().any(|c| c.fg == archive_color);
        assert!(has_archive, "dominant file should be visible");
        let has_distinct = buf.content().iter().any(|c| {
            c.symbol() != " " && c.fg != archive_color && c.fg != ratatui::style::Color::Reset
        });
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

    // --- Spatial navigation tests ---

    #[test]
    fn move_selection_right_finds_nearest_neighbor() {
        let mut state = TreemapState {
            selected_index: Some(0),
            layout: TreemapLayout {
                cells: vec![
                    CellLayout {
                        rect: Rect::new(0, 0, 10, 10),
                        ..Default::default()
                    },
                    CellLayout {
                        rect: Rect::new(10, 0, 10, 10),
                        ..Default::default()
                    },
                ],
            },
            ..Default::default()
        };
        assert!(state.move_selection(Direction::Right));
        assert_eq!(state.selected_index, Some(1));
    }

    #[test]
    fn move_selection_at_edge_is_noop() {
        let mut state = TreemapState {
            selected_index: Some(0),
            layout: TreemapLayout {
                cells: vec![CellLayout {
                    rect: Rect::new(0, 0, 10, 10),
                    ..Default::default()
                }],
            },
            ..Default::default()
        };
        assert!(!state.move_selection(Direction::Right));
        assert_eq!(state.selected_index, Some(0));
    }

    #[test]
    fn move_selection_left_finds_neighbor() {
        let mut state = TreemapState {
            selected_index: Some(1),
            layout: TreemapLayout {
                cells: vec![
                    CellLayout {
                        rect: Rect::new(0, 0, 10, 10),
                        ..Default::default()
                    },
                    CellLayout {
                        rect: Rect::new(10, 0, 10, 10),
                        ..Default::default()
                    },
                ],
            },
            ..Default::default()
        };
        assert!(state.move_selection(Direction::Left));
        assert_eq!(state.selected_index, Some(0));
    }

    #[test]
    fn move_selection_down_finds_neighbor() {
        let mut state = TreemapState {
            selected_index: Some(0),
            layout: TreemapLayout {
                cells: vec![
                    CellLayout {
                        rect: Rect::new(0, 0, 10, 5),
                        ..Default::default()
                    },
                    CellLayout {
                        rect: Rect::new(0, 5, 10, 5),
                        ..Default::default()
                    },
                ],
            },
            ..Default::default()
        };
        assert!(state.move_selection(Direction::Down));
        assert_eq!(state.selected_index, Some(1));
    }

    #[test]
    fn move_selection_up_finds_neighbor() {
        let mut state = TreemapState {
            selected_index: Some(1),
            layout: TreemapLayout {
                cells: vec![
                    CellLayout {
                        rect: Rect::new(0, 0, 10, 5),
                        ..Default::default()
                    },
                    CellLayout {
                        rect: Rect::new(0, 5, 10, 5),
                        ..Default::default()
                    },
                ],
            },
            ..Default::default()
        };
        assert!(state.move_selection(Direction::Up));
        assert_eq!(state.selected_index, Some(0));
    }

    #[test]
    fn move_selection_none_index_returns_false() {
        let mut state = TreemapState {
            layout: TreemapLayout {
                cells: vec![CellLayout {
                    rect: Rect::new(0, 0, 10, 10),
                    ..Default::default()
                }],
            },
            ..Default::default()
        };
        assert!(!state.move_selection(Direction::Right));
    }

    #[test]
    fn move_selection_picks_nearest_by_distance() {
        // Cell 0 at (0,0). Cell 1 far right at (100,0). Cell 2 close right at (15,0).
        let mut state = TreemapState {
            selected_index: Some(0),
            layout: TreemapLayout {
                cells: vec![
                    CellLayout {
                        rect: Rect::new(0, 0, 10, 10),
                        ..Default::default()
                    },
                    CellLayout {
                        rect: Rect::new(100, 0, 10, 10),
                        ..Default::default()
                    },
                    CellLayout {
                        rect: Rect::new(15, 0, 10, 10),
                        ..Default::default()
                    },
                ],
            },
            ..Default::default()
        };
        assert!(state.move_selection(Direction::Right));
        // Should pick cell 2 (closer) over cell 1 (farther).
        assert_eq!(state.selected_index, Some(2));
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
