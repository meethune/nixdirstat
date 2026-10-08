# VHS tape for logical-sizes warning banner on btrfs.
# Requires: btrfs loopback at /tmp/nixdirstat-btrfs-mount/test-data
# Set up with: sudo bash tests/vhs/setup-btrfs-loopback.sh
Output tests/vhs/screenshots/logical-warning.png

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 800
Set Padding 10
Set Theme "Builtin Dark"

# Scan the btrfs loopback — triggers logical-sizes warning
Type "cargo run -- scan /tmp/nixdirstat-btrfs-mount/test-data"
Enter

# Wait for scan to complete and explorer to appear
Sleep 5s
Screenshot tests/vhs/screenshots/20-logical-warning-explorer.png

Sleep 1s

# Quit
Type "q"
Sleep 500ms
