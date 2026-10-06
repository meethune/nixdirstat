//! Shared helpers for integration tests.

use std::{path::PathBuf, time::SystemTime};

use nixdirstat::{FileCategory, FileEntry, FileType};

/// Build a minimal [`FileEntry`] for use in storage integration tests.
///
/// - `path` is set to `/{name}` (absolute single-component path).
/// - `size` and `allocated_size` are both set to `size` so that size-ordered
///   queries behave predictably.
/// - All other metadata fields are zeroed / default.
pub fn create_test_entry(name: &str, size: u64, file_type: FileType) -> FileEntry {
    FileEntry {
        path: PathBuf::from(format!("/{name}")),
        size,
        allocated_size: size,
        file_type,
        category: FileCategory::NoExtension,
        inode: 0,
        device: 0,
        nlink: 1,
        uid: 0,
        gid: 0,
        mtime: SystemTime::UNIX_EPOCH,
        mode: 0,
    }
}
