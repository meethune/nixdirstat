# VHS tape for NixDirStat explorer view visual testing.
# Produces screenshots of the TUI for automated inspection.
#
# Usage:
#   bash tests/vhs/setup-test-data.sh /tmp/nixdirstat-vhs-data
#   cargo build
#   cargo run -- scan /tmp/nixdirstat-vhs-data --output /tmp/nixdirstat-vhs-scan.db
#   ~/go/bin/vhs tests/vhs/explore.tape

Output tests/vhs/screenshots/explore.png
Output tests/vhs/screenshots/explore.gif

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

# Capture the initial explorer view
Screenshot tests/vhs/screenshots/01-explorer-initial.png

# Navigate down into src/ directory (press down a few times, then Enter)
Down
Down
Sleep 500ms
Screenshot tests/vhs/screenshots/02-explorer-selection.png

# Try sorting by name
Tab
Sleep 500ms
Screenshot tests/vhs/screenshots/03-explorer-sort-name.png

# Sort by type
Tab
Sleep 500ms
Screenshot tests/vhs/screenshots/04-explorer-sort-type.png

# Go back to size sort
Tab
Tab
Tab
Sleep 500ms

# Quit
Type "q"
Sleep 500ms
