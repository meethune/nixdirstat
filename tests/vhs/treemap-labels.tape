# VHS tape for NixDirStat treemap filename label visual test.
# Produces a screenshot showing filename labels on large treemap cells.
#
# Usage:
#   bash tests/vhs/setup-test-data.sh /tmp/nixdirstat-vhs-data
#   cargo build
#   cargo run -- scan /tmp/nixdirstat-vhs-data --output /tmp/nixdirstat-vhs-scan.db
#   ~/go/bin/vhs tests/vhs/treemap-labels.tape

Output tests/vhs/screenshots/treemap-labels.png

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 800
Set Padding 10
Set Theme "Builtin Dark"

# Launch the explorer with a pre-scanned database
Type "./target/debug/nixdirstat explore /tmp/nixdirstat-vhs-scan.db"
Enter
Sleep 2s

# Capture the treemap with filename labels on large cells
Screenshot tests/vhs/screenshots/51-treemap-labels.png

# Quit
Type "q"
Sleep 500ms
