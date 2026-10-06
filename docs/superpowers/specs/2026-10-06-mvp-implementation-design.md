# NixDirStat MVP Implementation Design

## Overview

Full MVP implementation of NixDirStat: scan a directory tree, persist results to SQLite, and explore disk usage through an interactive TUI with treemap, sortable file table, file-type bar chart, and scan progress reporting.

This design translates the decisions already validated in `docs/specification.md` into concrete module boundaries, types, traits, data flow, and wiring. Every technical choice below traces back to a validated research finding or prototype.

## Module Structure

```
src/
  main.rs              # thin entry point — calls lib::run()
  lib.rs               # module declarations, run() dispatch
  cli.rs               # clap parsing (exists, minor updates)
  error.rs             # top-level error re-exports
  types.rs             # FileEntry, ScanMetadata, FileType, FileCategory, ScanConfig
  scanner/
    mod.rs             # Scanner trait, ScanProgress, EntryBatch
    walkdir.rs         # walkdir-based implementation
  storage/
    mod.rs             # Storage trait
    sqlite.rs          # rusqlite implementation, schema, migrations
  analyzer/
    mod.rs             # directory size aggregation, file-type stats, free space
  pipeline.rs          # async coordination (scanner → storage, progress)
  ui/
    mod.rs             # TUI app shell, terminal setup/teardown
    app.rs             # AppState state machine
    views/
      mod.rs
      progress.rs      # scan progress view
      explorer.rs      # main explore view (treemap + table + bar chart)
    widgets/
      mod.rs
      treemap.rs       # streemap-based StatefulWidget
      file_table.rs    # sortable Table widget
      type_chart.rs    # BarChart by file category
  platform/
    mod.rs             # Tier 2/3 dispatch (filesystem type detection)
    linux.rs
    macos.rs
    freebsd.rs
```

## Core Data Types

### FileEntry

Twelve metadata fields from stat, constructed from `std::fs::Metadata` via `MetadataExt`:

| Field | Type | Source |
|-------|------|--------|
| `path` | `PathBuf` | walkdir `DirEntry::path()` |
| `parent` | `PathBuf` | `path.parent()` |
| `size` | `u64` | `metadata.size()` (logical) |
| `allocated_size` | `u64` | `metadata.blocks() * 512` (physical) |
| `file_type` | `FileType` | `metadata.mode() & libc::S_IFMT` |
| `mode` | `u32` | `metadata.mode() as libc::mode_t as u32` |
| `uid` | `u32` | `metadata.uid()` |
| `gid` | `u32` | `metadata.gid()` |
| `mtime` | `i64` | `metadata.mtime()` |
| `inode` | `u64` | `metadata.ino()` |
| `device` | `u64` | `metadata.dev()` |
| `nlink` | `u64` | `metadata.nlink()` |

### FileType

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FileType {
    Regular,
    Directory,
    Symlink,
    Device,
    Socket,
    Pipe,
}
```

Derived from mode bits via `mode & libc::S_IFMT`. Structural classification only.

### FileCategory

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FileCategory {
    Code,
    Image,
    Document,
    Archive,
    Audio,
    Video,
    Binary,
    NoExtension,
    Other,
}
```

Extension-based classification resolved application-side via `Path::extension()`. Multi-dot extensions use the last component (`file.tar.gz` → `"gz"` → `Archive`). Extensionless files classified as `NoExtension`.

### ScanConfig

Builder pattern with validation at construction:

```rust
pub struct ScanConfig {
    root: PathBuf,           // required, validated to exist and be a directory
    cross_device: bool,      // default: false
    batch_size: usize,       // default: 10_000
}
```

### ScanMetadata

```rust
pub struct ScanMetadata {
    pub root_path: PathBuf,
    pub started_at: i64,          // unix timestamp
    pub duration_ms: u64,
    pub file_count: u64,
    pub total_size: u64,          // sum of allocated sizes
    pub warnings: Vec<ScanWarning>,
}
```

### EntryBatch

Newtype enforcing the batch invariant (non-empty):

```rust
pub struct EntryBatch(Vec<FileEntry>);
```

### Error Types

Each component defines its own `#[non_exhaustive]` error enum via `thiserror`:

- `ScanError` — `RootNotFound(PathBuf)`, `PermissionDenied(PathBuf)`, `Io(std::io::Error)`, `Cancelled`
- `StorageError` — `Sqlite(rusqlite::Error)`, `SchemaVersionMismatch { expected, found }`, `InvalidData(String)`
- `PipelineError` — `Scan(ScanError)`, `Storage(StorageError)`, `ChannelClosed`
- `UiError` — `Io(std::io::Error)`, `Storage(StorageError)`

## Scanner

### Trait

```rust
pub trait Scanner {
    fn scan(
        &self,
        config: &ScanConfig,
        batch_tx: mpsc::Sender<EntryBatch>,
        progress_tx: mpsc::Sender<ScanProgress>,
        cancel: CancellationToken,
    ) -> Result<ScanMetadata, ScanError>;
}
```

The scanner is blocking — called inside `tokio::spawn_blocking`. It owns the sending halves and drops them on completion to signal downstream.

### WalkdirScanner

- `WalkDir::new(root).follow_links(false)` — never follow symlinks.
- Cross-device detection: capture root's `dev()` upfront. Skip entries where `dev() != root_dev` unless `config.cross_device` is true.
- Hardlink dedup: `HashSet<(u64, u64)>` of `(ino, dev)`. For entries with `nlink > 1`, check the set. Only the first encounter contributes `allocated_size`. The set is local to the scan, not persisted. Measured at 68MB for 2M entries — within budget.
- Error handling: `WalkDir` yields `Result<DirEntry, walkdir::Error>`. Per-entry errors (permission denied, vanished files) are accumulated as `ScanWarning` in `ScanMetadata` and skipped. Root not found or unreadable is fatal.
- Batching: accumulate into `Vec<FileEntry>`, send as `EntryBatch` when batch reaches `config.batch_size`. Flush remainder on completion.
- Progress: `try_send` on the progress channel (lossy, non-blocking). `ScanProgress` carries file_count, current_path, elapsed time, files_per_sec.
- Cancellation: check `cancel.is_cancelled()` per walkdir entry. On cancel, flush current batch and return partial metadata with `ScanError::Cancelled` or partial `ScanMetadata`.

### ScanProgress

```rust
pub struct ScanProgress {
    pub file_count: u64,
    pub current_path: PathBuf,
    pub elapsed: Duration,
    pub files_per_sec: f64,
}
```

## Storage

### Trait

```rust
pub trait Storage: Send + Sync {
    fn init_schema(&mut self) -> Result<(), StorageError>;
    fn insert_batch(&self, batch: &EntryBatch) -> Result<(), StorageError>;
    fn save_scan_metadata(&self, metadata: &ScanMetadata) -> Result<(), StorageError>;
    fn load_scan_metadata(&self) -> Result<ScanMetadata, StorageError>;
    fn query_entries(&self, query: &EntryQuery) -> Result<Vec<FileEntry>, StorageError>;
    fn query_directory_children(&self, path: &Path) -> Result<Vec<FileEntry>, StorageError>;
    fn query_top_n_by_size(&self, n: usize) -> Result<Vec<FileEntry>, StorageError>;
    fn query_type_stats(&self) -> Result<Vec<TypeStat>, StorageError>;
    fn update_directory_sizes(&self, sizes: &HashMap<PathBuf, DirectoryStats>) -> Result<(), StorageError>;
    fn finalize_for_export(&self) -> Result<(), StorageError>;
}
```

### SQLite Schema

```sql
CREATE TABLE entries (
    id           INTEGER PRIMARY KEY,
    path_bytes   BLOB NOT NULL,
    path_text    TEXT NOT NULL,
    parent_bytes BLOB NOT NULL,
    parent_text  TEXT NOT NULL,
    size         INTEGER NOT NULL,
    allocated    INTEGER NOT NULL,
    file_type    INTEGER NOT NULL,
    mode         INTEGER NOT NULL,
    uid          INTEGER NOT NULL,
    gid          INTEGER NOT NULL,
    mtime        INTEGER NOT NULL,
    inode        INTEGER NOT NULL,
    device       INTEGER NOT NULL,
    nlink        INTEGER NOT NULL
);

CREATE TABLE scan_metadata (
    id             INTEGER PRIMARY KEY CHECK (id = 1),
    root_path      TEXT NOT NULL,
    started_at     INTEGER NOT NULL,
    duration_ms    INTEGER NOT NULL,
    file_count     INTEGER NOT NULL,
    total_size     INTEGER NOT NULL,
    schema_version INTEGER NOT NULL
);

CREATE INDEX idx_entries_parent ON entries(parent_text);
CREATE INDEX idx_entries_size ON entries(size DESC);
CREATE INDEX idx_entries_type ON entries(file_type);
```

Schema version: `PRAGMA user_version = 1`. Check on open, reject incompatible versions, support forward migration via `ALTER TABLE`.

Hybrid path storage: `path_bytes BLOB` (canonical, lossless via `OsStr::as_encoded_bytes()`) + `path_text TEXT` (queryable, lossy via `to_string_lossy()`). Same for `parent_bytes`/`parent_text`. Lossy paths detectable via `INSTR(path_text, U+FFFD) > 0`.

### SqliteStorage

- PRAGMAs on open: `synchronous=NORMAL`, `temp_store=MEMORY`, `mmap_size=268435456`, `busy_timeout=5000`.
- Journal mode set adaptively:
  - Batch mode (`scan --output`): DELETE (no WAL overhead).
  - Interactive mode (`scan` → TUI): WAL, except on ZFS (falls back to DELETE due to 2.15x overhead).
  - Filesystem type detected via `platform::detect_filesystem_type()`.
- Batch inserts: `BEGIN IMMEDIATE` transaction per `EntryBatch`, prepared `INSERT` statement reused.
- Finalize for export: `PRAGMA wal_checkpoint(TRUNCATE)` then `PRAGMA journal_mode = DELETE`.
- The storage writer runs in `spawn_blocking`, owning a dedicated `Connection`.

### EntryQuery

```rust
pub struct EntryQuery {
    pub path_prefix: Option<PathBuf>,
    pub size_range: Option<(u64, u64)>,
    pub file_type: Option<FileType>,
    pub sort_by: SortField,
    pub sort_direction: SortDirection,
    pub limit: Option<usize>,
}
```

## Analyzer

Free functions operating on `&dyn Storage`. Not a trait — single implementation, not an extension point.

### Directory Size Aggregation

Single O(n) pass after scan completes:

1. Query all entries from storage.
2. Build `HashMap<PathBuf, DirectoryStats>` where `DirectoryStats = { total_size: u64, allocated_size: u64, file_count: u64 }`.
3. For each file entry, walk the parent chain upward, accumulating sizes into each ancestor directory.
4. Persist aggregated sizes back to storage via `update_directory_sizes()` — updates existing directory entries' `size` and `allocated` fields in-place.

### File-Type Statistics

- `TypeStat { category: FileCategory, extension: String, count: u64, total_size: u64 }`.
- Extension extracted application-side via `Path::extension()` (not SQL — handles multi-dot correctly).
- Grouped by `FileCategory`, then by extension within each category.

### Free/Unknown Space

- Free: `nix::sys::statvfs::statvfs(root).f_bavail * f_bsize` (Tier 1, portable).
- Total: `f_blocks * f_bsize`.
- Unknown: `total - sum(scanned allocated sizes) - free`.
- Computed on demand, not persisted.

## Async Pipeline

### Topology

```
                                      ┌─────────────────┐
                                      │  TUI event loop  │
                                      │  (async task)    │
                                      └───────▲──────────┘
                                              │ ScanProgress
                                              │ (lossy try_send)
┌──────────────────┐   EntryBatch    ┌────────┴──────────┐
│  Scanner         │───────────────► │  Storage Writer    │
│  (spawn_blocking)│  mpsc(bounded)  │  (spawn_blocking)  │
└──────────────────┘   capacity: 4   └───────────────────┘
         │                                    │
         └──────── CancellationToken ─────────┘
                          │
                    oneshot::Sender ──────────► TUI (scan complete signal)
```

### Three Concurrent Tasks

1. **Scanner** (`spawn_blocking`): runs `WalkdirScanner::scan()`, sends `EntryBatch` on bounded channel (capacity 4 — backpressure when storage is slow), sends `ScanProgress` on separate channel via `try_send`.

2. **Storage writer** (`spawn_blocking`): owns `SqliteStorage` with dedicated `Connection`. Receives `EntryBatch`, calls `insert_batch()`. On channel close: commits, runs analyzer aggregation, sends `PipelineResult` on a `oneshot` channel.

3. **TUI event loop** (async): `tokio::select!` over crossterm events, progress channel, completion oneshot, tick interval, and cancellation. Renders progress during scan, transitions to explorer on completion.

### Batch Mode

Same scanner + storage writer, no TUI. Progress printed to stderr: `\r`-overwritten line with file count, rate, current path. Storage writer finalizes database and exits.

### PipelineConfig

```rust
pub struct PipelineConfig {
    pub scan: ScanConfig,
    pub batch_size: usize,         // default: 10_000
    pub channel_capacity: usize,   // default: 4
    pub journal_mode: JournalMode, // from platform detection
    pub storage_path: PathBuf,     // output file or tempfile
}
```

### PipelineResult

```rust
pub struct PipelineResult {
    pub metadata: ScanMetadata,
    pub storage_path: PathBuf,
    pub timing: PipelineTiming,
}

pub struct PipelineTiming {
    pub scan_duration: Duration,
    pub storage_duration: Duration,
    pub aggregation_duration: Duration,
    pub total_duration: Duration,
}
```

### Shutdown

1. User presses `q`/`Ctrl+C` → cancel token triggered.
2. Scanner checks `is_cancelled()` per entry, flushes current batch, returns.
3. Storage writer drains remaining batches, commits, aggregates partial data.
4. TUI receives completion, transitions to explorer (partial results) or exits.

## TUI

### App State Machine

```rust
pub enum AppState {
    Scanning(ScanProgressState),
    Exploring(ExplorerState),
}
```

Two states only. `Scanning` → `Exploring` on pipeline completion. `explore` subcommand starts in `Exploring`.

### Progress View

- `LineGauge` for files scanned (no percentage — total unknown).
- Text: file count, files/sec, elapsed time, current path.
- `q`/`Ctrl+C` triggers cancellation.
- Refreshes on `ScanProgress` receipt + 100ms tick for elapsed time.

### Explorer View Layout

```
┌────────────────────────────────────────────┐
│  Treemap (top half)                        │
│                                            │
├──────────────────────┬─────────────────────┤
│  File Table          │  File-Type BarChart  │
│  (bottom-left)       │  (bottom-right)      │
│                      │                      │
└──────────────────────┴─────────────────────┘
```

### Treemap Widget

`StatefulWidget` using `streemap::squarify`:

- Coordinate bridge: `streemap::Rect<f32>` → `ratatui::layout::Rect` (u16). `floor()` for x/y, `ceil()` for right/bottom, clamped to container. Produces zero rendering gaps.
- Shows entries at current navigation level, colored by `FileCategory`.
- Labels truncated with ellipsis when cell width < label length, hidden when < 3 columns.
- Arrow keys to select, Enter to drill into directories, Backspace to go up.

### File Table Widget

Sortable `Table`:

- Columns: name, size (human-readable KiB/MiB/GiB), type, modified, owner (uid).
- Tab/Shift+Tab to cycle sort column, `r` to reverse direction.
- Shows entries in the currently selected directory.
- Up/Down arrow keys to select rows, synced with treemap selection.

### File-Type Bar Chart

`BarChart` showing space by `FileCategory`:

- Horizontal bars, labeled with category name and total size.
- Updates when navigation changes (reflects current directory scope).

### Navigation Model

- `ExplorerState` holds `current_path: PathBuf` and `breadcrumb: Vec<PathBuf>`.
- Selecting a directory updates `current_path`, re-queries storage, recomputes treemap and bar chart.
- All data access goes through `Storage` trait — TUI never touches the filesystem.

### Event Loop

```rust
loop {
    terminal.draw(|f| render(&app_state, f))?;
    tokio::select! {
        event = crossterm_events.next() => { /* key/mouse/resize */ }
        progress = progress_rx.recv() => { /* update ScanProgressState */ }
        result = completion_rx => { /* transition to Exploring */ }
        _ = tick.tick() => { /* force redraw */ }
        _ = cancel.cancelled() => { break; }
    }
}
```

### Root Indicator

If `nix::unistd::geteuid().is_root()`, render a "ROOT" badge in the top-right corner (red background, white text).

### Terminal Setup/Teardown

Guard pattern: `enable_raw_mode()` + `EnterAlternateScreen` on entry, restored on exit. Panic hook installed to restore terminal before unwinding.

## CLI Wiring

### `scan` Subcommand

1. Validate root path exists and is a directory.
2. Detect filesystem type via `platform::detect_filesystem_type()` for journal mode.
3. Build `PipelineConfig`.
4. If `--output` provided: batch mode — progress on stderr, finalize database, exit.
5. If no `--output`: interactive mode — tempfile for storage, TUI with progress → explorer. Temp file cleaned up on exit.

### `explore` Subcommand

1. Open SQLite file, check `PRAGMA user_version` for schema compatibility.
2. Load `ScanMetadata`.
3. Launch TUI directly in `Exploring` state.

### `export` Subcommand

1. Open SQLite file, check schema version.
2. Query all entries.
3. Serialize to stdout:
   - CSV: header row + one row per entry (manual formatting).
   - JSON: array of entry objects via `serde_json`.
4. No TUI, no async — synchronous read and write.

## Platform Module

Tier 2/3 code behind `#[cfg(target_os)]` with catch-all fallbacks.

### Filesystem Type Detection

Used by `SqliteStorage` for adaptive journal mode selection:

- Linux: `libc::statfs` → `f_type` magic number lookup.
- macOS/FreeBSD: `libc::statfs` → `f_fstypename` string.
- Fallback: returns `"unknown"`.

### JournalMode Selection

```rust
pub fn recommended_journal_mode(fs_type: &str, interactive: bool) -> JournalMode {
    if !interactive {
        return JournalMode::Delete;  // batch mode always DELETE
    }
    match fs_type {
        "zfs" => JournalMode::Delete,  // 2.15x WAL overhead
        _ => JournalMode::Wal,
    }
}
```

## Key Interactions

### `nixdirstat scan /data`

1. CLI parses → `Command::Scan { path: "/data", output: None, cross_device: false }`.
2. `run()` validates path, detects filesystem, builds `PipelineConfig` with tempfile storage path and WAL mode.
3. Pipeline spawns scanner, storage writer, TUI event loop.
4. Scanner walks `/data`, sends batches to storage writer, progress to TUI.
5. TUI shows progress view: "42,391 files | 11,827 files/sec | 3.6s | /data/var/log/...".
6. Scanner completes → storage writer drains, aggregates, sends `PipelineResult`.
7. TUI transitions to explorer view with treemap, file table, bar chart.
8. User navigates with arrow keys, drills into directories, sorts table.
9. User presses `q` → TUI exits, tempfile cleaned up.

### `nixdirstat scan /data --output scan.db`

1. CLI parses → `Command::Scan { path: "/data", output: Some("scan.db"), cross_device: false }`.
2. `run()` builds `PipelineConfig` with `scan.db` path and DELETE journal mode.
3. Pipeline spawns scanner + storage writer (no TUI).
4. Progress printed to stderr.
5. Storage writer finalizes database (checkpoint, switch to DELETE).
6. Process exits.

### `nixdirstat explore scan.db`

1. CLI parses → `Command::Explore { scan_file: "scan.db" }`.
2. `run()` opens SQLite file, checks schema version, loads metadata.
3. TUI launches directly in explorer view.

## Testing Strategy

- **Unit tests** in each module: `FileEntry` construction from metadata, `FileType` classification from mode bits, `FileCategory` from extensions, `ScanConfig` builder validation, `EntryBatch` invariant, `EntryQuery` construction, human-readable size formatting.
- **Integration tests**: end-to-end scan of `tempfile::TempDir` with known structures (files, symlinks, hardlinks, nested dirs, permission-denied entries). Assert scan metadata, stored entry counts, aggregated directory sizes, type stats.
- **Property tests** (`proptest`): size aggregation (sum of children = parent), path parent chain traversal, extension classification consistency.
- **Platform-conditional tests**: sparse file detection (skip on macOS), non-UTF-8 path roundtrip (skip on macOS/APFS), cross-device detection (require root, use loopback mount on Linux).
- **TUI tests**: headless buffer rendering tests for each widget (treemap, file table, bar chart, progress view). No terminal required.
