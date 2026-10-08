# Allocated Size Resolver Design

**Issue:** [#61 — Accurate on-disk allocation reporting](https://github.com/meethune/nixdirstat/issues/61)
**Related:** [#77 — POSIX stat assumptions tracking](https://github.com/meethune/nixdirstat/issues/77) (H2: sparse detection must not conflict)
**Date:** 2026-10-08

## Problem

`st_blocks * 512` misrepresents actual on-disk usage on filesystems with transparent compression (btrfs, bcachefs, f2fs). The value reports logical (uncompressed) blocks, not physical allocation. A 1MB file compressed to 32KB on disk is reported as 1MB. nixdirstat currently computes `allocated_size` this way unconditionally.

This is not a collection of filesystem edge cases — it is one problem: `st_blocks` is not on-disk allocation on an increasing number of common filesystems.

## Approach

Trait object per scan. An `AllocatedSizeResolver` trait selected once at scan startup based on detected filesystem type. The scanner holds an `Arc<dyn AllocatedSizeResolver>` and calls it per-file instead of computing `st_blocks * 512` inline. A `SizeAccuracy` enum tracks whether the returned value is trustworthy, and propagates to the UI and export metadata.

## Core Types

### `SizeAccuracy`

Location: `src/types.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SizeAccuracy {
    /// st_blocks is ground truth (ext4, xfs, ZFS, NTFS3, tmpfs)
    Exact,
    /// st_blocks is approximate (future: FIEMAP-based estimates)
    Approximate,
    /// st_blocks reports logical/uncompressed blocks (btrfs, bcachefs, f2fs)
    Logical,
}
```

### `AllocatedSizeResolver`

Location: `src/platform/alloc.rs`, re-exported through `src/platform/mod.rs`

```rust
pub(crate) trait AllocatedSizeResolver: Send + Sync + std::fmt::Debug {
    fn resolve(&self, path: &Path, metadata: &Metadata) -> u64;
    fn accuracy(&self) -> SizeAccuracy;
}
```

`Send + Sync` because the scanner runs in `spawn_blocking`. `Debug` so `ScanConfig` can continue to derive `Debug`. `pub(crate)` — internal API, not part of the library surface.

## Resolver Implementations

Both in `src/platform/alloc.rs`.

### `PosixResolver`

For filesystems where `st_blocks` is ground truth: ext4, xfs, ZFS, NTFS3, tmpfs, vfat, NFS, unknown.

```rust
struct PosixResolver;

impl AllocatedSizeResolver for PosixResolver {
    fn resolve(&self, _path: &Path, metadata: &Metadata) -> u64 {
        metadata.blocks().saturating_mul(512)
    }
    fn accuracy(&self) -> SizeAccuracy { SizeAccuracy::Exact }
}
```

### `LogicalOnlyResolver`

For filesystems where `st_blocks` reports uncompressed logical blocks: btrfs, bcachefs, f2fs.

```rust
struct LogicalOnlyResolver;

impl AllocatedSizeResolver for LogicalOnlyResolver {
    fn resolve(&self, _path: &Path, metadata: &Metadata) -> u64 {
        metadata.blocks().saturating_mul(512)
    }
    fn accuracy(&self) -> SizeAccuracy { SizeAccuracy::Logical }
}
```

Same computation, different accuracy tag. The system now knows it cannot trust the value and can tell the user.

### Factory function

```rust
pub(crate) fn select_resolver(fs_type: &str) -> Box<dyn AllocatedSizeResolver> {
    match fs_type {
        "btrfs" | "bcachefs" | "f2fs" => Box::new(LogicalOnlyResolver),
        _ => Box::new(PosixResolver),
    }
}
```

Called once at scan startup. Returns `Box` which is wrapped in `Arc` when stored in `ScanConfig`.

### Future: `BtrfsTreeSearchResolver` (separate issue)

Will hold a cached root fd and call `BTRFS_IOC_TREE_SEARCH_V2` per file, returning `Exact`. Requires privilege detection at construction time and falls back to `LogicalOnlyResolver` if unprivileged. Not in this PR — tracked as a dedicated follow-up issue.

## Scanner Integration

### `ScanConfig` changes

```rust
pub struct ScanConfig {
    root: PathBuf,
    cross_device: bool,
    batch_size: usize,
    filesystem_type: Option<String>,
    resolver: Arc<dyn AllocatedSizeResolver>,  // new
}
```

`Arc` because `ScanConfig` is cloneable and the resolver is stateless/immutable. `ScanConfig` currently derives both `Clone` and `Debug`. `Arc<dyn Trait>` is `Clone`, but `Debug` requires either a supertrait bound or a manual `Debug` impl. Add `Debug` as a supertrait on `AllocatedSizeResolver` — both resolvers are unit structs that derive `Debug` trivially.

`ScanConfigBuilder::build()` calls `select_resolver` using the `filesystem_type` if present, defaulting to `PosixResolver` otherwise. The factory returns `Box<dyn AllocatedSizeResolver>` which the builder wraps in `Arc::from()`.

### `FileEntry::from_metadata` signature change

```rust
pub fn from_metadata(path: PathBuf, metadata: &Metadata, allocated_size: u64) -> Self
```

The method no longer computes `st_blocks * 512` — that knowledge lives in the resolver. The caller passes the resolved value.

### Walk loop change

In `walk_tree()`:

```rust
let alloc = config.resolver().resolve(dir_entry.path(), &metadata);
let mut file_entry = FileEntry::from_metadata(dir_entry.path().to_path_buf(), &metadata, alloc);
```

### `ScanMetadata` change

Add `size_accuracy: SizeAccuracy` field. Set from `config.resolver().accuracy()` when building metadata at scan completion.

## Accuracy Propagation

### TUI warning banner

When `ScanMetadata::size_accuracy` is `Logical`, display a one-line warning in the progress view and explorer view:

> "Sizes are logical (uncompressed) on {filesystem} — actual disk usage may be lower. Run with sudo for accurate sizes."

Filesystem-aware via the detected `filesystem_types` name. Styled line, not modal, not blocking. When accuracy is `Exact`, nothing is shown.

Warning text added to i18n locale files with filesystem name interpolation. Key: `warning.logical-sizes`.

### Batch mode

One-line warning to stderr after scan completion if accuracy is `Logical`.

### Export metadata

`size_accuracy` stored as a key in the SQLite `scan_metadata` table (additive — no schema version bump). Values: `"exact"`, `"approximate"`, `"logical"`.

Not per-entry — accuracy is uniform across the scan (one resolver per scan root; cross-device entries are skipped by default).

## Issue #77 H2 Forward Compatibility

The spec currently defines sparse detection as `blocks() * 512 < size()`. This conflicts with the resolver: if `allocated_size` becomes accurate compressed size (Phase 2), every compressed file false-positives as sparse.

Sparse detection (when implemented) MUST use `SEEK_HOLE`/`SEEK_DATA` (Linux 3.1+, FreeBSD 10+) instead of the blocks heuristic. macOS does not support this — document as a platform limitation. This is a design constraint, not work in this PR.

## Testing

### Unit tests

- `select_resolver` returns correct accuracy per filesystem: `"btrfs"` → `Logical`, `"ext4"` → `Exact`, `"unknown"` → `Exact`.
- `PosixResolver::resolve` and `LogicalOnlyResolver::resolve`: construct a real file, stat it, verify return matches `metadata.blocks() * 512`.
- `FileEntry::from_metadata` with the new parameter: verify it stores the passed value.
- Existing tests calling `from_metadata` updated to pass the third argument.

### Integration tests

- Pipeline test: `ScanMetadata` carries `size_accuracy` and round-trips through SQLite.
- Builder: `filesystem_type("btrfs")` → resolver reports `Logical`.
- Builder: no filesystem type → defaults to `Exact`.

### VHS visual test

After TUI warning banner implementation, add a VHS tape with forced `Logical` accuracy to verify warning layout and styling.

### What we don't test

- That btrfs actually misreports `st_blocks` (kernel behavior, not ours).
- Mock resolvers in production tests (real resolvers are trivial enough).

## Files Changed

| File | Change |
|---|---|
| `src/platform/alloc.rs` | **New.** Trait, two impls, factory function |
| `src/platform/mod.rs` | Re-export `alloc` module, `AllocatedSizeResolver`, `select_resolver` |
| `src/types.rs` | `SizeAccuracy` enum, `ScanConfig` gains `resolver` field, `ScanConfigBuilder` calls `select_resolver`, `FileEntry::from_metadata` gains `allocated_size` param, `ScanMetadata` gains `size_accuracy` |
| `src/scanner/walkdir.rs` | Walk loop calls resolver instead of inline computation |
| `src/storage/sqlite.rs` | Persist/read `size_accuracy` in `scan_metadata` table |
| `src/ui/views/progress.rs` | Warning banner when `Logical` |
| `src/ui/views/explorer.rs` | Warning banner when `Logical` |
| `src/lib.rs` | Batch mode warning |
| `locales/*.yml` | `warning.logical-sizes` key |
| `src/platform/linux.rs` | Add bcachefs magic number (from stash) |
| `docs/specification.md` | Updated research findings (from stash) |
