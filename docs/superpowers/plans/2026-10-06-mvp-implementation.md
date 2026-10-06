# NixDirStat MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the full NixDirStat MVP — scan a directory tree, persist to SQLite, and explore via an interactive TUI with treemap, sortable file table, file-type bar chart, and progress reporting.

**Architecture:** Async pipeline with three concurrent tasks: a blocking walkdir scanner sends `EntryBatch` over a bounded `mpsc` channel to a blocking storage writer, while a TUI event loop renders progress. After scan, bottom-up aggregation computes directory sizes, then the TUI transitions to an interactive explorer view. All subcommands (`scan`, `explore`, `export`) are wired through `lib::run()`.

**Tech Stack:** Rust (edition 2024, MSRV 1.95), walkdir, rusqlite (bundled), ratatui + crossterm, streemap, tokio, clap, thiserror/anyhow, nix/libc.

**Spec:** `docs/superpowers/specs/2026-10-06-mvp-implementation-design.md`

## Global Constraints

- `unsafe` code forbidden (`unsafe_code = "forbid"` in Cargo.toml).
- `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` must pass after every commit.
- `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented` all denied by clippy config.
- All public items require `///` doc comments (`missing_docs = "warn"`, promoted to error by `-D warnings`).
- `#[non_exhaustive]` on all public enums that may grow.
- Mode bit comparisons must cast through `libc::mode_t` (u16 on FreeBSD/macOS, u32 on Linux).
- Never use `/proc` outside `#[cfg(target_os = "linux")]`.
- Conventional commits. Work on `feat/mvp-implementation` branch; PR to main when complete.

## Review Focus

1. **Scan root is a symlink to a directory** — `walkdir` follows the root symlink by default; scanner should resolve and scan the target, not record the root as a symlink. Add test in Task 4: `scan_follows_root_symlink_to_directory`.
2. **File vanishes between readdir and stat** — walkdir yields `Err` for that entry; scanner must skip and log a `ScanWarning`, not crash. Add test in Task 4: `scan_handles_vanishing_file`.
3. **Database with future schema version** — `explore`/`export` must reject with `StorageError::SchemaVersionMismatch`, not silently corrupt. Add test in Task 3: `open_rejects_future_schema_version`.
4. **Terminal smaller than minimum viable size** — TUI must render without panic on terminals as small as 40x10. Add test in Task 7: `progress_view_renders_at_minimum_size`.
5. **Scan of empty directory** — aggregation must produce zero-size result; root directory should still appear in entries. Add test in Task 5: `aggregate_empty_directory_returns_zero`.

---

## File Structure

| Path | Responsibility | Task |
|------|---------------|------|
| `src/types.rs` | Core domain types: `FileEntry`, `FileType`, `FileCategory`, `ScanConfig`/Builder, `EntryBatch`, `ScanMetadata`, `ScanProgress`, `ScanWarning`, query/sort types, `DirectoryStats`, `SpaceInfo`, `JournalMode` | 1 |
| `src/error.rs` | Error enums: `ScanError`, `StorageError`, `PipelineError`, `UiError` | 1 |
| `src/platform/mod.rs` | `detect_filesystem_type()`, `recommended_journal_mode()` dispatch | 2 |
| `src/platform/linux.rs` | Linux `statfs` → `f_type` magic number lookup | 2 |
| `src/platform/macos.rs` | macOS `statfs` → `f_fstypename` | 2 |
| `src/platform/freebsd.rs` | FreeBSD `statfs` → `f_fstypename` | 2 |
| `src/storage/mod.rs` | `Storage` trait definition | 3 |
| `src/storage/sqlite.rs` | `SqliteStorage`: schema init, PRAGMAs, batch insert, queries, finalize | 3 |
| `src/scanner/mod.rs` | `Scanner` trait definition | 4 |
| `src/scanner/walkdir.rs` | `WalkdirScanner`: walk, hardlink dedup, cross-device, batching, progress | 4 |
| `src/analyzer/mod.rs` | `aggregate_directory_sizes()`, `compute_type_stats()`, `compute_free_space()` | 5 |
| `src/pipeline.rs` | `run_pipeline()`: spawn scanner + storage writer, return channels; `PipelineConfig`, `PipelineResult`, `PipelineTiming` | 6 |
| `src/ui/mod.rs` | `run_scan_ui()`, `run_explore_ui()`, terminal setup/teardown guard | 7, 8 |
| `src/ui/app.rs` | `AppState` enum, `ScanProgressState`, `ExplorerState`, input handling | 7, 8 |
| `src/ui/views/mod.rs` | View module declarations | 7 |
| `src/ui/views/progress.rs` | Progress view rendering (LineGauge, file count, rate, path) | 7 |
| `src/ui/views/explorer.rs` | Explorer view layout (treemap + table + bar chart panels) | 8 |
| `src/ui/widgets/mod.rs` | Widget module declarations | 8 |
| `src/ui/widgets/treemap.rs` | `TreemapWidget` StatefulWidget via `streemap::squarify` | 8 |
| `src/ui/widgets/file_table.rs` | Sortable `FileTableWidget` | 8 |
| `src/ui/widgets/type_chart.rs` | `TypeChartWidget` BarChart by FileCategory | 8 |
| `src/lib.rs` | Module declarations (all tasks), async `run()` dispatch (Task 9) | 1–9 |
| `src/main.rs` | `#[tokio::main]` wrapper (Task 9) | 9 |
| `src/cli.rs` | Existing; no changes needed | — |
| `tests/common/mod.rs` | Shared test helpers: `create_test_entry()`, `create_test_tree()` | 3 |

---

### Task 1: Core Types & Error Hierarchy

**Files:**
- Create: `src/types.rs`, `src/error.rs`
- Modify: `src/lib.rs` (add `pub mod types; pub mod error;` and re-exports)

**Interfaces:**
- Consumes: nothing (foundation task)
- Produces:
  - `FileType::from_mode(mode: u32) -> Self` — cast through `libc::mode_t`, mask with `S_IFMT`
  - `FileCategory::from_extension(ext: Option<&OsStr>) -> Self`
  - `FileEntry::from_metadata(path: PathBuf, metadata: &std::fs::Metadata) -> Self`
  - `ScanConfig::builder() -> ScanConfigBuilder`
  - `ScanConfigBuilder::root(self, path: impl Into<PathBuf>) -> Self`, `.cross_device(self, bool) -> Self`, `.batch_size(self, usize) -> Self`, `.build(self) -> Result<ScanConfig, ScanError>`
  - `ScanConfig::root(&self) -> &Path`, `.cross_device(&self) -> bool`, `.batch_size(&self) -> usize`
  - `EntryBatch::new(entries: Vec<FileEntry>) -> Option<Self>`, `.entries(&self) -> &[FileEntry]`, `.len(&self) -> usize`, `.is_empty(&self) -> bool`
  - `EntryQuery::default()` with `sort_by: SortField::Size`, `sort_direction: SortDirection::Descending`
  - `format_size(bytes: u64) -> String` — human-readable "1.5 GiB" format
  - All error enums: `ScanError`, `StorageError`, `PipelineError`, `UiError`

- [ ] **Step 1: Write `FileType` and `FileCategory` enums in `src/types.rs`**

`FileType`: `Regular`, `Directory`, `Symlink`, `Device`, `Socket`, `Pipe`. Derive `Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize`. `#[non_exhaustive]`.

`FileCategory`: `Code`, `Image`, `Document`, `Archive`, `Audio`, `Video`, `Binary`, `NoExtension`, `Other`. Same derives. `#[non_exhaustive]`.

- [ ] **Step 2: Write error enums in `src/error.rs`**

Four `#[non_exhaustive]` enums with `thiserror`. Variants as specified in the design spec. Include `#[from]` for wrapped errors (`std::io::Error`, `rusqlite::Error`, etc.).

- [ ] **Step 3: Add `FileEntry` struct and `from_metadata` constructor**

Twelve public fields per the design spec. `from_metadata(path: PathBuf, metadata: &std::fs::Metadata) -> Self` using `std::os::unix::fs::MetadataExt`. Derive `Debug, Clone, Serialize`.

- [ ] **Step 4: Add `ScanConfig` with builder, `EntryBatch` newtype, remaining types**

`ScanConfig`: private fields, public getters, builder with validation (`root` must exist and be a directory, `batch_size` must be > 0).

`EntryBatch`: newtype wrapping `Vec<FileEntry>`. `new()` returns `None` for empty vec.

Remaining types (all public fields, derive `Debug, Clone`): `ScanMetadata`, `ScanProgress`, `ScanWarning`, `EntryQuery`, `SortField`, `SortDirection`, `TypeStat`, `DirectoryStats`, `SpaceInfo`, `JournalMode`.

`format_size(bytes: u64) -> String`: use binary units (KiB/MiB/GiB/TiB), one decimal place, no decimal for exact values.

- [ ] **Step 5: Add module declarations to `src/lib.rs`**

Add `pub mod types;` and `pub mod error;`. Re-export key types at crate root for ergonomic imports.

- [ ] **Step 6: Verify compilation**

Run: `cargo check`
Expected: compiles with zero errors.

- [ ] **Step 7: Write unit tests for behavioral logic**

In `src/types.rs` `#[cfg(test)] mod tests`:

| Test | Assertion |
|------|-----------|
| `file_type_from_regular_mode` | `FileType::from_mode(libc::S_IFREG as u32) == FileType::Regular` |
| `file_type_from_directory_mode` | `from_mode(S_IFDIR as u32) == Directory` |
| `file_type_from_symlink_mode` | `from_mode(S_IFLNK as u32) == Symlink` |
| `file_type_from_mode_with_permission_bits` | `from_mode((S_IFREG \| 0o755) as u32) == Regular` (permission bits don't affect type) |
| `file_category_code` | `from_extension(Some("rs")) == Code`, same for `"py"`, `"js"`, `"c"`, `"h"` |
| `file_category_image` | `from_extension(Some("jpg")) == Image`, `"png"`, `"gif"`, `"svg"` |
| `file_category_archive` | `from_extension(Some("gz")) == Archive`, `"zip"`, `"tar"`, `"xz"` |
| `file_category_no_extension` | `from_extension(None) == NoExtension` |
| `file_category_unknown` | `from_extension(Some("xyz123")) == Other` |
| `scan_config_rejects_nonexistent_root` | `builder().root("/no/such/path").build()` is `Err(ScanError::RootNotFound(_))` |
| `scan_config_rejects_file_as_root` | create a tempfile, `builder().root(file_path).build()` is `Err` |
| `scan_config_accepts_directory` | `builder().root(tempdir.path()).build()` is `Ok` |
| `scan_config_defaults` | built config has `cross_device() == false`, `batch_size() == 10_000` |
| `entry_batch_rejects_empty` | `EntryBatch::new(vec![])` is `None` |
| `entry_batch_accepts_nonempty` | `EntryBatch::new(vec![entry])` is `Some`, `.len() == 1` |
| `format_size_bytes` | `format_size(500) == "500 B"` |
| `format_size_kib` | `format_size(1536) == "1.5 KiB"` |
| `format_size_gib` | `format_size(1_073_741_824) == "1.0 GiB"` |

- [ ] **Step 8: Run tests**

Run: `cargo test`
Expected: all tests pass.

- [ ] **Step 9: Lint and format check**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 10: Commit**

```bash
git add src/types.rs src/error.rs src/lib.rs
git commit -m "feat(types): add core domain types and error hierarchy"
```

---

### Task 2: Platform Module

**Files:**
- Create: `src/platform/mod.rs`, `src/platform/linux.rs`, `src/platform/macos.rs`, `src/platform/freebsd.rs`
- Modify: `src/lib.rs` (add `pub mod platform;`)

**Interfaces:**
- Consumes: `JournalMode` from Task 1
- Produces:
  - `detect_filesystem_type(path: &Path) -> Result<String, std::io::Error>` — Tier 2/3: Linux uses `libc::statfs` → `f_type` magic number, macOS/FreeBSD use `f_fstypename` string, fallback returns `"unknown"`
  - `recommended_journal_mode(fs_type: &str, interactive: bool) -> JournalMode` — pure logic, no I/O

- [ ] **Step 1: Write tests for `recommended_journal_mode` in `src/platform/mod.rs`**

| Test | Assertion |
|------|-----------|
| `batch_mode_always_delete` | `recommended_journal_mode("ext4", false) == Delete` |
| `zfs_interactive_is_delete` | `recommended_journal_mode("zfs", true) == Delete` |
| `ext4_interactive_is_wal` | `recommended_journal_mode("ext4", true) == Wal` |
| `unknown_interactive_is_wal` | `recommended_journal_mode("unknown", true) == Wal` |
| `apfs_interactive_is_wal` | `recommended_journal_mode("apfs", true) == Wal` |

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test platform`
Expected: FAIL — functions not defined.

- [ ] **Step 3: Implement platform modules**

`mod.rs`: `recommended_journal_mode` (pure match), `detect_filesystem_type` dispatching to platform submodules via `#[cfg]`. Catch-all `#[cfg(not(any(...)))]` returns `Ok("unknown".into())`.

`linux.rs`: `libc::statfs` call, match `f_type` against known magic numbers (`EXT4_SUPER_MAGIC`, `XFS_SUPER_MAGIC`, `BTRFS_SUPER_MAGIC`, `ZFS_SUPER_MAGIC`, etc.). Unknown types return hex format `"0x{:x}"`.

`macos.rs` / `freebsd.rs`: `libc::statfs` call, read `f_fstypename` as CStr, convert to `String`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test platform`
Expected: all pass. `detect_filesystem_type` tested with a platform-conditional integration test: `detect_filesystem_type_returns_nonempty_string` against the system temp directory.

- [ ] **Step 5: Lint and format check**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add src/platform/ src/lib.rs
git commit -m "feat(platform): add filesystem type detection and journal mode selection"
```

---

### Task 3: Storage Layer

**Files:**
- Create: `src/storage/mod.rs`, `src/storage/sqlite.rs`, `tests/common/mod.rs`
- Modify: `src/lib.rs` (add `pub mod storage;`)

**Interfaces:**
- Consumes: `FileEntry`, `EntryBatch`, `ScanMetadata`, `ScanWarning`, `EntryQuery`, `SortField`, `SortDirection`, `FileType`, `TypeStat`, `DirectoryStats`, `JournalMode`, `StorageError` from Tasks 1–2
- Produces:
  - `Storage` trait with all methods per design spec (all `&self` except `init_schema(&mut self)`)
  - `SqliteStorage::open(path: &Path, journal_mode: JournalMode) -> Result<Self, StorageError>` — opens or creates DB, sets PRAGMAs, checks/sets schema version
  - `SqliteStorage::open_readonly(path: &Path) -> Result<Self, StorageError>` — for explore/export; rejects incompatible schema versions
  - Schema version constant: `SCHEMA_VERSION: u32 = 1`
  - Shared test helper: `create_test_entry(name: &str, size: u64, file_type: FileType) -> FileEntry`

- [ ] **Step 1: Write failing tests in `src/storage/sqlite.rs`**

| Test | Assertion |
|------|-----------|
| `init_schema_creates_tables` | after `init_schema()`, `SELECT name FROM sqlite_master WHERE type='table'` returns `entries` and `scan_metadata` |
| `init_schema_sets_user_version` | `PRAGMA user_version` == `SCHEMA_VERSION` |
| `insert_batch_persists_entries` | insert batch of 3, `SELECT COUNT(*) FROM entries` == 3 |
| `insert_batch_stores_path_bytes_losslessly` | insert entry with known path, `SELECT path_bytes FROM entries` roundtrips via `OsStr::from_encoded_bytes_unchecked` |
| `insert_batch_stores_path_text_as_lossy` | path text matches `path.to_string_lossy()` |
| `save_and_load_scan_metadata_roundtrips` | save metadata, load it, all fields match |
| `query_directory_children_returns_direct_children` | insert `/a/b`, `/a/c`, `/a/b/d`; query children of `/a` returns `b` and `c`, not `d` |
| `query_top_n_returns_largest_first` | insert entries with sizes 100, 500, 200; `top_n(2)` returns 500, 200 |
| `query_entries_filters_by_type` | insert regular + directory; query with `file_type: Some(Regular)` returns only regular |
| `query_entries_sorts_by_name` | insert "b", "a", "c"; sort by Name ascending returns a, b, c |
| `update_directory_sizes_modifies_in_place` | insert directory with size=0, update to size=1000, re-query returns 1000 |
| `open_rejects_future_schema_version` | create DB, set `PRAGMA user_version = 99`, `open_readonly()` returns `Err(SchemaVersionMismatch)` |
| `finalize_switches_journal_mode` | open with WAL, finalize, `PRAGMA journal_mode` returns `delete` |

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test storage`
Expected: FAIL — types not defined.

- [ ] **Step 3: Implement `Storage` trait in `src/storage/mod.rs`**

Trait definition with all methods from the design spec. `Send + Sync` bounds removed (single-threaded usage per instance).

- [ ] **Step 4: Implement `SqliteStorage` in `src/storage/sqlite.rs`**

`open()`: create `Connection`, set PRAGMAs (`synchronous=NORMAL`, `temp_store=MEMORY`, `mmap_size=268435456`, `busy_timeout=5000`), set journal mode, call `init_schema()` which creates tables/indexes and sets `user_version`.

`open_readonly()`: open existing DB, check `user_version` against `SCHEMA_VERSION`, reject mismatches.

`insert_batch()`: `BEGIN IMMEDIATE` transaction, prepared `INSERT INTO entries` statement with 14 parameter bindings per row. Path bytes via `path.as_os_str().as_encoded_bytes()`, path text via `path.to_string_lossy()`.

Query methods: build SQL from `EntryQuery` fields. `query_directory_children` uses `WHERE parent_text = ?`. `query_top_n_by_size` uses `ORDER BY allocated DESC LIMIT ?`.

`finalize_for_export()`: `PRAGMA wal_checkpoint(TRUNCATE)` then `PRAGMA journal_mode = DELETE`.

- [ ] **Step 5: Create shared test helper in `tests/common/mod.rs`**

`create_test_entry(name: &str, size: u64, file_type: FileType) -> FileEntry` — builds a `FileEntry` with the given name as path, specified size and type, zeroed metadata for remaining fields.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test storage`
Expected: all pass.

- [ ] **Step 7: Lint and format check**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 8: Commit**

```bash
git add src/storage/ src/lib.rs tests/common/
git commit -m "feat(storage): add Storage trait and SqliteStorage implementation"
```

---

### Task 4: Scanner

**Files:**
- Create: `src/scanner/mod.rs`, `src/scanner/walkdir.rs`
- Modify: `src/lib.rs` (add `pub mod scanner;`)
- Modify: `tests/common/mod.rs` (add `create_test_tree()`)

**Interfaces:**
- Consumes: `FileEntry`, `FileType`, `ScanConfig`, `EntryBatch`, `ScanMetadata`, `ScanProgress`, `ScanWarning`, `ScanError` from Task 1
- Produces:
  - `Scanner` trait: `fn scan(&self, config: &ScanConfig, batch_tx: mpsc::Sender<EntryBatch>, progress_tx: mpsc::Sender<ScanProgress>, cancel: CancellationToken) -> Result<ScanMetadata, ScanError>`
  - `WalkdirScanner::new() -> Self`
  - Test helper: `create_test_tree(dir: &Path)` — creates `/dir/{a.rs, b.jpg, sub/{c.txt, d.rs}, empty/}`

- [ ] **Step 1: Write failing integration tests in `src/scanner/walkdir.rs`**

All tests create a `tempfile::TempDir`, build a `ScanConfig`, create `mpsc::channel` pairs, and run the scanner synchronously.

| Test | Setup | Assertion |
|------|-------|-----------|
| `scan_collects_regular_file_metadata` | single 5-byte file | metadata has `size == 5`, `file_type == Regular`, `nlink == 1` |
| `scan_collects_directory_entries` | dir with subdirectory | subdirectory entry has `file_type == Directory` |
| `scan_records_symlinks_without_following` | file + symlink to it | symlink entry has `file_type == Symlink`, target file scanned once |
| `scan_skips_cross_device_entries` | `cross_device: false` | only entries with matching `device` appear (verified: all entries share root's device, since tempdir is same device) |
| `scan_deduplicates_hardlink_allocated_size` | file + hardlink | first has `allocated_size > 0`, second has `allocated_size == 0` |
| `scan_reports_progress` | 5 files | progress receiver gets at least one `ScanProgress` with `file_count > 0` |
| `scan_respects_cancellation` | cancel token cancelled before scan | returns `Err(ScanError::Cancelled)` or metadata with `file_count == 0` |
| `scan_handles_permission_denied` | dir with `0o000` permissions | scan completes, `metadata.warnings` contains entry for that dir |
| `scan_batches_entries` | 5 files, `batch_size: 2` | batch receiver gets 3 batches (2 + 2 + 1) |
| `scan_follows_root_symlink_to_directory` | symlink to tempdir | scan succeeds, finds files in target directory |
| `scan_handles_vanishing_file` | (platform-specific, may need skip) | scanner does not panic on stale directory entry |

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test scanner`
Expected: FAIL.

- [ ] **Step 3: Implement `Scanner` trait in `src/scanner/mod.rs`**

Trait definition. `ScanProgress` re-exported from `crate::types`.

- [ ] **Step 4: Implement `WalkdirScanner` in `src/scanner/walkdir.rs`**

`scan()` method:
1. Capture `start_time`, root `dev()` from `symlink_metadata(config.root())`.
2. `WalkDir::new(root).follow_links(false)`.
3. Iterate: for each `Result<DirEntry, Error>`:
   - `Err`: push `ScanWarning`, continue.
   - `Ok(entry)`: get `entry.path().symlink_metadata()`. On error: push warning, continue.
   - If `!config.cross_device()` and `metadata.dev() != root_dev`: skip.
   - Build `FileEntry::from_metadata(path, &metadata)`.
   - Hardlink dedup: if `metadata.nlink() > 1` and `(ino, dev)` already in `HashSet`, set `allocated_size = 0`.
   - Push to batch vec. If batch full: `batch_tx.blocking_send(EntryBatch::new(batch))`.
   - `try_send` progress update with current count, path, elapsed, rate.
   - Check `cancel.is_cancelled()`: if true, flush and return.
4. Flush remaining batch.
5. Return `ScanMetadata` with totals.

- [ ] **Step 5: Add `create_test_tree` helper to `tests/common/mod.rs`**

Creates the standard test tree: `a.rs` (10 bytes), `b.jpg` (20 bytes), `sub/c.txt` (5 bytes), `sub/d.rs` (15 bytes), `empty/` (empty dir). Returns nothing; files created in the given `dir`.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test scanner`
Expected: all pass. `scan_handles_permission_denied` may need `#[cfg_attr]` to skip on CI without appropriate permissions.

- [ ] **Step 7: Lint and format check**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 8: Commit**

```bash
git add src/scanner/ src/lib.rs tests/common/
git commit -m "feat(scanner): add Scanner trait and WalkdirScanner implementation"
```

---

### Task 5: Analyzer

**Files:**
- Create: `src/analyzer/mod.rs`
- Modify: `src/lib.rs` (add `pub mod analyzer;`)

**Interfaces:**
- Consumes: `Storage` trait, `FileEntry`, `FileType`, `FileCategory`, `DirectoryStats`, `TypeStat`, `SpaceInfo`, `StorageError` from Tasks 1, 3
- Produces:
  - `aggregate_directory_sizes(storage: &dyn Storage) -> Result<(), StorageError>` — queries all entries, builds `HashMap<PathBuf, DirectoryStats>`, persists via `storage.update_directory_sizes()`
  - `compute_type_stats(entries: &[FileEntry]) -> Vec<TypeStat>` — groups by `FileCategory` + extension, returns sorted by total_size desc
  - `compute_free_space(path: &Path) -> Result<SpaceInfo, std::io::Error>` — `statvfs` call

- [ ] **Step 1: Write failing tests**

| Test | Assertion |
|------|-----------|
| `aggregate_flat_directory` | root with 3 files of sizes 10, 20, 30 → root dir `total_size == 60`, `file_count == 3` |
| `aggregate_nested_directories` | `/root/sub/file.txt` (size 100) → both `/root` and `/root/sub` have `total_size == 100` |
| `aggregate_multiple_levels` | `/a/b/c/file` → `/a`, `/a/b`, `/a/b/c` all include the file's size |
| `aggregate_empty_directory_returns_zero` | root dir with no files → root has `total_size == 0`, `file_count == 0` |
| `type_stats_groups_by_category` | 2 `.rs` files (10B each) + 1 `.jpg` (50B) → Code: count=2/size=20, Image: count=1/size=50 |
| `type_stats_sorted_by_size_desc` | largest category first in result |
| `free_space_returns_positive_values` | `compute_free_space(tempdir)` returns `total > 0`, `free > 0`, `free <= total` |

Tests for `aggregate_*` use `SqliteStorage` with an in-memory-like tempfile database: insert known entries, run aggregation, query directories to verify.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test analyzer`

- [ ] **Step 3: Implement analyzer functions**

`aggregate_directory_sizes`: query all entries via `storage.query_entries(&EntryQuery { limit: None, ..EntryQuery::default() })`. Build `HashMap<PathBuf, DirectoryStats>`. For each non-directory entry, walk `path.parent()` chain upward, accumulating `size` and `allocated_size` into each ancestor. Call `storage.update_directory_sizes(&map)`.

`compute_type_stats`: iterate entries, classify each via `FileCategory::from_extension(path.extension())`, accumulate into `HashMap<(FileCategory, String), TypeStat>`, collect and sort by `total_size` descending.

`compute_free_space`: `nix::sys::statvfs::statvfs(path)`, compute total/free/unknown from `f_blocks`, `f_bsize`, `f_bavail`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test analyzer`

- [ ] **Step 5: Lint and format check**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add src/analyzer/ src/lib.rs
git commit -m "feat(analyzer): add directory size aggregation and file-type statistics"
```

---

### Task 6: Async Pipeline

**Files:**
- Create: `src/pipeline.rs`
- Modify: `src/lib.rs` (add `pub mod pipeline;`)

**Interfaces:**
- Consumes: `ScanConfig`, `EntryBatch`, `ScanMetadata`, `ScanProgress`, `JournalMode`, `PipelineError` from Task 1; `SqliteStorage` from Task 3; `WalkdirScanner` from Task 4; `aggregate_directory_sizes` from Task 5
- Produces:
  - `PipelineConfig { scan: ScanConfig, channel_capacity: usize, journal_mode: JournalMode, storage_path: PathBuf }`
  - `PipelineResult { metadata: ScanMetadata, storage_path: PathBuf, timing: PipelineTiming }`
  - `PipelineTiming { scan_duration: Duration, storage_duration: Duration, aggregation_duration: Duration, total_duration: Duration }`
  - `async fn run_pipeline(config: PipelineConfig, cancel: CancellationToken) -> Result<(mpsc::Receiver<ScanProgress>, oneshot::Receiver<Result<PipelineResult, PipelineError>>), PipelineError>` — spawns scanner + storage writer, returns channels for the caller to consume

- [ ] **Step 1: Write failing tests**

Use `#[tokio::test]` with `tempfile::TempDir` + `create_test_tree`.

| Test | Assertion |
|------|-----------|
| `pipeline_scans_and_persists` | run pipeline on test tree, open resulting DB, `SELECT COUNT(*) FROM entries > 0` |
| `pipeline_runs_aggregation` | after pipeline, directory entries have `size > 0` |
| `pipeline_reports_progress` | drain progress receiver, at least one `ScanProgress` with `file_count > 0` |
| `pipeline_signals_completion` | completion receiver yields `Ok(PipelineResult)` with `metadata.file_count > 0` |
| `pipeline_respects_cancellation` | cancel immediately, completion yields result (possibly partial or error) |
| `pipeline_finalizes_in_batch_mode` | (DELETE journal mode) after pipeline, `PRAGMA journal_mode` is `delete` |

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test pipeline`

- [ ] **Step 3: Implement `run_pipeline`**

1. Create `SqliteStorage::open(config.storage_path, config.journal_mode)`, call `init_schema()`.
2. Create bounded `mpsc::channel::<EntryBatch>(config.channel_capacity)`.
3. Create unbounded `mpsc::channel::<ScanProgress>()`.
4. Create `oneshot::channel::<Result<PipelineResult, PipelineError>>()`.
5. `tokio::task::spawn_blocking` for scanner: `WalkdirScanner::new().scan(config.scan, batch_tx, progress_tx, cancel.clone())`. On completion, drop senders.
6. `tokio::task::spawn_blocking` for storage writer: loop `batch_rx.blocking_recv()`, call `storage.insert_batch()`. On channel close: save scan metadata, run `aggregate_directory_sizes(&storage)`, if DELETE mode call `finalize_for_export()`, send `PipelineResult` on oneshot.
7. Return `(progress_rx, completion_rx)`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test pipeline`

- [ ] **Step 5: Lint and format check**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add src/pipeline.rs src/lib.rs
git commit -m "feat(pipeline): add async scan-to-storage pipeline with progress and cancellation"
```

---

### Task 7: TUI App Shell & Progress View

**Files:**
- Create: `src/ui/mod.rs`, `src/ui/app.rs`, `src/ui/views/mod.rs`, `src/ui/views/progress.rs`
- Modify: `src/lib.rs` (add `pub mod ui;`)

**Interfaces:**
- Consumes: `ScanProgress`, `ScanMetadata`, `PipelineConfig`, `PipelineResult`, `UiError` from Task 1; `run_pipeline` from Task 6; `SqliteStorage` from Task 3; `format_size` from Task 1
- Produces:
  - `async fn run_scan_ui(config: PipelineConfig) -> Result<(), UiError>` — setup terminal, run pipeline, show progress → explorer transition
  - `async fn run_explore_ui(storage_path: &Path) -> Result<(), UiError>` — setup terminal, open DB, show explorer
  - `setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>, UiError>` — enable raw mode, alternate screen, panic hook
  - `restore_terminal(terminal: &mut Terminal<...>) -> Result<(), UiError>`
  - `AppState` enum: `Scanning(ScanProgressState)`, `Exploring(ExplorerState)`
  - `ScanProgressState { file_count: u64, files_per_sec: f64, elapsed: Duration, current_path: PathBuf }`
  - `render_progress(frame: &mut Frame, state: &ScanProgressState, area: Rect)` — renders LineGauge + text stats

- [ ] **Step 1: Write failing tests for progress view rendering**

Headless tests using `ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24))`.

| Test | Assertion |
|------|-----------|
| `progress_view_renders_file_count` | buffer string contains `"42"` when `file_count: 42` |
| `progress_view_renders_scan_rate` | buffer contains `"files/sec"` |
| `progress_view_renders_current_path` | buffer contains at least the last path component |
| `progress_view_renders_elapsed_time` | buffer contains a time format like `"0:03"` or `"3s"` |
| `progress_view_renders_at_minimum_size` | 40x10 buffer renders without panic |
| `root_indicator_shown_for_root_user` | when `is_root` flag is true, buffer contains `"ROOT"` |

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test ui`

- [ ] **Step 3: Implement terminal setup/teardown in `src/ui/mod.rs`**

`setup_terminal()`: `enable_raw_mode()`, `stdout().execute(EnterAlternateScreen)`, install panic hook that calls `restore_terminal`, create `Terminal::new(CrosstermBackend::new(stdout()))`.

`restore_terminal()`: `disable_raw_mode()`, `stdout().execute(LeaveAlternateScreen)`.

`run_scan_ui()` and `run_explore_ui()`: setup terminal, delegate to inner async function, restore on exit (including error path).

- [ ] **Step 4: Implement `AppState` and `ScanProgressState` in `src/ui/app.rs`**

`AppState` enum. `ScanProgressState` struct with fields for display. `update(&mut self, progress: ScanProgress)` method to update state from channel messages.

- [ ] **Step 5: Implement progress view in `src/ui/views/progress.rs`**

`render_progress(frame, state, area)`: use `Layout` to split area into rows. Top: title "Scanning...". Middle: `LineGauge` (indeterminate, animated). Bottom rows: file count, rate, elapsed, current path (truncated to fit). Root indicator in top-right if `nix::unistd::geteuid().is_root()`.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test ui`

- [ ] **Step 7: Lint and format check**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 8: Commit**

```bash
git add src/ui/ src/lib.rs
git commit -m "feat(ui): add TUI app shell, terminal management, and progress view"
```

---

### Task 8: TUI Explorer View & Widgets

**Files:**
- Create: `src/ui/views/explorer.rs`, `src/ui/widgets/mod.rs`, `src/ui/widgets/treemap.rs`, `src/ui/widgets/file_table.rs`, `src/ui/widgets/type_chart.rs`
- Modify: `src/ui/app.rs` (add `ExplorerState`)
- Modify: `src/ui/views/mod.rs` (add explorer)
- Modify: `src/ui/mod.rs` (wire explorer into event loop)

**Interfaces:**
- Consumes: `Storage` trait, `FileEntry`, `FileType`, `FileCategory`, `TypeStat`, `DirectoryStats`, `SpaceInfo`, `EntryQuery`, `SortField`, `SortDirection`, `format_size` from Tasks 1, 3; `compute_type_stats`, `compute_free_space` from Task 5; `AppState::Exploring` from Task 7
- Produces:
  - `ExplorerState` struct: `current_path`, `breadcrumb: Vec<PathBuf>`, `entries: Vec<FileEntry>`, `type_stats: Vec<TypeStat>`, `selected_index: usize`, `sort_field: SortField`, `sort_direction: SortDirection`, `treemap_state: TreemapState`
  - `ExplorerState::navigate_into(&mut self, storage: &dyn Storage, path: PathBuf) -> Result<(), UiError>` — update current_path, re-query, push breadcrumb
  - `ExplorerState::navigate_up(&mut self, storage: &dyn Storage) -> Result<(), UiError>` — pop breadcrumb
  - `ExplorerState::cycle_sort(&mut self)` / `reverse_sort(&mut self)` — update sort, re-sort entries
  - `render_explorer(frame: &mut Frame, state: &ExplorerState, area: Rect)`
  - `TreemapWidget` / `TreemapState`: `StatefulWidget` impl using `streemap::squarify`
  - `FileTableWidget`: renders sortable `Table`
  - `TypeChartWidget`: renders `BarChart`

- [ ] **Step 1: Write failing tests for widgets**

Headless `TestBackend` tests:

| Test | Assertion |
|------|-----------|
| `treemap_renders_proportional_cells` | 75/25 split items: larger item occupies ≥60% of buffer cells |
| `treemap_truncates_long_labels` | item named "very_long_filename.rs" in narrow cell contains "..." |
| `treemap_hides_labels_in_tiny_cells` | item in 2-column cell has no text label |
| `treemap_colors_by_category` | Regular(Code) and Regular(Image) items render with different colors |
| `file_table_renders_header` | buffer contains "Name", "Size", "Type" |
| `file_table_sorts_by_size_desc` | first data row has largest size value |
| `file_table_highlights_selected_row` | selected row has different style than unselected |
| `type_chart_renders_categories` | buffer contains category names ("Code", "Image") |
| `type_chart_shows_sizes` | buffer contains formatted size strings |
| `explorer_three_panel_layout` | 80x24 buffer: treemap occupies top half, table bottom-left, chart bottom-right |
| `navigation_into_directory` | after `navigate_into(sub_path)`, `current_path == sub_path`, breadcrumb has old path |
| `navigation_up_restores_parent` | after navigate_into then navigate_up, `current_path` back to original |

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test ui::widgets && cargo test ui::views::explorer`

- [ ] **Step 3: Implement `TreemapWidget` in `src/ui/widgets/treemap.rs`**

`TreemapState { selected: Option<usize> }`. `TreemapWidget { items: Vec<TreemapItem> }` where `TreemapItem { label: String, size: u64, category: FileCategory, is_directory: bool }`.

`StatefulWidget::render`: call `streemap::squarify` with sizes and container rect (as `streemap::Rect<f32>`). Convert each output rect to `ratatui::layout::Rect` (u16): `floor()` for x/y, `ceil()` for right/bottom, clamped to container. Fill each cell region with category color. Render label: truncate with "..." if width < label length, hide if width < 3. Highlight selected item border.

Category color palette: Code=blue, Image=green, Document=yellow, Archive=red, Audio=magenta, Video=cyan, Binary=dark_gray, NoExtension=gray, Other=white.

- [ ] **Step 4: Implement `FileTableWidget` in `src/ui/widgets/file_table.rs`**

Renders a `ratatui::widgets::Table` with columns: Name, Size (`format_size`), Type (`FileType` display), Modified (formatted timestamp), Owner (uid). Header row with sort indicator (▲/▼) on active column. Selected row highlighted.

- [ ] **Step 5: Implement `TypeChartWidget` in `src/ui/widgets/type_chart.rs`**

Renders a `ratatui::widgets::BarChart` from `Vec<TypeStat>`. Each bar labeled with category name. Bar value is `total_size`. Colors match treemap category palette.

- [ ] **Step 6: Implement `ExplorerState` in `src/ui/app.rs`**

Navigation methods query `Storage` for children of `current_path`. Sort methods re-sort the cached `entries` vec. Breadcrumb stack for up-navigation.

- [ ] **Step 7: Implement `render_explorer` in `src/ui/views/explorer.rs`**

Three-panel layout via `Layout`: top half → treemap, bottom split 60/40 → file table / type chart. Pass state slices to each widget. Handle minimum terminal size gracefully (collapse panels if too small).

- [ ] **Step 8: Wire explorer into event loop in `src/ui/mod.rs`**

In the `select!` loop: when `AppState::Exploring`, handle key events: arrow keys (select treemap/table), Enter (navigate into), Backspace (navigate up), Tab/Shift+Tab (cycle sort), `r` (reverse sort), `q` (quit).

- [ ] **Step 9: Run tests to verify they pass**

Run: `cargo test ui`

- [ ] **Step 10: Lint and format check**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 11: Commit**

```bash
git add src/ui/
git commit -m "feat(ui): add explorer view with treemap, file table, and type chart widgets"
```

---

### Task 9: CLI Wiring & Export

**Files:**
- Modify: `src/main.rs` (add `#[tokio::main]`, make main async)
- Modify: `src/lib.rs` (make `run()` async, implement subcommand dispatch)
- Create: `tests/cli_integration.rs`

**Interfaces:**
- Consumes: everything from Tasks 1–8
- Produces: working `nixdirstat` binary with `scan`, `explore`, `export` subcommands

- [ ] **Step 1: Write failing integration tests in `tests/cli_integration.rs`**

Use `assert_cmd` pattern (run binary via `std::process::Command`), or test `run()` directly with constructed CLI args.

| Test | Assertion |
|------|-----------|
| `scan_batch_creates_database_file` | run scan with `--output`, file exists after |
| `scan_batch_database_contains_entries` | open output file, `SELECT COUNT(*) > 0` |
| `scan_batch_database_is_finalized` | `PRAGMA journal_mode` returns `delete` |
| `explore_rejects_nonexistent_file` | returns error containing "not found" or similar |
| `explore_rejects_invalid_database` | create a text file, pass as scan_file, returns error |
| `export_json_valid` | run export with `--format json`, stdout parses as JSON array |
| `export_csv_has_header` | run export with `--format csv`, first line contains "path" |
| `scan_rejects_nonexistent_path` | returns error for missing scan target |

Note: interactive scan (`scan` without `--output`) cannot be integration-tested without a PTY. Test the batch path; the interactive path is validated manually.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test cli_integration`

- [ ] **Step 3: Update `src/main.rs` to async**

```rust
#[tokio::main]
async fn main() -> ExitCode {
    match nixdirstat::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err:#}");
            ExitCode::FAILURE
        },
    }
}
```

- [ ] **Step 4: Implement `run()` dispatch in `src/lib.rs`**

Make `run()` async. Match on `cli.command`:

**`Scan`**: validate root path via `ScanConfig::builder()`. Detect filesystem type. Build `PipelineConfig` with storage path (user-provided `--output` or `tempfile::NamedTempFile`). If `--output`: run batch pipeline — spawn a progress-printing task that reads `progress_rx` and writes `\r`-overwritten line to stderr, await completion, print summary. If no `--output`: call `ui::run_scan_ui(config)`.

**`Explore`**: validate file exists, call `ui::run_explore_ui(&scan_file)`.

**`Export`**: open `SqliteStorage::open_readonly(&scan_file)`. Query all entries. Match on format: JSON → `serde_json::to_writer_pretty(stdout, &entries)`. CSV → write header line then one comma-separated line per entry to stdout. `FileEntry` needs `Serialize` derive (already from Task 1).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test cli_integration`

- [ ] **Step 6: Run full test suite**

Run: `cargo test --features parallel`
Expected: all tests across all modules pass.

- [ ] **Step 7: Full CI parity check**

Run: `just check`
Expected: fmt, clippy, test, doc, deny all pass.

- [ ] **Step 8: Commit**

```bash
git add src/main.rs src/lib.rs tests/cli_integration.rs
git commit -m "feat(cli): wire subcommands to pipeline, TUI, and export"
```

- [ ] **Step 9: Push branch and open PR**

```bash
git push -u origin feat/mvp-implementation
gh pr create --title "feat: implement MVP" --body "..."
```

PR description should summarize the MVP scope: scanner, storage, analyzer, async pipeline, TUI (progress + explorer with treemap/table/chart), CLI wiring, and export.
