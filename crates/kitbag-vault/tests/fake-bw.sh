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

# `create` and `edit` answer with the item, and kitbag parses that answer. A
# stub that returns nothing there turns into "EOF while parsing a value at
# line 1 column 0" several layers away, which says nothing about where it
# happened — once, on a loaded CI runner, and not reproducibly. Now it says.
speaks() {
    local out
    out="$(state "$@")"
    if [[ -z "$out" ]]; then
        echo "fake bw: state $1 produced nothing (args: $*)" >&2
        exit 1
    fi
    printf '%s' "$out"
}

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

    "create item")      printf '%s' "$3" | base64 -d | speaks create ;;
    "edit item")
        # Armed by a test. The real client refuses a write whose base another
        # machine has moved past, and node's own deprecation chatter arrives
        # on the same stream, ahead of the reason.
        if [[ -f "$S/stale-once" || -f "$S/refuse-edit" ]]; then
            rm -f "$S/stale-once"
            # Optionally, that other machine's write actually lands, which is
            # what turns the second look into a conflict rather than a retry.
            if [[ -f "$S/stale-also-writes" ]]; then
                rm -f "$S/stale-also-writes"
                state bump "$3"
            fi
            echo "(node:73029) [DEP0040] DeprecationWarning: The \`punycode\` module is deprecated." >&2
            echo "(Use \`node --trace-deprecation ...\` to show where the warning was created)" >&2
            echo "The client copy of this cipher is out of date. Resync the client and try again." >&2
            exit 1
        fi
        printf '%s' "$4" | base64 -d | speaks edit "$3"
        ;;
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
