# justfile — local CI parity for nixdirstat
# Run `just` to list all available recipes.

set shell := ["bash", "-euo", "pipefail", "-c"]

# List available recipes
default:
    @just --list

# Install all development tools (run once after cloning)
setup:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "=== Checking Rust toolchain ==="
    rustc --version || { echo "ERROR: rustc not found. Install via https://rustup.rs"; exit 1; }
    cargo --version
    echo ""
    echo "=== Installing cargo tools ==="
    cargo install --locked cargo-deny cargo-audit cargo-llvm-cov cargo-mutants cargo-watch git-cliff
    echo ""
    echo "=== Checking system tools ==="
    missing=()
    command -v ttyd  >/dev/null 2>&1 || missing+=(ttyd)
    command -v ffmpeg >/dev/null 2>&1 || missing+=(ffmpeg)
    if [ ${#missing[@]} -gt 0 ]; then
        echo "WARNING: missing system packages (needed for VHS visual tests): ${missing[*]}"
        echo "  Debian/Kali: sudo apt install ${missing[*]}"
        echo "  macOS:       brew install ${missing[*]}"
        echo "  FreeBSD:     pkg install ${missing[*]}"
    else
        echo "  ttyd:   $(ttyd --version 2>&1 | head -1)"
        echo "  ffmpeg: $(ffmpeg -version 2>&1 | head -1)"
    fi
    echo ""
    echo "=== Checking VHS ==="
    if command -v vhs >/dev/null 2>&1; then
        echo "  vhs: $(vhs --version 2>&1)"
    elif [ -x "$HOME/go/bin/vhs" ]; then
        echo "  vhs: $($HOME/go/bin/vhs --version 2>&1) (at ~/go/bin/vhs)"
    else
        echo "WARNING: vhs not found. Install: go install github.com/charmbracelet/vhs@latest"
    fi
    echo ""
    echo "=== Checking optional toolchains (via rustup) ==="
    if command -v rustup >/dev/null 2>&1; then
        echo "  rustup: $(rustup --version 2>&1 | head -1)"
        echo "  For MSRV check:  rustup toolchain install 1.95"
        echo "  For Miri:        rustup toolchain install nightly && rustup component add miri --toolchain nightly"
    else
        echo "  rustup not found — 'just msrv' and 'just miri' require rustup"
        echo "  Install from: https://rustup.rs"
    fi
    echo ""
    echo "=== Verifying core pipeline ==="
    just check
    echo ""
    echo "=== Setup complete ==="

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

# === Visual Testing (TUI) ===

# Run full VHS visual test suite (requires vhs, ttyd, ffmpeg)
vhs: build
    bash tests/vhs/run-visual-tests.sh

# Run VHS explore view test only
vhs-explore: build
    bash tests/vhs/setup-test-data.sh /tmp/nixdirstat-vhs-data
    @rm -f /tmp/nixdirstat-vhs-scan.db
    cargo run -- scan /tmp/nixdirstat-vhs-data --output /tmp/nixdirstat-vhs-scan.db 2>/dev/null
    mkdir -p tests/vhs/screenshots
    ~/go/bin/vhs tests/vhs/explore.tape

# Run VHS batch scan test only
vhs-scan: build
    bash tests/vhs/setup-test-data.sh /tmp/nixdirstat-vhs-data
    mkdir -p tests/vhs/screenshots
    ~/go/bin/vhs tests/vhs/scan-batch.tape

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

# Check release binary size against 5MB threshold
binary-size:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release
    SIZE=$(stat -c%s target/release/nixdirstat 2>/dev/null || stat -f%z target/release/nixdirstat)
    echo "Binary size: $((SIZE / 1024))KB ($SIZE bytes)"
    if [ "$SIZE" -gt 5242880 ]; then
        echo "ERROR: binary exceeds 5MB limit (spec: 3-5MB stripped+LTO)"
        exit 1
    fi

# Dry-run a crates.io publish
publish-dry:
    cargo publish --dry-run

# Generate changelog for the latest release
changelog:
    git-cliff --latest --strip header
