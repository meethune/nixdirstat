# VHS tape to test tree selection → treemap highlight syncing
Output tests/vhs/screenshots/highlight.png

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 800
Set Padding 10
Set Theme "Builtin Dark"

Type "cargo run -- explore /tmp/nixdirstat-vhs-scan.db"
Enter
Sleep 2s
Screenshot tests/vhs/screenshots/40-highlight-first.png

# Move down to second item
Down
Sleep 500ms
Screenshot tests/vhs/screenshots/41-highlight-second.png

# Move down again
Down
Sleep 500ms
Screenshot tests/vhs/screenshots/42-highlight-third.png

# Move down again
Down
Sleep 500ms
Screenshot tests/vhs/screenshots/43-highlight-fourth.png

Type "q"
Sleep 500ms
