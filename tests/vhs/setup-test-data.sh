#!/bin/bash
# Create a predictable test directory tree for VHS visual testing.
# Sizes are chosen so treemap cells are visibly distinct.
set -euo pipefail

DIR="${1:-/tmp/nixdirstat-vhs-data}"
rm -rf "$DIR"
mkdir -p "$DIR"

# Code files (~40% of total)
mkdir -p "$DIR/src"
dd if=/dev/zero bs=1024 count=200 2>/dev/null | tr '\0' 'a' > "$DIR/src/main.rs"
dd if=/dev/zero bs=1024 count=150 2>/dev/null | tr '\0' 'b' > "$DIR/src/lib.rs"
dd if=/dev/zero bs=1024 count=100 2>/dev/null | tr '\0' 'c' > "$DIR/src/utils.py"

# Images (~25% of total)
mkdir -p "$DIR/images"
dd if=/dev/zero bs=1024 count=120 2>/dev/null > "$DIR/images/photo.jpg"
dd if=/dev/zero bs=1024 count=80 2>/dev/null > "$DIR/images/icon.png"
dd if=/dev/zero bs=1024 count=50 2>/dev/null > "$DIR/images/logo.svg"

# Documents (~20% of total)
mkdir -p "$DIR/docs"
dd if=/dev/zero bs=1024 count=100 2>/dev/null | tr '\0' 'd' > "$DIR/docs/readme.md"
dd if=/dev/zero bs=1024 count=80 2>/dev/null | tr '\0' 'e' > "$DIR/docs/guide.txt"
dd if=/dev/zero bs=1024 count=20 2>/dev/null | tr '\0' 'f' > "$DIR/docs/notes.txt"

# Archives (~10% of total)
mkdir -p "$DIR/backups"
dd if=/dev/zero bs=1024 count=60 2>/dev/null > "$DIR/backups/data.tar.gz"
dd if=/dev/zero bs=1024 count=40 2>/dev/null > "$DIR/backups/old.zip"

# Small misc files (~5% of total)
echo "config = true" > "$DIR/config.toml"
echo "MIT License" > "$DIR/LICENSE"
mkdir -p "$DIR/empty-dir"

echo "Test data created in $DIR"
du -sh "$DIR"
