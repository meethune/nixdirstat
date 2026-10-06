# VHS tape to test per-extension coloring in a directory full of build artifacts
Output tests/vhs/screenshots/deps-view.png

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 800
Set Padding 10
Set Theme "Builtin Dark"

Type "cargo run -- explore /tmp/nixdirstat-deps-scan.db"
Enter
Sleep 2s
Screenshot tests/vhs/screenshots/30-deps-initial.png
Type "q"
Sleep 500ms
