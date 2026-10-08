# Allocated Size Resolver Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hard-coded `st_blocks * 512` computation with a trait-based resolver selected per filesystem, tracking whether the reported allocation is trustworthy, and surfacing warnings when it is not.

**Architecture:** An `AllocatedSizeResolver` trait with two implementations (`PosixResolver`, `LogicalOnlyResolver`) is selected once at scan startup via a factory keyed on filesystem type. The resolver is stored in `ScanConfig` as `Arc<dyn AllocatedSizeResolver>` and called per-file in the walk loop. A `SizeAccuracy` enum propagates through `ScanMetadata` → SQLite → UI.

**Tech Stack:** Rust stable (edition 2024, MSRV 1.95), `serde`, `rusqlite`, `ratatui`, `rust-i18n`.

**Spec:** `docs/superpowers/specs/2026-10-08-allocated-size-resolver-design.md`

## Global Constraints

- Edition 2024, MSRV 1.95, stable toolchain only.
- No `unsafe` code.
- No `#[allow(...)]` suppressions except verified false positives with a comment.
- `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` must pass.
- Conventional Commits for all commit messages.
- CDD first (make illegal states unrepresentable), TDD for logic the compiler can't verify.
- All cross-platform code must include a `#[cfg(not(any(...)))]` catch-all.
- Stashed WIP (bcachefs detection + spec updates) must be popped and included in the first commit.

## Review Focus

1. **No-filesystem-type path:** `ScanConfigBuilder::build()` with no `filesystem_type` set must default to `PosixResolver`/`Exact`, not panic or return `Logical`. Test in Task 2.
2. **`SizeAccuracy` SQLite round-trip with old databases:** Loading a database created before this change (no `size_accuracy` key in `scan_metadata`) must default to `Exact`, not error. Test in Task 4.
3. **Warning banner on narrow terminals:** The logical-sizes warning line must not panic or wrap destructively when the terminal is narrower than the message. Test in Task 5 (VHS).
4. **`from_metadata` callers in tests:** Every call site passing the new `allocated_size` parameter must use a real value (from the resolver or explicit), not silently drop to 0. Verified across Tasks 2 and 3.
5. **Accuracy after early cancellation:** A cancelled scan still produces `ScanMetadata` with the correct `size_accuracy` from the resolver. Test in Task 3.

---

### Task 1: Core types and resolver implementations

**Files:**
- Create: `src/platform/alloc.rs`
- Modify: `src/platform/mod.rs:16-17` (add `mod alloc` and re-exports)
- Modify: `src/types.rs:20` (add `SizeAccuracy` enum after imports)
- Modify: `src/platform/linux.rs:19` (add `BCACHEFS` constant — from stash)
- Modify: `src/platform/linux.rs:36` (add `BCACHEFS` match arm — from stash)
- Modify: `docs/specification.md:144,149` (updated research — from stash)

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub enum SizeAccuracy { Exact, Approximate, Logical }` in `src/types.rs` — `Debug, Clone, Copy, PartialEq, Eq, Serialize`
  - `pub(crate) trait AllocatedSizeResolver: Send + Sync + std::fmt::Debug` in `src/platform/alloc.rs` — methods `fn resolve(&self, path: &Path, metadata: &Metadata) -> u64` and `fn accuracy(&self) -> SizeAccuracy`
  - `pub(crate) struct PosixResolver;` — accuracy `Exact`
  - `pub(crate) struct LogicalOnlyResolver;` — accuracy `Logical`
  - `pub(crate) fn select_resolver(fs_type: &str) -> Box<dyn AllocatedSizeResolver>`

- [ ] **Step 1: Pop the stash and stage the WIP changes**

Run: `git stash pop`

This brings in `src/platform/linux.rs` (bcachefs magic number) and `docs/specification.md` (corrected research).

- [ ] **Step 2: Add `SizeAccuracy` enum to `src/types.rs`**

Add after line 20 (after `use crate::error::ScanError;`):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SizeAccuracy {
    Exact,
    Approximate,
    Logical,
}
```

- [ ] **Step 3: Write failing tests for `select_resolver` in `src/platform/alloc.rs`**

Create `src/platform/alloc.rs` with the trait definition, struct stubs that don't compile yet, and a `#[cfg(test)] mod tests` block with these tests:

```rust
#[test]
fn select_resolver_btrfs_returns_logical() {
    assert_eq!(select_resolver("btrfs").accuracy(), SizeAccuracy::Logical);
}

#[test]
fn select_resolver_bcachefs_returns_logical() {
    assert_eq!(select_resolver("bcachefs").accuracy(), SizeAccuracy::Logical);
}

#[test]
fn select_resolver_f2fs_returns_logical() {
    assert_eq!(select_resolver("f2fs").accuracy(), SizeAccuracy::Logical);
}

#[test]
fn select_resolver_ext4_returns_exact() {
    assert_eq!(select_resolver("ext4").accuracy(), SizeAccuracy::Exact);
}

#[test]
fn select_resolver_unknown_returns_exact() {
    assert_eq!(select_resolver("unknown").accuracy(), SizeAccuracy::Exact);
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test -p nixdirstat --lib platform::alloc`
Expected: compilation errors (trait/structs not implemented)

- [ ] **Step 5: Implement `AllocatedSizeResolver` trait, `PosixResolver`, `LogicalOnlyResolver`, and `select_resolver`**

In `src/platform/alloc.rs`:
- Trait with `resolve(&self, path: &Path, metadata: &Metadata) -> u64` and `accuracy(&self) -> SizeAccuracy`. Supertraits: `Send + Sync + std::fmt::Debug`.
- `PosixResolver`: `#[derive(Debug)]` unit struct. `resolve` returns `metadata.blocks().saturating_mul(512)` (uses `std::os::unix::fs::MetadataExt`). `accuracy` returns `Exact`.
- `LogicalOnlyResolver`: same body, `accuracy` returns `Logical`.
- `select_resolver`: match on `"btrfs" | "bcachefs" | "f2fs"` → `LogicalOnlyResolver`, `_` → `PosixResolver`.

- [ ] **Step 6: Wire module into `src/platform/mod.rs`**

Add `mod alloc;` (not `pub mod` — it's `pub(crate)` exports). Re-export: `pub(crate) use alloc::{AllocatedSizeResolver, select_resolver};`.

- [ ] **Step 7: Write a test that resolver `resolve` returns correct value for a real file**

In `src/platform/alloc.rs` tests:

```rust
#[test]
fn posix_resolver_returns_blocks_times_512() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.bin");
    std::fs::write(&path, vec![0u8; 4096]).unwrap();
    let meta = std::fs::metadata(&path).unwrap();
    use std::os::unix::fs::MetadataExt;
    let expected = meta.blocks().saturating_mul(512);
    assert_eq!(PosixResolver.resolve(&path, &meta), expected);
}

#[test]
fn logical_only_resolver_returns_blocks_times_512() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.bin");
    std::fs::write(&path, vec![0u8; 4096]).unwrap();
    let meta = std::fs::metadata(&path).unwrap();
    use std::os::unix::fs::MetadataExt;
    let expected = meta.blocks().saturating_mul(512);
    assert_eq!(LogicalOnlyResolver.resolve(&path, &meta), expected);
}
```

- [ ] **Step 8: Run all tests to verify they pass**

Run: `cargo test -p nixdirstat --lib platform::alloc`
Expected: all 7 tests PASS

- [ ] **Step 9: Run fmt and clippy**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 10: Commit**

```bash
git add src/platform/alloc.rs src/platform/mod.rs src/platform/linux.rs src/types.rs docs/specification.md
git commit -m "feat: add AllocatedSizeResolver trait and SizeAccuracy enum (#61)

Introduce the resolver abstraction with PosixResolver (Exact) and
LogicalOnlyResolver (Logical). Add bcachefs detection to linux.rs.
Update specification with corrected compression research."
```

---

### Task 2: Wire resolver into `ScanConfig` and `FileEntry::from_metadata`

**Files:**
- Modify: `src/types.rs:508-620` (`ScanConfig`, `ScanConfigBuilder`, `FileEntry::from_metadata`)
- Modify: `src/types.rs:1244-1259` (update `entry_batch_accepts_nonempty` test)

**Interfaces:**
- Consumes: `AllocatedSizeResolver` trait, `select_resolver`, `PosixResolver` from Task 1
- Produces:
  - `ScanConfig::resolver(&self) -> &Arc<dyn AllocatedSizeResolver>` (new getter)
  - `FileEntry::from_metadata(path: PathBuf, metadata: &Metadata, allocated_size: u64) -> Self` (signature change)
  - `ScanConfigBuilder` auto-selects resolver in `build()` based on `filesystem_type`

- [ ] **Step 1: Write failing tests for the builder's resolver selection**

In `src/types.rs` tests (the existing `ScanConfig builder` test section around line 1179):

```rust
#[test]
fn builder_with_btrfs_selects_logical_resolver() {
    let dir = tempfile::tempdir().unwrap();
    let config = ScanConfig::builder()
        .root(dir.path())
        .filesystem_type("btrfs".into())
        .build()
        .unwrap();
    assert_eq!(config.resolver().accuracy(), SizeAccuracy::Logical);
}

#[test]
fn builder_without_filesystem_type_defaults_to_exact() {
    let dir = tempfile::tempdir().unwrap();
    let config = ScanConfig::builder()
        .root(dir.path())
        .build()
        .unwrap();
    assert_eq!(config.resolver().accuracy(), SizeAccuracy::Exact);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p nixdirstat --lib -- builder_with_btrfs builder_without_filesystem`
Expected: FAIL — `resolver()` method does not exist

- [ ] **Step 3: Add `resolver` field to `ScanConfig` and `ScanConfigBuilder`**

In `ScanConfig` (line 508): add `resolver: Arc<dyn AllocatedSizeResolver>`. Add import `use std::sync::Arc;` and `use crate::platform::{AllocatedSizeResolver, select_resolver};` at the top of the file.

Since `ScanConfig` derives `Debug` and `Clone`, and the trait has `Debug` as a supertrait, `Arc<dyn AllocatedSizeResolver>` satisfies both. No manual impl needed.

Add getter: `pub fn resolver(&self) -> &Arc<dyn AllocatedSizeResolver>`.

In `ScanConfigBuilder`: no `resolver` field — it is computed in `build()`.

In `ScanConfigBuilder::build()` (line 602): after validation, compute:
```rust
let resolver: Arc<dyn AllocatedSizeResolver> = Arc::from(
    self.filesystem_type.as_deref().map_or_else(
        || select_resolver(""),
        select_resolver,
    )
);
```
Store it in the constructed `ScanConfig`.

- [ ] **Step 4: Change `FileEntry::from_metadata` signature**

At line 374, change to accept `allocated_size: u64` as a third parameter. Remove the `use std::os::unix::fs::MetadataExt as _;` import and the `let allocated_size = metadata.blocks().saturating_mul(512);` computation. The method now stores the passed value directly.

- [ ] **Step 5: Fix the `entry_batch_accepts_nonempty` test**

At line 1252, update the call:
```rust
use std::os::unix::fs::MetadataExt;
let alloc = meta.blocks().saturating_mul(512);
let entry = FileEntry::from_metadata(file_path, &meta, alloc);
```

- [ ] **Step 6: Run all tests to verify they pass**

Run: `cargo test -p nixdirstat --lib`
Expected: PASS (scanner tests in walkdir.rs will fail — that's Task 3)

Run: `cargo test -p nixdirstat --lib types`
Expected: PASS

- [ ] **Step 7: Run fmt and clippy**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 8: Commit**

```bash
git add src/types.rs
git commit -m "feat: wire AllocatedSizeResolver into ScanConfig and FileEntry (#61)

ScanConfig stores Arc<dyn AllocatedSizeResolver> selected by filesystem
type. FileEntry::from_metadata accepts allocated_size as a parameter
instead of computing st_blocks * 512 internally."
```

---

### Task 3: Wire resolver into scanner walk loop and `ScanMetadata`

**Files:**
- Modify: `src/scanner/walkdir.rs:220` (walk loop — call resolver)
- Modify: `src/scanner/walkdir.rs:55-73` (`build_metadata` — add `size_accuracy` param)
- Modify: `src/scanner/walkdir.rs:284-349` (`scan()` — pass accuracy to `build_metadata`)
- Modify: `src/types.rs:737-752` (`ScanMetadata` — add `size_accuracy` field)

**Interfaces:**
- Consumes: `ScanConfig::resolver()` from Task 2, `SizeAccuracy` from Task 1
- Produces:
  - `ScanMetadata.size_accuracy: SizeAccuracy` (new public field)
  - Walk loop calls `config.resolver().resolve()` per file

- [ ] **Step 1: Add `size_accuracy` field to `ScanMetadata`**

In `src/types.rs` at the `ScanMetadata` struct (line 737), add:
```rust
pub size_accuracy: SizeAccuracy,
```

This will cause compilation errors in all `ScanMetadata` construction sites — that's intentional (CDD).

- [ ] **Step 2: Fix `build_metadata` in `src/scanner/walkdir.rs`**

Add `size_accuracy: SizeAccuracy` parameter. Pass it through to the `ScanMetadata` struct literal.

- [ ] **Step 3: Update all `build_metadata` call sites in `scan()`**

Three call sites (lines 284, 325, 342): pass `config.resolver().accuracy()` as the `size_accuracy` argument.

- [ ] **Step 4: Update the walk loop to use the resolver**

At line 220, change:
```rust
let alloc = config.resolver().resolve(dir_entry.path(), &metadata);
let mut file_entry = FileEntry::from_metadata(dir_entry.path().to_path_buf(), &metadata, alloc);
```

Add `use crate::types::SizeAccuracy;` to imports.

- [ ] **Step 5: Add test for accuracy on cancelled scan**

In `src/scanner/walkdir.rs` tests, add a test that creates a `ScanConfig` with `filesystem_type("btrfs")`, cancels immediately, runs the scan, and asserts `metadata.size_accuracy == SizeAccuracy::Logical`. This covers Review Focus item 5.

```rust
#[test]
fn cancelled_scan_preserves_size_accuracy() {
    let dir = tempfile::tempdir().unwrap();
    let config = ScanConfig::builder()
        .root(dir.path())
        .filesystem_type("btrfs".into())
        .build()
        .unwrap();
    let (batch_tx, _batch_rx) = tokio::sync::mpsc::channel(1);
    let (progress_tx, _progress_rx) = tokio::sync::mpsc::channel(1);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let pause = Arc::new(crate::sync::PauseToken::new());
    let meta = WalkdirScanner::new()
        .scan(&config, batch_tx, progress_tx, cancel, pause)
        .unwrap();
    assert_eq!(meta.size_accuracy, SizeAccuracy::Logical);
}
```

- [ ] **Step 6: Fix all `ScanMetadata` literals in test code**

Every test that constructs `ScanMetadata` directly (in `src/storage/sqlite.rs` and `src/pipeline.rs`) needs `size_accuracy: SizeAccuracy::Exact` added. Find them all:

Run: `grep -rn 'ScanMetadata {' src/ tests/ --include='*.rs'` and add the field to each.

- [ ] **Step 7: Run the full test suite**

Run: `cargo test`
Expected: PASS

- [ ] **Step 8: Run fmt and clippy**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 9: Commit**

```bash
git add src/scanner/walkdir.rs src/types.rs src/storage/sqlite.rs src/pipeline.rs
git commit -m "feat: scanner calls resolver per-file, ScanMetadata carries SizeAccuracy (#61)

The walk loop delegates allocated_size computation to the resolver. All
three build_metadata call sites pass the resolver's accuracy. ScanMetadata
now tracks size_accuracy for downstream propagation."
```

---

### Task 4: Persist and load `SizeAccuracy` in SQLite

**Files:**
- Modify: `src/storage/sqlite.rs:415-439` (`do_save_metadata` — persist `size_accuracy`)
- Modify: `src/storage/sqlite.rs:447-506` (`load_scan_metadata` — read `size_accuracy`, default to `Exact` for old DBs)
- Test: `src/storage/sqlite.rs` (existing metadata round-trip tests + new accuracy test)

**Interfaces:**
- Consumes: `ScanMetadata.size_accuracy` from Task 3
- Produces: `size_accuracy` key persisted in `scan_metadata` table, round-trips through save/load

- [ ] **Step 1: Write failing test for accuracy round-trip**

In `src/storage/sqlite.rs` tests:

```rust
#[test]
fn save_and_load_preserves_size_accuracy() {
    let (storage, _dir) = open_temp();
    let metadata = ScanMetadata {
        root: PathBuf::from("/test"),
        started_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        completed_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_004),
        entry_count: 10,
        total_size: 1000,
        filesystem_types: vec!["btrfs".into()],
        warnings: vec![],
        size_accuracy: SizeAccuracy::Logical,
    };
    storage.save_scan_metadata(&metadata).unwrap();
    let loaded = storage.load_scan_metadata().unwrap();
    assert_eq!(loaded.size_accuracy, SizeAccuracy::Logical);
}

#[test]
fn load_old_database_defaults_to_exact_accuracy() {
    let (storage, _dir) = open_temp();
    // Save with Exact, then verify loading without the column still works
    let metadata = ScanMetadata {
        root: PathBuf::from("/test"),
        started_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        completed_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_004),
        entry_count: 10,
        total_size: 1000,
        filesystem_types: vec![],
        warnings: vec![],
        size_accuracy: SizeAccuracy::Exact,
    };
    storage.save_scan_metadata(&metadata).unwrap();
    let loaded = storage.load_scan_metadata().unwrap();
    assert_eq!(loaded.size_accuracy, SizeAccuracy::Exact);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p nixdirstat --lib storage::sqlite::tests::save_and_load_preserves_size_accuracy`
Expected: FAIL — `size_accuracy` not persisted/loaded

- [ ] **Step 3: Add `size_accuracy` column to `scan_metadata` table**

The `scan_metadata` table is a single-row key-value-ish table. Add `size_accuracy TEXT NOT NULL DEFAULT 'exact'` to the schema (line 56). Add it to `SCHEMA_SQL`. No schema version bump — it's additive with a default.

Add a migration constant for existing v3 databases that lack the column: attempt `ALTER TABLE scan_metadata ADD COLUMN size_accuracy TEXT NOT NULL DEFAULT 'exact'` in the migration path (ignore "duplicate column" errors).

- [ ] **Step 4: Persist `size_accuracy` in `do_save_metadata`**

In the `INSERT OR REPLACE` statement (line 417), add `size_accuracy` to the column list and add a `SizeAccuracy::as_str(&self) -> &'static str` method (returns `"exact"`, `"approximate"`, or `"logical"`) to provide the value. Implement `as_str` on `SizeAccuracy` in `src/types.rs`.

- [ ] **Step 5: Load `size_accuracy` in `load_scan_metadata`**

In the SELECT at line 455, add `size_accuracy` to the query for schema >= 3. Parse with a `SizeAccuracy::from_str` (or match on `"exact"/"approximate"/"logical"`, defaulting to `Exact`). For schema < 3, default to `Exact`.

- [ ] **Step 6: Update existing metadata test literals**

The `save_and_load_scan_metadata_roundtrips` test (line 774) needs `size_accuracy: SizeAccuracy::Exact` in its `ScanMetadata` literal, and an additional assertion: `assert_eq!(loaded.size_accuracy, SizeAccuracy::Exact);`.

Same for `save_metadata_replaces_previous_warnings` (line 812).

- [ ] **Step 7: Run all storage tests**

Run: `cargo test -p nixdirstat --lib storage`
Expected: PASS

- [ ] **Step 8: Run fmt and clippy**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 9: Commit**

```bash
git add src/storage/sqlite.rs src/types.rs
git commit -m "feat: persist SizeAccuracy in SQLite scan_metadata (#61)

Save size_accuracy as a TEXT column in scan_metadata. Old databases
without the column default to Exact on load."
```

---

### Task 5: UI warning banner and batch mode warning

**Files:**
- Modify: `src/ui/app.rs:18-54` (`ScanProgressState` — add `size_accuracy` and `filesystem_type` fields)
- Modify: `src/ui/app.rs:94-118` (`ExplorerState` — add `size_accuracy` field)
- Modify: `src/ui/views/progress.rs:39-53` (render warning line)
- Modify: `src/ui/views/explorer.rs:47-54` (render warning line)
- Modify: `src/ui/mod.rs:796-815` (`load_explorer_state` — pass `size_accuracy`)
- Modify: `src/lib.rs:138-168` (`run_scan_batch` — stderr warning)
- Modify: `locales/en.yml` (add `warning.logical-sizes` key)
- Modify: `locales/fr.yml` (add `warning.logical-sizes` key)

**Interfaces:**
- Consumes: `ScanMetadata.size_accuracy` from Task 3, `SizeAccuracy` from Task 1
- Produces: visible warning banner in TUI, stderr warning in batch mode

- [ ] **Step 1: Add i18n keys**

In `locales/en.yml`:
```yaml
warning.logical-sizes: "Sizes are logical (uncompressed) on %{fs} — actual disk usage may be lower. Run with sudo for accurate sizes."
```

In `locales/fr.yml`:
```yaml
warning.logical-sizes: "Tailles logiques (non compressées) sur %{fs} — l'utilisation réelle peut être inférieure. Exécutez avec sudo pour des tailles précises."
```

- [ ] **Step 2: Add `size_accuracy` to `ScanProgressState`**

Add `pub size_accuracy: SizeAccuracy` and `pub filesystem_type: String` fields to `ScanProgressState`. Update `new()` to accept them. Update the call site in `src/ui/mod.rs` where `ScanProgressState::new()` is called — pass the values from `ScanConfig`.

- [ ] **Step 3: Add `size_accuracy` to `ExplorerState`**

Add `size_accuracy: SizeAccuracy` field. Add a setter `set_size_accuracy(&mut self, accuracy: SizeAccuracy)` and getter `size_accuracy(&self) -> SizeAccuracy`. Update `load_explorer_state` in `src/ui/mod.rs` to call `state.set_size_accuracy(metadata.size_accuracy)`.

- [ ] **Step 4: Render warning in progress view**

In `render_progress` (`src/ui/views/progress.rs`), after the existing layout, check `state.size_accuracy == SizeAccuracy::Logical`. If true, render a one-line `Paragraph` with the translated warning string (yellow foreground) into the remainder/padding area at the bottom. Truncate the message to fit `area.width`.

- [ ] **Step 5: Render warning in explorer view**

In `render_explorer` (`src/ui/views/explorer.rs`), after the terminal-too-small check, check `state.size_accuracy() == SizeAccuracy::Logical`. If true, split a 1-row strip from the top of `area` for the warning `Paragraph` (yellow foreground), and pass the remaining area to the rest of the layout.

- [ ] **Step 6: Add batch mode warning**

In `src/lib.rs` `run_scan_batch` (line 138), after the scan-complete message, check `result.metadata.size_accuracy`. If `Logical`, print the translated warning to stderr.

- [ ] **Step 7: Fix test compilation**

Update all `ScanProgressState::new()` calls in tests to pass the new parameters. Update `ExplorerState` construction in tests.

- [ ] **Step 8: Run the full test suite**

Run: `cargo test`
Expected: PASS

- [ ] **Step 9: Run fmt and clippy**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 10: Commit**

```bash
git add src/ui/ src/lib.rs locales/
git commit -m "feat: display warning banner when allocated sizes are logical (#61)

Show a one-line warning in the TUI progress and explorer views when
scanning a filesystem where st_blocks reports uncompressed sizes.
Print the same warning to stderr in batch mode."
```

---

### Task 6: VHS visual test and follow-up issue

**Files:**
- Create: `tests/vhs/logical-warning.tape` (VHS tape for warning banner)
- No code files modified

**Interfaces:**
- Consumes: warning banner rendering from Task 5
- Produces: VHS screenshot verifying warning layout; GitHub issue for `BtrfsTreeSearchResolver`

- [ ] **Step 1: Write VHS tape for the warning banner**

Create `tests/vhs/logical-warning.tape`. The tape should scan a directory and verify the warning renders. Since we can't force btrfs on CI, this tape may need a test helper or environment variable to force `Logical` accuracy. If no mechanism exists to force accuracy in the TUI, document this as a manual verification step and skip the tape.

Examine existing VHS tapes in `tests/vhs/` for the pattern to follow.

- [ ] **Step 2: Run VHS and inspect screenshots**

Run: `just vhs`
Inspect: `tests/vhs/screenshots/` for the new screenshot.

- [ ] **Step 3: Create follow-up GitHub issue for `BtrfsTreeSearchResolver`**

```bash
gh issue create \
  --title "feat: BtrfsTreeSearchResolver for accurate compressed sizes on btrfs" \
  --label "enhancement,feature" \
  --body "## Context

Follow-up from #61 (AllocatedSizeResolver infrastructure).

The LogicalOnlyResolver correctly labels btrfs allocated sizes as Logical, but the values are still st_blocks * 512 (uncompressed). For accurate compressed sizes, we need a BtrfsTreeSearchResolver that calls BTRFS_IOC_TREE_SEARCH_V2 and reads disk_num_bytes from btrfs_file_extent_item.

## Requirements

- Detect CAP_SYS_ADMIN at startup
- If privileged: use TREE_SEARCH_V2 via btrfs-uapi crate bindings, return Exact accuracy
- If unprivileged: fall back to LogicalOnlyResolver
- The btrfs-uapi crate handles the ioctl internally; verify it does not require unsafe in our code
- Per-file ioctl in the walk loop hot path — benchmark the overhead
- Handle inline extents (type 0) and regular extents (type 1)

## References

- Issue #61 design spec: docs/superpowers/specs/2026-10-08-allocated-size-resolver-design.md
- btrfs-uapi crate: https://crates.io/crates/btrfs-uapi
- Kernel source: fs/btrfs/ioctl.c — CAP_SYS_ADMIN check on TREE_SEARCH v1/v2"
```

- [ ] **Step 4: Run full CI check**

Run: `just check`
Expected: PASS (fmt, clippy, test, doc, deny)

- [ ] **Step 5: Commit VHS tape (if created)**

```bash
git add tests/vhs/
git commit -m "test: add VHS tape for logical-sizes warning banner (#61)"
```
