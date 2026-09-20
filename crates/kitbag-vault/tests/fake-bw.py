"""The state behind fake-bw.sh: a list of items in one JSON file."""
import json
import pathlib
import sys

state, op = pathlib.Path(sys.argv[1]), sys.argv[2]
store = state / "items.json"
items = json.loads(store.read_text()) if store.exists() else []


def save():
    store.write_text(json.dumps(items))


def index_of(item_id):
    return next(i for i, it in enumerate(items) if it["id"] == item_id)


if op == "list":
    print(json.dumps(items))

elif op == "one":
    print(json.dumps(items[index_of(sys.argv[3])]))

elif op == "create":
    item = json.loads(sys.stdin.read())
    item["id"] = f"item-{len(items) + 1}"
    item.setdefault("attachments", [])
    items.append(item)
    save()
    print(json.dumps(item))

elif op == "edit":
    at = index_of(sys.argv[3])
    new = json.loads(sys.stdin.read())
    new["id"] = items[at]["id"]
    new["attachments"] = items[at].get("attachments", [])
    items[at] = new
    save()
    print(json.dumps(new))

elif op == "attach":
    item_id, att_id, name = sys.argv[3], sys.argv[4], sys.argv[5]
    items[index_of(item_id)].setdefault("attachments", []).append(
        {"id": att_id, "fileName": name}
    )
    save()

elif op == "detach":
    item_id, att_id = sys.argv[3], sys.argv[4]
    at = index_of(item_id)
    items[at]["attachments"] = [
        a for a in items[at].get("attachments", []) if a["id"] != att_id
    ]
    save()

elif op == "find-attachment":
    item_id, name = sys.argv[3], sys.argv[4]
    match = [a for a in items[index_of(item_id)].get("attachments", []) if a["fileName"] == name]
    print(match[-1]["id"] if match else "")

else:
    sys.exit(f"fake-bw.py: unknown op {op}")
