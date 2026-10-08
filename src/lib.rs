//! NixDirStat — disk usage analyzer and cleanup assistant for POSIX-compliant systems.
//!
//! Scan local filesystems, devices, and directories then explore disk usage
//! through sortable file lists, file-type statistics, and interactive treemaps.

rust_i18n::i18n!("locales", fallback = "en");

mod cli;

pub mod analyzer;
pub mod error;
pub mod pipeline;
pub mod platform;
pub mod scanner;
pub mod storage;
pub mod sync;
pub mod types;
pub mod ui;

use std::io::Write;

use cli::{Command, ExportFormat};
use tokio_util::sync::CancellationToken;

use crate::storage::{ReadStorage as _, sqlite::SqliteStorage};

pub use error::{PipelineError, ScanError, StorageError, UiError};

/// Look up a translated string by key (for use in integration tests).
pub fn translate(key: &str) -> String {
    rust_i18n::t!(key).to_string()
}

/// Initialise the global locale for translated strings.
///
/// Precedence: `cli_override` > `sys_locale::get_locale()` > `"en"`.
/// Called once at startup, before any UI or output.
pub fn init_locale(cli_override: Option<&str>) {
    let locale = cli_override
        .map(String::from)
        .or_else(sys_locale::get_locale)
        .map_or_else(
            || "en".to_string(),
            |s| {
                let stripped = s.split('.').next().unwrap_or(&s);
                if stripped.is_empty() || stripped == "C" || stripped == "POSIX" {
                    "en".to_string()
                } else {
                    stripped.replace('_', "-")
                }
            },
        );

    rust_i18n::set_locale(&locale);
}
pub use pipeline::{PipelineConfig, PipelineResult, PipelineTiming, run_pipeline};
pub use scanner::{Scanner, WalkdirScanner};
pub use sync::PauseToken;
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
            .mtime()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map_or(0_u64, |d| d.as_secs());
        let path_str = entry.path().to_string_lossy();
        let escaped = path_str.replace('"', "\"\"");
        let quoted_path = if escaped.starts_with(['=', '+', '-', '@']) {
            format!("\"\t{escaped}\"")
        } else {
            format!("\"{escaped}\"")
        };
        writeln!(
            out,
            "{},{},{},{},{},{},{},{},{},{},{}",
            quoted_path,
            entry.size(),
            entry.allocated_size(),
            entry.file_type(),
            entry.mode(),
            entry.uid(),
            entry.gid(),
            mtime_secs,
            entry.inode(),
            entry.device(),
            entry.nlink(),
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
    let pause = sync::PauseToken::new();
    let (progress_rx, completion_rx) = run_pipeline(pipeline_config, cancel, pause).await?;

    let progress_task = tokio::spawn(drain_progress(progress_rx));
    let result = completion_rx.await.map_err(|_| {
        anyhow::anyhow!("scan pipeline terminated unexpectedly without producing a result")
    })??;
    progress_task.abort();

    eprintln!(
        "\r{}",
        rust_i18n::t!(
            "batch.scan-complete",
            count = result.metadata.entry_count,
            size = format_size(result.metadata.total_size)
        )
    );
    Ok(())
}

/// Drain the progress channel, printing each update to stderr as a single
/// overwriting line (using `\r`).
async fn drain_progress(mut progress_rx: tokio::sync::mpsc::Receiver<ScanProgress>) {
    while let Some(p) = progress_rx.recv().await {
        eprint!(
            "\r{}",
            rust_i18n::t!(
                "batch.progress",
                count = p.entries_scanned,
                rate = format!("{:.0}", p.entries_per_second),
                path = p.current_path.display()
            )
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
    let (command, lang) = cli::parse();
    init_locale(lang.as_deref());
    match command {
        Command::Scan {
            path,
            output,
            cross_device,
        } => {
            let fs_type = platform::detect_filesystem_type(&path).unwrap_or_else(|e| {
                eprintln!(
                    "{}",
                    rust_i18n::t!("batch.warning-fs-type", path = path.display(), error = e)
                );
                "unknown".into()
            });
            let interactive = output.is_none();
            let journal_mode = platform::recommended_journal_mode(&fs_type, interactive);

            let config = ScanConfig::builder()
                .root(path)
                .cross_device(cross_device)
                .filesystem_type(fs_type)
                .build()?;

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
                "{}",
                rust_i18n::t!("batch.scan-file-not-found", path = scan_file.display())
            );
            ui::run_explore_ui(&scan_file).await?;
        },

        Command::Completions { shell } => {
            cli::write_completions(shell, &mut std::io::stdout());
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
