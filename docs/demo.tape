# kitbag — the README GIF.
#
#   cargo build --release && vhs docs/demo.tape   →   docs/demo.gif
#
# What the viewer sees is the real binary: the real grouping, the real markers,
# the real wrapping. What it reports is a sample, because the engine behind the
# commands is not built yet — the last line of the run says so, on purpose. A
# demo that implied otherwise would be the first thing a reader found untrue.
#
# Arc:
#   0–4s    the question   — `kitbag status`, and what the answer looks like
#   4–9s    the shape      — grouped by whose it is, marked with what would change
#   9–14s   the surface    — the commands, from --help

Output docs/demo.gif

# ffmpeg turns the frames into the GIF, and VHS reports its absence in a way
# that reads like a font problem. Fail on the tape instead.
Require ffmpeg

Set Shell zsh
Set FontSize 16
Set Width 1000
Set Height 700
Set Padding 24
Set Theme "Catppuccin Mocha"
Set TypingSpeed 55ms
Set PlaybackSpeed 1

Hide
Type "export PATH=$PWD/target/release:$PATH COLUMNS=94 CLICOLOR_FORCE=1"  Enter
Type "clear"  Enter
Show

Sleep 700ms
Type "kitbag status"  Sleep 400ms  Enter
Sleep 5s

Type "kitbag --help"  Sleep 400ms  Enter
Sleep 5s

Hide
Type "clear"  Enter
Show
Sleep 300ms

# Re-record after changing the sample report or the renderer:
#   cargo build --release && vhs docs/demo.tape
