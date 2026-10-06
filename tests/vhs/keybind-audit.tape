# Comprehensive keybinding audit for NixDirStat explorer view.
# Tests every documented keybinding against a real directory scan.
# Uses Reporting directory for realistic content.

Output tests/vhs/screenshots/keybind-audit.png

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 800
Set Padding 10
Set Theme "Builtin Dark"

# Launch explorer with pre-scanned Reporting data
Type "cargo run -- explore /tmp/nixdirstat-reporting.db"
Enter
Sleep 2s
Screenshot tests/vhs/screenshots/kb-01-initial.png

# === NAVIGATION ===
# Down (j) — move selection down
Down
Sleep 300ms
Screenshot tests/vhs/screenshots/kb-02-down.png

Down
Sleep 300ms
Screenshot tests/vhs/screenshots/kb-03-down2.png

# Up (k) — move selection up
Up
Sleep 300ms
Screenshot tests/vhs/screenshots/kb-04-up.png

# === EXPAND/COLLAPSE ===
# Right/Enter — expand directory
Right
Sleep 300ms
Screenshot tests/vhs/screenshots/kb-05-expand.png

# Down into expanded children
Down
Sleep 300ms
Screenshot tests/vhs/screenshots/kb-06-child.png

# Left — collapse
Left
Sleep 300ms
Screenshot tests/vhs/screenshots/kb-07-collapse.png

# === ZOOM ===
# z — zoom treemap to selected directory
# First select a directory
Down
Sleep 200ms
Type "z"
Sleep 500ms
Screenshot tests/vhs/screenshots/kb-08-zoom-in.png

# z again on a child — should zoom deeper
Down
Sleep 200ms
Right
Sleep 200ms
Down
Sleep 200ms
Type "z"
Sleep 500ms
Screenshot tests/vhs/screenshots/kb-09-zoom-deeper.png

# Z — zoom back to root
Type "Z"
Sleep 500ms
Screenshot tests/vhs/screenshots/kb-10-zoom-root.png

# Left/Backspace — zoom out one level (after zooming in first)
Down
Sleep 200ms
Type "z"
Sleep 300ms
Screenshot tests/vhs/screenshots/kb-11-zoom-in-again.png
Backspace
Sleep 300ms
Screenshot tests/vhs/screenshots/kb-12-zoom-out.png

# === SORTING ===
# n — sort by name
Type "n"
Sleep 300ms
Screenshot tests/vhs/screenshots/kb-13-sort-name.png

# s — sort by size (default)
Type "s"
Sleep 300ms
Screenshot tests/vhs/screenshots/kb-14-sort-size.png

# r — reverse sort
Type "r"
Sleep 300ms
Screenshot tests/vhs/screenshots/kb-15-sort-reverse.png

# Back to normal
Type "r"
Sleep 200ms
Type "s"
Sleep 300ms

# === FOCUS ===
# Tab — switch focus to legend
Tab
Sleep 300ms
Screenshot tests/vhs/screenshots/kb-16-focus-legend.png

# Down in legend — scroll
Down
Down
Down
Sleep 300ms
Screenshot tests/vhs/screenshots/kb-17-legend-scroll.png

# Tab — switch back to tree
Tab
Sleep 300ms
Screenshot tests/vhs/screenshots/kb-18-focus-tree.png

# === HOME/END ===
# g — jump to first
Type "G"
Sleep 300ms
Screenshot tests/vhs/screenshots/kb-19-end.png

Type "g"
Sleep 300ms
Screenshot tests/vhs/screenshots/kb-20-home.png

# === HELP ===
# ? — toggle help
Type "?"
Sleep 500ms
Screenshot tests/vhs/screenshots/kb-21-help.png

# ? again — close help
Type "?"
Sleep 300ms
Screenshot tests/vhs/screenshots/kb-22-help-close.png

# === QUIT ===
Type "q"
Sleep 500ms
