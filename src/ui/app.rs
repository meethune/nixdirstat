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
    /// Create a new scan progress state with zeroed counters.
    pub const fn new(is_root: bool) -> Self {
        Self {
            file_count: 0,
            files_per_sec: 0.0,
            elapsed: Duration::ZERO,
            current_path: PathBuf::new(),
            is_root,
        }
    }

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
///
/// Fields are private; access through getters and guarded setters.
#[derive(Debug)]
pub struct ExplorerState {
    tree: DirNode,
    tree_state: TreeState<String>,
    extension_stats: Vec<ExtensionStat>,
    treemap_root: Vec<String>,
    treemap_state: TreemapState,
    focus: PanelFocus,
    sort_field: TreeSortField,
    sort_ascending: bool,
    show_help: bool,
    show_info: bool,
    error_message: Option<String>,
    legend_scroll: usize,
    scan_root: PathBuf,
    free_space: Option<crate::types::SpaceInfo>,
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
            show_info: false,
            error_message: None,
            legend_scroll: 0,
            scan_root,
            free_space: None,
        }
    }

    /// Get the `DirNode` at the current treemap zoom level.
    pub fn current_treemap_node(&self) -> &DirNode {
        find_node(&self.tree, &self.treemap_root).unwrap_or(&self.tree)
    }

    /// Zoom the treemap into the currently selected directory.
    ///
    /// The selected path from the tree widget is relative to the current
    /// treemap root. This method resolves it to an absolute path from the
    /// scan root, validates it's a directory, and resets the tree state.
    pub fn zoom_into_selected(&mut self) {
        let selected = self.tree_state.selected().to_vec();
        if selected.is_empty() {
            return;
        }
        // Build full path: treemap_root + selected path from tree widget.
        let mut full_path = self.treemap_root.clone();
        full_path.extend(selected);
        if find_node(&self.tree, &full_path).is_some_and(|n| n.is_dir) {
            self.treemap_root = full_path;
            self.reset_after_zoom();
        }
    }

    /// Zoom the treemap out one level.
    pub fn zoom_out(&mut self) {
        if !self.treemap_root.is_empty() {
            self.treemap_root.pop();
            self.reset_after_zoom();
        }
    }

    /// Reset the treemap to the scan root.
    pub fn zoom_to_root(&mut self) {
        if !self.treemap_root.is_empty() {
            self.treemap_root.clear();
            self.reset_after_zoom();
        }
    }

    /// Reset tree and legend state after a zoom operation.
    fn reset_after_zoom(&mut self) {
        self.tree_state = TreeState::default();
        self.tree_state.select_first();
        self.legend_scroll = 0;
        self.recompute_extension_stats();
    }

    /// Toggle keyboard focus between tree and legend panels.
    #[allow(clippy::missing_const_for_fn)] // &mut self methods are not const-eligible // &mut self methods are not const-eligible
    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            PanelFocus::Tree => PanelFocus::Legend,
            PanelFocus::Legend => PanelFocus::Tree,
        };
    }

    /// Set the sort field for tree children.
    #[allow(clippy::missing_const_for_fn)] // &mut self methods are not const-eligible // &mut self methods are not const-eligible
    pub fn set_sort(&mut self, field: TreeSortField) {
        self.sort_field = field;
        self.sort_ascending = false;
    }

    /// Toggle the sort direction.
    #[allow(clippy::missing_const_for_fn)] // &mut self methods are not const-eligible // &mut self methods are not const-eligible
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

    // --- Getters ---

    /// The full directory tree from the scan.
    pub const fn tree(&self) -> &DirNode {
        &self.tree
    }

    /// State for the `tui-tree-widget` tree panel (shared ref).
    pub const fn tree_state(&self) -> &TreeState<String> {
        &self.tree_state
    }

    /// State for the `tui-tree-widget` tree panel (mutable ref).
    pub const fn tree_state_mut(&mut self) -> &mut TreeState<String> {
        &mut self.tree_state
    }

    /// Split borrow: `tree` (shared) + `tree_state` (mutable).
    pub const fn tree_and_tree_state_mut(&mut self) -> (&DirNode, &mut TreeState<String>) {
        (&self.tree, &mut self.tree_state)
    }

    /// Split borrow: `tree` (shared) + `treemap_state` (mutable).
    pub const fn tree_and_treemap_state_mut(&mut self) -> (&DirNode, &mut TreemapState) {
        (&self.tree, &mut self.treemap_state)
    }

    /// Path components from scan root to current treemap zoom level.
    pub fn treemap_root(&self) -> &[String] {
        &self.treemap_root
    }

    /// Treemap highlight state (mutable ref).
    pub const fn treemap_state_mut(&mut self) -> &mut TreemapState {
        &mut self.treemap_state
    }

    /// Per-extension statistics scoped to current treemap root.
    pub fn extension_stats(&self) -> &[ExtensionStat] {
        &self.extension_stats
    }

    /// Current sort field for tree children.
    pub const fn sort_field(&self) -> TreeSortField {
        self.sort_field
    }

    /// Whether sort is ascending.
    pub const fn sort_ascending(&self) -> bool {
        self.sort_ascending
    }

    /// Whether the help overlay is visible.
    pub const fn show_help(&self) -> bool {
        self.show_help
    }

    /// Whether the file info popup is visible.
    pub const fn show_info(&self) -> bool {
        self.show_info
    }

    /// Transient error message displayed as a status line.
    pub fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    /// Scroll offset for the extension legend.
    pub const fn legend_scroll(&self) -> usize {
        self.legend_scroll
    }

    /// The scan root path.
    pub fn scan_root(&self) -> &std::path::Path {
        &self.scan_root
    }

    /// Free/total/unknown disk space at the scan root.
    pub const fn free_space(&self) -> Option<&crate::types::SpaceInfo> {
        self.free_space.as_ref()
    }

    // --- Setters ---

    /// Set the free space info.
    #[allow(clippy::missing_const_for_fn)] // &mut self methods are not const-eligible
    pub fn set_free_space(&mut self, space: Option<crate::types::SpaceInfo>) {
        self.free_space = space;
    }

    /// Toggle the help overlay.
    #[allow(clippy::missing_const_for_fn)] // &mut self methods are not const-eligible
    pub fn toggle_show_help(&mut self) {
        self.show_help = !self.show_help;
    }

    /// Toggle the file info popup.
    #[allow(clippy::missing_const_for_fn)] // &mut self methods are not const-eligible
    pub fn toggle_show_info(&mut self) {
        self.show_info = !self.show_info;
    }

    /// Clear the transient error message.
    #[allow(clippy::missing_const_for_fn)] // &mut self methods are not const-eligible
    pub fn clear_error(&mut self) {
        self.error_message = None;
    }

    /// Sync the treemap highlight to the current tree selection.
    pub fn sync_treemap_highlight(&mut self) {
        let selected = self.tree_state.selected();
        self.treemap_state.highlighted_path = if selected.is_empty() {
            None
        } else {
            Some(selected.to_vec())
        };
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
