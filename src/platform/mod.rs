//! Platform abstraction layer for filesystem type detection and `SQLite` journal mode selection.
//!
//! This module provides two public functions:
//! - [`detect_filesystem_type`]: queries the OS for the filesystem type at a given path.
//! - [`recommended_journal_mode`]: selects the optimal `SQLite` WAL strategy based on the
//!   filesystem type and whether the application is running in interactive (TUI) mode.
//!
//! ## Cross-platform strategy
//!
//! | Tier | Platform       | Implementation                                             |
//! |------|----------------|------------------------------------------------------------|
//! | 2/3  | Linux          | `statfs(2)` → `f_type` magic number → string name         |
//! | 2/3  | macOS, FreeBSD | `statfs(2)` → `f_fstypename` C string → owned String      |
//! | 4    | Other          | Returns `"unknown"` without I/O                            |

use std::path::Path;

use crate::types::JournalMode;

mod alloc;
pub(crate) use alloc::{AllocatedSizeResolver, select_resolver};

// Sanctioned unsafe exception: btrfs_ioctl wraps one kernel ioctl call in a
// safe API. It is the sole module with unsafe code, auditable in isolation.
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub(crate) mod btrfs_ioctl;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "freebsd")]
mod freebsd;

/// Returns `true` if the filesystem at `path` is a virtual/pseudo filesystem
/// (procfs, sysfs, debugfs, etc.) that should be skipped during scanning.
///
/// # Platform behaviour
///
/// - **Linux**: calls `statfs(2)` and matches the magic number against known
///   virtual filesystem types.
/// - **Other**: always returns `false`.
pub(crate) fn is_virtual_filesystem(path: &Path) -> bool {
    is_virtual_impl(path)
}

#[cfg(target_os = "linux")]
fn is_virtual_impl(path: &Path) -> bool {
    linux::is_virtual_filesystem(path)
}

#[cfg(not(target_os = "linux"))]
fn is_virtual_impl(_path: &Path) -> bool {
    false
}

/// Detect the filesystem type for the path's mount point.
///
/// The returned string is a lowercase name like `"ext4"`, `"btrfs"`, `"apfs"`, or `"zfs"`.
/// On Linux, unrecognised types are returned as `"0x{hex}"` (e.g. `"0xef53"`).
///
/// # Platform behaviour
///
/// - **Linux**: calls `statfs(2)` via [`nix`] and matches the `f_type` magic number against
///   known constants. Unrecognised types return the magic number in hex.
/// - **macOS / FreeBSD**: calls `statfs(2)` via [`nix`] and reads the `f_fstypename` field.
/// - **Other**: returns `"unknown"` without making any system call.
///
/// # Errors
///
/// Returns an [`std::io::Error`] if the `statfs` system call fails (e.g. the path does not
/// exist or the process lacks permission to stat it).
pub fn detect_filesystem_type(path: &Path) -> Result<String, std::io::Error> {
    detect_impl(path)
}

#[cfg(target_os = "linux")]
fn detect_impl(path: &Path) -> Result<String, std::io::Error> {
    linux::detect_filesystem_type(path)
}

#[cfg(target_os = "macos")]
fn detect_impl(path: &Path) -> Result<String, std::io::Error> {
    macos::detect_filesystem_type(path)
}

#[cfg(target_os = "freebsd")]
fn detect_impl(path: &Path) -> Result<String, std::io::Error> {
    freebsd::detect_filesystem_type(path)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "freebsd")))]
fn detect_impl(_path: &Path) -> Result<String, std::io::Error> {
    Ok("unknown".into())
}

/// Returns `true` if the regular file at `path` is sparse (contains holes).
///
/// Uses `SEEK_HOLE` to detect holes without reading file data. A file is sparse
/// when `lseek(fd, 0, SEEK_HOLE)` returns an offset less than `file_size`,
/// indicating the file contains at least one hole before the end.
///
/// # Platform behaviour
///
/// - **Linux / FreeBSD**: uses `lseek` with `SEEK_HOLE`.
/// - **macOS / Other**: always returns `false` (APFS does not support sparse files).
///
/// Returns `false` on any error (permission denied, file disappeared, etc.).
pub(crate) fn is_sparse(path: &Path, file_size: u64) -> bool {
    if file_size == 0 {
        return false;
    }
    is_sparse_impl(path, file_size)
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn is_sparse_impl(path: &Path, file_size: u64) -> bool {
    use nix::unistd::{Whence, lseek};
    use std::os::fd::AsFd as _;

    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(hole_offset) = lseek(file.as_fd(), 0, Whence::SeekHole) else {
        return false;
    };
    u64::try_from(hole_offset).is_ok_and(|off| off < file_size)
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
fn is_sparse_impl(_path: &Path, _file_size: u64) -> bool {
    false
}

/// Recommend the optimal [`JournalMode`] for `SQLite` based on filesystem and usage mode.
///
/// # Decision logic
///
/// - **Batch mode** (`interactive = false`): always [`JournalMode::Delete`].
///   WAL adds unnecessary reader/writer coordination for a single-writer batch import.
/// - **Interactive mode** (`interactive = true`):
///   - `"zfs"` → [`JournalMode::Delete`]: ZFS is copy-on-write; WAL on top of ZFS causes
///     double-journaling (≈2.15× write amplification, per the specification).
///   - All other types → [`JournalMode::Wal`]: WAL enables concurrent readers during the
///     scan, which the TUI requires to display live progress.
pub fn recommended_journal_mode(fs_type: &str, interactive: bool) -> JournalMode {
    if !interactive {
        return JournalMode::Delete;
    }
    match fs_type {
        "zfs" => JournalMode::Delete,
        _ => JournalMode::Wal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- recommended_journal_mode ---

    #[test]
    fn batch_mode_always_delete() {
        assert_eq!(recommended_journal_mode("ext4", false), JournalMode::Delete);
    }

    #[test]
    fn zfs_interactive_is_delete() {
        assert_eq!(recommended_journal_mode("zfs", true), JournalMode::Delete);
    }

    #[test]
    fn ext4_interactive_is_wal() {
        assert_eq!(recommended_journal_mode("ext4", true), JournalMode::Wal);
    }

    #[test]
    fn unknown_interactive_is_wal() {
        assert_eq!(recommended_journal_mode("unknown", true), JournalMode::Wal);
    }

    #[test]
    fn apfs_interactive_is_wal() {
        assert_eq!(recommended_journal_mode("apfs", true), JournalMode::Wal);
    }

    // --- is_sparse ---

    #[test]
    fn sparse_file_detected() {
        use std::io::{Seek, SeekFrom, Write};

        let dir = std::env::temp_dir();
        let path = dir.join("nixdirstat_test_sparse");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            // Write one byte at a large offset to create a hole.
            f.seek(SeekFrom::Start(1_048_576)).unwrap();
            f.write_all(b"x").unwrap();
        }
        let meta = std::fs::metadata(&path).unwrap();
        assert!(
            is_sparse(&path, meta.len()),
            "file with a 1MB hole should be detected as sparse"
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn dense_file_not_sparse() {
        let dir = std::env::temp_dir();
        let path = dir.join("nixdirstat_test_dense");
        std::fs::write(&path, vec![0u8; 4096]).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        assert!(
            !is_sparse(&path, meta.len()),
            "fully written file should not be sparse"
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn empty_file_not_sparse() {
        assert!(!is_sparse(Path::new("/dev/null"), 0));
    }

    #[test]
    fn nonexistent_file_not_sparse() {
        assert!(!is_sparse(Path::new("/nonexistent_xyz"), 100));
    }

    // --- detect_filesystem_type (platform integration test) ---

    #[test]
    fn detect_filesystem_type_returns_nonempty_string() {
        let tmp = std::env::temp_dir();
        let result = detect_filesystem_type(&tmp);
        assert!(result.is_ok(), "expected Ok, got {result:?}");
        let fs_type = result.expect("detect_filesystem_type should succeed on temp dir");
        assert!(
            !fs_type.is_empty(),
            "filesystem type string should not be empty"
        );
    }
}
