//! TUI application state types.
//!
//! Defines the top-level [`AppState`] enum and per-state data structs
//! that drive what the TUI renders.

use std::{path::PathBuf, time::Duration};

use tui_tree_widget::TreeState;

use crate::{
    types::{ScanProgress, ScanWarning},
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
    /// Whether the scan is currently paused.
    pub paused: bool,
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
            paused: false,
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
    /// The squarified treemap panel.
    Treemap,
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
#[allow(clippy::struct_excessive_bools)]
pub struct ExplorerState {
    tree: DirNode,
    tree_state: TreeState<String>,
    extension_stats: Vec<ExtensionStat>,
    treemap_root: Vec<String>,
    treemap_state: TreemapState,
    focus: PanelFocus,
    sort_field: TreeSortField,
    sort_ascending: bool,
    popup: PopupState,
    error_message: Option<String>,
    legend_scroll: usize,
    scan_root: PathBuf,
    free_space: Option<crate::types::SpaceInfo>,
    warnings: Vec<ScanWarning>,
    warnings_scroll: usize,
    warnings_viewport: usize,
    search_active: bool,
    search_query: String,
    preview_content: Vec<String>,
    preview_scroll: usize,
    preview_title: String,
    refresh_requested: bool,
    filesystem_changed: bool,
}

/// Which popup overlay (if any) is currently displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PopupState {
    /// No popup is visible.
    #[default]
    None,
    /// The help overlay is visible.
    Help,
    /// The file info popup is visible.
    Info,
    /// The scan warnings popup is visible.
    Warnings,
    /// The file preview popup is visible.
    Preview,
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
            popup: PopupState::None,
            error_message: None,
            legend_scroll: 0,
            scan_root,
            free_space: None,
            warnings: Vec::new(),
            warnings_scroll: 0,
            warnings_viewport: 1,
            search_active: false,
            search_query: String::new(),
            preview_content: Vec::new(),
            preview_scroll: 0,
            preview_title: String::new(),
            refresh_requested: false,
            filesystem_changed: false,
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
        self.treemap_state.selected_index = None;
        self.recompute_extension_stats();
    }

    /// Cycle keyboard focus forward: Tree → Treemap → Legend → Tree.
    #[allow(clippy::missing_const_for_fn)] // &mut self methods are not const-eligible
    pub fn cycle_focus(&mut self) {
        self.focus = match self.focus {
            PanelFocus::Tree => PanelFocus::Treemap,
            PanelFocus::Treemap => PanelFocus::Legend,
            PanelFocus::Legend => PanelFocus::Tree,
        };
    }

    /// Zoom the treemap into a directory by its path relative to the current treemap root.
    ///
    /// Only descends if the resolved path points to a directory node.
    pub fn zoom_into_path(&mut self, relative_path: Vec<String>) {
        if relative_path.is_empty() {
            return;
        }
        let mut full_path = self.treemap_root.clone();
        full_path.extend(relative_path);
        if find_node(&self.tree, &full_path).is_some_and(|n| n.is_dir) {
            self.treemap_root = full_path;
            self.reset_after_zoom();
        }
    }

    /// Set the sort field for tree children.
    #[allow(clippy::missing_const_for_fn)] // &mut self methods are not const-eligible
    pub fn set_sort(&mut self, field: TreeSortField) {
        self.sort_field = field;
        self.sort_ascending = false;
    }

    /// Toggle the sort direction.
    #[allow(clippy::missing_const_for_fn)] // &mut self methods are not const-eligible
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

    /// Treemap widget state (shared ref).
    pub const fn treemap_state(&self) -> &TreemapState {
        &self.treemap_state
    }

    /// Treemap highlight state (mutable ref).
    pub const fn treemap_state_mut(&mut self) -> &mut TreemapState {
        &mut self.treemap_state
    }

    /// Per-extension statistics scoped to current treemap root.
    pub fn extension_stats(&self) -> &[ExtensionStat] {
        &self.extension_stats
    }

    /// Which panel currently has keyboard focus.
    pub const fn focus(&self) -> PanelFocus {
        self.focus
    }

    /// Directly set keyboard focus to the given panel.
    #[allow(clippy::missing_const_for_fn)] // &mut self methods are not const-eligible
    pub fn set_focus(&mut self, panel: PanelFocus) {
        self.focus = panel;
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
        matches!(self.popup, PopupState::Help)
    }

    /// Whether the file info popup is visible.
    pub const fn show_info(&self) -> bool {
        matches!(self.popup, PopupState::Info)
    }

    /// Whether the warnings popup is visible.
    pub const fn show_warnings(&self) -> bool {
        matches!(self.popup, PopupState::Warnings)
    }

    /// The current popup state.
    pub const fn popup(&self) -> PopupState {
        self.popup
    }

    /// Whether a modal popup is open that should consume all key events.
    pub const fn has_modal_popup(&self) -> bool {
        matches!(self.popup, PopupState::Warnings | PopupState::Preview)
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

    /// Set the scan warnings collected during the scan.
    pub fn set_warnings(&mut self, warnings: Vec<ScanWarning>) {
        self.warnings = warnings;
    }

    /// Non-fatal warnings collected during the scan.
    pub fn warnings(&self) -> &[ScanWarning] {
        &self.warnings
    }

    /// Current scroll offset for the warnings popup.
    pub const fn warnings_scroll(&self) -> usize {
        self.warnings_scroll
    }

    /// Scroll the warnings popup by `delta` lines (positive = down).
    ///
    /// Clamped so the last page fills the viewport using the viewport height
    /// recorded by the most recent render pass.
    pub fn scroll_warnings(&mut self, delta: isize) {
        let max = self.warnings.len().saturating_sub(self.warnings_viewport);
        if delta >= 0 {
            self.warnings_scroll = self
                .warnings_scroll
                .saturating_add(delta.unsigned_abs())
                .min(max);
        } else {
            self.warnings_scroll = self.warnings_scroll.saturating_sub(delta.unsigned_abs());
        }
    }

    /// Update the warnings popup viewport height (called by the renderer).
    pub fn set_warnings_viewport(&mut self, height: usize) {
        self.warnings_viewport = height.max(1);
        self.warnings_scroll = self
            .warnings_scroll
            .min(self.warnings.len().saturating_sub(self.warnings_viewport));
    }

    /// Toggle the help overlay.
    #[allow(clippy::missing_const_for_fn)] // &mut self methods are not const-eligible
    pub fn toggle_show_help(&mut self) {
        self.popup = if self.popup == PopupState::Help {
            PopupState::None
        } else {
            PopupState::Help
        };
    }

    /// Toggle the file info popup.
    #[allow(clippy::missing_const_for_fn)] // &mut self methods are not const-eligible
    pub fn toggle_show_info(&mut self) {
        self.popup = if self.popup == PopupState::Info {
            PopupState::None
        } else {
            PopupState::Info
        };
    }

    /// Toggle the warnings popup.
    #[allow(clippy::missing_const_for_fn)] // &mut self methods are not const-eligible
    pub fn toggle_show_warnings(&mut self) {
        self.popup = if self.popup == PopupState::Warnings {
            PopupState::None
        } else {
            PopupState::Warnings
        };
    }

    /// Whether the search input bar is active.
    pub const fn search_active(&self) -> bool {
        self.search_active
    }

    /// The current search/filter query.
    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    /// Open the search bar.
    #[allow(clippy::missing_const_for_fn)]
    pub fn open_search(&mut self) {
        self.search_active = true;
    }

    /// Close the search bar (keeps the query for continued filtering).
    #[allow(clippy::missing_const_for_fn)]
    pub fn close_search(&mut self) {
        self.search_active = false;
    }

    /// Close the search bar and clear the filter.
    pub fn cancel_search(&mut self) {
        self.search_active = false;
        self.search_query.clear();
    }

    /// Append a character to the search query.
    pub fn search_push(&mut self, c: char) {
        self.search_query.push(c);
    }

    /// Remove the last character from the search query.
    pub fn search_pop(&mut self) {
        self.search_query.pop();
    }

    /// Whether the preview popup is visible.
    pub const fn show_preview(&self) -> bool {
        matches!(self.popup, PopupState::Preview)
    }

    /// The preview content lines.
    pub fn preview_content(&self) -> &[String] {
        &self.preview_content
    }

    /// The preview title (filename).
    pub fn preview_title(&self) -> &str {
        &self.preview_title
    }

    /// Current scroll offset for the preview popup.
    pub const fn preview_scroll(&self) -> usize {
        self.preview_scroll
    }

    /// Set the preview content and open the preview popup.
    pub fn show_file_preview(&mut self, title: String, content: Vec<String>) {
        self.preview_title = title;
        self.preview_content = content;
        self.preview_scroll = 0;
        self.popup = PopupState::Preview;
    }

    /// Scroll the preview popup by `delta` lines (positive = down).
    pub fn scroll_preview(&mut self, delta: isize) {
        let max = self.preview_content.len().saturating_sub(1);
        if delta >= 0 {
            self.preview_scroll = self
                .preview_scroll
                .saturating_add(delta.unsigned_abs())
                .min(max);
        } else {
            self.preview_scroll = self.preview_scroll.saturating_sub(delta.unsigned_abs());
        }
    }

    /// Close any open popup.
    #[allow(clippy::missing_const_for_fn)]
    pub fn close_popup(&mut self) {
        self.popup = PopupState::None;
    }

    /// Whether the user has requested a refresh (re-scan).
    pub const fn refresh_requested(&self) -> bool {
        self.refresh_requested
    }

    /// Mark that the user wants to re-scan.
    #[allow(clippy::missing_const_for_fn)]
    pub fn request_refresh(&mut self) {
        self.refresh_requested = true;
    }

    /// Whether the filesystem has changed since the scan completed.
    pub const fn filesystem_changed(&self) -> bool {
        self.filesystem_changed
    }

    /// Mark that the filesystem has changed.
    #[allow(clippy::missing_const_for_fn)]
    pub fn set_filesystem_changed(&mut self) {
        self.filesystem_changed = true;
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

    /// Sync the tree widget selection to the currently keyboard-selected treemap cell.
    ///
    /// When the treemap panel has focus and a cell is selected, this expands all
    /// parent nodes in the tree and moves the tree cursor to match.  Does nothing
    /// when the treemap panel is not focused or no cell is selected.
    pub fn sync_tree_to_treemap_selection(&mut self) {
        if self.focus != PanelFocus::Treemap {
            return;
        }
        let Some(idx) = self.treemap_state.selected_index else {
            return;
        };
        let Some(cell) = self.treemap_state.layout.cells.get(idx) else {
            return;
        };
        let path = cell.path.clone();
        let extension = cell.extension.clone();
        if path.is_empty() {
            return;
        }
        // Open each ancestor directory so the selected item is visible.
        for prefix_len in 1..path.len() {
            self.tree_state.open(path[..prefix_len].to_vec());
        }
        self.tree_state.select(path);
        // Sync legend scroll to show the selected file's extension.
        // Only scroll up (never jump down past the current view) to avoid
        // the legend jumping unnecessarily when the item is already visible.
        if let Some(ext) = &extension
            && let Some(pos) = self
                .extension_stats
                .iter()
                .position(|s| s.extension.as_deref() == Some(ext.as_str()))
            && pos < self.legend_scroll
        {
            self.legend_scroll = pos;
        }
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ScanProgress;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    fn make_file(name: &str, size: u64) -> DirNode {
        DirNode {
            name: name.to_owned(),
            size,
            allocated: size,
            file_count: 1,
            children: vec![],
            is_dir: false,
            extension: name.rsplit('.').next().map(str::to_lowercase),
            mtime: SystemTime::UNIX_EPOCH,
        }
    }

    fn make_dir(name: &str, children: Vec<DirNode>) -> DirNode {
        let size: u64 = children.iter().map(|c| c.size).sum();
        DirNode {
            name: name.to_owned(),
            size,
            allocated: size,
            file_count: children.iter().map(|c| c.file_count).sum(),
            children,
            is_dir: true,
            extension: None,
            mtime: SystemTime::UNIX_EPOCH,
        }
    }

    fn make_test_tree() -> DirNode {
        make_dir(
            "root",
            vec![
                make_dir(
                    "subdir",
                    vec![make_file("file1.rs", 100), make_file("file2.py", 200)],
                ),
                make_file("file3.txt", 50),
            ],
        )
    }

    fn make_explorer_state() -> ExplorerState {
        ExplorerState::new(make_test_tree(), PathBuf::from("/test"))
    }

    // --- ScanProgressState ---

    #[test]
    fn scan_progress_new_zeroed() {
        let state = ScanProgressState::new(false);
        assert_eq!(state.file_count, 0);
        assert!(state.files_per_sec.abs() < f64::EPSILON);
        assert_eq!(state.elapsed, Duration::ZERO);
        assert_eq!(state.current_path, PathBuf::new());
        assert!(!state.is_root);
    }

    #[test]
    fn scan_progress_update() {
        let mut state = ScanProgressState::new(true);
        state.update(ScanProgress {
            entries_scanned: 100,
            entries_per_second: 50.0,
            elapsed_secs: 2.0,
            current_path: PathBuf::from("/test/path"),
        });
        assert_eq!(state.file_count, 100);
        assert!((state.files_per_sec - 50.0).abs() < f64::EPSILON);
        assert_eq!(state.elapsed, Duration::from_secs(2));
        assert_eq!(state.current_path, PathBuf::from("/test/path"));
        assert!(state.is_root);
    }

    #[test]
    fn scan_progress_update_invalid_elapsed() {
        let mut state = ScanProgressState::new(false);
        state.update(ScanProgress {
            entries_scanned: 10,
            entries_per_second: 5.0,
            elapsed_secs: -1.0,
            current_path: PathBuf::from("/neg"),
        });
        assert_eq!(state.elapsed, Duration::ZERO);

        state.update(ScanProgress {
            entries_scanned: 20,
            entries_per_second: 10.0,
            elapsed_secs: f64::NAN,
            current_path: PathBuf::from("/nan"),
        });
        assert_eq!(state.elapsed, Duration::ZERO);
    }

    // --- ExplorerState ---

    #[test]
    fn explorer_state_initial_defaults() {
        let state = make_explorer_state();
        assert_eq!(state.sort_field(), TreeSortField::Size);
        assert!(!state.sort_ascending());
        assert!(!state.show_help());
        assert!(!state.show_info());
        assert!(state.error_message().is_none());
        assert_eq!(state.legend_scroll(), 0);
        assert_eq!(state.treemap_root(), &[] as &[String]);
        assert_eq!(state.scan_root(), Path::new("/test"));
    }

    #[test]
    fn zoom_into_selected_enters_directory() {
        let mut state = make_explorer_state();
        state.tree_state_mut().select(vec!["subdir".to_owned()]);
        state.zoom_into_selected();
        assert_eq!(state.treemap_root(), &["subdir"]);
    }

    #[test]
    fn zoom_into_selected_ignores_file() {
        let mut state = make_explorer_state();
        state.tree_state_mut().select(vec!["file3.txt".to_owned()]);
        state.zoom_into_selected();
        assert_eq!(
            state.treemap_root(),
            &[] as &[String],
            "should not zoom into a file"
        );
    }

    #[test]
    fn zoom_out_pops_one_level() {
        let mut state = make_explorer_state();
        state.tree_state_mut().select(vec!["subdir".to_owned()]);
        state.zoom_into_selected();
        assert_eq!(state.treemap_root(), &["subdir"]);
        state.zoom_out();
        assert_eq!(state.treemap_root(), &[] as &[String]);
    }

    #[test]
    fn zoom_out_at_root_is_noop() {
        let mut state = make_explorer_state();
        state.zoom_out();
        assert_eq!(state.treemap_root(), &[] as &[String]);
    }

    #[test]
    fn zoom_to_root_clears_path() {
        let mut state = make_explorer_state();
        state.tree_state_mut().select(vec!["subdir".to_owned()]);
        state.zoom_into_selected();
        assert_ne!(state.treemap_root(), &[] as &[String]);
        state.zoom_to_root();
        assert_eq!(state.treemap_root(), &[] as &[String]);
    }

    #[test]
    fn set_sort_resets_ascending() {
        let mut state = make_explorer_state();
        state.toggle_sort_direction();
        assert!(state.sort_ascending());
        state.set_sort(TreeSortField::Name);
        assert_eq!(state.sort_field(), TreeSortField::Name);
        assert!(!state.sort_ascending());
    }

    #[test]
    fn toggle_sort_direction_roundtrips() {
        let mut state = make_explorer_state();
        assert!(!state.sort_ascending());
        state.toggle_sort_direction();
        assert!(state.sort_ascending());
        state.toggle_sort_direction();
        assert!(!state.sort_ascending());
    }

    #[test]
    fn breadcrumb_at_root() {
        let state = make_explorer_state();
        assert_eq!(state.breadcrumb_path(), "/test");
    }

    #[test]
    fn breadcrumb_with_zoom() {
        let mut state = make_explorer_state();
        state.tree_state_mut().select(vec!["subdir".to_owned()]);
        state.zoom_into_selected();
        assert_eq!(state.breadcrumb_path(), "/test/subdir");
    }

    #[test]
    fn toggle_help() {
        let mut state = make_explorer_state();
        assert!(!state.show_help());
        state.toggle_show_help();
        assert!(state.show_help());
        state.toggle_show_help();
        assert!(!state.show_help());
    }

    #[test]
    fn toggle_info() {
        let mut state = make_explorer_state();
        assert!(!state.show_info());
        state.toggle_show_info();
        assert!(state.show_info());
        state.toggle_show_info();
        assert!(!state.show_info());
    }

    #[test]
    fn clear_error() {
        let mut state = make_explorer_state();
        assert!(state.error_message().is_none());
        state.clear_error();
        assert!(state.error_message().is_none());
    }

    #[test]
    fn sync_treemap_highlight_empty_selection() {
        let mut state = make_explorer_state();
        state.sync_treemap_highlight();
        assert!(state.treemap_state_mut().highlighted_path.is_none());
    }

    #[test]
    fn sync_treemap_highlight_with_selection() {
        let mut state = make_explorer_state();
        state.tree_state_mut().select(vec!["subdir".to_owned()]);
        state.sync_treemap_highlight();
        assert_eq!(
            state.treemap_state_mut().highlighted_path,
            Some(vec!["subdir".to_owned()])
        );
    }

    #[test]
    fn cycle_focus_three_panels() {
        let mut state = make_explorer_state();
        assert_eq!(state.focus(), PanelFocus::Tree);
        state.cycle_focus();
        assert_eq!(state.focus(), PanelFocus::Treemap);
        state.cycle_focus();
        assert_eq!(state.focus(), PanelFocus::Legend);
        state.cycle_focus();
        assert_eq!(state.focus(), PanelFocus::Tree);
    }

    #[test]
    fn zoom_into_path_enters_directory() {
        let mut state = make_explorer_state();
        state.zoom_into_path(vec!["subdir".to_owned()]);
        assert_eq!(state.treemap_root(), &["subdir"]);
    }

    #[test]
    fn zoom_into_path_ignores_file() {
        let mut state = make_explorer_state();
        state.zoom_into_path(vec!["file3.txt".to_owned()]);
        assert_eq!(state.treemap_root(), &[] as &[String]);
    }

    #[test]
    fn zoom_into_path_empty_is_noop() {
        let mut state = make_explorer_state();
        state.zoom_into_path(vec![]);
        assert_eq!(state.treemap_root(), &[] as &[String]);
    }

    #[test]
    fn current_treemap_node_at_root() {
        let state = make_explorer_state();
        let node = state.current_treemap_node();
        assert_eq!(node.name, "root");
    }

    #[test]
    fn current_treemap_node_after_zoom() {
        let mut state = make_explorer_state();
        state.tree_state_mut().select(vec!["subdir".to_owned()]);
        state.zoom_into_selected();
        let node = state.current_treemap_node();
        assert_eq!(node.name, "subdir");
    }

    #[test]
    fn sync_tree_to_treemap_selection_noop_when_not_treemap_focus() {
        let mut state = make_explorer_state();
        // Default focus is Tree, not Treemap — sync should be a no-op.
        assert_eq!(state.focus(), PanelFocus::Tree);
        state.sync_tree_to_treemap_selection();
        // Tree selection should remain empty (default).
        assert_eq!(state.tree_state().selected(), &[] as &[String]);
    }

    #[test]
    fn sync_tree_to_treemap_selection_noop_when_no_cell_selected() {
        let mut state = make_explorer_state();
        state.set_focus(PanelFocus::Treemap);
        // No cell selected → no-op.
        state.sync_tree_to_treemap_selection();
        assert_eq!(state.tree_state().selected(), &[] as &[String]);
    }

    #[test]
    fn sync_tree_to_treemap_selection_sets_tree_selection() {
        use crate::ui::widgets::treemap::{CellLayout, TreemapLayout};

        let mut state = make_explorer_state();
        state.set_focus(PanelFocus::Treemap);

        // Inject a fake layout with one cell.
        let fake_cell = CellLayout {
            rect: ratatui::layout::Rect::new(0, 0, 10, 4),
            name: "file1.rs".to_owned(),
            extension: Some("rs".to_owned()),
            is_dir: false,
            size: 100,
            mtime: std::time::SystemTime::UNIX_EPOCH,
            path: vec!["subdir".to_owned(), "file1.rs".to_owned()],
        };
        state.treemap_state_mut().layout = TreemapLayout {
            cells: vec![fake_cell],
        };
        state.treemap_state_mut().selected_index = Some(0);

        state.sync_tree_to_treemap_selection();

        // Tree should now have "subdir/file1.rs" selected.
        let sel = state.tree_state().selected();
        assert_eq!(
            sel,
            &["subdir", "file1.rs"],
            "tree selection should match treemap cell path"
        );
    }

    #[test]
    fn zoom_resets_treemap_selection() {
        let mut state = make_explorer_state();
        state.treemap_state_mut().selected_index = Some(5);
        state.tree_state_mut().select(vec!["subdir".to_owned()]);
        state.zoom_into_selected();
        assert_eq!(state.treemap_state_mut().selected_index, None);
    }
}
