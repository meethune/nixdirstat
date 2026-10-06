//! TUI application state types.
//!
//! Defines the top-level [`AppState`] enum and per-state data structs
//! that drive what the TUI renders.

use std::{path::PathBuf, time::Duration};

use crate::{
    analyzer::compute_type_stats,
    error::UiError,
    storage::Storage,
    types::{FileEntry, ScanProgress, SortDirection, SortField, TypeStat},
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
    /// Transient error message displayed as a status line in the explorer view.
    ///
    /// Set when a navigation action fails; cleared on the next successful action.
    pub error_message: Option<String>,
}

impl ExplorerState {
    /// Create a new [`ExplorerState`] with the given root path and initial data.
    ///
    /// Entries are sorted by size descending on construction.
    pub fn new(root_path: PathBuf, entries: Vec<FileEntry>, type_stats: Vec<TypeStat>) -> Self {
        let mut state = Self {
            current_path: root_path,
            breadcrumb: Vec::new(),
            entries,
            type_stats,
            selected_index: 0,
            sort_field: SortField::Size,
            sort_direction: SortDirection::Descending,
            treemap_state: TreemapState::default(),
            error_message: None,
        };
        state.sort_entries();
        state
    }

    /// Navigate into a subdirectory, loading its children from `storage`.
    ///
    /// Pushes the current path onto the breadcrumb stack before navigating.
    ///
    /// # Errors
    ///
    /// Returns [`UiError::StorageLoad`] if querying the storage fails.
    pub fn navigate_into(&mut self, storage: &dyn Storage, path: PathBuf) -> Result<(), UiError> {
        let children = storage
            .query_directory_children(&path)
            .map_err(UiError::StorageLoad)?;
        let type_stats = compute_type_stats(&children);
        self.breadcrumb.push(self.current_path.clone());
        self.current_path = path;
        self.entries = children;
        self.type_stats = type_stats;
        self.selected_index = 0;
        self.treemap_state.selected = None;
        self.error_message = None;
        self.sort_entries();
        Ok(())
    }

    /// Navigate up to the parent directory using the breadcrumb stack.
    ///
    /// If already at the root (breadcrumb is empty), this is a no-op.
    ///
    /// # Errors
    ///
    /// Returns [`UiError::StorageLoad`] if querying the storage fails.
    pub fn navigate_up(&mut self, storage: &dyn Storage) -> Result<(), UiError> {
        let Some(parent) = self.breadcrumb.pop() else {
            return Ok(());
        };
        let children = storage
            .query_directory_children(&parent)
            .map_err(UiError::StorageLoad)?;
        let type_stats = compute_type_stats(&children);
        self.current_path = parent;
        self.entries = children;
        self.type_stats = type_stats;
        self.selected_index = 0;
        self.treemap_state.selected = None;
        self.error_message = None;
        self.sort_entries();
        Ok(())
    }

    /// Advance the sort field to the next one in the cycle and re-sort entries.
    ///
    /// Cycle order: `Size` → `Name` → `Modified` → `Type` → `Size`.
    pub fn cycle_sort(&mut self) {
        self.sort_field = match self.sort_field {
            SortField::Size => SortField::Name,
            SortField::Name => SortField::Modified,
            SortField::Modified => SortField::Type,
            // SortField::Type and any future variants cycle back to Size.
            _ => SortField::Size,
        };
        self.sort_entries();
    }

    /// Flip the sort direction between ascending and descending and re-sort.
    pub fn reverse_sort(&mut self) {
        self.sort_direction = match self.sort_direction {
            SortDirection::Ascending => SortDirection::Descending,
            // SortDirection::Descending and any future variants → Ascending.
            _ => SortDirection::Ascending,
        };
        self.sort_entries();
    }

    /// Return the currently selected [`FileEntry`], or `None` if the list is empty.
    pub fn selected_entry(&self) -> Option<&FileEntry> {
        self.entries.get(self.selected_index)
    }

    /// Move the selection cursor up by one row (wrapping at the top).
    // missing_const_for_fn: Vec::len() and modular arithmetic are not const-stable.
    #[allow(clippy::missing_const_for_fn)]
    pub fn select_prev(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected_index = if self.selected_index == 0 {
            self.entries.len() - 1
        } else {
            self.selected_index - 1
        };
        self.treemap_state.selected = Some(self.selected_index);
    }

    /// Move the selection cursor down by one row (wrapping at the bottom).
    // missing_const_for_fn: Vec::len() and modular arithmetic are not const-stable.
    #[allow(clippy::missing_const_for_fn)]
    pub fn select_next(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected_index = (self.selected_index + 1) % self.entries.len();
        self.treemap_state.selected = Some(self.selected_index);
    }

    /// Sort `entries` according to the current [`sort_field`] and [`sort_direction`].
    ///
    /// [`sort_field`]: ExplorerState::sort_field
    /// [`sort_direction`]: ExplorerState::sort_direction
    fn sort_entries(&mut self) {
        let direction = self.sort_direction;
        let field = self.sort_field;
        self.entries.sort_by(|a, b| {
            let ord = match field {
                SortField::Size => a.size.cmp(&b.size),
                SortField::Name => a.path.file_name().cmp(&b.path.file_name()),
                SortField::Modified => a.mtime.cmp(&b.mtime),
                // SortField::Type and any future variants sort by file-type discriminant.
                _ => a
                    .file_type
                    .as_discriminant()
                    .cmp(&b.file_type.as_discriminant()),
            };
            match direction {
                SortDirection::Ascending => ord,
                // SortDirection::Descending and any future variants → descending.
                _ => ord.reverse(),
            }
        });
    }
}

/// Top-level application state: which view is currently displayed.
#[derive(Debug)]
pub enum AppState {
    /// The scan is in progress; display the progress view.
    Scanning(ScanProgressState),
    /// The scan is complete; display the file explorer.
    Exploring(ExplorerState),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        path::{Path, PathBuf},
        time::SystemTime,
    };

    use super::*;
    use crate::{
        error::StorageError,
        storage::Storage,
        types::{
            DirectoryStats, EntryBatch, EntryQuery, FileCategory, FileEntry, FileType,
            ScanMetadata, TypeStat,
        },
    };

    // -----------------------------------------------------------------------
    // MockStorage
    // -----------------------------------------------------------------------

    /// In-memory storage stub used in navigation tests.
    struct MockStorage {
        children: HashMap<PathBuf, Vec<FileEntry>>,
    }

    impl MockStorage {
        fn new() -> Self {
            Self {
                children: HashMap::new(),
            }
        }

        fn add_children(&mut self, dir: impl Into<PathBuf>, entries: Vec<FileEntry>) {
            self.children.insert(dir.into(), entries);
        }
    }

    impl Storage for MockStorage {
        fn init_schema(&mut self) -> Result<(), StorageError> {
            Ok(())
        }

        fn insert_batch(&self, _batch: &EntryBatch) -> Result<(), StorageError> {
            Ok(())
        }

        fn save_scan_metadata(&self, _metadata: &ScanMetadata) -> Result<(), StorageError> {
            Ok(())
        }

        fn load_scan_metadata(&self) -> Result<ScanMetadata, StorageError> {
            Err(StorageError::Io(std::io::Error::other("not implemented")))
        }

        fn query_entries(&self, _query: &EntryQuery) -> Result<Vec<FileEntry>, StorageError> {
            Ok(Vec::new())
        }

        fn query_directory_children(&self, path: &Path) -> Result<Vec<FileEntry>, StorageError> {
            Ok(self.children.get(path).cloned().unwrap_or_default())
        }

        fn query_top_n_by_size(&self, _n: usize) -> Result<Vec<FileEntry>, StorageError> {
            Ok(Vec::new())
        }

        fn query_type_stats(&self) -> Result<Vec<TypeStat>, StorageError> {
            Ok(Vec::new())
        }

        fn update_directory_sizes(
            &self,
            _sizes: &HashMap<PathBuf, DirectoryStats>,
        ) -> Result<(), StorageError> {
            Ok(())
        }

        fn finalize_for_export(&self) -> Result<(), StorageError> {
            Ok(())
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn make_entry(path: &str) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            size: 100,
            allocated_size: 100,
            file_type: FileType::Regular,
            category: FileCategory::Code,
            inode: 0,
            device: 0,
            nlink: 1,
            uid: 1000,
            gid: 1000,
            mtime: SystemTime::UNIX_EPOCH,
            mode: 0o644,
        }
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[test]
    fn navigation_into_directory() {
        let root = PathBuf::from("/root");
        let sub = PathBuf::from("/root/sub");

        let mut storage = MockStorage::new();
        storage.add_children(sub.clone(), vec![make_entry("/root/sub/file.rs")]);

        let mut state = ExplorerState::new(root.clone(), vec![], vec![]);
        assert_eq!(state.current_path, root);
        assert!(state.breadcrumb.is_empty());

        state
            .navigate_into(&storage, sub.clone())
            .expect("navigate_into");

        assert_eq!(state.current_path, sub, "current_path should update to sub");
        assert_eq!(
            state.breadcrumb,
            vec![root],
            "breadcrumb should contain root"
        );
        assert_eq!(
            state.entries.len(),
            1,
            "entries should be loaded from storage"
        );
    }

    #[test]
    fn navigation_up_restores_parent() {
        let root = PathBuf::from("/root");
        let sub = PathBuf::from("/root/sub");

        let mut storage = MockStorage::new();
        storage.add_children(sub.clone(), vec![make_entry("/root/sub/file.rs")]);
        storage.add_children(root.clone(), vec![make_entry("/root/file.rs")]);

        let mut state = ExplorerState::new(root.clone(), vec![make_entry("/root/file.rs")], vec![]);

        state
            .navigate_into(&storage, sub.clone())
            .expect("navigate_into");
        assert_eq!(state.current_path, sub);

        state.navigate_up(&storage).expect("navigate_up");
        assert_eq!(
            state.current_path, root,
            "navigate_up should restore the parent path"
        );
        assert!(
            state.breadcrumb.is_empty(),
            "breadcrumb should be empty after navigate_up"
        );
    }

    #[test]
    fn navigate_up_at_root_is_noop() {
        let root = PathBuf::from("/root");
        let storage = MockStorage::new();
        let mut state = ExplorerState::new(root.clone(), vec![], vec![]);

        state
            .navigate_up(&storage)
            .expect("navigate_up at root should be ok");
        assert_eq!(state.current_path, root, "path unchanged at root");
        assert!(state.breadcrumb.is_empty());
    }

    #[test]
    fn cycle_sort_advances_field() {
        let mut state = ExplorerState::new(PathBuf::from("/r"), vec![], vec![]);
        assert_eq!(state.sort_field, SortField::Size);
        state.cycle_sort();
        assert_eq!(state.sort_field, SortField::Name);
        state.cycle_sort();
        assert_eq!(state.sort_field, SortField::Modified);
        state.cycle_sort();
        assert_eq!(state.sort_field, SortField::Type);
        state.cycle_sort();
        assert_eq!(
            state.sort_field,
            SortField::Size,
            "should wrap back to Size"
        );
    }

    #[test]
    fn reverse_sort_toggles_direction() {
        let mut state = ExplorerState::new(PathBuf::from("/r"), vec![], vec![]);
        assert_eq!(state.sort_direction, SortDirection::Descending);
        state.reverse_sort();
        assert_eq!(state.sort_direction, SortDirection::Ascending);
        state.reverse_sort();
        assert_eq!(state.sort_direction, SortDirection::Descending);
    }
}
