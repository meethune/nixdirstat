//! Domain error types for NixDirStat.
//!
//! Four error enums cover the four main subsystems:
//! - [`ScanError`]: filesystem scanning failures
//! - [`StorageError`]: `SQLite` persistence failures
//! - [`PipelineError`]: async pipeline coordination failures
//! - [`UiError`]: TUI rendering and terminal failures

use std::path::PathBuf;

/// Errors that occur during filesystem scanning.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ScanError {
    /// The scan root path does not exist.
    #[error("scan root not found: {0}")]
    RootNotFound(PathBuf),

    /// The scan root path exists but is not a directory.
    #[error("scan root is not a directory: {0}")]
    RootNotDirectory(PathBuf),

    /// Batch size was set to zero, which is invalid.
    #[error("batch_size must be greater than zero")]
    InvalidBatchSize,

    /// An I/O error occurred while accessing the filesystem.
    #[error("I/O error during scan: {0}")]
    Io(#[from] std::io::Error),

    /// A directory entry could not be read (e.g. permission denied on a subtree).
    #[error("failed to read directory entry at {path}: {source}")]
    EntryRead {
        /// The path that could not be read.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}

/// Errors that occur during `SQLite` storage operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StorageError {
    /// A `SQLite` error was returned by `rusqlite`.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// The database schema version is incompatible with this version of NixDirStat.
    #[error("incompatible database schema version {found}, expected {expected}")]
    IncompatibleSchema {
        /// The schema version found in the database file.
        found: u32,
        /// The schema version this binary expects.
        expected: u32,
    },

    /// The database file could not be opened or created.
    #[error("failed to open database at {path}: {source}")]
    Open {
        /// The database file path.
        path: PathBuf,
        /// The underlying `SQLite` error.
        source: rusqlite::Error,
    },

    /// An I/O error occurred while accessing the database file.
    #[error("I/O error accessing database: {0}")]
    Io(#[from] std::io::Error),
}

/// Errors that occur in the async scan→storage pipeline.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PipelineError {
    /// The scanner task panicked or was cancelled.
    #[error("scanner task failed: {0}")]
    ScannerFailed(String),

    /// The storage writer task panicked or was cancelled.
    #[error("storage writer task failed: {0}")]
    WriterFailed(String),

    /// The pipeline channel was closed unexpectedly.
    #[error("pipeline channel closed unexpectedly")]
    ChannelClosed,

    /// An underlying scan error propagated through the pipeline.
    #[error("scan error in pipeline: {0}")]
    Scan(#[from] ScanError),

    /// An underlying storage error propagated through the pipeline.
    #[error("storage error in pipeline: {0}")]
    Storage(#[from] StorageError),
}

/// Errors that occur in the TUI layer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum UiError {
    /// An I/O error occurred while initialising or restoring the terminal.
    #[error("terminal I/O error: {0}")]
    Terminal(#[from] std::io::Error),

    /// A crossterm error occurred.
    #[error("crossterm error: {0}")]
    Crossterm(String),

    /// The terminal is too small to render the UI.
    #[error(
        "terminal too small: need at least {min_width}x{min_height}, got {actual_width}x{actual_height}"
    )]
    TerminalTooSmall {
        /// Minimum required width in columns.
        min_width: u16,
        /// Minimum required height in rows.
        min_height: u16,
        /// Actual terminal width in columns.
        actual_width: u16,
        /// Actual terminal height in rows.
        actual_height: u16,
    },

    /// The event stream ended unexpectedly.
    #[error("event stream ended unexpectedly")]
    EventStreamEnded,
}
