//! Async scan-to-storage pipeline coordinator.
//!
//! This module orchestrates the complete scan lifecycle:
//! 1. A blocking scanner task walks the filesystem and emits [`crate::types::EntryBatch`] items.
//! 2. A blocking storage writer receives batches and persists them to `SQLite`.
//! 3. A post-processing step saves scan metadata, runs directory size aggregation,
//!    and optionally finalises the WAL journal for export.
//!
//! Callers receive two channels: a stream of [`ScanProgress`] updates and a
//! [`oneshot`] receiver that delivers the final [`PipelineResult`] (or a
//! [`PipelineError`]) when the pipeline completes.
//!
//! [`PipelineError`]: crate::error::PipelineError
//! [`oneshot`]: tokio::sync::oneshot

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::{
    analyzer::aggregate_directory_sizes,
    error::PipelineError,
    scanner::{Scanner, WalkdirScanner},
    storage::{Storage, sqlite::SqliteStorage},
    types::{JournalMode, ScanConfig, ScanMetadata, ScanProgress},
};

/// Capacity of the bounded progress-update channel.
///
/// Large enough to absorb bursts without blocking the scanner,
/// small enough that stale updates are not queued for long.
const PROGRESS_CHANNEL_CAPACITY: usize = 64;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Configuration for the async scan-to-storage pipeline.
///
/// Passed to [`run_pipeline`] to start a scan run.
pub struct PipelineConfig {
    /// Filesystem scan configuration (root path, cross-device, batch size).
    pub scan: ScanConfig,
    /// Capacity of the bounded [`crate::types::EntryBatch`] channel between scanner and writer.
    ///
    /// A value of `4` is appropriate for most workloads; higher values reduce
    /// back-pressure on the scanner at the cost of additional memory.
    pub channel_capacity: usize,
    /// `SQLite` journal mode to use when writing the database.
    pub journal_mode: JournalMode,
    /// Path where the output `SQLite` database will be created or overwritten.
    pub storage_path: PathBuf,
}

/// Timing breakdown for a completed pipeline run.
#[derive(Debug, Clone)]
pub struct PipelineTiming {
    /// Wall-clock time from pipeline start until the scanner task finished.
    pub scan_duration: Duration,
    /// Time spent inside the storage writer task (from its start to channel close).
    pub storage_duration: Duration,
    /// Time spent in post-processing (metadata save, aggregation, optional finalise).
    pub aggregation_duration: Duration,
    /// Total wall-clock time from [`run_pipeline`] entry to completion signal.
    pub total_duration: Duration,
}

/// Result of a successfully completed pipeline run.
#[derive(Debug)]
pub struct PipelineResult {
    /// Summary metadata produced by the scanner.
    pub metadata: ScanMetadata,
    /// Path to the output `SQLite` database.
    pub storage_path: PathBuf,
    /// Per-stage timing breakdown.
    pub timing: PipelineTiming,
}

// ---------------------------------------------------------------------------
// run_pipeline
// ---------------------------------------------------------------------------

/// Start the async scan-to-storage pipeline and return progress and completion channels.
///
/// The pipeline runs in three stages:
///
/// 1. **Scanner** (`spawn_blocking`): walks the filesystem, sends [`crate::types::EntryBatch`]
///    items over a bounded channel, and emits [`ScanProgress`] snapshots.
/// 2. **Writer** (`spawn_blocking`): drains the batch channel and inserts each batch
///    into the `SQLite` database. Finishes when the scanner drops the sender.
/// 3. **Post-processing** (`spawn_blocking`): opens a fresh connection to save scan
///    metadata, run directory size aggregation, and (for [`JournalMode::Delete`])
///    finalise the WAL journal for export.
///
/// Returns immediately with two channel receivers:
/// - `progress_rx`: receive [`ScanProgress`] updates while the scan runs.
/// - `completion_rx`: `.await` to obtain the [`PipelineResult`] when done.
///
/// # Errors
///
/// Always returns `Ok`; all runtime errors are delivered through `completion_rx`
/// as `Err(PipelineError)`.
pub async fn run_pipeline(
    config: PipelineConfig,
    cancel: CancellationToken,
) -> Result<
    (
        mpsc::Receiver<ScanProgress>,
        oneshot::Receiver<Result<PipelineResult, PipelineError>>,
    ),
    PipelineError,
> {
    let (batch_tx, batch_rx) = mpsc::channel(config.channel_capacity);
    let (progress_tx, progress_rx) = mpsc::channel(PROGRESS_CHANNEL_CAPACITY);
    let (completion_tx, completion_rx) = oneshot::channel();

    drop(tokio::spawn(run_coordinator(
        config,
        batch_tx,
        batch_rx,
        progress_tx,
        completion_tx,
        cancel,
    )));

    Ok((progress_rx, completion_rx))
}

// ---------------------------------------------------------------------------
// Private coordinator
// ---------------------------------------------------------------------------

/// Coordinator task: wires together scanner, writer, and post-processing.
///
/// Spawns the scanner and writer as `spawn_blocking` tasks, awaits both in
/// order, then opens a fresh connection for aggregation and metadata saving.
/// The final [`PipelineResult`] (or error) is sent on `completion_tx`.
async fn run_coordinator(
    config: PipelineConfig,
    batch_tx: mpsc::Sender<crate::types::EntryBatch>,
    mut batch_rx: mpsc::Receiver<crate::types::EntryBatch>,
    progress_tx: mpsc::Sender<ScanProgress>,
    completion_tx: oneshot::Sender<Result<PipelineResult, PipelineError>>,
    cancel: CancellationToken,
) {
    let total_start = Instant::now();

    let PipelineConfig {
        scan: scan_config,
        channel_capacity: _,
        journal_mode,
        storage_path,
    } = config;

    // --- Stage 1 + 2: Scanner and writer run concurrently. ---

    let scan_start = Instant::now();
    let scanner_handle = tokio::task::spawn_blocking(move || {
        WalkdirScanner::new().scan(&scan_config, batch_tx, progress_tx, cancel)
    });

    let storage_path_for_writer = storage_path.clone();
    let writer_handle = tokio::task::spawn_blocking(move || {
        let writer_start = Instant::now();
        let storage = SqliteStorage::open(&storage_path_for_writer, journal_mode)
            .map_err(PipelineError::from)?;
        while let Some(batch) = batch_rx.blocking_recv() {
            storage.insert_batch(&batch).map_err(PipelineError::from)?;
        }
        Ok::<Duration, PipelineError>(writer_start.elapsed())
    });

    // Await scanner — when it returns, `batch_tx` is dropped, signalling the writer.
    let (scan_duration, scan_metadata) = match scanner_handle.await {
        Ok(Ok(meta)) => (scan_start.elapsed(), meta),
        Ok(Err(e)) => {
            let _ = completion_tx.send(Err(PipelineError::Scan(e)));
            return;
        },
        Err(e) => {
            let _ = completion_tx.send(Err(PipelineError::ScannerFailed(e.to_string())));
            return;
        },
    };

    // Await writer — finishes once the batch channel is drained.
    let storage_duration = match writer_handle.await {
        Ok(Ok(dur)) => dur,
        Ok(Err(e)) => {
            let _ = completion_tx.send(Err(e));
            return;
        },
        Err(e) => {
            let _ = completion_tx.send(Err(PipelineError::WriterFailed(e.to_string())));
            return;
        },
    };

    // --- Stage 3: Post-processing. ---

    let agg_start = Instant::now();
    let scan_metadata_for_post = scan_metadata.clone();
    let storage_path_for_post = storage_path.clone();

    let post_result = tokio::task::spawn_blocking(move || {
        let storage = SqliteStorage::open(&storage_path_for_post, journal_mode)
            .map_err(PipelineError::from)?;
        storage
            .save_scan_metadata(&scan_metadata_for_post)
            .map_err(PipelineError::from)?;
        aggregate_directory_sizes(&storage).map_err(PipelineError::from)?;
        if journal_mode == JournalMode::Delete {
            storage.finalize_for_export().map_err(PipelineError::from)?;
        }
        Ok::<(), PipelineError>(())
    })
    .await;

    let aggregation_duration = agg_start.elapsed();

    let post_ok = match post_result {
        Ok(result) => result,
        Err(e) => Err(PipelineError::WriterFailed(e.to_string())),
    };

    if let Err(e) = post_ok {
        let _ = completion_tx.send(Err(e));
        return;
    }

    let total_duration = total_start.elapsed();
    let _ = completion_tx.send(Ok(PipelineResult {
        metadata: scan_metadata,
        storage_path,
        timing: PipelineTiming {
            scan_duration,
            storage_duration,
            aggregation_duration,
            total_duration,
        },
    }));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use rusqlite::Connection;
    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::types::ScanConfig;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    /// Build a small test directory tree under `dir`.
    ///
    /// Layout (sizes are byte counts):
    /// ```text
    /// dir/
    ///   a.rs        (10 bytes)
    ///   b.jpg       (20 bytes)
    ///   sub/
    ///     c.txt     (5 bytes)
    ///     d.rs      (15 bytes)
    ///   empty/      (empty directory)
    /// ```
    fn setup_test_tree(dir: &Path) {
        fs::write(dir.join("a.rs"), "a".repeat(10)).expect("write a.rs");
        fs::write(dir.join("b.jpg"), "b".repeat(20)).expect("write b.jpg");
        let sub = dir.join("sub");
        fs::create_dir(&sub).expect("create sub/");
        fs::write(sub.join("c.txt"), "c".repeat(5)).expect("write c.txt");
        fs::write(sub.join("d.rs"), "d".repeat(15)).expect("write d.rs");
        fs::create_dir(dir.join("empty")).expect("create empty/");
    }

    fn make_pipeline_config(
        scan_root: &Path,
        db_path: &Path,
        journal_mode: JournalMode,
    ) -> PipelineConfig {
        PipelineConfig {
            scan: ScanConfig::builder()
                .root(scan_root)
                .build()
                .expect("build scan config"),
            channel_capacity: 4,
            journal_mode,
            storage_path: db_path.to_path_buf(),
        }
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    /// Pipeline writes at least one entry to the database.
    #[tokio::test]
    async fn pipeline_scans_and_persists() {
        let tree_dir = TempDir::new().expect("tree tempdir");
        let db_dir = TempDir::new().expect("db tempdir");
        setup_test_tree(tree_dir.path());

        let db_path = db_dir.path().join("scan.db");
        let config = make_pipeline_config(tree_dir.path(), &db_path, JournalMode::Wal);
        let cancel = CancellationToken::new();

        let (_progress_rx, completion_rx) =
            run_pipeline(config, cancel).await.expect("run_pipeline");

        let result = completion_rx
            .await
            .expect("completion channel")
            .expect("pipeline ok");

        let conn = Connection::open(&result.storage_path).expect("open db");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM entries", [], |row| row.get(0))
            .expect("count");
        assert!(count > 0, "expected entries in database, got {count}");
    }

    /// After the pipeline, directory entries have aggregated size > 0
    /// for directories that contain files.
    #[tokio::test]
    async fn pipeline_runs_aggregation() {
        let tree_dir = TempDir::new().expect("tree tempdir");
        let db_dir = TempDir::new().expect("db tempdir");
        setup_test_tree(tree_dir.path());

        let db_path = db_dir.path().join("scan.db");
        let config = make_pipeline_config(tree_dir.path(), &db_path, JournalMode::Wal);
        let cancel = CancellationToken::new();

        let (_progress_rx, completion_rx) =
            run_pipeline(config, cancel).await.expect("run_pipeline");
        let result = completion_rx
            .await
            .expect("completion channel")
            .expect("pipeline ok");

        let conn = Connection::open(&result.storage_path).expect("open db");
        // The `sub/` directory should have a nonzero aggregated size.
        let dir_with_size: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE file_type = 1 AND size > 0",
                [],
                |row| row.get(0),
            )
            .expect("query dirs");
        assert!(
            dir_with_size > 0,
            "expected at least one directory with aggregated size > 0"
        );
    }

    /// The progress channel delivers at least one update with `entries_scanned` > 0.
    #[tokio::test]
    async fn pipeline_reports_progress() {
        let tree_dir = TempDir::new().expect("tree tempdir");
        let db_dir = TempDir::new().expect("db tempdir");
        setup_test_tree(tree_dir.path());

        let db_path = db_dir.path().join("scan.db");
        let config = make_pipeline_config(tree_dir.path(), &db_path, JournalMode::Wal);
        let cancel = CancellationToken::new();

        let (mut progress_rx, completion_rx) =
            run_pipeline(config, cancel).await.expect("run_pipeline");

        // Drain progress while waiting for completion.
        let mut got_progress = false;
        let completion_fut = completion_rx;
        tokio::pin!(completion_fut);

        loop {
            tokio::select! {
                Some(p) = progress_rx.recv() => {
                    got_progress |= p.entries_scanned > 0;
                }
                result = &mut completion_fut => {
                    result.expect("completion channel").expect("pipeline ok");
                    break;
                }
            }
        }

        // Drain any remaining progress updates.
        while let Ok(p) = progress_rx.try_recv() {
            got_progress |= p.entries_scanned > 0;
        }

        assert!(
            got_progress,
            "expected at least one progress update with entries_scanned > 0"
        );
    }

    /// The completion channel yields `Ok(PipelineResult)` with `entry_count` > 0.
    #[tokio::test]
    async fn pipeline_signals_completion() {
        let tree_dir = TempDir::new().expect("tree tempdir");
        let db_dir = TempDir::new().expect("db tempdir");
        setup_test_tree(tree_dir.path());

        let db_path = db_dir.path().join("scan.db");
        let config = make_pipeline_config(tree_dir.path(), &db_path, JournalMode::Wal);
        let cancel = CancellationToken::new();

        let (_progress_rx, completion_rx) =
            run_pipeline(config, cancel).await.expect("run_pipeline");

        let result = completion_rx
            .await
            .expect("completion channel")
            .expect("pipeline ok");
        assert!(
            result.metadata.entry_count > 0,
            "expected entry_count > 0, got {}",
            result.metadata.entry_count
        );
    }

    /// Cancelling before the pipeline runs still delivers a result on `completion_rx`.
    #[tokio::test]
    async fn pipeline_respects_cancellation() {
        let tree_dir = TempDir::new().expect("tree tempdir");
        let db_dir = TempDir::new().expect("db tempdir");
        setup_test_tree(tree_dir.path());

        let db_path = db_dir.path().join("scan.db");
        let config = make_pipeline_config(tree_dir.path(), &db_path, JournalMode::Wal);

        let cancel = CancellationToken::new();
        cancel.cancel(); // Signal cancellation before the pipeline starts.

        let (_progress_rx, completion_rx) =
            run_pipeline(config, cancel).await.expect("run_pipeline");

        // The pipeline should complete with either Ok (partial/empty) or Err.
        let outcome = completion_rx.await.expect("completion channel");
        // We do not assert Ok here — partial or empty results are acceptable.
        // We only assert that the pipeline terminates and delivers something.
        let _ = outcome;
    }

    /// In DELETE journal mode the pipeline calls `finalize_for_export`, leaving
    /// the database with `journal_mode` = 'delete'.
    #[tokio::test]
    async fn pipeline_finalizes_in_delete_mode() {
        let tree_dir = TempDir::new().expect("tree tempdir");
        let db_dir = TempDir::new().expect("db tempdir");
        setup_test_tree(tree_dir.path());

        let db_path = db_dir.path().join("scan.db");
        let config = make_pipeline_config(tree_dir.path(), &db_path, JournalMode::Delete);
        let cancel = CancellationToken::new();

        let (_progress_rx, completion_rx) =
            run_pipeline(config, cancel).await.expect("run_pipeline");

        let result = completion_rx
            .await
            .expect("completion channel")
            .expect("pipeline ok");

        let conn = Connection::open(&result.storage_path).expect("open db");
        let mode: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .expect("journal_mode pragma");
        assert_eq!(
            mode, "delete",
            "expected journal_mode=delete after finalize"
        );
    }
}
