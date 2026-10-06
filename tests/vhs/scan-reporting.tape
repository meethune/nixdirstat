Output tests/vhs/screenshots/scan-reporting.png

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 800
Set Padding 10
Set Theme "Builtin Dark"

Type "cargo run -- scan /home/izaak/Projects/Reporting"
Enter
Sleep 3s
Screenshot tests/vhs/screenshots/20-reporting-3s.png
Sleep 7s
Screenshot tests/vhs/screenshots/21-reporting-10s.png
Sleep 10s
Screenshot tests/vhs/screenshots/22-reporting-20s.png
Type "q"
Sleep 500ms
