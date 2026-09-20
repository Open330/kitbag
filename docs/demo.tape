# kitbag — the README GIF.
#
#   cargo build --release && vhs docs/demo.tape   →   docs/demo.gif
#
# The real binary against a made-up home, built by docs/demo-setup.sh - nothing
# here is anybody's actual machine, which is the rule this repository follows
# everywhere else.
#
# Arc:
#   0–6s    discover  — what is here that nothing is tracking
#   6–14s   status    — grouped by whose it is, marked against the store
#   14–20s  push      — and then a second push, which costs nothing

Output docs/demo.gif
Require ffmpeg

Set Shell zsh
Set FontSize 16
Set Width 1240
Set Height 740
Set Padding 24
Set Theme "Catppuccin Mocha"
Set TypingSpeed 45ms

Hide
Type "export PATH=$PWD/target/release:$PATH COLUMNS=96 CLICOLOR_FORCE=1" Enter
Type "export HOME=$(sh docs/demo-setup.sh)" Enter
Type "export KITBAG_CONFIG=$HOME/.config/kitbag/machine.toml" Enter
Type "clear" Enter
Show

Sleep 600ms
Type "kitbag discover" Sleep 300ms Enter
Sleep 5s

Hide
Type "kitbag discover --write >/dev/null" Enter
Type "clear" Enter
Show

Type "kitbag status --backend age" Sleep 300ms Enter
Sleep 6s

Hide
Type "clear" Enter
Show
Type "kitbag push --backend age" Sleep 300ms Enter
Sleep 3s
Type "kitbag push --backend age" Sleep 300ms Enter
Sleep 3s
