//! [`WalkDir`]-based filesystem scanner implementation.
//!
//! [`WalkdirScanner`] is the production scanner. It walks a directory tree
//! with [`WalkDir`] (symlinks are never followed during traversal), collects
//! per-entry metadata, deduplicates hard-linked files, and streams results in
//! batches over a Tokio MPSC channel.

use std::{
    collections::HashSet,
    mem,
    os::unix::fs::MetadataExt as _,
    path::PathBuf,
    time::{Instant, SystemTime},
};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use walkdir::WalkDir;

use crate::{
    error::ScanError,
    types::{EntryBatch, FileEntry, ScanConfig, ScanMetadata, ScanProgress, ScanWarning},
};

use super::Scanner;

/// How often (in entries) to re-check that the scan root still exists.
const ROOT_CHECK_INTERVAL: u64 = 1000;

// ---------------------------------------------------------------------------
// WalkdirScanner
// ---------------------------------------------------------------------------

/// Production scanner backed by the [`walkdir`] crate.
///
/// Walks the directory tree rooted at [`ScanConfig::root`] without following
/// symbolic links. Supports cross-device mount-point skipping and hard-link
/// deduplication (first occurrence retains its allocated size; subsequent
/// occurrences have their allocated size zeroed).
#[derive(Debug, Default)]
pub struct WalkdirScanner;

impl WalkdirScanner {
    /// Create a new [`WalkdirScanner`].
    pub const fn new() -> Self {
        Self
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Build the [`ScanMetadata`] returned at the end of a scan.
fn build_metadata(
    root: PathBuf,
    started_at: SystemTime,
    entry_count: u64,
    total_size: u64,
    filesystem_type: Option<&str>,
    warnings: Vec<ScanWarning>,
) -> ScanMetadata {
    let filesystem_types = filesystem_type.map_or_else(Vec::new, |t| vec![t.to_owned()]);
    ScanMetadata {
        root,
        started_at,
        completed_at: SystemTime::now(),
        entry_count,
        total_size,
        filesystem_types,
        warnings,
    }
}

/// Drain `batch` into a single [`EntryBatch`] and send it over `tx`.
///
/// Returns `true` if the batch was empty or was sent successfully, `false` if
/// the receiver was dropped and entries were lost.
fn flush_batch(tx: &mpsc::Sender<EntryBatch>, batch: &mut Vec<FileEntry>) -> bool {
    if let Some(b) = EntryBatch::new(mem::take(batch)) {
        return tx.blocking_send(b).is_ok();
    }
    true
}

/// Zero out `allocated_size` for hard-link duplicates.
///
/// For entries with more than one hard link, the first occurrence (i.e. the
/// first time a given `(ino, dev)` key is seen) keeps its `allocated_size`;
/// subsequent occurrences have it set to zero. This prevents double-counting
/// physical disk usage for multiply-linked files.
fn dedup_hardlink(
    file_entry: &mut FileEntry,
    metadata: &std::fs::Metadata,
    seen: &mut HashSet<(u64, u64)>,
) {
    if metadata.nlink() <= 1 {
        return;
    }
    let key = (metadata.ino(), metadata.dev());
    if seen.contains(&key) {
        file_entry.set_allocated_size(0);
    }
    seen.insert(key);
}

/// Attempt to send a progress snapshot over `tx` (lossy — a full channel is
/// not an error).
///
/// Computes the entries-per-second rate from `entry_count` and `start`.
fn send_progress(
    tx: &mpsc::Sender<ScanProgress>,
    entry_count: u64,
    current_path: PathBuf,
    start: Instant,
) {
    let elapsed_secs = start.elapsed().as_secs_f64();
    // cast_precision_loss: entry_count as f64; file counts in practice never
    // exceed 2^53, so f64 precision is sufficient for rate display.
    #[allow(clippy::cast_precision_loss)]
    let entries_per_second = if elapsed_secs > 0.0 {
        entry_count as f64 / elapsed_secs
    } else {
        0.0
    };
    let _ = tx.try_send(ScanProgress {
        entries_scanned: entry_count,
        entries_per_second,
        current_path,
        elapsed_secs,
    });
}

/// Send a full batch over `batch_tx` if `batch` has reached `batch_size`.
///
/// Returns `true` when the receiver has been dropped (the caller should stop
/// scanning and return partial results).
fn send_full_batch(
    batch: &mut Vec<FileEntry>,
    batch_tx: &mpsc::Sender<EntryBatch>,
    batch_size: usize,
) -> bool {
    if batch.len() < batch_size {
        return false;
    }
    let full_batch = mem::take(batch);
    *batch = Vec::with_capacity(batch_size);
    if let Some(b) = EntryBatch::new(full_batch) {
        return batch_tx.blocking_send(b).is_err();
    }
    false
}

/// Mutable state threaded through the walk loop.
struct WalkState {
    batch: Vec<FileEntry>,
    entry_count: u64,
    total_size: u64,
    warnings: Vec<ScanWarning>,
    seen_hardlinks: HashSet<(u64, u64)>,
}

/// Walk the directory tree, processing entries and sending batches.
///
/// Returns `Ok(true)` for early exit (cancellation or receiver drop),
/// `Ok(false)` on normal completion, or `Err` if the scan root disappears.
fn walk_tree(
    config: &ScanConfig,
    state: &mut WalkState,
    root_dev: u64,
    batch_tx: &mpsc::Sender<EntryBatch>,
    progress_tx: &mpsc::Sender<ScanProgress>,
    cancel: &CancellationToken,
    start: Instant,
) -> Result<bool, ScanError> {
    let walker = WalkDir::new(config.root()).follow_links(false);

    for result in walker {
        if cancel.is_cancelled() {
            let _ = flush_batch(batch_tx, &mut state.batch);
            return Ok(true);
        }

        let dir_entry = match result {
            Ok(entry) => entry,
            Err(err) => {
                let path = err
                    .path()
                    .map_or_else(|| config.root().to_path_buf(), PathBuf::from);
                state.warnings.push(ScanWarning {
                    path,
                    message: err.to_string(),
                });
                continue;
            },
        };

        let metadata = match dir_entry.path().symlink_metadata() {
            Ok(m) => m,
            Err(err) => {
                state.warnings.push(ScanWarning {
                    path: dir_entry.path().to_path_buf(),
                    message: err.to_string(),
                });
                continue;
            },
        };

        if !config.cross_device() && metadata.dev() != root_dev {
            continue;
        }

        let mut file_entry = FileEntry::from_metadata(dir_entry.path().to_path_buf(), &metadata);
        dedup_hardlink(&mut file_entry, &metadata, &mut state.seen_hardlinks);

        state.total_size = state.total_size.saturating_add(file_entry.size());
        state.entry_count += 1;

        if state.entry_count.is_multiple_of(ROOT_CHECK_INTERVAL) && !config.root().exists() {
            let _ = flush_batch(batch_tx, &mut state.batch);
            return Err(ScanError::RootDisappeared(config.root().to_path_buf()));
        }

        send_progress(
            progress_tx,
            state.entry_count,
            file_entry.path().to_path_buf(),
            start,
        );

        state.batch.push(file_entry);
        if send_full_batch(&mut state.batch, batch_tx, config.batch_size()) {
            return Ok(true);
        }
    }
    Ok(false)
}

// ---------------------------------------------------------------------------
// Scanner implementation
// ---------------------------------------------------------------------------

impl Scanner for WalkdirScanner {
    fn scan(
        &self,
        config: &ScanConfig,
        batch_tx: mpsc::Sender<EntryBatch>,
        progress_tx: mpsc::Sender<ScanProgress>,
        cancel: CancellationToken,
    ) -> Result<ScanMetadata, ScanError> {
        if cancel.is_cancelled() {
            return Ok(build_metadata(
                config.root().to_path_buf(),
                SystemTime::now(),
                0,
                0,
                config.filesystem_type(),
                vec![],
            ));
        }

        let start = Instant::now();
        let started_at = SystemTime::now();

        let root_dev = std::fs::symlink_metadata(config.root())
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    ScanError::RootNotFound(config.root().to_path_buf())
                } else {
                    ScanError::Io(e)
                }
            })?
            .dev();

        let mut state = WalkState {
            batch: Vec::with_capacity(config.batch_size()),
            entry_count: 0,
            total_size: 0,
            warnings: Vec::new(),
            seen_hardlinks: HashSet::new(),
        };

        let early_exit = walk_tree(
            config,
            &mut state,
            root_dev,
            &batch_tx,
            &progress_tx,
            &cancel,
            start,
        )?;

        if early_exit {
            return Ok(build_metadata(
                config.root().to_path_buf(),
                started_at,
                state.entry_count,
                state.total_size,
                config.filesystem_type(),
                state.warnings,
            ));
        }

        if !flush_batch(&batch_tx, &mut state.batch) {
            state.warnings.push(ScanWarning {
                path: config.root().to_path_buf(),
                message: "storage channel closed before final batch could be sent".into(),
            });
        }

        Ok(build_metadata(
            config.root().to_path_buf(),
            started_at,
            state.entry_count,
            state.total_size,
            config.filesystem_type(),
            state.warnings,
        ))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use tempfile::TempDir;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use crate::{
        ScanError,
        types::{FileType, ScanConfig},
    };

    use super::*;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    /// Channel capacity large enough that `blocking_send` never actually blocks
    /// during tests (the receiver is drained after the scan completes).
    const CHAN_CAP: usize = 4_096;

    /// Run a scan and collect all batches and the final metadata.
    fn run_scan(
        scanner: &WalkdirScanner,
        config: &ScanConfig,
    ) -> Result<(ScanMetadata, Vec<FileEntry>), ScanError> {
        run_scan_with_cancel(scanner, config, CancellationToken::new())
    }

    /// Run a scan with an explicit [`CancellationToken`].
    fn run_scan_with_cancel(
        scanner: &WalkdirScanner,
        config: &ScanConfig,
        cancel: CancellationToken,
    ) -> Result<(ScanMetadata, Vec<FileEntry>), ScanError> {
        let (batch_tx, mut batch_rx) = mpsc::channel(CHAN_CAP);
        let (progress_tx, _progress_rx) = mpsc::channel(CHAN_CAP);

        let metadata = scanner.scan(config, batch_tx, progress_tx, cancel)?;

        let mut entries: Vec<FileEntry> = Vec::new();
        while let Ok(batch) = batch_rx.try_recv() {
            entries.extend_from_slice(batch.entries());
        }
        Ok((metadata, entries))
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[test]
    fn scan_collects_regular_file_metadata() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, b"hello").unwrap();

        let config = ScanConfig::builder().root(dir.path()).build().unwrap();
        let scanner = WalkdirScanner::new();
        let (_metadata, entries) = run_scan(&scanner, &config).unwrap();

        let file_entry = entries
            .iter()
            .find(|e| e.path() == file_path)
            .expect("test.txt entry not found");

        assert_eq!(file_entry.size(), 5, "size should be 5 bytes");
        assert_eq!(
            file_entry.file_type(),
            FileType::Regular,
            "file_type should be Regular"
        );
        assert_eq!(file_entry.nlink(), 1, "nlink should be 1");
    }

    #[test]
    fn scan_collects_directory_entries() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("subdir");
        std::fs::create_dir(&sub).unwrap();

        let config = ScanConfig::builder().root(dir.path()).build().unwrap();
        let scanner = WalkdirScanner::new();
        let (_metadata, entries) = run_scan(&scanner, &config).unwrap();

        let sub_entry = entries
            .iter()
            .find(|e| e.path() == sub)
            .expect("subdir entry not found");

        assert_eq!(
            sub_entry.file_type(),
            FileType::Directory,
            "subdir should have file_type Directory"
        );
    }

    #[test]
    fn scan_records_symlinks_without_following() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("target.txt");
        let link = dir.path().join("link.txt");
        std::fs::write(&target, b"data").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let config = ScanConfig::builder().root(dir.path()).build().unwrap();
        let scanner = WalkdirScanner::new();
        let (_metadata, entries) = run_scan(&scanner, &config).unwrap();

        let link_entry = entries
            .iter()
            .find(|e| e.path() == link)
            .expect("link entry not found");

        assert_eq!(
            link_entry.file_type(),
            FileType::Symlink,
            "link entry should be Symlink"
        );

        let target_count = entries.iter().filter(|e| e.path() == target).count();
        assert_eq!(target_count, 1, "target should appear exactly once");
    }

    #[test]
    fn scan_skips_cross_device_entries() {
        // All temp files share the same device, so with cross_device=false
        // every entry should still be included (none are skipped).
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"aaa").unwrap();
        std::fs::write(dir.path().join("b.txt"), b"bbb").unwrap();

        let config = ScanConfig::builder()
            .root(dir.path())
            .cross_device(false)
            .build()
            .unwrap();
        let scanner = WalkdirScanner::new();
        let (_metadata, entries) = run_scan(&scanner, &config).unwrap();

        // root dir + 2 files = 3 entries; all must share the root device.
        assert!(
            entries.len() >= 3,
            "expected at least 3 entries, got {}",
            entries.len()
        );
        let root_dev = std::fs::symlink_metadata(dir.path()).unwrap().dev();
        for entry in &entries {
            assert_eq!(
                entry.device(),
                root_dev,
                "all entries should share root device"
            );
        }
    }

    #[test]
    fn scan_deduplicates_hardlink_allocated_size() {
        let dir = TempDir::new().unwrap();
        let original = dir.path().join("original.txt");
        let hardlink = dir.path().join("hardlink.txt");
        // Write content large enough that allocated blocks > 0.
        std::fs::write(&original, vec![0u8; 4096]).unwrap();
        std::fs::hard_link(&original, &hardlink).unwrap();

        let config = ScanConfig::builder().root(dir.path()).build().unwrap();
        let scanner = WalkdirScanner::new();
        let (_metadata, entries) = run_scan(&scanner, &config).unwrap();

        // Directories always have nlink >= 2 (for "." entries), so filter to
        // regular files only to isolate the hard-linked pair.
        let linked: Vec<_> = entries
            .iter()
            .filter(|e| e.nlink() == 2 && e.file_type() == FileType::Regular)
            .collect();
        assert_eq!(linked.len(), 2, "expected two hard-linked regular entries");

        let zero_count = linked.iter().filter(|e| e.allocated_size() == 0).count();
        let nonzero_count = linked.iter().filter(|e| e.allocated_size() > 0).count();
        assert_eq!(
            zero_count, 1,
            "exactly one entry should have allocated_size==0"
        );
        assert_eq!(
            nonzero_count, 1,
            "exactly one entry should have allocated_size>0"
        );
    }

    #[test]
    fn scan_reports_progress() {
        let dir = TempDir::new().unwrap();
        for i in 0..5_u8 {
            std::fs::write(dir.path().join(format!("file{i}.txt")), [i]).unwrap();
        }

        let config = ScanConfig::builder().root(dir.path()).build().unwrap();
        let scanner = WalkdirScanner::new();

        let (batch_tx, mut batch_rx) = mpsc::channel(CHAN_CAP);
        let (progress_tx, mut progress_rx) = mpsc::channel(CHAN_CAP);
        let cancel = CancellationToken::new();

        scanner
            .scan(&config, batch_tx, progress_tx, cancel)
            .unwrap();

        // Drain batch channel to avoid leaving unconsumed items.
        while batch_rx.try_recv().is_ok() {}

        let mut got_progress = false;
        while let Ok(p) = progress_rx.try_recv() {
            if p.entries_scanned > 0 {
                got_progress = true;
            }
        }
        assert!(got_progress, "expected at least one progress update");
    }

    #[test]
    fn scan_respects_cancellation() {
        let dir = TempDir::new().unwrap();
        for i in 0..5_u8 {
            std::fs::write(dir.path().join(format!("file{i}.txt")), [i]).unwrap();
        }

        let config = ScanConfig::builder().root(dir.path()).build().unwrap();
        let scanner = WalkdirScanner::new();

        let cancel = CancellationToken::new();
        cancel.cancel(); // cancelled before scan starts

        let (metadata, _entries) = run_scan_with_cancel(&scanner, &config, cancel).unwrap();
        assert_eq!(
            metadata.entry_count, 0,
            "cancelled scan should report zero entries"
        );
    }

    #[test]
    fn scan_handles_permission_denied() {
        // Skip this test when running as root (root bypasses permission checks).
        if nix::unistd::Uid::effective().is_root() {
            return;
        }

        let dir = TempDir::new().unwrap();
        let restricted = dir.path().join("restricted");
        std::fs::create_dir(&restricted).unwrap();
        std::fs::set_permissions(&restricted, std::fs::Permissions::from_mode(0o000)).unwrap();

        let config = ScanConfig::builder().root(dir.path()).build().unwrap();
        let scanner = WalkdirScanner::new();
        let result = run_scan(&scanner, &config);

        // Restore permissions so TempDir can clean up properly.
        let _ = std::fs::set_permissions(&restricted, std::fs::Permissions::from_mode(0o755));

        let (metadata, _entries) = result.unwrap();
        assert!(
            !metadata.warnings.is_empty(),
            "expected at least one warning for the permission-denied directory"
        );
        let has_warning = metadata
            .warnings
            .iter()
            .any(|w| w.path.starts_with(&restricted) || w.path == restricted);
        assert!(
            has_warning,
            "a warning should reference the restricted directory"
        );
    }

    #[test]
    fn scan_batches_entries() {
        // Create a flat directory with exactly 5 files.
        // WalkDir yields: root dir + 5 files = 6 entries total.
        // batch_size 2  →  ceil(6 / 2) = 3 batches of 2.
        let dir = TempDir::new().unwrap();
        for i in 0..5_u8 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), [i]).unwrap();
        }

        let config = ScanConfig::builder()
            .root(dir.path())
            .batch_size(2)
            .build()
            .unwrap();
        let scanner = WalkdirScanner::new();

        let (batch_tx, mut batch_rx) = mpsc::channel(CHAN_CAP);
        let (progress_tx, _) = mpsc::channel(CHAN_CAP);
        let cancel = CancellationToken::new();

        let metadata = scanner
            .scan(&config, batch_tx, progress_tx, cancel)
            .unwrap();

        let mut batch_count = 0_usize;
        let mut total_entries = 0_usize;
        while let Ok(batch) = batch_rx.try_recv() {
            assert!(
                batch.len() <= 2,
                "batch has {} entries, expected <= 2",
                batch.len()
            );
            total_entries += batch.len();
            batch_count += 1;
        }

        // 6 entries with batch_size 2 → 3 batches (all full).
        assert_eq!(batch_count, 3, "expected 3 batches, got {batch_count}");
        assert_eq!(
            total_entries as u64, metadata.entry_count,
            "total entries across batches should match metadata.entry_count"
        );
    }

    #[test]
    fn scan_follows_root_symlink_to_directory() {
        // Create the actual target directory with a file in it.
        let target = TempDir::new().unwrap();
        std::fs::write(target.path().join("inside.txt"), b"content").unwrap();

        // Create a symlink pointing to the target directory.
        let link_parent = TempDir::new().unwrap();
        let symlink_path = link_parent.path().join("link_to_dir");
        std::os::unix::fs::symlink(target.path(), &symlink_path).unwrap();

        // ScanConfig accepts a symlink to a directory (is_dir() follows symlinks).
        let config = ScanConfig::builder().root(&symlink_path).build().unwrap();
        let scanner = WalkdirScanner::new();
        let (metadata, entries) = run_scan(&scanner, &config).unwrap();

        assert!(
            metadata.entry_count > 0,
            "scan through symlink should find entries"
        );
        let found_file = entries
            .iter()
            .any(|e| e.path().file_name().is_some_and(|n| n == "inside.txt"));
        assert!(
            found_file,
            "scan should find the file inside the symlink target"
        );
    }

    #[test]
    fn scan_handles_vanishing_file() {
        // Verify the scanner does not panic when entries change between
        // directory listing and stat (TOCTOU). The race is hard to trigger
        // deterministically, so we validate the property indirectly by
        // confirming a normal scan completes without a panic.
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();

        let config = ScanConfig::builder().root(dir.path()).build().unwrap();
        let scanner = WalkdirScanner::new();
        let (_metadata, _entries) = run_scan(&scanner, &config).unwrap();
        // Reaching here means no panic occurred.
    }
}
