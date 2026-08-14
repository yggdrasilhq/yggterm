#!/usr/bin/env bash
# Real-key injection A/B, in a private sandbox GUI. Answers ONE question:
# "does `web do type` reach the page on this build, on this machine?" — with
# `web do click` as the control in the same run, on the same surface, at the
# same moment.
#
# ⭐ WHY A CONTROL IS NOT OPTIONAL HERE. `delivered:false` has two readings:
# the keys did not arrive, or the surface was never drivable at all. A click
# that answers `delivered:true` on the same surface separates them, and
# without it a broken sandbox reads exactly like a broken mechanism. Every arm
# below reports both.
#
# The sandbox is private by construction (own compositor, own HOME, own
# daemon, no window on anyone's screen), so this is safe to run on the machine
# that owns the live GUI — which is the point: the machine is the variable
# most worth flipping, and it cannot be flipped by testing elsewhere.
#
#   ./scripts/web-real-keys-ab.sh [--name <sandbox>] [--gui-bin <path>]
#
# --gui-bin is the other variable: point it at an older GUI build to A/B the
# version. Everything else is held fixed by this script on purpose.
#
# Exit 0 = keys delivered AND the value read back. Exit 1 = a real failure
# (control passed, arm did not). Exit 2 = the harness never got a surface, so
# the run says nothing about the mechanism either way.

set -uo pipefail

NAME=real-keys-ab
GUI_BIN=""
while [ $# -gt 0 ]; do
  case "$1" in
    --name) NAME="$2"; shift 2 ;;
    --gui-bin) GUI_BIN="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

REPO="$(cd "$(dirname "$0")/.." && pwd)"
SANDBOX="$REPO/scripts/underglass-sandbox.sh"
: "${XDG_RUNTIME_DIR:=/run/user/$(id -u)}"
HOME_DIR="$XDG_RUNTIME_DIR/yggterm-uglass/$NAME/home"

# The CLI is driven with the sandbox's env but the REAL binary path: the
# sandbox exports HOME, so any ~-relative path would resolve into the sandbox
# and fail with "no such file", which reads as a broken harness rather than a
# moved path.
CLI="$REPO/target/debug/yggterm-headless"
[ -x "$CLI" ] || CLI="$REPO/target/release/yggterm-headless"
[ -x "$CLI" ] || { echo "no yggterm-headless build in $REPO/target" >&2; exit 2; }

sb() { ( eval "$(bash "$SANDBOX" env --name "$NAME")"; "$CLI" "$@" ); }
field() { python3 -c 'import json,sys; d=json.load(sys.stdin).get("data",{}); print(json.dumps({k:d.get(k) for k in sys.argv[1:]}))' "$@"; }

echo "== start =="
mkdir -p "$HOME_DIR/.yggterm/apps"
# The app registry is per-HOME and read at startup, so the manifest has to be
# in place BEFORE the GUI comes up — installing it afterwards leaves the verb
# reported as unknown, which looks like a missing app rather than a late copy.
for manifest in "$HOME/.yggterm/apps/ychrome.json"; do
  [ -f "$manifest" ] && cp "$manifest" "$HOME_DIR/.yggterm/apps/"
done
if [ -n "$GUI_BIN" ]; then
  YGGTERM_GUI_BIN="$GUI_BIN" bash "$SANDBOX" start --name "$NAME" || exit 2
else
  bash "$SANDBOX" start --name "$NAME" || exit 2
fi
bash "$SANDBOX" backend --name "$NAME" | head -1

echo "== browser row =="
sb server app launch-app ychrome incognito >/dev/null 2>&1
sleep 10
ROW="$(sb server app rows 2>/dev/null | python3 -c '
import json, sys
for row in json.load(sys.stdin)["data"]["rows"]:
    if row.get("kind") == "Session":
        print(row["full_path"])
        break
')"
[ -n "$ROW" ] || { echo "no session row appeared"; exit 2; }
echo "row=$ROW"

# The profile picker is a GUI screen, not the page: it hands its choice to the
# app over the app's own control endpoint. Driving that endpoint directly is
# the same call the click makes, minus a pointer this sandbox may not have.
CONTROL="$(python3 - "$HOME_DIR/.yggterm/event-trace.jsonl" <<'PY'
import json, sys
url = None
for line in open(sys.argv[1], errors="ignore"):
    if "control_url" not in line:
        continue
    try:
        record = json.loads(line)
    except Exception:
        continue
    payload = record.get("payload", {})
    if isinstance(payload, dict) and payload.get("control_url"):
        url = payload["control_url"]
print(url or "")
PY
)"
[ -n "$CONTROL" ] && curl -sS -m 10 "${CONTROL}open?url=&profile=temp" -o /dev/null
sleep 10

echo "== surface =="
sb server app web ensure --session "$ROW" --ttl 600 2>/dev/null | field accepted reason
alive="$(sb server app web ensure --session "$ROW" --ttl 600 2>/dev/null \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"].get("probe_after",{}).get("alive"))')"
[ "$alive" = "True" ] || { echo "no live web surface (alive=$alive) — this run says nothing"; exit 2; }

echo "== probe input =="
sb server app web eval --session "$ROW" \
  "(function(){document.body.innerHTML='<input id=probe>';return document.visibilityState;})()" \
  2>/dev/null | field value

echo "== CONTROL: click =="
click="$(sb server app web do click --selector '#probe' --session "$ROW" 2>/dev/null \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"].get("delivered"))')"
echo "click delivered=$click"

echo "== ARM: type =="
typed="$(sb server app web do type --selector '#probe' --text 'abc' --session "$ROW" 2>/dev/null \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"].get("delivered"))')"
echo "type delivered=$typed"

echo "== read back =="
value="$(sb server app web eval --session "$ROW" "document.getElementById('probe').value" \
  2>/dev/null | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"].get("value"))')"
echo "value=$value"

bash "$SANDBOX" stop --name "$NAME" >/dev/null 2>&1

if [ "$click" != "True" ]; then
  echo "RESULT: inconclusive — the control failed too, so the surface was never drivable"
  exit 2
fi
if [ "$typed" = "True" ] && [ "$value" = "abc" ]; then
  echo "RESULT: real keys deliver and land"
  exit 0
fi
echo "RESULT: REPRODUCED — click delivers, keys do not (delivered=$typed value=$value)"
exit 1
