# justfile — local CI parity for nixdirstat
# Run `just` to list all available recipes.

set shell := ["bash", "-euo", "pipefail", "-c"]

# List available recipes
default:
    @just --list

# === Core Development ===

# Full CI check: fmt, clippy, test, doc, deny
check: fmt-check lint test doc-build deny

# Build in debug mode
build:
    cargo build

# Build in release mode
build-release:
    cargo build --release

# Run the binary
run *ARGS:
    cargo run -- {{ ARGS }}

# Run all tests
test:
    cargo test --features parallel

# Run tests with stdout visible
test-verbose:
    cargo test --features parallel -- --nocapture

# Run a specific test by name
test-single NAME:
    cargo test {{ NAME }}

# Run privileged tests (requires root)
test-privileged:
    cargo test --features parallel -- --ignored

# Build and open documentation
doc:
    cargo doc --no-deps --features parallel --open

# Build documentation without opening
doc-build:
    cargo doc --no-deps --features parallel

# Watch for changes and re-run tests
watch:
    cargo watch -x 'test --features parallel'

# === Linting & Formatting ===

# Format code
fmt:
    cargo fmt

# Check formatting without modifying files
fmt-check:
    cargo fmt -- --check

# Run clippy with CI-equivalent flags
lint:
    cargo clippy --all-targets --features parallel -- -D warnings

# Run clippy and auto-fix what it can
lint-fix:
    cargo clippy --all-targets --features parallel --fix --allow-dirty

# === Security & Audit ===

# Run cargo-deny supply chain checks
deny:
    cargo deny check

# Run cargo-audit advisory database check
audit:
    cargo audit --deny warnings

# === Coverage ===

# Generate LCOV coverage report
coverage:
    cargo llvm-cov --features parallel --lcov --output-path lcov.info

# Generate HTML coverage report
coverage-html:
    cargo llvm-cov --features parallel --html --output-dir coverage-html

# Print coverage summary to stdout
coverage-summary:
    cargo llvm-cov --features parallel --summary-only

# === Advanced Testing ===

# Check against minimum supported Rust version
msrv:
    cargo +1.95 check --features parallel

# Run tests under Miri for undefined behavior detection
miri:
    cargo +nightly miri test

# Run benchmarks
bench:
    cargo bench --workspace

# Run mutation testing
mutants:
    cargo mutants --output mutants.out --json

# === Release ===

# Dry-run a crates.io publish
publish-dry:
    cargo publish --dry-run

# Generate changelog for the latest release
changelog:
    git-cliff --latest --strip header
