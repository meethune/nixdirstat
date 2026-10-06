//! TUI application state types.
//!
//! Defines the top-level [`AppState`] enum and per-state data structs
//! that drive what the TUI renders.

use std::{path::PathBuf, time::Duration};

use crate::types::{FileEntry, ScanProgress, SortDirection, SortField, TypeStat};

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

/// State for the treemap panel within the explorer view.
///
/// Task 8 will add widget wiring and display logic to this struct.
#[derive(Debug, Default)]
pub struct TreemapState {
    /// Index of the currently selected treemap cell, or `None` if nothing is selected.
    pub selected: Option<usize>,
}

/// State for the file explorer view.
///
/// Task 8 will add constructors, methods, and widget wiring to this struct.
/// For Task 7, only the shape is defined to allow [`AppState`] to compile.
#[derive(Debug)]
pub struct ExplorerState {
    /// Directory currently being explored.
    pub current_path: PathBuf,
    /// Navigation breadcrumb: ancestor paths from root to the current directory.
    pub breadcrumb: Vec<PathBuf>,
    /// File entries visible in the current directory listing.
    pub entries: Vec<FileEntry>,
    /// File-type statistics for the current scan.
    pub type_stats: Vec<TypeStat>,
    /// Index of the currently selected entry in [`entries`].
    pub selected_index: usize,
    /// Field used to sort [`entries`].
    pub sort_field: SortField,
    /// Direction used to sort [`entries`].
    pub sort_direction: SortDirection,
    /// State for the treemap panel.
    pub treemap_state: TreemapState,
}

/// Top-level application state: which view is currently displayed.
#[derive(Debug)]
pub enum AppState {
    /// The scan is in progress; display the progress view.
    Scanning(ScanProgressState),
    /// The scan is complete; display the file explorer.
    Exploring(ExplorerState),
}
