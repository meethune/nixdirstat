//! NixDirStat — disk usage analyzer and cleanup assistant for POSIX-compliant systems.
//!
//! Scan local filesystems, devices, and directories then explore disk usage
//! through sortable file lists, file-type statistics, and interactive treemaps.

mod cli;

pub mod analyzer;
pub mod error;
pub mod pipeline;
pub mod platform;
pub mod scanner;
pub mod storage;
pub mod types;
pub mod ui;

use std::io::Write;

use cli::{Command, ExportFormat};
use tokio_util::sync::CancellationToken;

use crate::storage::{Storage, sqlite::SqliteStorage};

pub use error::{PipelineError, ScanError, StorageError, UiError};
pub use pipeline::{PipelineConfig, PipelineResult, PipelineTiming, run_pipeline};
pub use scanner::{Scanner, WalkdirScanner};
pub use types::{
    DirectoryStats, EntryBatch, EntryQuery, FileCategory, FileEntry, FileType, JournalMode,
    ScanConfig, ScanConfigBuilder, ScanMetadata, ScanProgress, ScanWarning, SortDirection,
    SortField, SpaceInfo, TypeStat, format_size,
};

/// Write file entries in CSV format to the given writer.
///
/// The first line is a header row. Subsequent lines contain one entry per row
/// with fields separated by commas. The `path` field is always double-quoted
/// (with internal double-quotes escaped as `""`). All other fields are numeric
/// values that need no quoting.
///
/// The `mtime` field is written as Unix seconds since the epoch.
///
/// # Errors
///
/// Returns an error if writing to `out` fails.
pub fn write_entries_csv(entries: &[FileEntry], out: &mut impl Write) -> anyhow::Result<()> {
    writeln!(
        out,
        "path,size,allocated,type,mode,uid,gid,mtime,inode,device,nlink"
    )?;
    for entry in entries {
        let mtime_secs = entry
            .mtime
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map_or(0_u64, |d| d.as_secs());
        // Always quote the path to handle commas and special characters.
        let path_str = entry.path.to_string_lossy();
        let quoted_path = format!("\"{}\"", path_str.replace('"', "\"\""));
        writeln!(
            out,
            "{},{},{},{},{},{},{},{},{},{},{}",
            quoted_path,
            entry.size,
            entry.allocated_size,
            entry.file_type,
            entry.mode,
            entry.uid,
            entry.gid,
            mtime_secs,
            entry.inode,
            entry.device,
            entry.nlink,
        )?;
    }
    Ok(())
}

/// Write file entries in JSON format to the given writer.
///
/// The output is a pretty-printed JSON array of entry objects followed by a
/// trailing newline. Field names and values match the public fields of
/// [`FileEntry`].
///
/// # Errors
///
/// Returns an error if serialization or writing to `out` fails.
pub fn write_entries_json(entries: &[FileEntry], out: &mut impl Write) -> anyhow::Result<()> {
    serde_json::to_writer_pretty(&mut *out, entries)?;
    writeln!(out)?;
    Ok(())
}

/// Run a batch (non-interactive) scan, writing progress to stderr and the
/// final database to `output_path`.
///
/// Progress is printed as a single overwriting line using `\r`. A summary line
/// is printed when the scan completes.
///
/// # Errors
///
/// Returns an error if the pipeline fails to start, if the scanner errors, or
/// if the storage writer fails.
async fn run_scan_batch(
    config: ScanConfig,
    output_path: std::path::PathBuf,
    journal_mode: JournalMode,
) -> anyhow::Result<()> {
    let pipeline_config = PipelineConfig {
        scan: config,
        channel_capacity: 4,
        journal_mode,
        storage_path: output_path,
    };
    let cancel = CancellationToken::new();
    let (progress_rx, completion_rx) = run_pipeline(pipeline_config, cancel).await?;

    let progress_task = tokio::spawn(drain_progress(progress_rx));
    let result = completion_rx.await??;
    progress_task.abort();

    eprintln!(
        "\rScan complete: {} files, {} total",
        result.metadata.entry_count,
        format_size(result.metadata.total_size)
    );
    Ok(())
}

/// Drain the progress channel, printing each update to stderr as a single
/// overwriting line (using `\r`).
async fn drain_progress(mut progress_rx: tokio::sync::mpsc::Receiver<ScanProgress>) {
    while let Some(p) = progress_rx.recv().await {
        eprint!(
            "\r{} files | {:.0} files/sec | {}",
            p.entries_scanned,
            p.entries_per_second,
            p.current_path.display()
        );
    }
}

/// Application entry point, called from `main.rs`.
///
/// Parses CLI arguments, dispatches to the appropriate subcommand,
/// and returns any errors for display by the caller.
///
/// # Errors
///
/// Returns an error if CLI parsing, scanning, storage, UI initialisation,
/// or export fails.
pub async fn run() -> anyhow::Result<()> {
    let cli = cli::parse();
    match cli.command {
        Command::Scan {
            path,
            output,
            cross_device,
        } => {
            let config = ScanConfig::builder()
                .root(path)
                .cross_device(cross_device)
                .build()?;

            let fs_type = platform::detect_filesystem_type(config.root())
                .unwrap_or_else(|_| "unknown".into());
            let interactive = output.is_none();
            let journal_mode = platform::recommended_journal_mode(&fs_type, interactive);

            if let Some(output_path) = output {
                // Batch mode: write to user-specified output file.
                run_scan_batch(config, output_path, journal_mode).await?;
            } else {
                // Interactive: use a temp file; keep it alive for the TUI session.
                let tmp = tempfile::NamedTempFile::new()
                    .map_err(|e| anyhow::anyhow!("failed to create temp file: {e}"))?;
                let storage_path = tmp.path().to_path_buf();
                let pipeline_config = PipelineConfig {
                    scan: config,
                    channel_capacity: 4,
                    journal_mode,
                    storage_path,
                };
                ui::run_scan_ui(pipeline_config).await?;
                drop(tmp); // delete temp file after TUI exits
            }
        },

        Command::Explore { scan_file } => {
            anyhow::ensure!(
                scan_file.exists(),
                "scan file not found: {}",
                scan_file.display()
            );
            ui::run_explore_ui(&scan_file).await?;
        },

        Command::Export { scan_file, format } => {
            let storage = SqliteStorage::open_readonly(&scan_file)?;
            let entries = storage.query_entries(&EntryQuery::default())?;
            let stdout = std::io::stdout();
            let mut out = stdout.lock();
            match format {
                ExportFormat::Csv => write_entries_csv(&entries, &mut out)?,
                ExportFormat::Json => write_entries_json(&entries, &mut out)?,
            }
        },
    }
    Ok(())
}
