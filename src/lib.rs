//! NixDirStat — disk usage analyzer and cleanup assistant for POSIX-compliant systems.
//!
//! Scan local filesystems, devices, and directories then explore disk usage
//! through sortable file lists, file-type statistics, and interactive treemaps.

mod cli;

pub mod error;
pub mod platform;
pub mod scanner;
pub mod storage;
pub mod types;

pub use error::{PipelineError, ScanError, StorageError, UiError};
pub use scanner::{Scanner, WalkdirScanner};
pub use types::{
    DirectoryStats, EntryBatch, EntryQuery, FileCategory, FileEntry, FileType, JournalMode,
    ScanConfig, ScanConfigBuilder, ScanMetadata, ScanProgress, ScanWarning, SortDirection,
    SortField, SpaceInfo, TypeStat, format_size,
};

/// Application entry point, called from `main.rs`.
///
/// Parses CLI arguments, dispatches to the appropriate subcommand,
/// and returns any errors for display by the caller.
///
/// # Errors
///
/// Returns an error if CLI parsing, scanning, or UI initialization fails.
pub fn run() -> anyhow::Result<()> {
    let _cli = cli::parse();
    Ok(())
}
