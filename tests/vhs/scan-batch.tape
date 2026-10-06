# VHS tape for NixDirStat batch scan visual testing.
# Tests the progress output on stderr during batch scan.
#
# Usage:
#   bash tests/vhs/setup-test-data.sh /tmp/nixdirstat-vhs-data
#   cargo build
#   ~/go/bin/vhs tests/vhs/scan-batch.tape

Output tests/vhs/screenshots/scan-batch.png

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 400
Set Padding 10
Set Theme "Builtin Dark"

# Run batch scan
Type "cargo run -- scan /tmp/nixdirstat-vhs-data --output /tmp/nixdirstat-vhs-scan.db"
Enter
Sleep 3s

# Capture the final output
Screenshot tests/vhs/screenshots/05-scan-batch-complete.png

Sleep 500ms
