//! Treemap data types and spatial navigation.
//!
//! This module defines the core data structures for the squarified treemap:
//! - [`CellLayout`] — per-cell geometry captured during render for navigation.
//! - [`TreemapLayout`] — the full spatial layout from the most recent render.
//! - [`TreemapState`] — mutable state (selection, highlight, layout).
//! - [`Direction`] — spatial navigation direction enum.
//!
//! Rendering is implemented in [`crate::ui::visualization::treemap::TreemapVisualization`],
//! which conforms to the [`crate::ui::visualization::Visualization`] trait.

use std::time::SystemTime;

use crate::ui::tree::DirNode;

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
    pub rect: ratatui::layout::Rect,
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
    pub fn from_node(child: &DirNode, rect: ratatui::layout::Rect, child_path: &[String]) -> Self {
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
            rect: ratatui::layout::Rect::default(),
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

/// Mutable state for the treemap visualization.
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use ratatui::layout::Rect;

    use super::*;

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
    fn cell_layout_from_node() {
        use crate::ui::tree::DirNode;
        let node = DirNode {
            name: "foo.rs".to_owned(),
            size: 42,
            allocated: 42,
            file_count: 1,
            children: Vec::new(),
            is_dir: false,
            extension: Some("rs".to_owned()),
            mtime: SystemTime::UNIX_EPOCH,
        };
        let rect = Rect::new(1, 2, 10, 5);
        let path = vec!["foo.rs".to_owned()];
        let cell = CellLayout::from_node(&node, rect, &path);
        assert_eq!(cell.name, "foo.rs");
        assert_eq!(cell.size, 42);
        assert!(!cell.is_dir);
        assert_eq!(cell.path, path);
    }
}
