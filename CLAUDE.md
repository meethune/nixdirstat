# NixDirStat Development Directives

## ALL DEVELOPMENT DIRECTIVES ARE INVIOLABLE. IMMUTABLE. YOU ARE FORBIDDEN FROM DEVIATION.

## Project Overview

NixDirStat is a disk usage analyzer and cleanup assistant for Unix and Unix-like systems
(Linux, macOS, FreeBSD). Scan local filesystems, devices, and directories then explore
disk usage through sortable file lists, file-type statistics, and interactive treemaps.

## Commands

```bash
just setup          # First-time setup: installs all tools, verifies everything works
just check          # Full CI: fmt, clippy, test, doc, deny
cargo test          # Run all tests
cargo run -- <path> # Interactive scan (bare path shorthand)
cargo run -- scan <path> --output out.db  # Batch scan to file
just lint           # Clippy with CI flags
just vhs            # VHS visual test suite
cargo bench         # Run criterion benchmarks
just audit          # Security advisory check (requires cargo-audit)
just binary-size    # Check release binary stays under 5MB threshold
just coverage-summary  # Print code coverage summary
```

## CLI Usage

`nixdirstat <path>` is shorthand for `nixdirstat scan <path>` (interactive TUI).
Subcommands: `scan`, `explore`, `export`. See `nixdirstat --help`.

## Development Methodology: CDD + TDD

- Compiler Driven Development (CDD) and Test Driven Development (TDD) are MANDATORY and complementary.
- CDD first: design types to encode invariants — use newtypes, enums, and the typestate pattern to make illegal states unrepresentable. Let compiler errors guide structural correctness.
- TDD for logic the compiler cannot verify: algorithm correctness, I/O behavior, aggregation accuracy, cross-device detection, hardlink dedup, SQLite query results. Red-green-refactor for these.
- Property-based testing (`proptest`) SHOULD be used for edge cases in path handling, size calculations, and aggregation logic.
- CDD replaces: null-check tests, type-validation tests, state-transition-validity tests, exhaustiveness tests. Do NOT write tests for things the compiler already guarantees.

## Code Quality

- Don't Repeat Yourself (DRY) patterns are MANDATORY.
- Defensive Programming is MANDATORY.
- Code readability is ESSENTIAL.
- Avoid deep nesting. Invert conditional statements to handle exit conditions early (early returns, `?` operator).
- Combine related validation checks into a single condition to improve readability.

## Rust Conventions

### Safety
- `unsafe` code is FORBIDDEN. No `unsafe` blocks, `unsafe impl`, or `unsafe fn` in this project.
- If a dependency requires `unsafe` internally that is acceptable, but this project's code MUST be 100% safe Rust.
- NEVER use `#[allow(...)]` or suppression directives to bypass clippy, compiler warnings, or security scanner errors. Except for verified false positives with a comment explaining why.

### Style (Rust Style Guide RFC 2436 + API Guidelines + Clippy)
- `cargo fmt` (rustfmt) for formatting is MANDATORY. Code MUST pass `cargo fmt --check`. Project-specific overrides are in `rustfmt.toml`.
- `cargo clippy --all-targets -- -D warnings` for linting is MANDATORY. Treat all clippy warnings as errors.
- Naming follows Rust API Guidelines (RFC 430): `snake_case` for functions/variables, `UpperCamelCase` for types/traits, `SCREAMING_SNAKE_CASE` for constants.
- Conversion methods use standard prefixes: `as_` (cheap ref-to-ref), `to_` (expensive conversion), `into_` (ownership transfer).
- Boolean methods use `is_` / `has_` predicates. No `get_` prefix on getters.

### Project Structure
- Rust stable toolchain, edition 2024, MSRV 1.95.
- Thin `main.rs` calling `lib.rs`. Benchmarks in `benches/` with criterion (`harness = false`). Integration tests in `tests/`.
- Error handling: `thiserror` for library/domain errors (typed enums). `anyhow` for application/CLI code.
- SQLite schema version is 2 (`SCHEMA_VERSION` in `storage/sqlite.rs`). The `entries` table includes a `category` column for SQL-level `FileCategory` aggregation. Bump the version when changing the schema.
- Use `std::hint::black_box` in benchmarks, not criterion's deprecated re-export.

### Cross-Platform Compatibility

Four-tier preference hierarchy — always use the highest tier available:

- **Tier 1 — Portable POSIX (always prefer):** APIs that work identically across Linux/FreeBSD/macOS with zero platform code. The vast majority of the application lives here.
- **Tier 2 — Portable call, platform-conditional interpretation:** Use `#[cfg]` on the *interpretation*, not the call.
- **Tier 3 — Platform-gated implementation (`#[cfg]` compile-time):** Per-platform implementations. Always include a catch-all `#[cfg(not(any(...)))]` that returns `Err`/`None`/`"unknown"`.
- **Tier 4 — External tool invocation (last resort):** Only for tools that are genuinely external. Always gate with `#[cfg]`.

Rules: Never use `unsafe`. Never use `/proc` outside `#[cfg(target_os = "linux")]`. Never shell out for something `nix`/`sysinfo`/`std` can do. Use `libc::mode_t` for type casts. Document platform behavioral differences with `cfg!()` runtime checks in tests.

## Build and Integration

- NEVER commit code without verifying build succeeds (`cargo check` minimum, `cargo test` preferred).
- Full CI parity locally: `just check` runs fmt, clippy, test, doc, deny.
- New components/services MUST be wired into their entry points (main, CLI) before considering the implementation complete.
- Git commit messages MUST adhere to Conventional Commits specification.

## Visual Testing (TUI)

- Any change to TUI rendering (widgets, layout, colors, views) MUST be verified with VHS visual tests before delivery.
- Unit tests with `TestBackend` verify text presence and structure but NOT visual quality — colors, proportions, layout, and real-data behavior require VHS.
- Run `just vhs` to execute the full VHS visual test suite. Inspect the screenshots in `tests/vhs/screenshots/` to verify the TUI looks correct.
- Always test against real-world data (not just synthetic test fixtures). Bugs like missing indexes and ancestor-chain blowups only surface at scale.
- VHS tapes live in `tests/vhs/`. Add new tapes when adding new views or interactions.

## Research and Decision Making

- NEVER present unverified guesses as evidence-based recommendations. State uncertainty explicitly.
- NEVER guess. About anything. Read the documentation FIRST.

## Key Documents

- **Specification:** `docs/specification.md` — architecture, data model, technology decisions, research findings.
- **Contributing:** `CONTRIBUTING.md` — code style, naming, cross-platform patterns, extensibility, testing, git workflow.

<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **nixdirstat**.

> Index stale? Run `node .gitnexus/run.cjs analyze --index-only` from the project root — it auto-selects an available runner. No `.gitnexus/run.cjs` yet? Bootstrap with `npx`, `bunx`, or `pnpm dlx` — e.g. `bunx gitnexus@latest analyze` (npm 11 npx crash; #1939).

## Always Do

- **MUST run impact before editing.** Use `impact({target: "symbolName", direction: "upstream"})` or `node .gitnexus/run.cjs impact "symbolName" --direction upstream --repo .`; report callers, processes, and risk. Never substitute grep for graph analysis.
- **MUST analyze graph changes before committing.** Use `detect_changes({scope: "all"})` (MCP) or `node .gitnexus/run.cjs detect-changes --scope all --repo .` (CLI fallback). `partial: true` or `truncated: true` is not a clean check — a zero means unseen, not unaffected; re-run it. For regression review: `detect_changes({scope: "compare", base_ref: "main"})` or `node .gitnexus/run.cjs detect-changes --scope compare --base-ref "main" --repo .`.
- MUST warn on HIGH/CRITICAL `risk` pre-edit; never use `riskSharedAxes` to waive a HIGH/CRITICAL `risk` warning. Compare File/symbol: MCP File omits axes; Graph-RAG expands File.
- **MUST treat `risk: UNKNOWN` as unresolved, not as low.** An empty caller set is not evidence the symbol is unused — it can also mean the callers are not resolvable by the index (plain-object property access, dynamic dispatch, cross-language calls). `impact` pairs `UNKNOWN` with a `riskNote` saying so. Confirm with a text search before treating the symbol as safe to change or delete; do not proceed on the strength of a zero.
- **MUST use `query({search_query: "concept"})` for concepts/flows, `context({name: "symbolName"})` for a named symbol, or `impact` for blast radius, on read-only callers, dependencies, imports, or execution flow.** Graph first; text search only for empty/`UNKNOWN`/literals.
- For security review, `explain({target: "fileOrSymbol"})` lists taint findings (source→sink flows; needs `analyze --pdg`).

## Never Do

- NEVER edit a function, class, or method before MCP/CLI impact analysis.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis, and never read `UNKNOWN` as an all-clear — it means the walk could not answer, which is the one verdict that requires confirming by other means.
- NEVER rename symbols with find-and-replace — use `rename` which understands the call graph.
- NEVER commit before MCP/CLI graph change analysis.

## Resources

| Resource | Use for |
| --- | --- |
| `gitnexus://repo/nixdirstat/context` | Codebase overview, check index freshness |
| `gitnexus://repo/nixdirstat/clusters` | All functional areas |
| `gitnexus://repo/nixdirstat/processes` | All execution flows |
| `gitnexus://repo/nixdirstat/process/{name}` | Step-by-step execution trace |

## CLI

| Task | Read this skill file |
| --- | --- |
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->
