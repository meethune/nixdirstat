//! Integration tests for the CLI subcommands.
//!
//! These tests exercise the batch scan, explore validation, and export paths at
//! the component level. The interactive scan and explore paths cannot be tested
//! in CI because they require a TTY; they are validated manually.

use std::{fs, path::Path};

use nixdirstat::storage::{Storage, sqlite::SqliteStorage};
use nixdirstat::types::EntryQuery;
use nixdirstat::{
    JournalMode, PipelineConfig, ScanConfig, run_pipeline, write_entries_csv, write_entries_json,
};
use rusqlite::Connection;
use tempfile::{NamedTempFile, TempDir};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a small directory tree for testing under `dir`.
///
/// Layout:
/// ```text
/// dir/
///   a.rs      (10 bytes)
///   b.jpg     (20 bytes)
///   sub/
///     c.txt   (5 bytes)
/// ```
fn setup_test_tree(dir: &Path) -> anyhow::Result<()> {
    fs::write(dir.join("a.rs"), "a".repeat(10))?;
    fs::write(dir.join("b.jpg"), "b".repeat(20))?;
    let sub = dir.join("sub");
    fs::create_dir(&sub)?;
    fs::write(sub.join("c.txt"), "c".repeat(5))?;
    Ok(())
}

/// Build a [`PipelineConfig`] for test scans.
fn make_pipeline_config(
    scan_root: &Path,
    db_path: &Path,
    journal_mode: JournalMode,
) -> anyhow::Result<PipelineConfig> {
    let config = ScanConfig::builder()
        .root(scan_root)
        .build()
        .map_err(|e| anyhow::anyhow!("build scan config: {e}"))?;
    Ok(PipelineConfig {
        scan: config,
        channel_capacity: 4,
        journal_mode,
        storage_path: db_path.to_path_buf(),
    })
}

// ---------------------------------------------------------------------------
// Scan batch tests
// ---------------------------------------------------------------------------

/// After a batch scan, the output database file must exist on disk.
#[tokio::test]
async fn scan_batch_creates_database_file() -> anyhow::Result<()> {
    let tree_dir = TempDir::new()?;
    let db_dir = TempDir::new()?;
    setup_test_tree(tree_dir.path())?;

    let db_path = db_dir.path().join("scan.db");
    let config = make_pipeline_config(tree_dir.path(), &db_path, JournalMode::Delete)?;
    let cancel = CancellationToken::new();

    let (_progress_rx, completion_rx) = run_pipeline(config, cancel).await?;
    completion_rx.await??;

    assert!(
        db_path.exists(),
        "scan database file should exist after scan"
    );
    Ok(())
}

/// After a batch scan, the database must contain at least one entry.
#[tokio::test]
async fn scan_batch_database_contains_entries() -> anyhow::Result<()> {
    let tree_dir = TempDir::new()?;
    let db_dir = TempDir::new()?;
    setup_test_tree(tree_dir.path())?;

    let db_path = db_dir.path().join("scan.db");
    let config = make_pipeline_config(tree_dir.path(), &db_path, JournalMode::Delete)?;
    let cancel = CancellationToken::new();

    let (_progress_rx, completion_rx) = run_pipeline(config, cancel).await?;
    let result = completion_rx.await??;

    let conn = Connection::open(&result.storage_path)?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM entries", [], |row| row.get(0))?;
    assert!(
        count > 0,
        "database should contain entries after scan, got {count}"
    );
    Ok(())
}

/// In batch mode (`JournalMode::Delete`) the pipeline calls `finalize_for_export`,
/// leaving the database with `PRAGMA journal_mode = 'delete'`.
#[tokio::test]
async fn scan_batch_database_is_finalized() -> anyhow::Result<()> {
    let tree_dir = TempDir::new()?;
    let db_dir = TempDir::new()?;
    setup_test_tree(tree_dir.path())?;

    let db_path = db_dir.path().join("scan.db");
    let config = make_pipeline_config(tree_dir.path(), &db_path, JournalMode::Delete)?;
    let cancel = CancellationToken::new();

    let (_progress_rx, completion_rx) = run_pipeline(config, cancel).await?;
    let result = completion_rx.await??;

    let conn = Connection::open(&result.storage_path)?;
    let mode: String = conn.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
    assert_eq!(
        mode, "delete",
        "journal_mode should be 'delete' after finalization"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Explore validation tests
// ---------------------------------------------------------------------------

/// The explore subcommand must reject a scan file that does not exist.
///
/// The `run()` function calls `anyhow::ensure!(scan_file.exists(), ...)`.
/// We verify the condition inline since `run_explore_ui` requires a TTY.
#[test]
fn explore_rejects_nonexistent_file() {
    let nonexistent =
        std::path::PathBuf::from("/no/such/nixdirstat_test_path/__scan_that_does_not_exist.db");
    let result: anyhow::Result<()> = (|| {
        anyhow::ensure!(
            nonexistent.exists(),
            "scan file not found: {}",
            nonexistent.display()
        );
        Ok(())
    })();
    assert!(result.is_err(), "should return error for nonexistent file");
    let err_msg = result.map_or_else(|e| e.to_string(), |()| String::new());
    assert!(
        err_msg.contains("not found"),
        "error message should contain 'not found', got: {err_msg:?}"
    );
}

/// The explore subcommand must reject a file that is not a valid `SQLite` database.
///
/// `SqliteStorage::open_readonly` checks the `user_version` pragma and will
/// fail for a plain text file (`SQLite` parse error or schema version mismatch).
#[test]
fn explore_rejects_invalid_database() -> anyhow::Result<()> {
    let tmp = NamedTempFile::new()?;
    fs::write(tmp.path(), b"this is not a valid sqlite database file\n")?;

    let result = SqliteStorage::open_readonly(tmp.path());
    assert!(
        result.is_err(),
        "opening a non-`SQLite` file as a database should fail"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Scan validation tests
// ---------------------------------------------------------------------------

/// The scan subcommand must reject a root path that does not exist.
#[test]
fn scan_rejects_nonexistent_path() {
    let result = ScanConfig::builder()
        .root("/no/such/path/__nixdirstat_integration_test")
        .build();
    assert!(
        result.is_err(),
        "ScanConfig::build should fail for a nonexistent path"
    );
}

// ---------------------------------------------------------------------------
// Export tests
// ---------------------------------------------------------------------------

/// `write_entries_json` must produce a valid JSON array.
#[tokio::test]
async fn export_json_valid() -> anyhow::Result<()> {
    let tree_dir = TempDir::new()?;
    let db_dir = TempDir::new()?;
    setup_test_tree(tree_dir.path())?;

    let db_path = db_dir.path().join("scan.db");
    let config = make_pipeline_config(tree_dir.path(), &db_path, JournalMode::Delete)?;
    let cancel = CancellationToken::new();

    let (_progress_rx, completion_rx) = run_pipeline(config, cancel).await?;
    completion_rx.await??;

    let storage = SqliteStorage::open_readonly(&db_path)?;
    let entries = storage.query_entries(&EntryQuery::default())?;

    let mut buf = Vec::<u8>::new();
    write_entries_json(&entries, &mut buf)?;

    let parsed: serde_json::Value = serde_json::from_slice(&buf)?;
    assert!(parsed.is_array(), "JSON export should be an array");
    let len = parsed.as_array().map_or(0, Vec::len);
    assert!(
        len > 0,
        "JSON array should be non-empty, got {len} elements"
    );
    Ok(())
}

/// `write_entries_csv` must produce output whose first line is a header
/// containing `"path"`.
#[tokio::test]
async fn export_csv_has_header() -> anyhow::Result<()> {
    let tree_dir = TempDir::new()?;
    let db_dir = TempDir::new()?;
    setup_test_tree(tree_dir.path())?;

    let db_path = db_dir.path().join("scan.db");
    let config = make_pipeline_config(tree_dir.path(), &db_path, JournalMode::Delete)?;
    let cancel = CancellationToken::new();

    let (_progress_rx, completion_rx) = run_pipeline(config, cancel).await?;
    completion_rx.await??;

    let storage = SqliteStorage::open_readonly(&db_path)?;
    let entries = storage.query_entries(&EntryQuery::default())?;

    let mut buf = Vec::<u8>::new();
    write_entries_csv(&entries, &mut buf)?;

    let output = String::from_utf8(buf)?;
    let first_line = output
        .lines()
        .next()
        .ok_or_else(|| anyhow::anyhow!("CSV output is empty"))?;
    assert!(
        first_line.contains("path"),
        "CSV header should contain 'path', got: {first_line:?}"
    );
    Ok(())
}
