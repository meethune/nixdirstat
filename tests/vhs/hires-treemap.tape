# VHS tape for NixDirStat HalfBlock treemap visual test.
# Produces a screenshot showing the high-fidelity treemap with edge darkening and labels.
#
# Usage:
#   bash tests/vhs/setup-test-data.sh /tmp/nixdirstat-vhs-data
#   cargo build
#   cargo run -- scan /tmp/nixdirstat-vhs-data --output /tmp/nixdirstat-vhs-scan.db
#   ~/go/bin/vhs tests/vhs/hires-treemap.tape

Output tests/vhs/screenshots/hires-treemap.png

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

# Capture the HalfBlock treemap with edge darkening and labels
Screenshot tests/vhs/screenshots/50-hires-treemap.png

# Quit
Type "q"
Sleep 500ms
