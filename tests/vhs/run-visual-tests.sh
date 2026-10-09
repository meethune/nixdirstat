#!/bin/bash
# Run VHS visual tests for NixDirStat.
# Produces screenshots in tests/vhs/screenshots/ for inspection.
#
# Usage: bash tests/vhs/run-visual-tests.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
VHS="${VHS:-$HOME/go/bin/vhs}"
TEST_DATA="/tmp/nixdirstat-vhs-data"
SCAN_DB="/tmp/nixdirstat-vhs-scan.db"
SCREENSHOT_DIR="$SCRIPT_DIR/screenshots"

cd "$PROJECT_DIR"

echo "=== NixDirStat Visual Tests ==="

# Check prerequisites
if ! command -v "$VHS" &>/dev/null; then
    echo "ERROR: vhs not found. Install: go install github.com/charmbracelet/vhs@latest"
    exit 1
fi

# Build
echo "Building..."
cargo build --quiet

# Create test data
echo "Creating test data..."
bash "$SCRIPT_DIR/setup-test-data.sh" "$TEST_DATA"

# Clean screenshots (preserve .gitkeep)
find "$SCREENSHOT_DIR" -mindepth 1 ! -name '.gitkeep' -delete
mkdir -p "$SCREENSHOT_DIR"

FAILURES=0

# Run batch scan first (creates the DB for explore test)
echo "Running batch scan tape..."
rm -f "$SCAN_DB"
if ! "$VHS" "$SCRIPT_DIR/scan-batch.tape" 2>&1; then
    echo "WARN: scan-batch tape had issues"
    FAILURES=$((FAILURES + 1))
fi

# If VHS didn't create the DB (it runs inside a pty), create it directly
if [ ! -f "$SCAN_DB" ]; then
    echo "Creating scan DB directly..."
    ./target/debug/nixdirstat scan "$TEST_DATA" --output "$SCAN_DB" 2>/dev/null
fi

# Run explore tape
echo "Running explore tape..."
if ! "$VHS" "$SCRIPT_DIR/explore.tape" 2>&1; then
    echo "WARN: explore tape had issues"
    FAILURES=$((FAILURES + 1))
fi

# Run explore-highlight tape
echo "Running explore-highlight tape..."
if ! "$VHS" "$SCRIPT_DIR/explore-highlight.tape" 2>&1; then
    echo "WARN: explore-highlight tape had issues"
    FAILURES=$((FAILURES + 1))
fi

# Run hires-treemap tape
echo "Running hires-treemap tape..."
if ! "$VHS" "$SCRIPT_DIR/hires-treemap.tape" 2>&1; then
    echo "WARN: hires-treemap tape had issues"
    FAILURES=$((FAILURES + 1))
fi

# Run treemap-navigation tape
echo "Running treemap-navigation tape..."
if ! "$VHS" "$SCRIPT_DIR/treemap-navigation.tape" 2>&1; then
    echo "WARN: treemap-navigation tape had issues"
    FAILURES=$((FAILURES + 1))
fi

# Run sub-block-bars tape
echo "Running sub-block-bars tape..."
if ! "$VHS" "$SCRIPT_DIR/sub-block-bars.tape" 2>&1; then
    echo "WARN: sub-block-bars tape had issues"
    FAILURES=$((FAILURES + 1))
fi

# Run treemap-labels tape
echo "Running treemap-labels tape..."
if ! "$VHS" "$SCRIPT_DIR/treemap-labels.tape" 2>&1; then
    echo "WARN: treemap-labels tape had issues"
    FAILURES=$((FAILURES + 1))
fi

# Run no-color tape
echo "Running no-color tape..."
if ! "$VHS" "$SCRIPT_DIR/no-color.tape" 2>&1; then
    echo "WARN: no-color tape had issues"
    FAILURES=$((FAILURES + 1))
fi

# Run i18n-french tape
echo "Running i18n-french tape..."
if ! "$VHS" "$SCRIPT_DIR/i18n-french.tape" 2>&1; then
    echo "WARN: i18n-french tape had issues"
    FAILURES=$((FAILURES + 1))
fi

# Run resolution-adaptive tape
echo "Running resolution-adaptive tape..."
if ! "$VHS" "$SCRIPT_DIR/resolution-adaptive.tape" 2>&1; then
    echo "WARN: resolution-adaptive tape had issues"
    FAILURES=$((FAILURES + 1))
fi

# Run color-modes tape
echo "Running color-modes tape..."
if ! "$VHS" "$SCRIPT_DIR/color-modes.tape" 2>&1; then
    echo "WARN: color-modes tape had issues"
    FAILURES=$((FAILURES + 1))
fi

# Run overview-detail tape
echo "Running overview-detail tape..."
if ! "$VHS" "$SCRIPT_DIR/overview-detail.tape" 2>&1; then
    echo "WARN: overview-detail tape had issues"
    FAILURES=$((FAILURES + 1))
fi

# Run logical-warning tape (requires btrfs loopback)
if [ -d "/tmp/nixdirstat-btrfs-mount/test-data" ]; then
    echo "Running logical-warning tape..."
    if ! "$VHS" "$SCRIPT_DIR/logical-warning.tape" 2>&1; then
        echo "WARN: logical-warning tape had issues"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo "SKIP: logical-warning tape (no btrfs loopback; run setup-btrfs-loopback.sh first)"
fi

echo ""
echo "=== Screenshots ==="
if ls "$SCREENSHOT_DIR"/*.png &>/dev/null 2>&1; then
    for f in "$SCREENSHOT_DIR"/*.png; do
        echo "  $(basename "$f") ($(du -h "$f" | cut -f1))"
    done
    echo ""
    echo "View screenshots in: $SCREENSHOT_DIR/"
else
    echo "  No screenshots produced. VHS may have encountered errors."
    echo "  Try running manually: $VHS $SCRIPT_DIR/explore.tape"
fi

if [ "$FAILURES" -gt 0 ]; then
    echo ""
    echo "ERROR: $FAILURES tape(s) failed."
    exit 1
fi
