# Refactor: Encapsulation and DRY Cleanup — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Encapsulate `FileEntry`, `ExplorerState`, and `Storage` to enforce invariants at the type level, and eliminate duplicated test helpers — all without changing observable behavior.

**Architecture:** Four ordered commits, each compilable and test-passing independently. Commit 1 encapsulates `FileEntry` and adds a test builder. Commit 2 deduplicates test helpers and removes dead code. Commit 3 splits `Storage` into `ReadStorage`/`WriteStorage`. Commit 4 encapsulates `ExplorerState` and adds a `ScanProgressState` constructor.

**Tech Stack:** Rust stable, edition 2024, MSRV 1.95. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-06-refactor-encapsulation-design.md`

## Global Constraints

- `unsafe` code is FORBIDDEN.
- `cargo fmt --check` must pass (rustfmt defaults).
- `cargo clippy --all-targets -- -D warnings` must pass.
- No `#[allow(...)]` except verified false positives with a comment.
- Conventional Commits for all commit messages.
- Feature branches + PRs — never commit to main.
- CDD + TDD mandatory per CLAUDE.md.

## Review Focus

1. **`Serialize` derive on private-field `FileEntry`** — `serde::Serialize` derives work on private fields, but the JSON field names are derived from field names. Verify `write_entries_json` output preserves the same JSON shape (field names unchanged). Add assertion in Task 1 Step 8.
2. **`entry.path` in `build_tree` compared with `strip_prefix`** — After encapsulation, `entry.path()` returns `&Path` not `PathBuf`. Verify `strip_prefix` still works on `&Path` (it does — `Path::strip_prefix` takes `AsRef<Path>`). No test needed — compiler verifies.
3. **`EntryBatch::entries()` iteration after encapsulation** — `do_insert_entries` iterates `batch.entries()` and accesses fields. Ensure all 14 field reads switch to getters. Covered by compiler errors from making fields private.
4. **`aggregate_directory_sizes` trait bound change** — After splitting to `WriteStorage`, the function signature changes from `&dyn Storage` to `&dyn WriteStorage`. The pipeline passes a concrete `SqliteStorage` which implements `WriteStorage`. Existing tests use `SqliteStorage` directly. No behavioral change.
5. **Integration test `cli_integration.rs` imports `Storage`** — Line 9 uses `use nixdirstat::storage::Storage`. After the split, this must change to `ReadStorage`. Covered in Task 3.

---

### Task 1: Encapsulate `FileEntry` — private fields, getters, `from_raw`, builder

**Files:**
- Modify: `src/types.rs` — make fields private, add getters, `set_allocated_size`, `display_name`, `from_raw`, `FileEntryBuilder`
- Modify: `src/storage/sqlite.rs:100-161,279-300,440-460` — switch to getters and `from_raw`
- Modify: `src/scanner/walkdir.rs:96,240,243` — switch to `set_allocated_size` and getters
- Modify: `src/analyzer/mod.rs:47-79,96-108` — switch to getters
- Modify: `src/ui/tree.rs:55-56,69-104` — switch to getters, use `display_name` for root
- Modify: `src/lib.rs:51-80` — switch to getters in `write_entries_csv` and `write_entries_json`
- Modify: `tests/common/mod.rs:36-48` — switch `create_test_entry` to use `FileEntryBuilder`

**Interfaces:**
- Produces:
  - `FileEntry::path(&self) -> &Path`
  - `FileEntry::size(&self) -> u64`
  - `FileEntry::allocated_size(&self) -> u64`
  - `FileEntry::file_type(&self) -> FileType`
  - `FileEntry::category(&self) -> FileCategory`
  - `FileEntry::inode(&self) -> u64`
  - `FileEntry::device(&self) -> u64`
  - `FileEntry::nlink(&self) -> u64`
  - `FileEntry::uid(&self) -> u32`
  - `FileEntry::gid(&self) -> u32`
  - `FileEntry::mtime(&self) -> SystemTime`
  - `FileEntry::mode(&self) -> u32`
  - `FileEntry::set_allocated_size(&mut self, size: u64)`
  - `FileEntry::display_name(&self) -> String`
  - `FileEntry::from_raw(path, size, allocated_size, file_type, category, inode, device, nlink, uid, gid, mtime, mode) -> Self` — `pub(crate)`
  - `FileEntryBuilder::new() -> Self` — `#[cfg(test)]`
  - `FileEntryBuilder::path(&mut self, p: impl Into<PathBuf>) -> &mut Self`
  - `FileEntryBuilder::size(&mut self, s: u64) -> &mut Self`
  - `FileEntryBuilder::allocated_size(&mut self, s: u64) -> &mut Self`
  - `FileEntryBuilder::file_type(&mut self, ft: FileType) -> &mut Self`
  - `FileEntryBuilder::category(&mut self, c: FileCategory) -> &mut Self`
  - `FileEntryBuilder::mode(&mut self, m: u32) -> &mut Self`
  - `FileEntryBuilder::nlink(&mut self, n: u64) -> &mut Self`
  - `FileEntryBuilder::mtime(&mut self, t: SystemTime) -> &mut Self`
  - `FileEntryBuilder::inode(&mut self, i: u64) -> &mut Self`
  - `FileEntryBuilder::device(&mut self, d: u64) -> &mut Self`
  - `FileEntryBuilder::build(&self) -> FileEntry`

- [ ] **Step 1: Create feature branch**

```bash
git checkout -b refactor/encapsulation-cleanup main
```

- [ ] **Step 2: Make `FileEntry` fields private and add getters in `src/types.rs`**

Remove `pub` from all 12 fields in `FileEntry` (lines 300-324). Add an `impl FileEntry` block with:
- 12 getters (signatures in Interfaces above). `path()` returns `&Path` (call `.as_path()`). All numeric fields return `Copy` values. `mtime()` returns `SystemTime` (it's `Copy`).
- `set_allocated_size(&mut self, size: u64)` — sets `self.allocated_size = size`.
- `display_name(&self) -> String` — `self.path.file_name().map_or_else(|| self.path.display().to_string(), |n| n.to_string_lossy().into_owned())`.
- Add `debug_assert!(path.is_absolute())` as the first line of `from_metadata`.

- [ ] **Step 3: Add `from_raw` constructor in `src/types.rs`**

Add `pub(crate) fn from_raw(...)` with the 12-parameter signature from Interfaces. Body: `Self { path, size, allocated_size, file_type, category, inode, device, nlink, uid, gid, mtime, mode }`.

- [ ] **Step 4: Add `FileEntryBuilder` in `src/types.rs`**

Below the existing `#[cfg(test)] mod tests` block, add a `#[cfg(test)]` module-level struct and impl. Defaults per spec: `path="/test"`, `size=0`, `allocated_size=0`, `file_type=Regular`, `category=NoExtension`, `inode=0`, `device=0`, `nlink=1`, `uid=0`, `gid=0`, `mtime=UNIX_EPOCH`, `mode=0o644`. Builder methods take `&mut self` and return `&mut Self` for chaining. `build(&self) -> FileEntry` constructs from the stored fields. Mark it `pub` so other crate modules' test code can use it via `crate::types::FileEntryBuilder`.

- [ ] **Step 5: Update `src/storage/sqlite.rs`**

- `entry_parent` (line 100-102): `entry.path()` instead of `entry.path`, `.parent().unwrap_or(entry.path())`.
- `entry_mtime_secs` (line 104-105): `entry.mtime()` instead of `entry.mtime`.
- `row_to_entry` (lines 124-162): replace the `Ok(FileEntry { ... })` struct literal with `Ok(FileEntry::from_raw(path, size, allocated_size, ...))` using the same local variables.
- `do_insert_entries` (lines 280-300): replace all `entry.path`, `entry.size`, etc. with getter calls: `entry.path()`, `entry.size()`, `entry.allocated_size()`, `entry.file_type().as_discriminant()`, `entry.mode()`, `entry.uid()`, `entry.gid()`, `entry.inode()`, `entry.device()`, `entry.nlink()`.
- `query_type_stats` (lines 440-465): `entry.category()`, `entry.size()`, `entry.allocated_size()`.

- [ ] **Step 6: Update `src/scanner/walkdir.rs`**

- `dedup_hardlink` (line 96): `file_entry.set_allocated_size(0)` instead of `file_entry.allocated_size = 0`.
- Scanner loop (line 240): `file_entry.size()` instead of `file_entry.size`.
- Scanner loop (line 243): `file_entry.path().to_path_buf()` instead of `file_entry.path.clone()`.

- [ ] **Step 7: Update `src/analyzer/mod.rs`**

Replace all `entry.file_type` with `entry.file_type()`, `entry.path` with `entry.path()`, `entry.size` with `entry.size()`, `entry.allocated_size` with `entry.allocated_size()`, `entry.category` with `entry.category()` throughout `aggregate_directory_sizes` (lines 47-84) and `compute_type_stats` (lines 92-114).

- [ ] **Step 8: Update `src/ui/tree.rs`**

- `build_tree` root name (line 55-57): replace `root_path.file_name().map_or_else(...)` with the same pattern (this extracts from `root_path`, not `FileEntry`, so no change needed here).
- `build_tree` entry loop (lines 69-107): `entry.path()` for `strip_prefix`, `entry.size()`, `entry.allocated_size()`, `entry.file_type()`, `entry.mtime()`. `entry.path().extension()` instead of `entry.path.extension()`.

- [ ] **Step 9: Update `src/lib.rs`**

- `write_entries_csv` (lines 51-80): `entry.path()` (returns `&Path`, call `.to_string_lossy()`), `entry.size()`, `entry.allocated_size()`, `entry.file_type()`, `entry.mode()`, `entry.uid()`, `entry.gid()`, `entry.mtime()`, `entry.inode()`, `entry.device()`, `entry.nlink()`.

- [ ] **Step 10: Update test call sites to use `FileEntryBuilder`**

In every `#[cfg(test)]` module that constructs `FileEntry` via struct literal, replace with `FileEntryBuilder`. Files:
- `src/types.rs` tests (line 948 `entry_batch_accepts_nonempty`) — uses `FileEntry::from_metadata` on a real file, no change needed.
- `src/analyzer/mod.rs` tests (lines 166-215, 346-360) — replace `make_file`, `make_dir`, `make_file_with_ext` bodies and the inline struct literal at line 346 with builder calls.
- `src/storage/sqlite.rs` tests (lines 506-538, 603-616, 633-649, 781-795) — replace `make_entry`, `make_entry_at` bodies and inline struct literals with builder calls.
- `src/ui/tree.rs` tests (lines 196-215, 279) — replace `make_entry` body and inline mutation at line 279 with builder.
- `tests/common/mod.rs` (lines 35-48) — replace `create_test_entry` body with builder.

- [ ] **Step 11: Verify JSON serialization preserves field names**

Add a unit test in `src/types.rs` tests:
```rust
#[test]
fn file_entry_json_field_names_preserved() {
    let entry = FileEntryBuilder::new().path("/test.rs").size(42).build();
    let json = serde_json::to_string(&entry).expect("serialize");
    assert!(json.contains("\"path\""), "JSON should contain 'path' field");
    assert!(json.contains("\"size\""), "JSON should contain 'size' field");
    assert!(json.contains("\"allocated_size\""), "JSON should contain 'allocated_size' field");
}
```

- [ ] **Step 12: Run full verification**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
All must pass.

- [ ] **Step 13: Commit**

```bash
git add src/types.rs src/storage/sqlite.rs src/scanner/walkdir.rs src/analyzer/mod.rs src/ui/tree.rs src/lib.rs tests/common/mod.rs
git commit -m "refactor(types): encapsulate FileEntry fields — private fields, getters, builder

Make all FileEntry fields private. Add public getters, set_allocated_size
for hardlink dedup, display_name for filename extraction, from_raw for
database reconstruction, and a #[cfg(test)] FileEntryBuilder.

Update ~60 call sites across 7 source files and all test modules.

Closes #17"
```

---

### Task 2: DRY cleanup — remove duplicated test helpers and dead `compute_type_stats`

**Files:**
- Modify: `src/analyzer/mod.rs` — remove `compute_type_stats` function (lines 92-114), its tests (lines 305-362), and update module doc (lines 8-9)
- Modify: `src/analyzer/mod.rs` tests — replace remaining `make_file`/`make_dir`/`make_file_with_ext` with `FileEntryBuilder`
- Modify: `src/storage/sqlite.rs` tests — replace `make_entry`/`make_entry_at` with `FileEntryBuilder`
- Modify: `src/ui/tree.rs` tests — replace `make_entry` with `FileEntryBuilder`

**Interfaces:**
- Consumes: `FileEntryBuilder` from Task 1

Note: Task 1 already converted test helper *bodies* to use `FileEntryBuilder`. This task goes further: remove the wrapper functions entirely and inline `FileEntryBuilder` calls at each test call site, since the wrapper functions no longer add value over the builder.

- [ ] **Step 1: Remove `compute_type_stats` from `src/analyzer/mod.rs`**

Delete the function (lines 92-114) and its three tests (`type_stats_groups_by_category`, `type_stats_sorted_by_size_desc`, `type_stats_skips_non_regular_entries`). Update the module doc comment (line 8) to remove the bullet referencing `compute_type_stats`. Remove unused imports that were only needed by this function (`FileCategory` in the function body — check if other code in the module still uses it; it does not in production code, only tests used `make_file_with_ext` which is also being removed).

- [ ] **Step 2: Remove duplicated test helper functions**

In each file, delete the local `make_*` helper functions and replace all call sites with direct `FileEntryBuilder` calls:

**`src/analyzer/mod.rs` tests:**
- Delete `make_file`, `make_dir`, `make_file_with_ext` (lines 166-215).
- Example replacement: `make_file("/d/a", 10)` becomes `FileEntryBuilder::new().path("/d/a").size(10).build()`.
- `make_dir("/d")` becomes `FileEntryBuilder::new().path("/d").file_type(FileType::Directory).build()`.

**`src/storage/sqlite.rs` tests:**
- Delete `make_entry`, `make_entry_at` (lines 506-538).
- `make_entry("a", 100, FileType::Regular)` becomes `FileEntryBuilder::new().path("/a").size(100).file_type(FileType::Regular).build()`.
- `make_entry_at("/a/b", 10)` becomes `FileEntryBuilder::new().path("/a/b").size(10).build()`.

**`src/ui/tree.rs` tests:**
- Delete `make_entry` (lines 196-215).
- Same pattern. Note this `make_entry` set `uid: 1000, gid: 1000` and conditional `mode` — if any test depends on those values, set them in the builder; otherwise the defaults suffice.

- [ ] **Step 3: Run full verification**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- [ ] **Step 4: Commit**

```bash
git add src/analyzer/mod.rs src/storage/sqlite.rs src/ui/tree.rs
git commit -m "refactor: deduplicate test helpers, remove dead compute_type_stats

Replace 5 duplicated make_entry/make_file/make_dir test helper functions
across 3 modules with FileEntryBuilder. Remove compute_type_stats from
analyzer (dead code — UI calls storage.query_type_stats directly).

Closes #25"
```

---

### Task 3: Split `Storage` trait into `ReadStorage` / `WriteStorage`

**Files:**
- Modify: `src/storage/mod.rs` — replace `Storage` with `ReadStorage` + `WriteStorage` traits
- Modify: `src/storage/sqlite.rs` — implement both traits, make `finalize_for_export` inherent
- Modify: `src/analyzer/mod.rs:16,35` — import and use `WriteStorage`
- Modify: `src/pipeline.rs:28-29` — update import
- Modify: `src/lib.rs:22` — update import
- Modify: `src/ui/mod.rs:32` — update import
- Modify: `tests/cli_integration.rs:9` — update import

**Interfaces:**
- Produces:
  - `trait ReadStorage { load_scan_metadata, query_entries, query_directory_children, query_top_n_by_size, query_type_stats }`
  - `trait WriteStorage: ReadStorage { init_schema, insert_batch, save_scan_metadata, update_directory_sizes }`
  - `SqliteStorage::finalize_for_export(&self) -> Result<(), StorageError>` (inherent, not trait)

- [ ] **Step 1: Rewrite `src/storage/mod.rs`**

Replace the single `Storage` trait with two traits:

`ReadStorage` — the 5 query methods (same signatures as current `Storage`):
- `load_scan_metadata(&self) -> Result<ScanMetadata, StorageError>`
- `query_entries(&self, query: &EntryQuery) -> Result<Vec<FileEntry>, StorageError>`
- `query_directory_children(&self, path: &Path) -> Result<Vec<FileEntry>, StorageError>`
- `query_top_n_by_size(&self, n: usize) -> Result<Vec<FileEntry>, StorageError>`
- `query_type_stats(&self) -> Result<Vec<TypeStat>, StorageError>`

`WriteStorage: ReadStorage` — the 4 mutation methods:
- `init_schema(&mut self) -> Result<(), StorageError>`
- `insert_batch(&self, batch: &EntryBatch) -> Result<(), StorageError>`
- `save_scan_metadata(&self, metadata: &ScanMetadata) -> Result<(), StorageError>`
- `update_directory_sizes(&self, sizes: &HashMap<PathBuf, DirectoryStats>) -> Result<(), StorageError>`

Remove `finalize_for_export` from both traits.

- [ ] **Step 2: Update `src/storage/sqlite.rs`**

Split the existing `impl Storage for SqliteStorage` into `impl ReadStorage for SqliteStorage` (5 methods) and `impl WriteStorage for SqliteStorage` (4 methods). Move `finalize_for_export` to an `impl SqliteStorage` block (inherent method). Update the `use super::Storage` import to `use super::{ReadStorage, WriteStorage}`.

- [ ] **Step 3: Update `src/analyzer/mod.rs`**

- Import: change `use crate::storage::Storage` to `use crate::storage::WriteStorage`.
- `aggregate_directory_sizes` signature: change `storage: &dyn Storage` to `storage: &dyn WriteStorage` (line 35). The function calls `query_entries` (from `ReadStorage`, available via supertrait) and `update_directory_sizes` (from `WriteStorage`).

- [ ] **Step 4: Update `src/pipeline.rs`**

- Import (line 28-29): change `use crate::storage::{Storage, sqlite::SqliteStorage}` to `use crate::storage::sqlite::SqliteStorage`. The pipeline uses `SqliteStorage` concretely everywhere — it never references the trait. Remove the `Storage` import. `finalize_for_export` is now an inherent method, so it works without any trait import.

- [ ] **Step 5: Update `src/lib.rs`**

- Import (line 22): change `use crate::storage::{Storage, sqlite::SqliteStorage}` to `use crate::storage::{ReadStorage, sqlite::SqliteStorage}`. The `Export` command uses `storage.query_entries()` which is on `ReadStorage`.

- [ ] **Step 6: Update `src/ui/mod.rs`**

- Import (line 32): change `storage::{Storage as _, sqlite::SqliteStorage}` to `storage::{ReadStorage as _, sqlite::SqliteStorage}`. `load_explorer_state` calls `load_scan_metadata` and `query_entries`, both on `ReadStorage`.

- [ ] **Step 7: Update `tests/cli_integration.rs`**

- Import (line 9): change `use nixdirstat::storage::{Storage, sqlite::SqliteStorage}` to `use nixdirstat::storage::{ReadStorage, sqlite::SqliteStorage}`. The test uses `storage.query_entries()` which is on `ReadStorage`.

- [ ] **Step 8: Run full verification**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- [ ] **Step 9: Commit**

```bash
git add src/storage/mod.rs src/storage/sqlite.rs src/analyzer/mod.rs src/pipeline.rs src/lib.rs src/ui/mod.rs tests/cli_integration.rs
git commit -m "refactor(storage): split Storage trait into ReadStorage / WriteStorage

ReadStorage exposes query methods; WriteStorage extends it with mutation
methods. Explorer UI now takes ReadStorage (least privilege). Move
finalize_for_export to SqliteStorage inherent method (SQLite-specific).

Closes #19"
```

---

### Task 4: Encapsulate `ExplorerState` and add `ScanProgressState::new`

**Files:**
- Modify: `src/ui/app.rs` — make fields private, add getters/setters, add `ScanProgressState::new`
- Modify: `src/ui/views/explorer.rs` — use getters
- Modify: `src/ui/mod.rs:130-136,300-376,390-393` — use constructor and setters

**Interfaces:**
- Produces:
  - `ExplorerState::tree(&self) -> &DirNode`
  - `ExplorerState::tree_state(&self) -> &TreeState<String>`
  - `ExplorerState::tree_state_mut(&mut self) -> &mut TreeState<String>`
  - `ExplorerState::treemap_root(&self) -> &[String]`
  - `ExplorerState::treemap_state_mut(&mut self) -> &mut TreemapState`
  - `ExplorerState::extension_stats(&self) -> &[ExtensionStat]`
  - `ExplorerState::sort_field(&self) -> TreeSortField`
  - `ExplorerState::sort_ascending(&self) -> bool`
  - `ExplorerState::show_help(&self) -> bool`
  - `ExplorerState::show_info(&self) -> bool`
  - `ExplorerState::error_message(&self) -> Option<&str>`
  - `ExplorerState::legend_scroll(&self) -> usize`
  - `ExplorerState::scan_root(&self) -> &Path`
  - `ExplorerState::free_space(&self) -> Option<&SpaceInfo>`
  - `ExplorerState::set_free_space(&mut self, space: Option<SpaceInfo>)`
  - `ExplorerState::toggle_show_help(&mut self)`
  - `ExplorerState::toggle_show_info(&mut self)`
  - `ExplorerState::clear_error(&mut self)`
  - `ExplorerState::sync_treemap_highlight(&mut self)`
  - `ScanProgressState::new(is_root: bool) -> Self`

- [ ] **Step 1: Add `ScanProgressState::new` in `src/ui/app.rs`**

```rust
pub fn new(is_root: bool) -> Self {
    Self {
        file_count: 0,
        files_per_sec: 0.0,
        elapsed: Duration::ZERO,
        current_path: PathBuf::new(),
        is_root,
    }
}
```

Keep `ScanProgressState` fields public for now (the `update` method and `render_progress` read them directly; encapsulating those is lower priority and not in scope).

- [ ] **Step 2: Make `ExplorerState` fields private and add getters/setters in `src/ui/app.rs`**

Remove `pub` from all fields in `ExplorerState` (lines 68-96). Add getters and setters per Interfaces above. Key details:
- `tree_state_mut` and `treemap_state_mut` return `&mut` references since external code calls methods on these widget states.
- `error_message(&self) -> Option<&str>` — use `self.error_message.as_deref()`.
- `clear_error(&mut self)` — sets `self.error_message = None`.
- `sync_treemap_highlight(&mut self)` — reads `self.tree_state.selected()`, sets `self.treemap_state.highlighted_path` to `Some(selected.to_vec())` if non-empty, else `None`.
- `free_space(&self) -> Option<&SpaceInfo>` — returns `self.free_space.as_ref()`.

- [ ] **Step 3: Update `src/ui/views/explorer.rs`**

Replace all direct field accesses with getter calls:
- `state.breadcrumb_path()` — already a method, no change.
- `state.free_space` (line 43) -> `state.free_space()`.
- `state.tree` (lines 85, 86, 122) -> `state.tree()`.
- `state.treemap_root` (lines 85, 122, 159) -> `state.treemap_root().to_vec()` where ownership needed.
- `state.tree_state` (line 155) -> `state.tree_state()`.
- `&mut state.tree_state` (line 92) -> `state.tree_state_mut()`.
- `&mut state.treemap_state` (line 124) -> `state.treemap_state_mut()`.
- `state.sort_field` (line 96) -> `state.sort_field()`.
- `state.sort_ascending` (line 96) -> `state.sort_ascending()`.
- `state.extension_stats` (line 107) -> `state.extension_stats()`.
- `state.legend_scroll` (line 110) -> `state.legend_scroll()`.
- `state.show_info` (line 129) -> `state.show_info()`.
- `state.show_help` (line 135) -> `state.show_help()`.
- `state.error_message` (line 139) -> `state.error_message()`.
- `state.scan_root` (line 237-239) -> `state.scan_root()`.
- `state.treemap_root` (line 238) -> `state.treemap_root()`.

- [ ] **Step 4: Update `src/ui/mod.rs`**

- `ScanProgressState` struct literal (lines 130-136): replace with `ScanProgressState::new(is_root)`.
- `handle_explorer_event` (lines 300-376):
  - `state.error_message = None` (line 313) -> `state.clear_error()`.
  - `state.tree_state.key_up()` etc. -> `state.tree_state_mut().key_up()`.
  - `state.tree_state.selected()` -> `state.tree_state().selected()` (immutable access).
  - `state.tree_state.key_right()` / `key_left()` / `key_down()` -> `state.tree_state_mut().*`.
  - `state.tree_state.select_relative(...)` -> `state.tree_state_mut().select_relative(...)`.
  - `state.tree_state.select_first()` / `select_last()` -> `state.tree_state_mut().*`.
  - `state.show_info = !state.show_info` (line 362) -> `state.toggle_show_info()`.
  - `state.show_help = !state.show_help` (line 365) -> `state.toggle_show_help()`.
  - Lines 370-375 (treemap highlight sync) -> `state.sync_treemap_highlight()`.
- `load_explorer_state` (line 391): `state.free_space = free_space` -> `state.set_free_space(free_space)`.

- [ ] **Step 5: Run full verification**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- [ ] **Step 6: Commit**

```bash
git add src/ui/app.rs src/ui/views/explorer.rs src/ui/mod.rs
git commit -m "refactor(ui): encapsulate ExplorerState internals

Make ExplorerState fields private. Add getters, guarded setters
(toggle_show_help, clear_error, sync_treemap_highlight), and
ScanProgressState::new constructor.

Closes #18"
```

---

### Task 5: Final verification and branch push

**Files:** None — verification only.

- [ ] **Step 1: Run full CI-parity check**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo doc --no-deps
```

- [ ] **Step 2: Run VHS visual tests**

```bash
just vhs
```

Inspect screenshots in `tests/vhs/screenshots/` for TUI regressions.

- [ ] **Step 3: Verify all four issues are addressed**

Quick checklist:
- `FileEntry` fields are private with getters (#17) ✓
- Duplicated test helpers removed, `compute_type_stats` removed (#25) ✓
- `Storage` split into `ReadStorage`/`WriteStorage` (#19) ✓
- `ExplorerState` fields private with getters/setters (#18) ✓
