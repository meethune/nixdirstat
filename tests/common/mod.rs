//! Shared helpers for integration tests.

use std::{path::Path, path::PathBuf, time::SystemTime};

use nixdirstat::{FileCategory, FileEntry, FileType};

/// Build a minimal [`FileEntry`] for use in storage integration tests.
///
/// - `path` is set to `/{name}` (absolute single-component path).
/// - `size` and `allocated_size` are both set to `size` so that size-ordered
///   queries behave predictably.
/// - All other metadata fields are zeroed / default.
/// Create the standard test directory tree under `dir`.
///
/// Layout (sizes are file content lengths):
/// ```text
/// dir/
///   a.rs        (10 bytes)
///   b.jpg       (20 bytes)
///   sub/
///     c.txt     (5 bytes)
///     d.rs      (15 bytes)
///   empty/      (empty directory)
/// ```
pub fn create_test_tree(dir: &Path) {
    std::fs::write(dir.join("a.rs"), "a".repeat(10)).expect("write a.rs");
    std::fs::write(dir.join("b.jpg"), "b".repeat(20)).expect("write b.jpg");
    let sub = dir.join("sub");
    std::fs::create_dir(&sub).expect("create sub/");
    std::fs::write(sub.join("c.txt"), "c".repeat(5)).expect("write sub/c.txt");
    std::fs::write(sub.join("d.rs"), "d".repeat(15)).expect("write sub/d.rs");
    std::fs::create_dir(dir.join("empty")).expect("create empty/");
}

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
