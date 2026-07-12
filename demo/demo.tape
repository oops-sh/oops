# The flagship demo, replayable: `make demo-gif` renders this tape to
# demo/demo.gif inside the sandbox-capable container (docker/demo.Dockerfile).
# Deterministic by construction: fixed geometry, fixed theme, fixed typing
# speed, no timestamps in any printed output.

Output demo/demo.gif

Set Shell "bash"
Set FontSize 20
Set Width 1160
Set Height 520
Set Padding 12
Set TypingSpeed 60ms
Set Theme "Dracula"

Hide
Type "cd /root/demo && mkdir -p project && echo 'fn main() {}' > project/main.rs && echo 'important data' > project/notes.txt && clear"
Enter
Show

Type "ls project/"
Enter
Sleep 1.5s

Type `oops run "rm -rf ./project"`
Enter
Sleep 2s

Type "oops diff"
Enter
Sleep 2.5s

Type "oops undo"
Enter
Sleep 2s

Type "ls project/"
Enter
Sleep 3.5s
