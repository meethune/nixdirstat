# Contributing to NixDirStat

NixDirStat is a disk usage analyzer for POSIX-compliant systems (Linux, macOS, FreeBSD).
This guide defines the conventions, patterns, and workflows that every contribution must follow.

## Table of Contents

- [Development Setup](#development-setup)
- [Code Style and Formatting](#code-style-and-formatting)
- [Naming Conventions](#naming-conventions)
- [Module Organization and Imports](#module-organization-and-imports)
- [Documentation and Comments](#documentation-and-comments)
- [Error Handling](#error-handling)
- [Cross-Platform Development](#cross-platform-development)
- [Extensibility Patterns](#extensibility-patterns)
- [Testing](#testing)
- [Benchmarks](#benchmarks)
- [Git Workflow](#git-workflow)
- [Pull Request Process](#pull-request-process)


## Development Setup

### Prerequisites

- Rust stable toolchain (edition 2024, MSRV 1.95.0)
- `cargo`, `rustfmt`, `clippy` (included with `rustup`)
- Python 3 with [`uv`](https://docs.astral.sh/uv/) (research infrastructure only)
- SQLite is bundled via `rusqlite` — no system library required

### Building and Checking

```bash
cargo check                                    # type-check without codegen
cargo build                                    # debug build
cargo build --release                          # optimized build
cargo fmt --check                              # verify formatting
cargo clippy --all-targets -- -D warnings      # lint (warnings are errors)
cargo test                                     # run all non-ignored tests
cargo test -- --ignored                        # run privileged tests (requires root)
```

All four checks — `fmt`, `clippy`, `check`, `test` — must pass before submitting a PR.


## Code Style and Formatting

### Enforced by Tooling

- **`cargo fmt`** (rustfmt defaults) is mandatory. Do not customize `rustfmt.toml`.
- **`cargo clippy --all-targets -- -D warnings`** is mandatory. All warnings are errors.
- Never use `#[allow(...)]` to suppress clippy or compiler warnings unless it is a verified
  false positive, documented with a comment explaining why.

### Manual Conventions

- **Indentation:** 4 spaces. No tabs. No trailing whitespace.
- **Line width:** 100 characters maximum.
- **Blank lines:** 0 or 1 between items. Never 2+.
- **Trailing commas:** Always in multi-line lists (function args, struct fields, match arms).
- **Expression style:** Prefer expression-oriented code where it improves clarity:
  ```rust
  let label = if size > THRESHOLD { "large" } else { "small" };
  ```
- **Block indent** over visual indent for wrapped lines.
- **`unsafe` is forbidden.** No `unsafe` blocks, `unsafe impl`, or `unsafe fn` in this project.
  Dependencies may use `unsafe` internally; project code must be 100% safe Rust.


## Naming Conventions

Follow [RFC 430](https://rust-lang.github.io/rfcs/0430-finalizing-naming-conventions.html)
and the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/).

| Item | Convention | Example |
|------|-----------|---------|
| Types, traits, enum variants | `UpperCamelCase` | `FileEntry`, `StorageBackend`, `RegularFile` |
| Functions, methods, variables | `snake_case` | `scan_directory`, `total_size` |
| Modules | `snake_case` | `scanner`, `storage` |
| Constants, statics | `SCREAMING_SNAKE_CASE` | `DEFAULT_BATCH_SIZE` |
| Generic type parameters | Single uppercase letter | `T`, `E` |
| Lifetimes | Short lowercase | `'a`, `'db` |
| Macros | `snake_case` | `ensure_root!` |

### Method Naming Patterns

| Pattern | Meaning | Example |
|---------|---------|---------|
| `as_*` | Cheap reference-to-reference conversion | `as_path()` |
| `to_*` | Expensive conversion, allocates | `to_string()` |
| `into_*` | Ownership-consuming conversion | `into_inner()` |
| `is_*` / `has_*` | Boolean predicates | `is_directory()`, `has_children()` |
| `iter()` / `iter_mut()` / `into_iter()` | Iterator access | Standard convention |

Do not prefix getters with `get_`. A method named `size()` is preferred over `get_size()`.


## Module Organization and Imports

### Project Layout

```
src/
  main.rs           # Thin entry point — calls lib.rs
  lib.rs            # Public API and module declarations
  scanner/
    mod.rs           # Scanner trait + common types
    walkdir.rs       # walkdir-based implementation
    parallel.rs      # dua-core-based implementation
  storage/
    mod.rs           # StorageBackend trait + common types
    sqlite.rs        # rusqlite implementation
  analyzer/
    mod.rs           # Aggregation, statistics
  ui/
    mod.rs           # TUI application
  cli.rs             # clap argument parsing
  platform/
    mod.rs           # Platform dispatch (Tier 3 code)
    linux.rs
    macos.rs
    freebsd.rs
tests/               # Integration tests
benches/             # Criterion benchmarks (harness = false)
```

### Import Organization

Group imports in this order, separated by blank lines:

```rust
// 1. Standard library
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// 2. External crates
use rusqlite::Connection;
use walkdir::WalkDir;

// 3. Crate-internal (self, super, crate)
use crate::scanner::FileEntry;
use crate::storage::StorageBackend;
```

- Version-sort within each group.
- Prefer individual imports over globs (`*`), except for preludes.
- Normalize: `use a::{b};` should be `use a::b;`.


## Documentation and Comments

### Doc Comments

- All public items (types, traits, functions, modules) must have `///` doc comments.
- Doc comments precede attributes (`#[derive(...)]`).
- Module-level docs use `//!` — only at the top of the file.
- Write complete sentences (capital letter, period).
- Document error conditions and panics.
- Use `?` in examples, not `unwrap()`.

```rust
/// Scans a directory tree and collects filesystem metadata.
///
/// Returns an error if the root path does not exist or is not readable.
///
/// # Errors
///
/// Returns [`ScanError::RootNotFound`] if `root` does not exist.
/// Returns [`ScanError::PermissionDenied`] if `root` is not readable.
#[derive(Debug)]
pub struct Scanner { /* ... */ }
```

### Inline Comments

- Prefer `//` over `/* */`.
- Comment line length: 80 characters (excluding indentation).
- Only comment the **why**, not the what. Well-named identifiers explain the what.
- Do not reference issues, PRs, or callers in comments — that context belongs in commit messages.

### Attributes

- One attribute per line.
- Combine all derives into a single `#[derive(...)]`.
- Standard derive order: `Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize`.


## Error Handling

### Strategy

- **Library/domain code:** `thiserror` for typed error enums. Each component defines its own error type
  (`ScanError`, `StorageError`, `UiError`).
- **Application/CLI code:** `anyhow` for error propagation with context.

### Conventions

- Error enums must be `#[non_exhaustive]` to allow adding variants without breaking downstream.
- Implement `std::fmt::Display` via `thiserror` `#[error("...")]` attributes.
- Use the `?` operator and early returns to avoid deep nesting.
- Never `unwrap()` or `expect()` in library code. In application code, `expect()` is acceptable
  only for invariants that are truly impossible to violate.
- Error messages should be lowercase, without trailing punctuation, and without a "failed to" prefix
  (the caller provides context).

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ScanError {
    #[error("root path not found: {0}")]
    RootNotFound(PathBuf),

    #[error("permission denied: {0}")]
    PermissionDenied(PathBuf),

    #[error("storage write failed")]
    Storage(#[from] StorageError),
}
```

### Defensive Programming

- Validate inputs at system boundaries (CLI arguments, user input, external API responses).
- Error on precondition failures — do not silently default or coerce.
- Use newtypes and the typestate pattern to make illegal states unrepresentable at the type level.
- Combine related validation checks into a single condition for readability.


## Cross-Platform Development

NixDirStat targets Linux, macOS, and FreeBSD. Cross-platform correctness is a project pillar.

### The Four-Tier Preference Hierarchy

Always use the highest applicable tier:

**Tier 1 — Portable POSIX (always prefer).** APIs that work identically on all targets with zero
platform code. The vast majority of this application lives here.

```rust
// Tier 1: works everywhere, no #[cfg] needed
use std::os::unix::fs::MetadataExt;
let size = metadata.blocks() * 512;
```

**Tier 2 — Portable call, platform-conditional interpretation.** One function call works everywhere,
but the result semantics differ. Apply `#[cfg]` to the *interpretation*, not the call.

```rust
// Tier 2: same call, different interpretation
let fs_info = nix::sys::statfs::statfs(path)?;
#[cfg(target_os = "linux")]
let fs_name = format!("0x{:x}", fs_info.filesystem_type().0);
#[cfg(any(target_os = "macos", target_os = "freebsd"))]
let fs_name = fs_info.filesystem_type_name().to_string();
```

**Tier 3 — Platform-gated implementation (`#[cfg]` compile-time).** Per-platform implementations
behind `#[cfg(target_os)]`. Always include a catch-all fallback.

```rust
// Tier 3: module-level dispatch in platform/mod.rs
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "freebsd")]
mod freebsd;

// Catch-all for unsupported platforms
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "freebsd")))]
pub fn filesystem_type(_path: &Path) -> Result<String> {
    Ok("unknown".to_string())
}
```

**Tier 4 — External tool invocation (last resort).** Only for tools that are genuinely external
(not syscall wrappers): `losetup`, `mdconfig`, `hdiutil`. Always gate with `#[cfg]`.

### Platform-Specific Module Structure

Use the **dispatch module** pattern for Tier 3 code: a `mod.rs` re-exports from platform-specific
submodules. Callers import from the parent module without knowing the platform.

### Known Platform Differences

| Behavior | Linux | FreeBSD | macOS |
|----------|-------|---------|-------|
| `libc::mode_t` width | `u32` | `u16` | `u16` |
| Non-UTF-8 filenames | Allowed | Allowed | Rejected (APFS) |
| Sparse file support | Yes | Yes | No (APFS) |
| GID inheritance | Process GID | Parent dir GID | Parent dir GID |
| WAL overhead on native FS | ~1.1x (ext4) | ~2.15x (ZFS) | ~1.17x (APFS) |

### Rules

- Always cast through `libc::mode_t` — never hardcode integer widths.
- Never use `/proc` outside `#[cfg(target_os = "linux")]`.
- Never shell out for something `nix`, `sysinfo`, or `std` can do.
- Use `entry.path().symlink_metadata()` for portable `MetadataExt` access, not `dua_core::Metadata`
  (which is a platform-conditional type).
- Document platform behavioral differences with `cfg!()` runtime checks in tests.


## Extensibility Patterns

NixDirStat must accommodate arbitrary filesystems, storage backends, output formats, and scanner
implementations. Use these patterns consistently.

### When to Use What

| Extension Point | Pattern | Rationale |
|----------------|---------|-----------|
| Scanner backend | Generic trait (`impl Walker`) | Hot path, known at compile time |
| Storage backend | `dyn Trait` (boxed) | Runtime selection via CLI flags |
| Output format | `dyn Trait` (boxed) | Runtime selection (CSV, JSON, TUI) |
| File type classification | Enum | Closed set, exhaustive matching |
| FS-specific metadata | Sealed trait | Controlled internal extensibility |
| Configuration structs | Builder pattern | Many optional fields, validated |
| Error types | `#[non_exhaustive]` enum | Forward-compatible |

### Decision Heuristic

- **Closed, known set of variants** (file types, error kinds) → **Enum**. Exhaustive matching,
  no vtable overhead, compiler-enforced completeness.
- **Open set, all implementations inside this crate** → **Sealed trait**. Public API but no
  external implementations.
- **Open set, may have external implementations** → **Trait**. Use generics for hot paths,
  `dyn Trait` for runtime dispatch.

### Sealed Traits

For interfaces where the project defines all implementations but external code should call methods:

```rust
mod private {
    pub trait Sealed {}
}

pub trait FsMetadata: private::Sealed {
    fn physical_size(&self) -> u64;
    fn filesystem_type(&self) -> &str;
}
```

### Non-Exhaustive Enums

All public enums that may grow must use `#[non_exhaustive]`. This forces downstream `match` arms
to include a wildcard, preventing breakage when variants are added.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
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

Within the crate, `#[non_exhaustive]` has no effect — internal matches remain exhaustive.

### Builder Pattern

Use builders for structs with many optional fields. `build()` returns `Result` to validate
invariants at construction time.

```rust
let config = ScanConfig::builder()
    .root("/data")
    .cross_device(false)
    .batch_size(10_000)
    .build()?;
```

### Adding a New Backend

To add a new scanner, storage backend, or output format:

1. Create a new module implementing the relevant trait (e.g., `storage/postgres.rs`).
2. Add a feature flag in `Cargo.toml` gating any new dependencies.
3. Wire the new implementation into CLI dispatch (e.g., a new `--storage` flag).
4. Add integration tests covering the new backend.
5. Update this document and the specification if the change affects architecture.


## Testing

### Methodology

This project uses **Compiler-Driven Development (CDD)** and **Test-Driven Development (TDD)** together:

- **CDD first:** Design types to encode invariants (newtypes, enums, typestate). Let compiler errors
  guide structural correctness.
- **TDD for logic the compiler cannot verify:** Algorithm correctness, I/O behavior, aggregation
  accuracy, cross-device detection, hardlink dedup, SQLite query results.
- Do **not** write tests for things the compiler already guarantees (null checks, type validation,
  exhaustiveness, state-transition validity).

### Test Organization

| Kind | Location | Scope |
|------|----------|-------|
| Unit tests | `#[cfg(test)] mod tests` in source files | Private internals, pure logic |
| Integration tests | `tests/*.rs` | Public API, end-to-end scenarios |
| Property tests | Inside unit or integration tests | Edge cases via `proptest` |
| Benchmarks | `benches/*.rs` | Performance regression detection |
| Doc tests | `///` doc comment examples | API usage examples |

Shared test helpers go in `tests/common/mod.rs` (not `tests/common.rs`, which Cargo treats as a
test crate).

### Test Naming

Use descriptive `snake_case` names that read as a sentence:

```rust
#[test]
fn scan_skips_entries_on_different_device() { /* ... */ }

#[test]
fn hardlink_dedup_counts_physical_size_once() { /* ... */ }

#[test]
#[should_panic(expected = "root path not found")]
fn scan_panics_on_missing_root() { /* ... */ }
```

### Platform-Specific Tests

Use `#[cfg_attr]` to skip tests on unsupported platforms. This makes skipped tests visible in test
output (as "ignored") rather than silently absent:

```rust
#[test]
#[cfg_attr(target_os = "macos", ignore = "APFS rejects non-UTF-8 filenames")]
fn non_utf8_filename_roundtrips_through_storage() { /* ... */ }
```

Use `cfg!(target_os = "...")` runtime checks for conditional assertions within a shared test:

```rust
#[test]
fn physical_size_reflects_allocation() {
    // ...
    if cfg!(target_os = "macos") {
        // APFS does not support sparse files
        assert_eq!(physical, logical);
    } else {
        assert!(physical < logical);
    }
}
```

### Privileged Tests

Tests requiring root (loopback mounts, cross-device detection) must:

1. Be marked `#[ignore]`.
2. Check `nix::unistd::geteuid().is_root()` at the start and skip gracefully if not root.
3. Document the privilege requirement in the test name or a comment.

Run with: `cargo test -- --ignored`

### Property-Based Testing

Use `proptest` for edge cases in path handling, size calculations, and aggregation logic:

```rust
proptest! {
    #[test]
    fn aggregated_size_equals_sum_of_children(
        sizes in prop::collection::vec(0u64..u64::MAX / 1000, 1..100)
    ) {
        let total: u64 = sizes.iter().sum();
        let aggregated = aggregate(&sizes);
        prop_assert_eq!(aggregated, total);
    }
}
```

### Filesystem Test Fixtures

Use `tempfile::TempDir` for test directories. Create known filesystem structures (files, symlinks,
hardlinks, nested directories) and assert scan results match expected metadata.

```rust
#[test]
fn scan_collects_file_metadata() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("file.txt"), "hello")?;

    let entries = scan(dir.path())?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].size(), 5);
    Ok(())
}
```


## Benchmarks

Benchmarks use [Criterion](https://bheisler.github.io/criterion.rs/book/) with `harness = false`.

### Conventions

- Each benchmark file in `benches/` must be declared in `Cargo.toml`.
- Use `std::hint::black_box` to prevent optimizer elimination (not Criterion's deprecated re-export).
- Group related benchmarks with `criterion_group!`.
- Use descriptive benchmark IDs: `c.bench_function("scan_10k_files", |b| ...)`.

### Running Benchmarks

```bash
cargo bench                          # run all benchmarks
cargo bench --bench b01_end_to_end   # run a specific benchmark
```

Benchmark results are written to `target/criterion/` with HTML reports.


## Git Workflow

### Branches

- `main` — stable, release-ready code. Protected.
- Feature branches: `feat/<short-description>`
- Bug fixes: `fix/<short-description>`
- Research/experiments: `research/<topic>`

### Commit Messages

Follow the [Conventional Commits](https://www.conventionalcommits.org/) specification:

```
<type>(<scope>): <description>

[optional body]

[optional footer(s)]
```

**Types:** `feat`, `fix`, `refactor`, `test`, `bench`, `docs`, `build`, `ci`, `chore`, `perf`.

**Scopes:** `scanner`, `storage`, `ui`, `cli`, `platform`, `research`.

Examples:

```
feat(scanner): add cross-device boundary detection
fix(storage): handle non-UTF-8 paths in sqlite queries
test(scanner): add hardlink dedup property tests
docs(spec): update performance benchmarks for FreeBSD
refactor(platform): extract filesystem type detection to Tier 3 module
```

### Pre-Commit Checklist

Before every commit:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Never commit code that fails any of these checks.


## Pull Request Process

1. **Branch from `main`** and keep your branch up to date.
2. **One concern per PR.** A bug fix should not include unrelated refactoring.
3. **All CI checks must pass:** `fmt`, `clippy`, `test` on all three platforms.
4. **Include tests** for new functionality and bug fixes.
5. **Update documentation** if the change affects public API, architecture, or the specification.
6. **PR description** must explain what changed and why, not just what files were modified.

### Review Criteria

- Does the code follow the conventions in this document?
- Are cross-platform implications considered?
- Is the extensibility pattern appropriate (enum vs trait vs sealed trait)?
- Are error cases handled, not silently swallowed?
- Do tests cover the logic the compiler cannot verify?
- Is the change minimal — no unrelated cleanup, no speculative features?
