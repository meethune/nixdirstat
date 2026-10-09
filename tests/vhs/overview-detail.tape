# VHS tape for NixDirStat overview+detail split view visual test.
# Verifies that at high resolution (300×80), the explorer renders a split view
# showing overview on the left and detail panel on the right, with synchronized
# navigation between panels.
#
# Usage:
#   bash tests/vhs/setup-test-data.sh /tmp/nixdirstat-vhs-data
#   cargo build
#   cargo run -- scan /tmp/nixdirstat-vhs-data --output /tmp/nixdirstat-vhs-scan.db
#   ~/go/bin/vhs tests/vhs/overview-detail.tape

Output tests/vhs/screenshots/overview-detail.png

Set Shell "bash"
Set FontSize 14
Set Width 2400
Set Height 800
Set Padding 10
Set Theme "Builtin Dark"

# Launch the explorer with a pre-scanned database at high resolution
Type "./target/debug/nixdirstat explore /tmp/nixdirstat-vhs-scan.db"
Enter
Sleep 2s

# Capture the initial overview+detail split view
Screenshot tests/vhs/screenshots/110-overview-detail-initial.png

# Navigate down in the tree to select a different item
Down
Sleep 500ms
Screenshot tests/vhs/screenshots/111-overview-detail-after-nav.png

# Tab to focus the treemap panel to show the split layout more clearly
Tab
Sleep 500ms
Screenshot tests/vhs/screenshots/112-overview-detail-treemap-focus.png

# Navigate within the treemap to show interactive behavior
Right
Sleep 500ms
Screenshot tests/vhs/screenshots/113-overview-detail-treemap-nav.png

# Quit
Type "q"
Sleep 500ms
