//! Disk usage analysis: directory size aggregation, file-type statistics,
//! and free space computation.
//!
//! This module provides three free functions that operate on the [`Storage`] trait
//! or on slices of [`FileEntry`] values produced by a scan:
//!
//! - [`aggregate_directory_sizes`]: bottom-up aggregation of recursive directory sizes.
//! - [`compute_type_stats`]: per-category file-type statistics from an entry slice.
//! - [`compute_free_space`]: `statvfs`-based free space query for a filesystem path.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::{
    error::StorageError,
    storage::Storage,
    types::{DirectoryStats, EntryQuery, FileCategory, FileEntry, FileType, SpaceInfo, TypeStat},
};

/// Aggregate recursive directory sizes from all entries in `storage`.
///
/// Queries every stored [`FileEntry`], then for each non-directory entry walks
/// its ancestor chain, accumulating the entry's `size` and `allocated_size`
/// into each ancestor's [`DirectoryStats`]. The resulting map is written back
/// via [`Storage::update_directory_sizes`].
///
/// Directory entries are initialised with zero statistics so that empty
/// directories are represented explicitly in the output.
///
/// # Errors
///
/// Returns [`StorageError`] if the underlying query or update operation fails.
pub fn aggregate_directory_sizes(storage: &dyn Storage) -> Result<(), StorageError> {
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

/// Compute per-category file-type statistics from a slice of [`FileEntry`] values.
///
/// Only [`FileType::Regular`] entries are counted; directories, symlinks, and
/// other special files are skipped. Results are sorted by `total_size` descending,
/// so the largest category appears first.
pub fn compute_type_stats(entries: &[FileEntry]) -> Vec<TypeStat> {
    let mut map: HashMap<FileCategory, TypeStat> = HashMap::new();

    for entry in entries {
        if entry.file_type() != FileType::Regular {
            continue;
        }

        let stat = map.entry(entry.category()).or_insert_with(|| TypeStat {
            category: entry.category(),
            count: 0,
            total_size: 0,
            total_allocated: 0,
        });
        stat.count = stat.count.saturating_add(1);
        stat.total_size = stat.total_size.saturating_add(entry.size());
        stat.total_allocated = stat.total_allocated.saturating_add(entry.allocated_size());
    }

    let mut result: Vec<TypeStat> = map.into_values().collect();
    result.sort_by_key(|ts| std::cmp::Reverse(ts.total_size));
    result
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
    // f_frsize is the fundamental fragment size used by f_blocks and f_bavail.
    // cast_possible_truncation: on 32-bit targets fsblkcnt_t and c_ulong are
    // u32; widening to u64 is safe. On 64-bit targets the types are already u64.
    #[allow(clippy::cast_possible_truncation)]
    {
        let stat = nix::sys::statvfs::statvfs(path).map_err(std::io::Error::from)?;
        let frsize = stat.fragment_size() as u64;
        let total = (stat.blocks() as u64).saturating_mul(frsize);
        let free = (stat.blocks_available() as u64).saturating_mul(frsize);
        let unknown = total.saturating_sub(free);

        Ok(SpaceInfo {
            total_bytes: total,
            free_bytes: free,
            unknown_bytes: unknown,
        })
    }
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
        storage::sqlite::SqliteStorage,
        types::{EntryBatch, JournalMode},
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

    fn make_file_with_ext(path: &str, size: u64, category: FileCategory) -> FileEntry {
        FileEntryBuilder::new()
            .path(path)
            .size(size)
            .category(category)
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
    // compute_type_stats tests
    // -----------------------------------------------------------------------

    #[test]
    fn type_stats_groups_by_category() {
        let entries = vec![
            make_file_with_ext("/a.rs", 10, FileCategory::Code),
            make_file_with_ext("/b.rs", 10, FileCategory::Code),
            make_file_with_ext("/c.jpg", 50, FileCategory::Image),
        ];
        let stats = compute_type_stats(&entries);

        let code = stats.iter().find(|s| s.category == FileCategory::Code);
        let image = stats.iter().find(|s| s.category == FileCategory::Image);

        let code = code.expect("Code stat present");
        assert_eq!(code.count, 2);
        assert_eq!(code.total_size, 20);

        let image = image.expect("Image stat present");
        assert_eq!(image.count, 1);
        assert_eq!(image.total_size, 50);
    }

    #[test]
    fn type_stats_sorted_by_size_desc() {
        let entries = vec![
            make_file_with_ext("/small.rs", 5, FileCategory::Code),
            make_file_with_ext("/big.jpg", 100, FileCategory::Image),
        ];
        let stats = compute_type_stats(&entries);
        assert_eq!(stats.len(), 2);
        assert!(
            stats[0].total_size >= stats[1].total_size,
            "results should be sorted largest first"
        );
        assert_eq!(stats[0].category, FileCategory::Image);
    }

    #[test]
    fn type_stats_skips_non_regular_entries() {
        let dir_entry = FileEntryBuilder::new()
            .path("/mydir")
            .size(999)
            .file_type(FileType::Directory)
            .build();
        let stats = compute_type_stats(&[dir_entry]);
        assert!(stats.is_empty(), "directories should not be counted");
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
