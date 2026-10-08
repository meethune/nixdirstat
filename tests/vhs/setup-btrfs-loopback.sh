#!/bin/bash
# Create a btrfs loopback filesystem with zstd compression for VHS testing.
# Requires: sudo, btrfs-progs, losetup
# Creates test data that will show the logical-sizes warning banner.
set -euo pipefail

IMG="/tmp/nixdirstat-btrfs-test.img"
MNT="/tmp/nixdirstat-btrfs-mount"
DATA="$MNT/test-data"

cleanup() {
    sudo umount "$MNT" 2>/dev/null || true
    LOOP=$(losetup -j "$IMG" 2>/dev/null | cut -d: -f1)
    [ -n "$LOOP" ] && sudo losetup -d "$LOOP" 2>/dev/null || true
    rm -f "$IMG"
    rmdir "$MNT" 2>/dev/null || true
}

# Clean up any previous run
cleanup

# Create 128MB sparse image (btrfs requires ~109MB minimum)
truncate -s 128M "$IMG"

# Set up loop device and format as btrfs with zstd compression
LOOP=$(sudo losetup --find --show "$IMG")
sudo mkfs.btrfs -f -q "$LOOP"

# Mount with zstd compression
mkdir -p "$MNT"
sudo mount -o compress=zstd "$LOOP" "$MNT"
sudo chown "$(id -u):$(id -g)" "$MNT"

# Create test data — compressible files that will show the discrepancy
mkdir -p "$DATA/src" "$DATA/docs" "$DATA/images"

# Highly compressible: zeros compress ~100:1 on zstd
dd if=/dev/zero bs=1024 count=200 2>/dev/null > "$DATA/src/main.rs"
dd if=/dev/zero bs=1024 count=150 2>/dev/null > "$DATA/src/lib.rs"
dd if=/dev/zero bs=1024 count=100 2>/dev/null > "$DATA/docs/readme.md"
dd if=/dev/zero bs=1024 count=80  2>/dev/null > "$DATA/docs/guide.txt"
dd if=/dev/zero bs=1024 count=120 2>/dev/null > "$DATA/images/photo.jpg"

# Force write-back so btrfs compression kicks in
sync

echo "btrfs loopback ready at $DATA"
echo "Loop device: $LOOP"
du -sh "$DATA"
sudo btrfs filesystem du -s "$DATA" 2>/dev/null || true
