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

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "freebsd")]
mod freebsd;

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
