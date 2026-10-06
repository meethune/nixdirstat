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

# Clean screenshots
rm -rf "$SCREENSHOT_DIR"
mkdir -p "$SCREENSHOT_DIR"

# Run batch scan first (creates the DB for explore test)
echo "Running batch scan tape..."
rm -f "$SCAN_DB"
"$VHS" "$SCRIPT_DIR/scan-batch.tape" 2>&1 || echo "WARN: scan-batch tape had issues"

# If VHS didn't create the DB (it runs inside a pty), create it directly
if [ ! -f "$SCAN_DB" ]; then
    echo "Creating scan DB directly..."
    ./target/debug/nixdirstat scan "$TEST_DATA" --output "$SCAN_DB" 2>/dev/null
fi

# Run explore tape
echo "Running explore tape..."
"$VHS" "$SCRIPT_DIR/explore.tape" 2>&1 || echo "WARN: explore tape had issues"

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
