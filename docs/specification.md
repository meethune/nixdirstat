# NixDirStat - Directory Statistics for POSIX-Compliant Systems

## Description
NixDirStat is a disk usage analyzer and cleanup assistant for Unix and Unix-like systems. Scan local filesystems, devices, and directories then explore disk usage through sortable file lists, file-type statistics, and interactive treemaps.

### Inspiration
- WinDirStat
    - https://github.com/windirstat/windirstat
    - https://windirstat.net/index-selected.html
    - https://github.com/windirstat/windirstat/wiki


## Target Audience
- System Administrators who manage large storage servers
- Forensic Analysts who want to quickly see whats on a disk
- Power Users who want to clean up their personal machine

## Minimum Viable Product (MVP)
- Scan a single directory tree
- Store results in a local database
- Display sortable file/directory list with sizes
- Display interactive treemap visualization
- Basic CLI invocation: `nixdirstat <path>`


## Constraints
- Rust (stable toolchain, latest edition)
- Target OS: Linux, macOS, FreeBSD
- Distributed as a single static binary per platform
- Memory budget: should be able to run on systems with at least 2GB RAM with average usage ~10% total memory.
- Performance target: Entire root filesystem scan+persist in under 10 seconds for 2M files. **Empirically validated (all 3 platforms):** end-to-end scan+persist: Linux/ext4 ~277K files/sec (3.6ms/1K), FreeBSD/ZFS ~124K files/sec (8.1ms/1K), macOS/APFS ~187K files/sec (5.3ms/1K). 2M-file projection: Linux ~7.2s, macOS ~10.7s (borderline), FreeBSD ~16s. macOS bottleneck is APFS metadata scanning (slowest of all platforms); FreeBSD bottleneck is WAL write amplification on ZFS. rusqlite inserts: macOS ~102M rows/min (fastest — M3 Pro NVMe), Linux ~51M, FreeBSD/ZFS ~26M. walkdir is 1.8x faster than `find` on Linux/FreeBSD, **3.1x faster on macOS** (macOS `find` is dramatically slow). dua-core parallel speedup is marginal below ~50K files on Linux, zero on FreeBSD/ZFS, but provides genuine 1.3x speedup on macOS (slow APFS metadata gives threads room to hide latency). (R01, R08, R13-R17)
- User is responsible for invoking sudo. Application must have prominent visual indication when running as root.
- Don't cross devices without explicit permissions (ie the "find" program's `-xdev` cli command)


## Architecture Overview

```
CLI -> Interactive -> Ratatui TUI -> User Input -> Scanner -> Storage -> Output
                         ^                                                |
                         |------------------------------------------------|

CLI -> Batch -> Scanner -> Storage -> CSV/JSON output
```

### Concurrency Model
- **Scanner**: `tokio::spawn_blocking` for blocking `std::fs::read_dir` / `walkdir` traversal
- **Storage writer**: dedicated thread with owned `rusqlite::Connection`
- **TUI event loop**: async via `crossterm::event::EventStream` in `tokio::select!`
- **Scanner -> TUI**: `tokio::sync::mpsc` channel carrying progress updates (file count, current path, scan rate)
- **Scanner -> Storage**: `tokio::sync::mpsc` channel carrying batched `Vec<FileEntry>` (batch size: 10K entries per send)
- **Shutdown**: `tokio_util::sync::CancellationToken` for graceful teardown

No asyncio-vs-threads tension — Rust's ownership model and typed channels provide clean boundaries between subsystems. (R13)

**Empirically validated (P06, Linux):** async pipeline (tokio 1.x, `spawn_blocking` scanner + `spawn_blocking` storage writer, bounded `mpsc` channels) measured at **-8.4% overhead vs synchronous baseline** — the async version is slightly faster due to pipeline parallelism (scanner continues walking while storage writer flushes). Backpressure confirmed working: bounded channel (capacity=1) with small batch size (100) produces zero data loss. Cancellation via `CancellationToken::is_cancelled()` poll per walkdir entry is cheap and reliable — scanner breaks, storage writer commits partial data. Progress reporting via `try_send` (lossy, non-blocking) works without scanner stall. Batch size 100–50K all perform within noise at 1K-file scale; 10K default confirmed reasonable. Key types: `FileEntry` (12 metadata fields), `EntryBatch` (newtype enforcing batch invariant), `ScanProgress`, `PipelineConfig` (tuning knobs), `PipelineResult` (timing breakdown). (P06)

### Components
- **Scanner** — walks filesystem via `walkdir` (single-threaded) or `dua-core` (parallel), collects metadata
- **Storage** — persists scan results via `rusqlite` (bundled sqlite, WAL mode) for querying and incremental updates
- **Analyzer** — computes aggregations (file-type stats, duplicates, largest files)
- **UI (TUI)** — Ratatui-based interactive interface with crossterm backend
- **CLI** — command-line entry point via `clap`, non-interactive export/reporting

### Data Model
- What metadata per file entry? (path, size, allocated size, type, owner, permissions,
  mtime, inode, device, hardlink count)
- What metadata per scan? (root path, start time, duration, file count, total size, filesystem type(s))
- How are directories represented? (aggregated sizes, child counts)
- How are hardlinks tracked? Deduplicate via `HashSet<(u64, u64)>` of `(ino, dev)`. Only count `blocks * 512` toward parent size if inode not yet seen. **Empirically measured:** 68MB at 2M entries (3.32% of 2GB budget) — above the 40-64MB projection due to hashbrown power-of-two capacity rounding and control byte overhead (theoretical 37MB + 31MB overhead). Still well within budget. Cross-device hardlinks are impossible per POSIX (`EXDEV`). (R02)
- How are symlinks handled? (record target, never follow during walk)
- Physical size over logical. Store both `st_size` (logical) and `st_blocks * 512` (physical/allocated). (R10)
- Non-UTF-8 file paths: handled at the type system level via `OsStr`/`PathBuf`. **Empirically validated (P07):** hybrid storage strategy — store `path_bytes BLOB` (canonical, lossless) + `path_text TEXT` (queryable, lossy via `to_string_lossy()`). TEXT-only CANNOT reconstruct invalid-UTF-8 paths on the filesystem (every tested invalid sequence fails). BLOB-only works for exact match and parent_path queries but breaks SQL string functions (LIKE, extension extraction). Hybrid gives lossless roundtrip via BLOB + full SQL query ergonomics via TEXT, at 67.7% storage overhead (10K entries: TEXT 1200KB, BLOB 1200KB, hybrid 2012KB). Lossy paths detectable via `INSTR(path_text, U+FFFD) > 0`. Extension extraction should use application-side `Path::extension()` (SQL-only extraction is fragile for multi-dot paths). OsStr↔bytes roundtrip is lossless on Unix; BLOB↔sqlite roundtrip is lossless for all byte values 0x00-0xFF. walkdir discovers and returns full metadata for invalid-UTF-8 filenames without issue. (P07, R15)
- What is the sqlite schema? **Empirically explored:** single `entries` table (files + directories with `file_type` column) is simpler and supports all required query patterns (top-N by size, group-by-extension, subtree via LIKE prefix, directory children). Split tables add complexity without clear benefit at MVP scale. Path columns use hybrid storage: `path_bytes BLOB` (lossless canonical) + `path_text TEXT` (queryable lossy), same for `parent_bytes`/`parent_text` (P07). Extension extraction recommended application-side via `Path::extension()` — SQL-only extraction is fragile for multi-dot filenames and paths with dots in directory names.
- How are file types classified? **Empirically explored:** hybrid approach — mode bits for structural type (regular/directory/symlink/device/socket/pipe), extension for category (code, image, document, archive, binary, other). Extensionless files classified as "no extension". Multi-dot extensions use last component (file.tar.gz → "gz"). MIME detection deferred (adds I/O overhead).
- Schema versioning: **Empirically validated:** `PRAGMA user_version` works cleanly. Set version on creation, check on open, reject incompatible future versions with clear error, support forward migration via ALTER TABLE. Data preserves through migration. Default `user_version` is 0 (detects unversioned files).
- Total memory budget: **Empirically measured (Linux):** HashSet 68MB + HashMap aggregation 10MB + rusqlite WAL 2MB + TUI buffer ~0MB = **~80MB total (3.90% of 2GB)**. Well within budget with ~1.9GB headroom for larger scans or additional features.


## Development Principles
- Research correct answers instead of quickly generating working code.
- Adhere to industry best practices.
- All solutions must take cross-compatibility into consideration.
- Code must be extensible to accommodate arbitrary devices (e.g., network shares vs. local drives) and filesystems (xfs, fat32, ufs, etc.).
- Use tokio for async coordination; blocking filesystem I/O in `spawn_blocking`.
- UI must be responsive during heavy activity.
- UI must report progress to user in real time.
- Application should respect system resources and not monopolize I/O or CPU while running.

### Cross-Platform Compatibility Strategy

Four-tier preference hierarchy — always use the highest tier available:

- **Tier 1 — Portable POSIX (always prefer):** APIs that work identically across Linux/FreeBSD/macOS with zero platform code. `nix::unistd::geteuid()`, `nix::sys::statvfs::statvfs()`, `std::os::unix::fs::MetadataExt`, `walkdir`, `rusqlite`. The vast majority of the application lives here.
- **Tier 2 — Portable call, platform-conditional interpretation:** One function call works everywhere, but the result type or semantics differ. Use `#[cfg]` on the *interpretation*, not the call. Example: `nix::sys::statfs::statfs()` — Linux returns `f_type` magic numbers, FreeBSD/macOS return `f_fstypename` strings.
- **Tier 3 — Platform-gated implementation (`#[cfg]` compile-time):** When no portable syscall exists, provide per-platform implementations behind `#[cfg(target_os = "...")]`. Always include a catch-all `#[cfg(not(any(...)))]` that returns `Err`/`None`/`"unknown"`. Example: kernel filesystem enumeration (`/proc/filesystems` on Linux, `lsvfs` on FreeBSD).
- **Tier 4 — External tool invocation (last resort):** Only for tools that are genuinely external (not syscall wrappers): `losetup`, `mkfs.*`, `zpool`, `rustc --version`. Always gate with `#[cfg]` when platform-specific.

**Rules:** Never use `unsafe` — `nix` wraps all needed syscalls. Never use `/proc` outside `#[cfg(target_os = "linux")]`. Never shell out for something `nix`/`sysinfo`/`std` can do. Use `libc::mode_t` for type casts (resolves u16/u32 difference automatically). Document platform behavioral differences with `cfg!()` runtime checks in tests.

### Platform Behavioral Differences

| Behavior | Linux | FreeBSD | macOS |
|----------|-------|---------|-------|
| Sparse files | Supported | Supported | Not supported (APFS) |
| Non-UTF-8 filenames | Allowed (ext4) | Allowed (UFS/ZFS) | Rejected (APFS EILSEQ) |
| GID inheritance | Process effective GID | Parent directory GID (ZFS) | Parent directory GID (APFS) |
| WAL overhead | ~1.1x (ext4) | ~2.15x (ZFS) | ~1.17x (APFS) |
| Loopback devices | losetup + mkfs | mdconfig + newfs | hdiutil |
| Kernel FS enumeration | /proc/filesystems | lsvfs | mount output |
| `mode_t` / `libc::S_IF*` width | u32 | u16 | u16 |
| Native filesystems | ext4, xfs, btrfs | ufs, zfs | apfs, hfs |
| ZFS df accounting | Immediate | Deferred (TXG) | N/A |

### Agentic Coding Guidelines
- Delegate to domain-specific agents whenever possible.
- When in doubt, subagents should ask the main agent orchestrator for clarification.
- Orchestrator should defer to human user if there is ambiguity.
- Trust but verify.


## Research Questions

### Data Model & Storage Backend
- Storage access pattern is write-heavy during scan, read-heavy during explore. Row-oriented inserts dominate the scan phase; the explore phase issues filtered/sorted queries. (R05)
- Data is relational/hierarchical — file entries with parent paths, queried by path prefix, size, type, and owner. (R05)
- **rusqlite with `bundled` feature and platform-adaptive WAL** is the storage backend. **Empirically measured (all 3 platforms):** macOS/APFS: ~102M rows/min (fastest — M3 Pro NVMe), Linux/ext4: ~51M rows/min, FreeBSD/ZFS: ~26M rows/min. WAL overhead varies by filesystem: macOS/APFS 1.17x (52ms→61ms, moderate), Linux/ext4 1.10x (107ms→118ms, minimal), FreeBSD/ZFS 2.15x (108ms→232ms, severe — double-journaling on CoW). **Empirically validated (P08):** journal_mode switching at runtime works cleanly — DELETE→WAL→DELETE preserves data, mode switch from WAL triggers automatic checkpoint. Strategy: batch mode (`scan --output`) uses DELETE (no WAL overhead); interactive mode (`scan` → TUI) uses WAL for concurrent reader/writer; ZFS interactive falls back to DELETE. Filesystem type detection via `libc::statfs` `f_type` field (Linux) with `/proc/mounts` fallback. WAL concurrent read/write confirmed: reader sees committed data incrementally, zero errors during parallel writer activity. DELETE mode can produce SQLITE_BUSY for readers during writes. Bundled sqlite compiles the amalgamation into the binary — zero runtime dependency, consistent version across platforms. (R05, P08)
- Recommended PRAGMAs: `synchronous=NORMAL`, `temp_store=MEMORY`, `mmap_size=268435456`, `busy_timeout=5000`. `journal_mode` set adaptively per P08 strategy. Writer uses `BEGIN IMMEDIATE` transactions. **Empirically validated (all 3 platforms):** WAL overhead: macOS/APFS 1.17x, Linux/ext4 1.18x, FreeBSD/ZFS 2.15x. Notably, macOS `wal_full` (61ms) is actually faster than `wal_only` (66ms) — the mmap and temp_store PRAGMAs provide a measurable speedup on APFS. On ZFS, all WAL variants are equally slow (~232ms). WAL's value is concurrent read/write for TUI, not raw insert speed. Saved scan files should be checkpointed and switched to DELETE mode for portability (`PRAGMA wal_checkpoint(TRUNCATE)` then `PRAGMA journal_mode = DELETE`). (P08)
- MVP supports full rescans only. Incremental updates via inotify/fsevents/kqueue deferred to post-MVP. Rust `notify` crate wraps all three but adds complexity. (R04)

### Filesystem Access
- **`walkdir` crate** for single-threaded walking — cross-platform, constant memory. **Empirically measured (all 3 platforms, 1K files):** FreeBSD/ZFS: 716µs (fastest), Linux/ext4: 1.17ms, macOS/APFS: 3.07ms (slowest — APFS metadata is expensive). walkdir vs `find`: 1.8x faster on Linux/FreeBSD, **3.1x faster on macOS** (macOS `find` at 9.6ms is dramatically slow). Raw `std::fs::read_dir` is ~10% faster than walkdir on Linux/FreeBSD (776µs, 690µs) and ~25% faster on macOS (2.33ms vs 3.07ms) but lacks walkdir's error handling, symlink loop detection, and depth control. Clean skip-and-log via `filter_map(|e| e.ok())`.
- **`dua-core`** as optional parallel walker — work-stealing pool. **Empirically measured (all 3 platforms, 1K files):** macOS/APFS: dua-core at 2 threads 2.41ms vs walkdir 3.16ms — **genuine 1.3x speedup**, the only platform where dua-core beats walkdir at small scale (slow APFS metadata gives threads room to hide latency). Linux/ext4: slower at small scale (1.76ms vs 1.17ms), but scales from 3.5ms (1t) to 1.76ms (4t). FreeBSD/ZFS: **zero scaling** — all thread counts (1-8) produce ~2.3-2.4ms, consistently slower than walkdir's 716µs. The 3-6x speedup from third-party reports applies at 21M+ files. Replaces deprecated `jwalk`. **Note:** `dua_core::Metadata` is a platform-conditional type — `std::fs::Metadata` on Linux/FreeBSD, custom `macos::Metadata` on macOS. Use `entry.path().symlink_metadata()` for portable MetadataExt access.
- **`std::os::unix::fs::MetadataExt`** (the cross-Unix trait, NOT `linux::fs`) provides all 9 stat fields: `dev()`, `ino()`, `mode()`, `nlink()`, `uid()`, `gid()`, `size()`, `mtime()`, `blocks()`. **Empirically verified (all 3 platforms):** all 9 fields confirmed working across regular files, directories, symlinks, hardlinks, and sparse files. Full benchmark suite compiles and runs natively on Linux, FreeBSD, and macOS (ARM64).
- How to handle permission denied errors mid-scan? (skip and log)
- How to handle broken symlinks, circular symlinks, bind mounts? (skip and log — walkdir detects symlink loops)
- Cross-device boundary detection: compare each entry's `dev()` to the scan root's `dev()` — same algorithm as `find -xdev`. Bind mounts of the same device share `st_dev` (not detected — matches `find` behavior). overlayfs/FUSE get distinct `st_dev`. Skip entries with different `st_dev` unless user opts in via `--cross-device`. (R03)
- `st_gid` is collected and **recommended for inclusion**. **Empirically measured:** `gid()` is available on all platforms via MetadataExt, storage cost is <5 bytes/row (negligible), and sysadmin users (a named target audience) benefit from group-based queries. Collect and store; defer group-based UI features to post-MVP.

### I/O Methodology
- POSIX compliance provides the portable foundation: `std::os::unix::fs::MetadataExt` works across all target platforms with consistent stat fields. (R08, R15)
- ext4, btrfs, zfs, and Apple Filesystem are the initial targets.
- Incremental updates via inotify/fsevents/kqueue deferred to post-MVP. Rust `notify` crate available when needed. (R04)
- Filesystem-specific APIs (btrfs `FIEMAP`/compsize ioctls, ZFS dataset properties) can provide shortcuts for compression-aware sizes — deferred to post-MVP. Rust `nix` crate or raw `libc::ioctl()` provides direct access. (R07)

### Calculation Methodology
- **Bottom-up aggregation during walk.** **Empirically measured (all 3 platforms, 100K entries):** macOS: HashMap 36.6ms vs SQL 72.1ms (1.97x). FreeBSD: HashMap 37.8ms vs SQL 71.1ms (1.88x). Linux: HashMap 88.7ms vs SQL 155.1ms (1.75x). Bottom-up consistently wins across all platforms at ~1.8-2.0x, not the 25x seen in Python — Rust's fast SQL narrows the gap dramatically. Both strategies are viable; bottom-up still wins and avoids the post-scan query round-trip. macOS and FreeBSD are ~2.3x faster than Linux VM for CPU-bound work. Single O(n) pass with `HashMap<String, u64>` accumulation — walk each file's parent chain upward, then persist aggregated sizes as rows in sqlite. (R09)
- **Logical size:** `size()`. **Physical (allocated) size:** `blocks() * 512` — portable POSIX convention, accurate on ext4, xfs, ZFS. Store both per entry. Caveats: btrfs compression is NOT reflected in `st_blocks` (needs ioctl, post-MVP); APFS clones double-count shared CoW extents (no POSIX fix, same behavior as `du`); ZFS compression IS accurately reflected. (R10)
- **Sparse files:** detected via `blocks() * 512 < size()` — works on all platforms except APFS (no sparse file support). **Compressed files:** only ZFS reports accurate compressed allocation via `st_blocks`; btrfs requires filesystem-specific ioctls (deferred to post-MVP). **Deduplicated extents:** no portable POSIX detection — same limitation as `du`. (R11)
- Existing Rust tools (`dust`, `dua-cli`) are useful as code references but none cover scan+persist+query+TUI. Architecture review pending (P04). Build on `walkdir` + `rusqlite` + `ratatui` — empirically validated stack.


## Technology Decisions

### Language & Toolchain
- **Rust** (stable toolchain, latest edition)
- Cargo for build and dependency management
- `Cargo.lock` committed for reproducible builds
- Motivation: single-binary distribution eliminates FreeBSD dependency-hell, 22x faster sqlite inserts, 3-5x less memory for hardlink dedup, type-safe non-UTF-8 path handling, no GIL for true concurrent scan+TUI (R13-R17)

### User Interface
- **Ratatui** TUI framework (v0.30+)
    - https://ratatui.rs/
    - https://github.com/ratatui/ratatui
- **Crossterm** backend (v0.29+, default — Linux, macOS, FreeBSD, Windows)
    - v0.29 required for compatibility with ratatui 0.30.2 (via `ratatui-crossterm`)
    - Confirmed working on FreeBSD (compile fix in changelog)
    - ANSI escape codes directly, no terminfo dependency
- **Treemap widget**: Custom widget using `streemap` crate (v0.1.0, MIT/Apache-2.0) for layout algorithms, rendered via Ratatui direct Buffer cell rendering. `streemap` is a pure algorithm library (only depends on `num-traits`) offering six layout algorithms: Squarified (Bruls et al. 2000 — the standard for disk-usage treemaps, used by WinDirStat/KDirStat/GrandPerspective), Slice, Dice, Binary, and two Ordered Pivot variants (Shneiderman & Wattenberg 2001). API is generic: operates on `&mut [T]` with closures for size extraction and rect assignment. Default algorithm: Squarified. **Cushion treemap rendering** (van Wijk & van de Wetering 1999 — parabolic height-field + Phong shading for depth perception) deferred to post-MVP; no reusable Rust crate exists (only `dirstats-treemap`, GPL-3.0, tightly coupled to its own scanner). **Note:** `tui-treemap` crate (v0.1.0) exists but targets unmaintained `tui-rs` and wraps a different crate (`treemap` v0.3, squarified-only) — not suitable. **Empirically validated (P01):** `streemap::squarify` composes cleanly with Ratatui's `Buffer` cell rendering via a custom `Widget` impl. Coordinate bridge: `streemap::Rect<f32>` → `ratatui::layout::Rect` (u16) using `floor()` for x/y, `ceil()` for right/bottom edges, clamped to container — produces zero rendering gaps between adjacent cells. Labels rendered with truncation (ellipsis) when cell width < label length, hidden entirely when cell width < 3 columns. `StatefulWidget` pattern supports interactive selection highlighting. 10K items render without panic in a 40×20 buffer. Proportional sizing verified: 75%/25% items occupy approximately 75%/25% of cells. Reference: https://github.com/ratatui/templates/ for TUI application boilerplate. (P01)
- **Directory tree**: `tui-tree-widget` (v0.24, actively maintained)
- **Visualization tiers**:
    - MVP: Interactive treemap, sortable file table (`Table`), progress gauge (`Gauge`/`LineGauge`), file-type bar chart (`BarChart`)
    - Extended: Flame graph (stacked horizontal bars), histograms/heatmaps via `malevich`
    - Deferred: Sunburst (radial treemap via Canvas arcs — complex)
- Tmux Terminal Multiplexer
    - Optional but preferred. Compensates for features missing from older terminals.
    - https://github.com/tmux/tmux/wiki

### CLI Framework
- **`clap`** (derive API) for argument parsing — the standard Rust CLI library
    - Subcommand-based: `scan`, `explore`, `export`, `duplicates`
    - Shell completions via `clap_complete`


## CLI Interface
```
Examples:
  nixdirstat scan <path> [--output <file>] [--cross-device]
  nixdirstat explore <scan-file>
  nixdirstat export <scan-file> --format csv|json
  nixdirstat duplicates <path> [--hash sha256]
```
The storage format is a sqlite database file — self-contained and portable. A saved scan file includes all metadata needed to render the TUI without re-scanning the filesystem.

- `scan` without `--output` opens the TUI immediately after scan completes
- `scan --output <file>` writes the database and exits (batch mode, no TUI)
- `explore` opens a previously saved scan file in the TUI
- `export` reads a scan file and writes CSV or JSON to stdout (batch mode)

## Features

### Core (MVP)
- Scan a single directory tree with progress reporting
- Explore largest files, file types, physical size, hardlinks, and free/unknown space via treemap and sortable tables
- Search scanned results by name, size, file/folder type, or owner; filter by path, name, size, or age, with regular-expression support
- Progress reporting shows: file count, files/sec rate, current path, elapsed time. No percentage (total unknown upfront).
- What constitutes "free/unknown space"? **Empirically validated:** free space = `statvfs.f_bavail * f_bsize` (available to non-root). Total space = `f_blocks * f_bsize`. Unknown = total - sum(scanned entry sizes) - free. Writing data measurably reduces available space. `stat -f` provides portable access without unsafe syscalls.

### Extended
- Scan multiple drives or folders together, with pause/resume and refresh
- Parallel scanning via `dua-core` for multi-core speedup
- Detect duplicates using configurable hashes and cloud-file safeguards
- Inspect file and folder permission entries
- Watch filesystem changes (via `notify` crate), save/load scans as CSV or JSON, and export scan, duplicate, or permission reports from the command line
- Customize layouts, columns, colors, and toolbar size, with dark mode and portable settings

### Error Handling
- In general skip and log if not fatal
- Permission denied mid-scan: skip and log (walkdir yields `Err`, handled via `filter_map` or explicit match)
- Disappearing files/mount points during scan: situational — leaf file vanishes between readdir and stat is a normal race condition (skip and log). Scan root or parent subtree disappearing mid-walk is fatal (abort with clear error). Heuristic: if the scanner can no longer reach the current working subtree, stop; if a single entry fails, continue.
- Corrupt or truncated saved scan files? stop
- Disk full during scan database writes? stop


### Testing Strategy
- Integration tests: `tempdir` with known filesystem structures (hardlinks, symlinks, sparse files, nested directories). Assert scan results match expected metadata.
- Unit tests: scanner logic, aggregation, sqlite queries against in-memory databases.
- Platform-specific tests: loopback mounts for cross-device detection (Linux), mdconfig (FreeBSD). Run on native CI runners per platform.
- CI: GitHub Actions for Linux/macOS, Cirrus CI for FreeBSD native test execution. Configuration deferred (C02).

### Packaging & Distribution
- **Single binary** per platform, no runtime dependencies
- **Linux**: musl static linking (`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`). Use jemalloc as global allocator (musl malloc reported slow under contention — validation deferred to Phase 6, B09).
- **macOS**: native builds (`x86_64-apple-darwin`, `aarch64-apple-darwin`). Dynamic link to `libSystem.dylib` (stable ABI, effectively self-contained). Universal binary via `lipo`.
- **FreeBSD**: native builds (`x86_64-unknown-freebsd`). Cross-compilation via `cross-rs` — validation deferred (P02). Native test execution via Cirrus CI — CI setup deferred (C02).
- **Release automation**: `cargo-dist` for GitHub Releases with checksums, `release-plz` for version bumping, `cargo publish` for crates.io.
- **Package managers**: Homebrew tap, AUR (`-bin` and source), FreeBSD Ports, Nix `buildRustPackage`.
- **Binary size**: projected ~4-6MB raw, ~3-5MB stripped+LTO. Validation deferred until main binary exists (B10).

```toml
[profile.release]
strip = true
lto = "thin"
opt-level = "s"
panic = "abort"
codegen-units = 1
```


## Future / Out of Scope
- Native Windows Support
    - Utilize Master File Table (MFT)
    - [Recover deleted files](https://github.com/windirstat/windirstat/wiki/Recovering-Files) from local NTFS, exFAT, and FAT drives with administrator access; recovery depends on surviving metadata and file data
- Advanced WinDirStat Features
    - Estimate storage tier costs by file modification age
    - Open, move, or delete files; run hardlink deduplication and custom cleanup commands
    - Optional sparse hashing labels sampled matches as **Probable duplicates**
- Remote Filesystem Support
    - NFS
    - SMB


## Outstanding Research Gaps

Status snapshot as of 2026-10-06. All R01–R17 research tags and all medium-priority prototypes (P01, P03, P05–P08, R03) validated. Full test suite (183 tests) passes on all 3 target platforms (Linux, FreeBSD, macOS). Cross-platform compatibility strategy established and enforced. Remaining gaps are implementation-phase concerns that require the real binary.

### Validated — Research Phase Complete

- **P06: Async concurrency pipeline.** **VALIDATED.** `tokio::spawn_blocking` scanner → bounded `mpsc` channel → `spawn_blocking` storage writer, with `CancellationToken` shutdown and progress reporting. Async pipeline measured at -8.4% overhead vs synchronous baseline (faster due to pipeline parallelism). Backpressure, cancellation, and progress reporting all confirmed working. Batch size 10K validated. 6 unit + 6 integration tests. See `research/rust/src/pipeline.rs`, `research/rust/examples/p06_pipeline.rs`.
- **P01: Treemap widget prototype.** **VALIDATED.** `streemap::squarify` composes cleanly with Ratatui `Buffer` rendering via custom `Widget`/`StatefulWidget` impl. f32→u16 coordinate conversion produces zero rendering gaps. Label truncation, color-by-category palette, interactive selection, and 10K-item rendering all working. 11 unit + 6 integration tests. See `research/rust/src/treemap.rs`, `research/rust/examples/p01_treemap.rs`.
- **P05: Walker crate comparison.** **VALIDATED** on all 3 platforms. See `research/rust/benches/p05_walker_compare.rs`.
- **P07: Non-UTF-8 path roundtrip.** **VALIDATED.** Hybrid storage strategy chosen: `path_bytes BLOB` (canonical, lossless) + `path_text TEXT` (queryable, lossy). TEXT-only fails to reconstruct invalid-UTF-8 paths. BLOB-only breaks SQL string functions. Hybrid gives both lossless roundtrip and full query ergonomics at 67.7% storage overhead (acceptable — path columns dominate row size regardless). 8 invalid-UTF-8 filename patterns tested (Latin-1, raw high bytes, Shift-JIS, lone continuations, overlong encodings, truncated sequences). walkdir + MetadataExt work without issue on invalid-UTF-8 filenames. OsStr↔bytes and BLOB↔sqlite roundtrips are lossless. Extension extraction recommended application-side via `Path::extension()`. 18 unit + 3 property-based tests. See `research/rust/tests/p07_non_utf8_paths.rs`.
- **P08: Platform-adaptive WAL.** **VALIDATED.** Runtime journal_mode switching works cleanly — DELETE↔WAL preserves data, mode switch from WAL triggers automatic checkpoint. Strategy: batch mode uses DELETE; interactive mode uses WAL (except on ZFS, which falls back to DELETE due to 2.15x overhead). Filesystem type detection via `libc::statfs` `f_type` on Linux with `/proc/mounts` fallback. WAL concurrent read/write confirmed: reader sees committed data incrementally with zero errors during parallel writer activity; DELETE mode can produce SQLITE_BUSY. WAL/DELETE performance ratio: 1.18x on Linux/xfs (10K rows). Checkpoint via `PRAGMA wal_checkpoint(TRUNCATE)` + `PRAGMA journal_mode = DELETE` for portable saved files. Repeated mode switching (5 cycles) stable. Batch→interactive and interactive→save transitions validated. 21 tests (including multi-threaded concurrent access). See `research/rust/tests/p08_adaptive_wal.rs`.

- **R03: Cross-device detection end-to-end.** **VALIDATED.** Loopback ext4 mount creates a distinct `st_dev` value; `walk_same_device` (compare each entry's `dev()` to root's `dev()`, skip on mismatch) correctly excludes all entries on the foreign filesystem including the mount point directory itself. Multiple simultaneous cross-device mounts produce distinct `st_dev` values and are all correctly filtered. walkdir enters cross-device mounts by default (no built-in `-xdev`), so the dev filter is mandatory. 6 integration tests (require root for loopback mount). See `research/rust/tests/r03_cross_device.rs`.

- **P03: Crossterm on FreeBSD.** **VALIDATED.** Ratatui + Crossterm backend compiles and renders correctly on FreeBSD (ZFS, native). All 9 headless TUI tests pass: terminal init, paragraph, table, gauge, multi-widget layout, non-ASCII content, color palette, minimum terminal size, and direct buffer cell rendering (treemap pattern). Also validated on macOS/APFS. See `research/rust/tests/p03_crossterm_freebsd.rs`.

### Lower Priority — Implementation Phase

- **P04: Architecture review of `dust`/`dua-cli`.** Code reference for design patterns, not blocking.
- **P09: Error type hierarchy.** `thiserror` validated via P06 (`PipelineError`), but `ScanError`/`StorageError`/`UiError` enum design not sketched for the main application. Straightforward to add during implementation.
- **P02: FreeBSD cross-compilation via `cross-rs`.** Build infra, not blocking research.
- **B09: jemalloc on musl.** Needs a real binary to validate.
- **B10: Binary size validation.** Needs a real binary (~4-6MB projected).
- **C02: CI configuration.** GitHub Actions + Cirrus CI for FreeBSD. Implementation-phase concern.


## Research Lineage

This specification supersedes the Python+Textual draft (`specification.md`). The pivot to Rust+Ratatui was motivated by FreeBSD dependency-hell, single-binary distribution requirements, and performance advantages validated in R13-R17.

### Research carried forward (runtime-agnostic)
- R03: Cross-device detection (`st_dev` comparison, `find -xdev` algorithm)
- R04: Incremental updates deferred to post-MVP
- R07: Filesystem-specific ioctls deferred to post-MVP
- R10: Physical vs logical size (`st_blocks * 512` POSIX convention)
- R11: Sparse/compressed file detection and platform caveats

### Research carried forward — empirically validated in Rust (all 3 platforms)
- R01: Performance feasibility — **VALIDATED.** Linux: ~277K files/sec (7.2s/2M), macOS: ~187K files/sec (10.7s/2M, borderline), FreeBSD: ~124K files/sec (16s/2M). Bottlenecks are platform-specific: macOS=APFS metadata, FreeBSD=WAL-on-ZFS, Linux=VM overhead.
- R02: Hardlink dedup — **VALIDATED.** `HashSet<(u64, u64)>` measured at 68MB/2M entries (above 40-64MB projection, within budget)
- R05: Storage backend — **VALIDATED.** macOS: ~102M rows/min (fastest), Linux: ~51M, FreeBSD/ZFS: ~26M. WAL overhead: macOS 1.17x, Linux 1.10x, FreeBSD 2.15x. Platform-adaptive WAL recommended.
- R08: Scanner API — **VALIDATED.** `walkdir` + `MetadataExt` confirmed on all 3 platforms. walkdir vs `find`: 1.8x faster on Linux/FreeBSD, 3.1x on macOS. All 9 MetadataExt fields verified.
- R09: Bottom-up aggregation — **VALIDATED.** HashMap ~1.8-2.0x faster than SQL on all platforms (not 25x as in Python — Rust's fast SQL narrows the gap)

### Preliminary Rust-specific research — validated
- R13: Ratatui TUI evaluation — docs/ecosystem review complete. **Treemap prototype VALIDATED (P01).** `streemap` + Ratatui `Widget`/`StatefulWidget` composition confirmed. Crossterm FreeBSD testing pending (P03).
- R14: Rust filesystem scanning APIs — **VALIDATED (all 3 platforms).** Scanning speed: FreeBSD/ZFS > Linux/ext4 > macOS/APFS. dua-core helps on macOS (1.3x), marginal on Linux, zero on FreeBSD.
- R15: Walker crate empirical comparison — **VALIDATED (P05, all 3 platforms).** Manual read_dir fastest, walkdir best balance. dua-core beats walkdir only on macOS at small scale.
- R16: Rust SQLite storage — **VALIDATED (all 3 platforms).** Insert throughput: macOS > Linux > FreeBSD. PRAGMA tuning: mmap+temp_store help on macOS, minimal on Linux, irrelevant on ZFS.
- R17: Cross-compilation and distribution — build validation pending (P02, B09, B10, C02). Native builds confirmed on all 3 platforms.

### Research needing Rust-specific re-evaluation
- R06: FS library evaluation — **VALIDATED.** walkdir chosen over raw `std::fs::read_dir` (convenience vs 10-25% speed) and dua-core (overhead at small scale except macOS).
- R12: Existing libraries — architecture review pending (P04). Stack decision (`walkdir` + `rusqlite` + `ratatui` + `streemap` + `tokio`) empirically validated end-to-end (P06, P01).

### Platform-specific research — all empirically benchmarked
- **FreeBSD/ZFS:** Fastest scanning (1.6x over ext4). WAL mode 2.15x slower (double-journaling on CoW). dua-core zero parallel scaling. Default sqlite runs at parity with Linux. Full research crate compiles and runs natively.
- **macOS/APFS (M3 Pro ARM64):** Fastest CPU (2x over Linux VM for compute). Slowest filesystem scanning (APFS metadata expensive). `find` dramatically slow (3.1x slower than walkdir). WAL overhead moderate (1.17x). dua-core provides genuine 1.3x speedup (only platform where it helps at small scale). `dua_core::Metadata` is a custom macOS type — use `entry.path().symlink_metadata()` for portable MetadataExt. End-to-end 2M projection ~10.7s (borderline target).
