# VHS tape for NixDirStat treemap panel navigation visual test.
# Verifies: Tab focuses treemap, arrow keys move selection highlight,
# Enter drills into a directory (breadcrumb updates), Backspace drills back out.
#
# Usage:
#   bash tests/vhs/setup-test-data.sh /tmp/nixdirstat-vhs-data
#   cargo build
#   cargo run -- scan /tmp/nixdirstat-vhs-data --output /tmp/nixdirstat-vhs-scan.db
#   ~/go/bin/vhs tests/vhs/treemap-navigation.tape

Output tests/vhs/screenshots/treemap-navigation.png

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

# Press Tab to move focus to the treemap panel
Tab
Sleep 500ms
Screenshot tests/vhs/screenshots/60-treemap-focused.png

# Move selection right within the treemap
Right
Sleep 500ms
Screenshot tests/vhs/screenshots/61-treemap-selection.png

# Drill into the selected directory (Enter)
Enter
Sleep 500ms
Screenshot tests/vhs/screenshots/62-treemap-drilled.png

# Drill back out (Backspace)
Backspace
Sleep 500ms
Screenshot tests/vhs/screenshots/63-treemap-drilled-back.png

# Quit
Type "q"
Sleep 500ms
