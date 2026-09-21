#!/bin/bash
# A `bw` that keeps items in a directory: enough to exercise the paths bw.rs
# takes, and to count what it asks for. It is not a vault and does not pretend
# to be one. It answers the calls bw.rs makes, in the shapes it makes them, so
# that a change to those shapes — or to how many times they happen — is caught
# by a test rather than by a push against someone's real vault.
set -euo pipefail
S="$KITBAG_FAKE_STATE"
mkdir -p "$S/attachments"

# Every call, in order. `list items` decrypts a whole vault in the real client,
# so a test can assert how often kitbag asks for one.
echo "$1 ${2:-}" >> "$S/calls.log"

# The real client locks its own vault file, so kitbag may send several items
# at once. The mutation happens in fake-bw.py, which takes a real lock around
# it — a shell spinlock left a stale directory behind whenever anything failed
# and took the rest of the run down with it.
state() { python3 "$(dirname "${BASH_SOURCE[0]}")/fake-bw.py" "$S" "$@"; }

case "$1 ${2:-}" in
    "status ")      printf '{"status":"unlocked"}' ;;
    "sync ")        ;;   # the real one pulls the vault; here there is nowhere to pull from
    "encode ")      base64 | tr -d '\n' ;;
    "list folders") cat "$S/folders.json" 2>/dev/null || echo '[]' ;;
    "list items")   state list ;;
    "get item")     state one "$3" ;;

    "create folder")
        printf '[{"id":"folder-1","name":"kitbag"}]' > "$S/folders.json"
        printf '{"id":"folder-1","name":"kitbag"}'
        ;;

    "create item")      printf '%s' "$3" | base64 -d | state create ;;
    "edit item")        printf '%s' "$4" | base64 -d | state edit "$3" ;;
    "create attachment")
        file=""; itemid=""; prev=""
        for a in "$@"; do
            [[ "$prev" == "--file" ]] && file="$a"
            [[ "$prev" == "--itemid" ]] && itemid="$a"
            prev="$a"
        done
        n=$(find "$S/attachments" -type f | wc -l | tr -d ' ')
        id="att-$((n + 1))"
        cp "$file" "$S/attachments/$id"
        state attach "$itemid" "$id" "$(basename "$file")"
        ;;
    "get attachment")
        out=""; itemid=""; prev=""
        for a in "$@"; do
            [[ "$prev" == "--output" ]] && out="$a"
            [[ "$prev" == "--itemid" ]] && itemid="$a"
            prev="$a"
        done
        cp "$S/attachments/$(state find-attachment "$itemid" "$3")" "$out"
        ;;
    "delete attachment")
        # Armed by a test: the real client failed here two different ways.
        [[ -f "$S/refuse-delete" ]] && { echo "fake bw: refusing" >&2; exit 1; }
        itemid=""; prev=""
        for a in "$@"; do [[ "$prev" == "--itemid" ]] && itemid="$a"; prev="$a"; done
        state detach "$itemid" "$3"
        rm -f "$S/attachments/$3"
        ;;

    *) echo "fake bw: unhandled call: $*" >&2; exit 1 ;;
esac
