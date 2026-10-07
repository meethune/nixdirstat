# VHS tape for NixDirStat directory tree sub-block proportional bars visual test.
# Verifies that the file tree panel renders smooth sub-block size bars for each entry.
#
# Usage:
#   bash tests/vhs/setup-test-data.sh /tmp/nixdirstat-vhs-data
#   cargo build
#   cargo run -- scan /tmp/nixdirstat-vhs-data --output /tmp/nixdirstat-vhs-scan.db
#   ~/go/bin/vhs tests/vhs/sub-block-bars.tape

Output tests/vhs/screenshots/sub-block-bars.png

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 800
Set Padding 10
Set Theme "Builtin Dark"

# Launch the explorer with a pre-scanned database
Type "cargo run -- explore /tmp/nixdirstat-vhs-scan.db"
Enter
Sleep 2s

# Capture the directory tree with sub-block proportional bars
Screenshot tests/vhs/screenshots/70-sub-block-bars.png

# Quit
Type "q"
Sleep 500ms
