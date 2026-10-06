//! TUI application state types.
//!
//! Defines the top-level [`AppState`] enum and per-state data structs
//! that drive what the TUI renders.

use std::{path::PathBuf, time::Duration};

use tui_tree_widget::TreeState;

use crate::{
    types::ScanProgress,
    ui::tree::{DirNode, ExtensionStat, collect_extension_stats, find_node},
    ui::widgets::treemap::TreemapState,
};

/// State for the scan-in-progress view.
#[derive(Debug)]
pub struct ScanProgressState {
    /// Number of files scanned so far.
    pub file_count: u64,
    /// Current scan rate in files per second.
    pub files_per_sec: f64,
    /// Elapsed time since the scan started.
    pub elapsed: Duration,
    /// Path of the most recently scanned entry.
    pub current_path: PathBuf,
    /// Whether the current process is running as root (effective UID = 0).
    pub is_root: bool,
}

impl ScanProgressState {
    /// Update this state from a [`ScanProgress`] snapshot received from the pipeline.
    pub fn update(&mut self, progress: ScanProgress) {
        self.file_count = progress.entries_scanned;
        self.files_per_sec = progress.entries_per_second;
        self.elapsed = if progress.elapsed_secs.is_finite() && progress.elapsed_secs >= 0.0 {
            Duration::from_secs_f64(progress.elapsed_secs)
        } else {
            Duration::ZERO
        };
        self.current_path = progress.current_path;
    }
}

/// Which panel currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelFocus {
    /// The directory tree panel.
    Tree,
    /// The extension legend panel.
    Legend,
}

/// Sort field for directory tree children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeSortField {
    /// Sort by file/directory size.
    Size,
    /// Sort by name (alphabetical).
    Name,
    /// Sort by modification time.
    Modified,
}

/// State for the file explorer view.
#[derive(Debug)]
pub struct ExplorerState {
    /// The full directory tree from the scan.
    pub tree: DirNode,
    /// State for the `tui-tree-widget` tree panel.
    pub tree_state: TreeState<String>,
    /// Per-extension statistics (scoped to current treemap root).
    pub extension_stats: Vec<ExtensionStat>,
    /// Path components from scan root to current treemap zoom level.
    pub treemap_root: Vec<String>,
    /// Treemap highlight state.
    pub treemap_state: TreemapState,
    /// Which panel has keyboard focus.
    pub focus: PanelFocus,
    /// Current sort field for tree children.
    pub sort_field: TreeSortField,
    /// Whether sort is ascending (`false` = descending, the default).
    pub sort_ascending: bool,
    /// Whether the help overlay is visible.
    pub show_help: bool,
    /// Transient error message displayed as a status line.
    pub error_message: Option<String>,
    /// Scroll offset for the extension legend.
    pub legend_scroll: usize,
    /// The scan root path (for display in breadcrumb).
    pub scan_root: PathBuf,
}

impl ExplorerState {
    /// Create a new explorer state from a built tree.
    pub fn new(tree: DirNode, scan_root: PathBuf) -> Self {
        let extension_stats = collect_extension_stats(&tree);
        Self {
            tree,
            tree_state: TreeState::default(),
            extension_stats,
            treemap_root: Vec::new(),
            treemap_state: TreemapState::default(),
            focus: PanelFocus::Tree,
            sort_field: TreeSortField::Size,
            sort_ascending: false,
            show_help: false,
            error_message: None,
            legend_scroll: 0,
            scan_root,
        }
    }

    /// Get the `DirNode` at the current treemap zoom level.
    pub fn current_treemap_node(&self) -> &DirNode {
        find_node(&self.tree, &self.treemap_root).unwrap_or(&self.tree)
    }

    /// Zoom the treemap to the given path (from tree root).
    ///
    /// Only zooms if the target is a directory that exists in the tree.
    pub fn zoom_to(&mut self, path: Vec<String>) {
        if find_node(&self.tree, &path).is_some_and(|n| n.is_dir) {
            self.treemap_root = path;
            self.recompute_extension_stats();
        }
    }

    /// Zoom the treemap out one level.
    pub fn zoom_out(&mut self) {
        if !self.treemap_root.is_empty() {
            self.treemap_root.pop();
            self.recompute_extension_stats();
        }
    }

    /// Reset the treemap to the scan root.
    pub fn zoom_to_root(&mut self) {
        if !self.treemap_root.is_empty() {
            self.treemap_root.clear();
            self.recompute_extension_stats();
        }
    }

    /// Toggle keyboard focus between tree and legend panels.
    #[allow(clippy::missing_const_for_fn)]
    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            PanelFocus::Tree => PanelFocus::Legend,
            PanelFocus::Legend => PanelFocus::Tree,
        };
    }

    /// Set the sort field for tree children.
    #[allow(clippy::missing_const_for_fn)]
    pub fn set_sort(&mut self, field: TreeSortField) {
        self.sort_field = field;
        self.sort_ascending = false;
    }

    /// Toggle the sort direction.
    #[allow(clippy::missing_const_for_fn)]
    pub fn toggle_sort_direction(&mut self) {
        self.sort_ascending = !self.sort_ascending;
    }

    /// Build the breadcrumb path string for display.
    pub fn breadcrumb_path(&self) -> String {
        let root_display = self.scan_root.display().to_string();
        if self.treemap_root.is_empty() {
            root_display
        } else {
            format!("{}/{}", root_display, self.treemap_root.join("/"))
        }
    }

    /// Recompute extension stats scoped to the current treemap root.
    fn recompute_extension_stats(&mut self) {
        let node = find_node(&self.tree, &self.treemap_root).unwrap_or(&self.tree);
        self.extension_stats = collect_extension_stats(node);
        self.legend_scroll = 0;
    }
}

/// Top-level TUI application state.
///
/// The TUI is always in one of these two states — scanning (showing
/// progress) or exploring (showing the interactive treemap view).
#[derive(Debug)]
pub enum AppState {
    /// Scan in progress — show progress view.
    Scanning(ScanProgressState),
    /// Exploring scan results — show explorer view.
    Exploring(Box<ExplorerState>),
}
