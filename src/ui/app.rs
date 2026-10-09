//! TUI application state types.
//!
//! Defines the top-level [`AppState`] enum and per-state data structs
//! that drive what the TUI renders.

use std::{path::PathBuf, time::Duration};

use tui_tree_widget::TreeState;

use crate::{
    types::{ScanProgress, ScanWarning, SizeAccuracy},
    ui::{
        tree::{
            DirNode, ExtensionStat, collect_extension_stats, find_node, max_depth_for_node,
            time_range_for_node,
        },
        visualization::{
            ColorScheme, RenderParams, TimeRange, Visualization, treemap::TreemapVisualization,
        },
    },
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
    /// Trustworthiness of reported allocated sizes.
    pub size_accuracy: SizeAccuracy,
    /// Detected filesystem type name (e.g. `"btrfs"`).
    pub filesystem_type: String,
}

impl ScanProgressState {
    /// Create a new scan progress state with zeroed counters.
    pub const fn new(is_root: bool, size_accuracy: SizeAccuracy, filesystem_type: String) -> Self {
        Self {
            file_count: 0,
            files_per_sec: 0.0,
            elapsed: Duration::ZERO,
            current_path: PathBuf::new(),
            is_root,
            size_accuracy,
            filesystem_type,
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

/// Whether the scan data is current or needs refreshing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScanFreshness {
    /// Scan data matches the filesystem.
    #[default]
    Current,
    /// Filesystem changes have been detected since the scan.
    FilesystemChanged,
    /// The user has requested a re-scan.
    RefreshRequested,
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
    /// Active visualization mode.
    visualization: Box<dyn Visualization>,
    /// Optional overview visualization (shown in overview-detail split layouts).
    overview: Option<Box<dyn Visualization>>,
    /// Active color scheme.
    color_scheme: ColorScheme,
    /// Modification-time range across the full scanned tree.
    time_range: Option<TimeRange>,
    /// Maximum nesting depth in the full scanned tree.
    max_depth: u16,
    /// Render parameters from the most recent frame, for key-event routing.
    last_render_params: Option<RenderParams>,
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
    preview_viewport: usize,
    preview_title: String,
    freshness: ScanFreshness,
    size_accuracy: SizeAccuracy,
    filesystem_type: String,
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
        let time_range = time_range_for_node(&tree);
        let max_depth = max_depth_for_node(&tree);
        Self {
            tree,
            tree_state: TreeState::default(),
            extension_stats,
            treemap_root: Vec::new(),
            visualization: Box::new(TreemapVisualization::new()),
            overview: None,
            color_scheme: ColorScheme::default(),
            time_range,
            max_depth,
            last_render_params: None,
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
            preview_viewport: 1,
            preview_title: String::new(),
            freshness: ScanFreshness::Current,
            size_accuracy: SizeAccuracy::Exact,
            filesystem_type: String::new(),
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
        self.visualization.reset_on_zoom();
        self.recompute_extension_stats();
    }

    /// Cycle keyboard focus forward: Tree → Treemap → Legend → Tree.
    pub const fn cycle_focus(&mut self) {
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
    pub const fn set_sort(&mut self, field: TreeSortField) {
        self.sort_field = field;
        self.sort_ascending = false;
    }

    /// Toggle the sort direction.
    pub const fn toggle_sort_direction(&mut self) {
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

    /// Split borrow: `tree` (shared) + `visualization` (mutable).
    pub fn tree_and_visualization_mut(&mut self) -> (&DirNode, &mut dyn Visualization) {
        (&self.tree, self.visualization.as_mut())
    }

    /// Path components from scan root to current treemap zoom level.
    pub fn treemap_root(&self) -> &[String] {
        &self.treemap_root
    }

    /// Active visualization mode (shared ref).
    pub fn visualization(&self) -> &dyn Visualization {
        self.visualization.as_ref()
    }

    /// Active visualization mode (mutable ref).
    pub fn visualization_mut(&mut self) -> &mut dyn Visualization {
        self.visualization.as_mut()
    }

    /// Overview visualization used in split-layout mode (shared ref).
    ///
    /// Returns `None` until an overview mode is assigned.  The shell checks this
    /// together with [`RenderParams::use_overview_detail`] before choosing the
    /// layout mode.
    pub fn overview(&self) -> Option<&dyn Visualization> {
        self.overview.as_deref()
    }

    /// Overview visualization used in split-layout mode (mutable ref).
    ///
    /// Returns `None` when no overview has been created yet.
    ///
    /// The `+ 'static` bound matches the storage type `Box<dyn Visualization + 'static>`.
    pub fn overview_mut<'a>(&'a mut self) -> Option<&'a mut (dyn Visualization + 'static)> {
        self.overview.as_deref_mut()
    }

    /// Ensure the overview visualization exists, creating a [`TreemapVisualization`] if absent.
    pub fn ensure_overview(&mut self) {
        if self.overview.is_none() {
            self.overview = Some(Box::new(TreemapVisualization::new()));
        }
    }

    /// Drop the overview visualization, releasing its memory.
    pub fn drop_overview(&mut self) {
        self.overview = None;
    }

    /// Split borrow: `tree` (shared) + `overview` (mutable, if present).
    ///
    /// The `+ 'static` bound on the trait object matches `Box<dyn Visualization + 'static>`.
    pub fn tree_and_overview_mut<'a>(
        &'a mut self,
    ) -> (&'a DirNode, Option<&'a mut (dyn Visualization + 'static)>) {
        (&self.tree, self.overview.as_deref_mut())
    }

    /// Active color scheme.
    pub const fn color_scheme(&self) -> ColorScheme {
        self.color_scheme
    }

    /// Cycle the color scheme: `FileType` → `Mtime` → `Depth` → `FileType`.
    pub const fn cycle_color_scheme(&mut self) {
        self.color_scheme = match self.color_scheme {
            ColorScheme::FileType => ColorScheme::Mtime,
            ColorScheme::Mtime => ColorScheme::Depth,
            ColorScheme::Depth => ColorScheme::FileType,
        };
    }

    /// Modification-time range across the full scanned tree.
    pub const fn time_range(&self) -> Option<&TimeRange> {
        self.time_range.as_ref()
    }

    /// Maximum nesting depth in the full scanned tree.
    pub const fn max_depth(&self) -> u16 {
        self.max_depth
    }

    /// Render parameters from the most recent frame.
    pub const fn last_render_params(&self) -> Option<&RenderParams> {
        self.last_render_params.as_ref()
    }

    /// Store the render parameters from the current frame.
    pub const fn set_last_render_params(&mut self, params: RenderParams) {
        self.last_render_params = Some(params);
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
    pub const fn set_focus(&mut self, panel: PanelFocus) {
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
    pub const fn set_free_space(&mut self, space: Option<crate::types::SpaceInfo>) {
        self.free_space = space;
    }

    /// Set the scan warnings collected during the scan.
    pub fn set_warnings(&mut self, warnings: Vec<ScanWarning>) {
        self.warnings = warnings;
    }

    /// Set the size accuracy from the scan metadata.
    pub const fn set_size_accuracy(&mut self, accuracy: SizeAccuracy) {
        self.size_accuracy = accuracy;
    }

    /// Trustworthiness of reported allocated sizes.
    pub const fn size_accuracy(&self) -> SizeAccuracy {
        self.size_accuracy
    }

    /// Set the detected filesystem type from the scan metadata.
    pub fn set_filesystem_type(&mut self, fs_type: String) {
        self.filesystem_type = fs_type;
    }

    /// Detected filesystem type for this scan.
    pub fn filesystem_type(&self) -> &str {
        &self.filesystem_type
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
    pub fn toggle_show_help(&mut self) {
        self.popup = if self.popup == PopupState::Help {
            PopupState::None
        } else {
            PopupState::Help
        };
    }

    /// Toggle the file info popup.
    pub fn toggle_show_info(&mut self) {
        self.popup = if self.popup == PopupState::Info {
            PopupState::None
        } else {
            PopupState::Info
        };
    }

    /// Toggle the warnings popup.
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
    pub const fn open_search(&mut self) {
        self.search_active = true;
    }

    /// Close the search bar (keeps the query for continued filtering).
    pub const fn close_search(&mut self) {
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
    ///
    /// Clamped so the last page fills the viewport using the viewport height
    /// recorded by the most recent render pass.
    pub fn scroll_preview(&mut self, delta: isize) {
        let max = self
            .preview_content
            .len()
            .saturating_sub(self.preview_viewport);
        if delta >= 0 {
            self.preview_scroll = self
                .preview_scroll
                .saturating_add(delta.unsigned_abs())
                .min(max);
        } else {
            self.preview_scroll = self.preview_scroll.saturating_sub(delta.unsigned_abs());
        }
    }

    /// Update the preview popup viewport height (called by the renderer).
    pub fn set_preview_viewport(&mut self, height: usize) {
        self.preview_viewport = height.max(1);
        self.preview_scroll = self.preview_scroll.min(
            self.preview_content
                .len()
                .saturating_sub(self.preview_viewport),
        );
    }

    /// Close any open popup.
    pub const fn close_popup(&mut self) {
        self.popup = PopupState::None;
    }

    /// Whether the user has requested a refresh (re-scan).
    pub const fn refresh_requested(&self) -> bool {
        matches!(self.freshness, ScanFreshness::RefreshRequested)
    }

    /// Mark that the user wants to re-scan.
    pub const fn request_refresh(&mut self) {
        self.freshness = ScanFreshness::RefreshRequested;
    }

    /// Whether the filesystem has changed since the scan completed.
    pub const fn filesystem_changed(&self) -> bool {
        matches!(self.freshness, ScanFreshness::FilesystemChanged)
    }

    /// Mark that the filesystem has changed.
    pub fn set_filesystem_changed(&mut self) {
        if self.freshness == ScanFreshness::Current {
            self.freshness = ScanFreshness::FilesystemChanged;
        }
    }

    /// Set a transient error message displayed as a status line.
    pub fn set_error(&mut self, msg: String) {
        self.error_message = Some(msg);
    }

    /// Clear the transient error message.
    pub fn clear_error(&mut self) {
        self.error_message = None;
    }

    /// Reset freshness back to the filesystem-changed state (used when
    /// refresh is not possible, e.g. in explore-only mode).
    #[allow(clippy::missing_const_for_fn)] // == on derived PartialEq is not const-stable
    pub fn clear_refresh_request(&mut self) {
        if self.freshness == ScanFreshness::RefreshRequested {
            self.freshness = ScanFreshness::FilesystemChanged;
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
    use crate::ui::tree::test_fixtures::{make_dir, make_file};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

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
        let state = ScanProgressState::new(false, SizeAccuracy::Exact, String::new());
        assert_eq!(state.file_count, 0);
        assert!(state.files_per_sec.abs() < f64::EPSILON);
        assert_eq!(state.elapsed, Duration::ZERO);
        assert_eq!(state.current_path, PathBuf::new());
        assert!(!state.is_root);
    }

    #[test]
    fn scan_progress_update() {
        let mut state = ScanProgressState::new(true, SizeAccuracy::Exact, String::new());
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
        let mut state = ScanProgressState::new(false, SizeAccuracy::Exact, String::new());
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
    fn zoom_resets_visualization_selection() {
        let mut state = make_explorer_state();
        // After zoom, the visualization should have no selected item.
        state.tree_state_mut().select(vec!["subdir".to_owned()]);
        state.zoom_into_selected();
        assert!(
            state.visualization().selected_item().is_none(),
            "visualization selection should be cleared after zoom"
        );
    }

    #[test]
    fn color_scheme_default_is_file_type() {
        let state = make_explorer_state();
        assert_eq!(state.color_scheme(), ColorScheme::FileType);
    }

    #[test]
    fn cycle_color_scheme_roundtrips() {
        let mut state = make_explorer_state();
        assert_eq!(state.color_scheme(), ColorScheme::FileType);
        state.cycle_color_scheme();
        assert_eq!(state.color_scheme(), ColorScheme::Mtime);
        state.cycle_color_scheme();
        assert_eq!(state.color_scheme(), ColorScheme::Depth);
        state.cycle_color_scheme();
        assert_eq!(state.color_scheme(), ColorScheme::FileType);
    }

    #[test]
    fn last_render_params_initially_none() {
        let state = make_explorer_state();
        assert!(state.last_render_params().is_none());
    }

    #[test]
    fn set_last_render_params_roundtrips() {
        let mut state = make_explorer_state();
        let params = RenderParams::from_area(ratatui::layout::Rect::new(0, 0, 80, 24));
        // Capture width before moving params into state.
        let expected_width = params.area.width;
        state.set_last_render_params(params);
        assert!(state.last_render_params().is_some());
        assert_eq!(
            state.last_render_params().unwrap().area.width,
            expected_width
        );
    }

    #[test]
    fn max_depth_computed_from_tree() {
        let state = make_explorer_state();
        // root → subdir → file{1,2}: depth 2
        assert!(
            state.max_depth() >= 2,
            "expected depth >= 2 for nested tree"
        );
    }
}
