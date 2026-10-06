# VHS tape for interactive scan test - tests scan-to-explore transition
Output tests/vhs/screenshots/scan-interactive.png

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 800
Set Padding 10
Set Theme "Builtin Dark"

# Scan a real directory (small enough to finish quickly)
Type "cargo run -- scan /tmp/nixdirstat-vhs-data"
Enter

# Wait for scan to complete and explorer to appear
Sleep 5s
Screenshot tests/vhs/screenshots/10-scan-interactive.png

Sleep 1s
Screenshot tests/vhs/screenshots/11-scan-interactive-2.png

# Quit
Type "q"
Sleep 500ms
