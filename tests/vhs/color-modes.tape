# VHS tape for NixDirStat color mode cycling visual test.
# Verifies that pressing 'c' cycles through color schemes (FileType → Mtime → Depth → FileType)
# and that legends change appropriately with each mode.
#
# Usage:
#   bash tests/vhs/setup-test-data.sh /tmp/nixdirstat-vhs-data
#   cargo build
#   cargo run -- scan /tmp/nixdirstat-vhs-data --output /tmp/nixdirstat-vhs-scan.db
#   ~/go/bin/vhs tests/vhs/color-modes.tape

Output tests/vhs/screenshots/color-modes.png

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 400
Set Padding 10
Set Theme "Builtin Dark"

# Launch the explorer with a pre-scanned database
Type "./target/debug/nixdirstat explore /tmp/nixdirstat-vhs-scan.db"
Enter
Sleep 2s

# Capture initial FileType color mode
Screenshot tests/vhs/screenshots/100-color-mode-filetype.png

# Cycle to Mtime mode
Type "c"
Sleep 500ms
Screenshot tests/vhs/screenshots/101-color-mode-mtime.png

# Cycle to Depth mode
Type "c"
Sleep 500ms
Screenshot tests/vhs/screenshots/102-color-mode-depth.png

# Cycle back to FileType mode
Type "c"
Sleep 500ms
Screenshot tests/vhs/screenshots/103-color-mode-filetype-again.png

# Quit
Type "q"
Sleep 500ms
