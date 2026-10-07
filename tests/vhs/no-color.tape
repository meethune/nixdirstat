# VHS tape for NixDirStat NO_COLOR mode visual test.
# Produces a screenshot showing grayscale treemap rendering when NO_COLOR=1.
#
# Usage:
#   bash tests/vhs/setup-test-data.sh /tmp/nixdirstat-vhs-data
#   cargo build
#   cargo run -- scan /tmp/nixdirstat-vhs-data --output /tmp/nixdirstat-vhs-scan.db
#   ~/go/bin/vhs tests/vhs/no-color.tape

Output tests/vhs/screenshots/no-color.png

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 800
Set Padding 10
Set Theme "Builtin Dark"

# Launch the explorer with NO_COLOR=1 to trigger grayscale fallback
Type "NO_COLOR=1 ./target/debug/nixdirstat explore /tmp/nixdirstat-vhs-scan.db"
Enter
Sleep 2s

# Capture the grayscale treemap (no Okabe-Ito colours)
Screenshot tests/vhs/screenshots/80-no-color.png

# Quit
Type "q"
Sleep 500ms
