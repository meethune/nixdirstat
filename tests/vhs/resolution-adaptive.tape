# VHS tape for NixDirStat resolution-adaptive rendering visual test.
# Verifies that visualization adapts to different terminal sizes:
# - Labels thin out at higher resolutions
# - Vignettes scale appropriately
# - Directory indents widen
#
# Usage:
#   bash tests/vhs/setup-test-data.sh /tmp/nixdirstat-vhs-data
#   cargo build
#   ~/go/bin/vhs tests/vhs/resolution-adaptive.tape

Output tests/vhs/screenshots/resolution-adaptive.png

Set Shell "bash"
Set FontSize 14
Set Padding 10
Set Theme "Builtin Dark"

# Test at 80×24 (smallest terminal)
Set Width 640
Set Height 240
Type "cargo run -- scan /tmp/nixdirstat-vhs-data"
Enter
Sleep 3s
Screenshot tests/vhs/screenshots/90-resolution-80x24.png
Type "q"
Sleep 500ms

# Test at 120×40 (medium terminal)
Set Width 960
Set Height 400
Type "cargo run -- scan /tmp/nixdirstat-vhs-data"
Enter
Sleep 3s
Screenshot tests/vhs/screenshots/91-resolution-120x40.png
Type "q"
Sleep 500ms

# Test at 200×60 (large terminal)
Set Width 1600
Set Height 600
Type "cargo run -- scan /tmp/nixdirstat-vhs-data"
Enter
Sleep 3s
Screenshot tests/vhs/screenshots/92-resolution-200x60.png
Type "q"
Sleep 500ms

# Test at 300×80 (very large terminal, triggers split view)
Set Width 2400
Set Height 800
Type "cargo run -- scan /tmp/nixdirstat-vhs-data"
Enter
Sleep 3s
Screenshot tests/vhs/screenshots/93-resolution-300x80.png
Type "q"
Sleep 500ms
