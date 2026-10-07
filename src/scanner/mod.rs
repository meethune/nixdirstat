//! Filesystem scanner abstraction.
//!
//! Defines the [`Scanner`] trait implemented by all scanner backends, and
//! re-exports the production [`WalkdirScanner`] implementation.
//!
//! Scanners are expected to run inside [`tokio::task::spawn_blocking`] because
//! filesystem I/O is inherently blocking. The trait is synchronous by design;
//! progress updates and entry batches are communicated over
//! [`tokio::sync::mpsc`] channels using `blocking_send` and `try_send`.

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    error::ScanError,
    sync::PauseToken,
    types::{EntryBatch, ScanConfig, ScanMetadata, ScanProgress},
};

pub mod walkdir;
pub mod watcher;
pub use walkdir::WalkdirScanner;

/// A filesystem scanner that walks a directory tree and emits batches of entries.
///
/// Implementors run in a blocking context (e.g. inside
/// [`tokio::task::spawn_blocking`]). The scan method is synchronous; it blocks
/// the calling thread until the walk is complete or cancelled.
pub trait Scanner {
    /// Walk the directory tree rooted at [`ScanConfig::root`] and emit
    /// [`crate::types::FileEntry`] items in batches over `batch_tx`.
    ///
    /// - After each entry a [`ScanProgress`] snapshot is attempted via
    ///   `try_send` (lossy — dropped updates are not an error).
    /// - The scan stops early when `cancel` is signalled; partial results are
    ///   still returned rather than an error.
    ///
    /// # Errors
    ///
    /// Returns [`ScanError::RootNotFound`] if the root path cannot be accessed
    /// via [`std::fs::symlink_metadata`]. Other per-entry I/O errors are
    /// demoted to warnings and collected in the returned [`ScanMetadata`].
    fn scan(
        &self,
        config: &ScanConfig,
        batch_tx: mpsc::Sender<EntryBatch>,
        progress_tx: mpsc::Sender<ScanProgress>,
        cancel: CancellationToken,
        pause: Arc<PauseToken>,
    ) -> Result<ScanMetadata, ScanError>;
}
