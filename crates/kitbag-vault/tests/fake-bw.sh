#!/bin/bash
# A `bw` that holds one item in a directory, enough to exercise the path a
# payload takes when it is too big for a note. It is not a vault and does not
# pretend to be one: it answers the calls bw.rs makes, in the shapes it makes
# them, so that a change to those shapes is caught by a test rather than by a
# push against someone's real vault.
set -euo pipefail
S="$KITBAG_FAKE_STATE"
mkdir -p "$S/attachments"

case "$1 ${2:-}" in
    "status ")   printf '{"status":"unlocked"}' ;;
    "encode ")   base64 | tr -d '\n' ;;
    "list folders") cat "$S/folders.json" 2>/dev/null || echo '[]' ;;
    "list items")   cat "$S/items.json"   2>/dev/null || echo '[]' ;;

    "create folder")
        printf '{"id":"folder-1","name":"kitbag"}' | tee "$S/folder.json"
        printf '[{"id":"folder-1","name":"kitbag"}]' > "$S/folders.json"
        ;;

    "create item")
        # The item arrives base64-encoded, exactly as `bw encode` left it.
        printf '%s' "$3" | base64 -d > "$S/item.json"
        python3 - "$S" <<'PY'
import json, sys, pathlib
s = pathlib.Path(sys.argv[1])
item = json.loads((s / "item.json").read_text())
item["id"] = "item-1"
item.setdefault("attachments", [])
(s / "items.json").write_text(json.dumps([item]))
print(json.dumps(item))
PY
        ;;

    "create attachment")
        # bw takes --itemid and --file; the order here is the order bw.rs uses.
        file=""; prev=""; for a in "$@"; do [[ "$prev" == "--file" ]] && file="$a"; prev="$a"; done
        n=$(ls "$S/attachments" | wc -l | tr -d ' ')
        id="att-$((n + 1))"
        cp "$file" "$S/attachments/$id"
        python3 - "$S" "$id" "$(basename "$file")" <<'PY'
import json, sys, pathlib
s, att_id, name = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3]
items = json.loads((s / "items.json").read_text())
items[0].setdefault("attachments", []).append({"id": att_id, "fileName": name})
(s / "items.json").write_text(json.dumps(items))
PY
        ;;

    "get attachment")
        out=""; prev=""; for a in "$@"; do [[ "$prev" == "--output" ]] && out="$a"; prev="$a"; done
        python3 - "$S" "$3" <<'PY' > "$S/which"
import json, sys, pathlib
s, name = pathlib.Path(sys.argv[1]), sys.argv[2]
items = json.loads((s / "items.json").read_text())
# The newest attachment with that name, which is the one just written.
match = [a for a in items[0].get("attachments", []) if a["fileName"] == name]
print(match[-1]["id"] if match else "")
PY
        cp "$S/attachments/$(cat "$S/which")" "$out"
        ;;

    "delete attachment")
        python3 - "$S" "$3" <<'PY'
import json, sys, pathlib
s, att_id = pathlib.Path(sys.argv[1]), sys.argv[2]
items = json.loads((s / "items.json").read_text())
items[0]["attachments"] = [a for a in items[0].get("attachments", []) if a["id"] != att_id]
(s / "items.json").write_text(json.dumps(items))
PY
        rm -f "$S/attachments/$3"
        ;;

    "edit item")
        printf '%s' "$4" | base64 -d > "$S/new.json"
        python3 - "$S" <<'PY'
import json, sys, pathlib
s = pathlib.Path(sys.argv[1])
items = json.loads((s / "items.json").read_text())
new = json.loads((s / "new.json").read_text())
new["id"] = items[0]["id"]
new["attachments"] = items[0].get("attachments", [])
(s / "items.json").write_text(json.dumps([new]))
print(json.dumps(new))
PY
        ;;

    *) echo "fake bw: unhandled call: $*" >&2; exit 1 ;;
esac
