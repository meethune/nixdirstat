# Refactor: Encapsulation and DRY Cleanup

Addresses GitHub issues #17, #18, #19, #25.

## Goal

Restructure three core interfaces (`FileEntry`, `Storage`, `ExplorerState`) to enforce invariants at the type level, narrow capability surfaces, and eliminate duplicated code — all without changing observable behavior.

## Commit Order

Each commit compiles and passes tests independently. Order reflects the dependency chain.

### Commit 1: Encapsulate `FileEntry` (#17 + partial #25)

**Problem:** All 12 `FileEntry` fields are `pub`. Only `allocated_size` needs post-construction mutation (hardlink dedup). Constructor invariants (`mode` <-> `file_type`, `path` <-> `category`) can be bypassed via struct literals.

**Changes:**

1. **Make all fields private** in `FileEntry` (types.rs:299-324).

2. **Add public getters** — zero-cost, returning references or `Copy` types:
   - `path() -> &Path`
   - `size() -> u64`
   - `allocated_size() -> u64`
   - `file_type() -> FileType`
   - `category() -> FileCategory`
   - `inode() -> u64`
   - `device() -> u64`
   - `nlink() -> u64`
   - `uid() -> u32`
   - `gid() -> u32`
   - `mtime() -> SystemTime`
   - `mode() -> u32`

3. **Add `set_allocated_size(&mut self, size: u64)`** for hardlink dedup (scanner/walkdir.rs:87-99).

4. **Add `display_name(&self) -> String`** — extracts the filename component from `path`, falling back to `path.display().to_string()`. Replaces the duplicated pattern in `ui/tree.rs:55-57` (build_tree root name) and wherever else filename extraction appears.

5. **Add `debug_assert!(path.is_absolute())` in `from_metadata`.**

6. **Add `#[cfg(test)] pub struct FileEntryBuilder`** in `types.rs`:
   - Defaults: path="/test", size=0, allocated_size=0, file_type=Regular, category=NoExtension, inode=0, device=0, nlink=1, uid=0, gid=0, mtime=UNIX_EPOCH, mode=0o644.
   - Builder methods: `.path()`, `.size()`, `.allocated_size()`, `.file_type()`, `.category()`, `.mode()`, `.build() -> FileEntry`.
   - No validation — test-only, intentionally permissive.

7. **Update all production call sites** (~60 sites across 7 files):
   - `scanner/walkdir.rs`: `file_entry.allocated_size = 0` -> `file_entry.set_allocated_size(0)`. `file_entry.path.clone()` -> `file_entry.path().to_path_buf()`. `file_entry.size` -> `file_entry.size()`.
   - `storage/sqlite.rs`: `row_to_entry` constructs via `FileEntry::from_metadata` or a new `pub(crate) fn from_row(...)` constructor. All field reads in `do_insert_entries` use getters. `entry_parent` and `entry_mtime_secs` use getters.
   - `ui/tree.rs`: `entry.path`, `entry.size`, `entry.allocated_size`, `entry.file_type`, `entry.mtime` -> getters.
   - `lib.rs`: `write_entries_csv` and `write_entries_json` use getters.
   - `analyzer/mod.rs`: `entry.file_type`, `entry.path`, `entry.size`, `entry.allocated_size`, `entry.category` -> getters.

   **Note on `storage/sqlite.rs` `row_to_entry`:** This function reconstructs a `FileEntry` from database columns. It cannot use `from_metadata` (no `std::fs::Metadata`). Add a `pub(crate)` constructor:
   ```rust
   pub(crate) fn from_raw(
       path: PathBuf, size: u64, allocated_size: u64,
       file_type: FileType, category: FileCategory,
       inode: u64, device: u64, nlink: u64,
       uid: u32, gid: u32, mtime: SystemTime, mode: u32,
   ) -> Self
   ```

8. **Update all test call sites** to use `FileEntryBuilder`.

**Files modified:** `types.rs`, `scanner/walkdir.rs`, `storage/sqlite.rs`, `storage/mod.rs`, `analyzer/mod.rs`, `ui/tree.rs`, `lib.rs`.

### Commit 2: DRY Cleanup (#25 remainder)

**Problem:** Four separate `make_entry` / `make_file` / `make_dir` test helpers exist in `analyzer/mod.rs:166-215`, `storage/sqlite.rs:506-538`, `ui/tree.rs:196-215`, plus `tests/common/mod.rs` (integration only). Also `compute_type_stats()` in `analyzer/mod.rs:92-114` is dead code.

**Changes:**

1. **Replace all duplicated test helpers** with `FileEntryBuilder` from commit 1. Remove `make_entry`, `make_file`, `make_dir`, `make_file_with_ext`, `make_entry_at` from each test module. Tests call `FileEntryBuilder::new().path("/d/a").size(10).build()` instead.

2. **Remove `compute_type_stats()`** from `analyzer/mod.rs`. It duplicates `SqliteStorage::query_type_stats()` and has zero production callers. Remove its tests too (`type_stats_groups_by_category`, `type_stats_sorted_by_size_desc`, `type_stats_skips_non_regular_entries`). Update the module doc comment to remove the reference.

**Files modified:** `analyzer/mod.rs`, `storage/sqlite.rs`, `ui/tree.rs`.

### Commit 3: Split `Storage` Trait (#19)

**Problem:** The `Storage` trait bundles 10 methods covering both read and write operations. The explore UI only needs read access but receives mutation capabilities. `finalize_for_export()` is SQLite-specific.

**Changes:**

1. **Define `ReadStorage` trait** in `storage/mod.rs`:
   ```rust
   pub trait ReadStorage {
       fn load_scan_metadata(&self) -> Result<ScanMetadata, StorageError>;
       fn query_entries(&self, query: &EntryQuery) -> Result<Vec<FileEntry>, StorageError>;
       fn query_directory_children(&self, path: &Path) -> Result<Vec<FileEntry>, StorageError>;
       fn query_top_n_by_size(&self, n: usize) -> Result<Vec<FileEntry>, StorageError>;
       fn query_type_stats(&self) -> Result<Vec<TypeStat>, StorageError>;
   }
   ```

2. **Define `WriteStorage: ReadStorage` trait** in `storage/mod.rs`:
   ```rust
   pub trait WriteStorage: ReadStorage {
       fn init_schema(&mut self) -> Result<(), StorageError>;
       fn insert_batch(&self, batch: &EntryBatch) -> Result<(), StorageError>;
       fn save_scan_metadata(&self, metadata: &ScanMetadata) -> Result<(), StorageError>;
       fn update_directory_sizes(&self, sizes: &HashMap<PathBuf, DirectoryStats>) -> Result<(), StorageError>;
   }
   ```

3. **Remove the old `Storage` trait.** Implement `ReadStorage` and `WriteStorage` for `SqliteStorage`.

4. **Move `finalize_for_export()`** off the trait, keep it as an inherent method on `SqliteStorage`.

5. **Update consumers:**
   - `pipeline.rs:215`: calls `storage.finalize_for_export()` — change from `&dyn Storage` to concrete `SqliteStorage` (it already is).
   - `analyzer/mod.rs:35`: `aggregate_directory_sizes(storage: &dyn Storage)` — needs both read (`query_entries`) and write (`update_directory_sizes`). Change to `&dyn WriteStorage`.
   - `ui/mod.rs:380-388`: `load_explorer_state` — uses `load_scan_metadata` + `query_entries` (read only). Change to `&dyn ReadStorage`. Already uses concrete `SqliteStorage`, which implements `ReadStorage`.
   - `lib.rs:212-216`: export command — uses `query_entries` (read only). Change to `&dyn ReadStorage`.

6. **Update re-exports in `lib.rs`** — export `ReadStorage` and `WriteStorage` instead of `Storage`.

**Files modified:** `storage/mod.rs`, `storage/sqlite.rs`, `pipeline.rs`, `analyzer/mod.rs`, `ui/mod.rs`, `lib.rs`.

### Commit 4: Encapsulate `ExplorerState` (#18)

**Problem:** `ExplorerState` has all-pub fields. `treemap_root` can be manipulated to break navigation, `tree_state` and `treemap_state` can be desynchronized, and `show_help`/`show_info`/`error_message` lack any guarding. `ScanProgressState` has no constructor.

**Changes:**

1. **Make fields private** in `ExplorerState` (app.rs:67-96). All fields become private.

2. **Add getters:**
   - `tree(&self) -> &DirNode`
   - `tree_state(&self) -> &TreeState<String>`
   - `tree_state_mut(&mut self) -> &mut TreeState<String>`
   - `treemap_root(&self) -> &[String]`
   - `treemap_state(&self) -> &TreemapState`
   - `treemap_state_mut(&mut self) -> &mut TreemapState`
   - `extension_stats(&self) -> &[ExtensionStat]`
   - `focus(&self) -> PanelFocus`
   - `sort_field(&self) -> TreeSortField`
   - `sort_ascending(&self) -> bool`
   - `show_help(&self) -> bool`
   - `show_info(&self) -> bool`
   - `error_message(&self) -> Option<&str>`
   - `legend_scroll(&self) -> usize`
   - `scan_root(&self) -> &Path`
   - `free_space(&self) -> Option<&SpaceInfo>`

3. **Add setters:**
   - `set_free_space(&mut self, space: Option<SpaceInfo>)`
   - `set_show_help(&mut self, show: bool)`
   - `toggle_show_help(&mut self)`
   - `set_show_info(&mut self, show: bool)`
   - `toggle_show_info(&mut self)`
   - `set_error_message(&mut self, msg: Option<String>)`
   - `clear_error(&mut self)` (sugar for `set_error_message(None)`)
   - `set_legend_scroll(&mut self, offset: usize)`
   - `sync_treemap_highlight(&mut self)` — reads `tree_state.selected()` and writes `treemap_state.highlighted_path`.

4. **Add `ScanProgressState::new(is_root: bool) -> Self`** with zero/default fields. Update the struct literal in `ui/mod.rs:130-136` to use this constructor.

5. **Update call sites:**
   - `explorer.rs:34-150`: `render_explorer` — all field accesses go through getters. `state.breadcrumb_path()` already exists. `state.tree` -> `state.tree()`, `state.tree_state` -> `state.tree_state_mut()`, etc.
   - `ui/mod.rs:300-376`: `handle_explorer_event` — `state.error_message = None` -> `state.clear_error()`, `state.show_info = !state.show_info` -> `state.toggle_show_info()`, `state.treemap_state.highlighted_path = ...` -> `state.sync_treemap_highlight()`.
   - `ui/mod.rs:390-393`: `load_explorer_state` — `state.free_space = free_space` -> `state.set_free_space(free_space)`.

**Files modified:** `ui/app.rs`, `ui/views/explorer.rs`, `ui/mod.rs`.

## Testing Strategy

- **CDD first:** Each commit's type changes cause compiler errors at every site that needs updating. Fix all errors before running tests.
- **Existing tests pass unchanged** (modulo switching to `FileEntryBuilder`). No new behavioral tests needed — these are pure refactors.
- **`cargo fmt --check`** and **`cargo clippy --all-targets -- -D warnings`** after each commit.
- **VHS visual tests** (`just vhs`) after commit 4 to confirm no TUI regressions.

## Out of Scope

- New features, performance changes, or behavioral modifications.
- `FileEntry` field validation beyond `debug_assert`.
- Splitting `ExplorerState` into sub-structs (future work).
