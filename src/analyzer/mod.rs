//! Disk usage analysis: directory size aggregation and free space computation.
//!
//! This module provides two free functions:
//!
//! - [`aggregate_directory_sizes`]: bottom-up aggregation of recursive directory sizes.
//! - [`compute_free_space`]: `statvfs`-based free space query for a filesystem path.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::{
    error::StorageError,
    storage::WriteStorage,
    types::{DirectoryStats, EntryQuery, FileType, SpaceInfo},
};

/// Aggregate recursive directory sizes from all entries in `storage`.
///
/// Queries every stored [`crate::types::FileEntry`], then for each non-directory
/// entry walks its ancestor chain, accumulating the entry's `size` and
/// `allocated_size` into each ancestor's [`DirectoryStats`]. The resulting map
/// is written back via [`WriteStorage::update_directory_sizes`].
///
/// Directory entries are initialised with zero statistics so that empty
/// directories are represented explicitly in the output.
///
/// # Errors
///
/// Returns [`StorageError`] if the underlying query or update operation fails.
pub fn aggregate_directory_sizes(storage: &dyn WriteStorage) -> Result<(), StorageError> {
    let all_entries = storage.query_entries(&EntryQuery {
        limit: None,
        ..EntryQuery::default()
    })?;

    // Collect all known directory paths so we only walk ancestors that are
    // actually part of the scan — not all the way to `/`.
    let mut sizes: HashMap<PathBuf, DirectoryStats> = HashMap::new();
    let mut known_dirs: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    for entry in &all_entries {
        if entry.file_type() == FileType::Directory {
            known_dirs.insert(entry.path().to_path_buf());
            sizes
                .entry(entry.path().to_path_buf())
                .or_insert_with(|| DirectoryStats {
                    path: entry.path().to_path_buf(),
                    total_size: 0,
                    total_allocated: 0,
                    child_count: 0,
                });
        }
    }

    for entry in &all_entries {
        if entry.file_type() == FileType::Directory {
            continue;
        }

        let mut ancestor = entry.path().parent();
        while let Some(dir) = ancestor {
            if !known_dirs.contains(dir) {
                break;
            }
            let stats = sizes
                .entry(dir.to_path_buf())
                .or_insert_with(|| DirectoryStats {
                    path: dir.to_path_buf(),
                    total_size: 0,
                    total_allocated: 0,
                    child_count: 0,
                });
            stats.total_size = stats.total_size.saturating_add(entry.size());
            stats.total_allocated = stats.total_allocated.saturating_add(entry.allocated_size());
            ancestor = dir.parent();
        }
    }

    storage.update_directory_sizes(&sizes)
}

/// Query free and total disk space on the filesystem containing `path`.
///
/// Uses `statvfs(3)` (POSIX — Tier 1 portable). The returned [`SpaceInfo`]
/// sets `unknown_bytes` to `total_bytes - free_bytes`; callers that know the
/// scanned size can further subtract it to compute truly unknown space.
///
/// # Errors
///
/// Returns [`std::io::Error`] if the `statvfs` call fails (e.g. the path does
/// not exist or the process lacks permission to stat it).
pub fn compute_free_space(path: &Path) -> Result<SpaceInfo, std::io::Error> {
    let stat = nix::sys::statvfs::statvfs(path).map_err(std::io::Error::from)?;
    // statvfs fields are u64 on Linux, u32 on macOS/FreeBSD — u64::from()
    // is lossless on all platforms but triggers useless_conversion on Linux.
    #[allow(clippy::useless_conversion)]
    let frsize = u64::from(stat.fragment_size());
    #[allow(clippy::useless_conversion)]
    let total = u64::from(stat.blocks()).saturating_mul(frsize);
    #[allow(clippy::useless_conversion)]
    let free = u64::from(stat.blocks_available()).saturating_mul(frsize);
    let unknown = total.saturating_sub(free);

    Ok(SpaceInfo {
        total_bytes: total,
        free_bytes: free,
        unknown_bytes: unknown,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::*;
    use crate::{
        storage::{ReadStorage as _, sqlite::SqliteStorage},
        types::{EntryBatch, FileEntry, JournalMode},
    };

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    use crate::types::FileEntryBuilder;

    fn make_file(path: &str, size: u64) -> FileEntry {
        FileEntryBuilder::new().path(path).size(size).build()
    }

    fn make_dir(path: &str) -> FileEntry {
        FileEntryBuilder::new()
            .path(path)
            .file_type(FileType::Directory)
            .build()
    }

    fn open_temp_storage() -> (SqliteStorage, tempfile::TempDir) {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("test.db");
        let storage = SqliteStorage::open(&path, JournalMode::Delete).expect("open storage");
        (storage, dir)
    }

    fn insert(storage: &SqliteStorage, entries: Vec<FileEntry>) {
        let batch = EntryBatch::new(entries).expect("non-empty batch");
        storage.insert_batch(&batch).expect("insert_batch");
    }

    fn query_size(storage: &SqliteStorage, path: &str) -> Option<u64> {
        let query = EntryQuery {
            path_prefix: Some(PathBuf::from(path)),
            ..EntryQuery::default()
        };
        storage
            .query_entries(&query)
            .expect("query_entries")
            .into_iter()
            .find(|e| e.path() == Path::new(path))
            .map(|e| e.size())
    }

    // -----------------------------------------------------------------------
    // aggregate_directory_sizes tests
    // -----------------------------------------------------------------------

    #[test]
    fn aggregate_flat_directory() {
        let (storage, _dir) = open_temp_storage();
        insert(
            &storage,
            vec![
                make_dir("/d"),
                make_file("/d/a", 10),
                make_file("/d/b", 20),
                make_file("/d/c", 30),
            ],
        );
        aggregate_directory_sizes(&storage).expect("aggregate");
        assert_eq!(query_size(&storage, "/d"), Some(60));
    }

    #[test]
    fn aggregate_nested_directories() {
        let (storage, _dir) = open_temp_storage();
        insert(
            &storage,
            vec![
                make_dir("/r"),
                make_dir("/r/sub"),
                make_file("/r/sub/file.txt", 100),
            ],
        );
        aggregate_directory_sizes(&storage).expect("aggregate");
        assert_eq!(query_size(&storage, "/r"), Some(100));
        assert_eq!(query_size(&storage, "/r/sub"), Some(100));
    }

    #[test]
    fn aggregate_multiple_levels() {
        let (storage, _dir) = open_temp_storage();
        insert(
            &storage,
            vec![
                make_dir("/a"),
                make_dir("/a/b"),
                make_dir("/a/b/c"),
                make_file("/a/b/c/file", 42),
            ],
        );
        aggregate_directory_sizes(&storage).expect("aggregate");
        assert_eq!(query_size(&storage, "/a"), Some(42));
        assert_eq!(query_size(&storage, "/a/b"), Some(42));
        assert_eq!(query_size(&storage, "/a/b/c"), Some(42));
    }

    #[test]
    fn aggregate_empty_directory_returns_zero() {
        let (storage, _dir) = open_temp_storage();
        insert(&storage, vec![make_dir("/empty")]);
        aggregate_directory_sizes(&storage).expect("aggregate");
        assert_eq!(query_size(&storage, "/empty"), Some(0));
    }

    // -----------------------------------------------------------------------
    // compute_free_space tests
    // -----------------------------------------------------------------------

    #[test]
    fn free_space_returns_positive_values() {
        let tmp = tempdir().expect("tempdir");
        let info = compute_free_space(tmp.path()).expect("compute_free_space");
        assert!(info.total_bytes > 0, "total_bytes should be positive");
        assert!(info.free_bytes > 0, "free_bytes should be positive");
        assert!(
            info.free_bytes <= info.total_bytes,
            "free_bytes should not exceed total_bytes"
        );
    }
}
