# VHS tape for NixDirStat i18n French translation demo.
# Proves the --lang flag and French catalog work end-to-end.
#
# Usage:
#   bash tests/vhs/setup-test-data.sh /tmp/nixdirstat-vhs-data
#   cargo build
#   cargo run -- scan /tmp/nixdirstat-vhs-data --output /tmp/nixdirstat-vhs-scan.db
#   ~/go/bin/vhs tests/vhs/i18n-french.tape

Output tests/vhs/screenshots/i18n-french.png

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 800
Set Padding 10
Set Theme "Builtin Dark"

# Launch the explorer with --lang fr
Type "cargo run -- explore /tmp/nixdirstat-vhs-scan.db --lang fr"
Enter
Sleep 4s

# Capture the French explorer view
Screenshot tests/vhs/screenshots/90-i18n-french-explorer.png

# Open help overlay to show French keybinding descriptions
Type "?"
Sleep 1s
Screenshot tests/vhs/screenshots/91-i18n-french-help.png

# Close help
Type "?"
Sleep 1s

# Navigate to select a file, then open info popup
Down
Down
Down
Down
Sleep 500ms
Type "i"
Sleep 1s
Screenshot tests/vhs/screenshots/92-i18n-french-info.png

# Close info and quit
Type "i"
Sleep 200ms
Type "q"
Sleep 500ms
