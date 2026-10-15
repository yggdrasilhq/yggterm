#!/usr/bin/env bash
# lightpass-rig.sh — the [F7 s52] light-pass dedupe falsifier INSTRUMENT. v2
# Boot the harness first: bash tools/e2e/gui-harness.sh start
# Bars the seat judges from /tmp/s52-<label>/:
#   (a) clears/switch: fnV>=6 counters (full+light+forced) — bar 1; on fnV<6
#       (control) the counters CANNOT exist (the fields are new) — the
#       traceable count is xterm_forced_refresh/switch + the code-read light
#       clears, and the DIFFERENTIAL is the proof.
#   (b) ui.block max gap_ms in the switch window (differential).
#   (c) dwell leg: content burst, >=30s background dwell, one REAL click
#       back — full-coverage frame + stamp accuracy + legible screenshot.
# Usage: lightpass-rig.sh <label> [iters] [dwell_secs]
set -u
LABEL=${1:?label}
ITERS=${2:-3}
DWELL=${3:-35}
REPO=${REPO:-$HOME/gh/yggterm--f7light}
YGG=${YGG:-$HOME/.local/bin/yggterm}
PROBE="$REPO/tools/uxspeed/uxprobe.py"
OUT="/tmp/s52-$LABEL"
mkdir -p "$OUT"
export YGGTERM_HOME=/tmp/ygg-guiharness-home
export PATH="$HOME/.local/bin:$PATH"

CEXPR='return JSON.stringify({fnV: window.__yggtermMountFnV, hosts: Object.values(window.__yggtermXtermHosts).map((e) => ({h: String(e.hostId || ""), full: Number(e.fullHealCount || 0), light: Number(e.activationLightHealCount || 0), skip: Number(e.activationLightHealSkipCount || 0), forced: Number(e.forcedAtlasClearCount || 0)}))})'

phase() { echo; echo "== $* =="; }

counters() { # $1 = phase tag
  "$YGG" server app dom-eval "$CEXPR" > "$OUT/counters-$1.json" 2>&1
  python3 -c "import json;d=json.load(open('$OUT/counters-$1.json'));print(d['data']['result'])" 2>/dev/null || cat "$OUT/counters-$1.json"
}

phase "fnV + counters BEFORE"
counters before

phase "uxprobe switch x$ITERS (keep rows)"
T0=$(date +%s%3N); echo "$T0" > "$OUT/t0"
python3 "$PROBE" --actions switch --iters "$ITERS" --keep \
  --out "$OUT/switch.json" --artifacts "$OUT/artifacts" > "$OUT/switch.log" 2>&1
RC=$?
T1=$(date +%s%3N); echo "$T1" > "$OUT/t1"
echo "uxprobe rc=$RC window_ms=$((T1-T0))"
tail -3 "$OUT/switch.log"

phase "counters AFTER"
counters after

phase "trace slices inside the switch window"
T0V=$(cat "$OUT/t0"); T1V=$(cat "$OUT/t1")
python3 - "$YGGTERM_HOME/ytrace.jsonl" "$T0V" "$T1V" > "$OUT/window-trace.txt" <<'PY'
import json, sys
path, t0f, t1f = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
forced = skipped = blocks = 0
bmax = -1
frames = []
for line in open(path, errors="replace"):
    try:
        e = json.loads(line)
    except Exception:
        continue
    ts = e.get("ts_ms", 0)
    if not (t0f <= ts <= t1f):
        continue
    n = e.get("name", "")
    p = e.get("payload") or {}
    if n == "xterm_forced_refresh":
        forced += 1
        print(f"FORCED_REFRESH ts={ts} atlas_cleared={p.get('atlas_cleared')} "
              f"heal_dedupe_age_ms={p.get('heal_dedupe_age_ms')}")
    elif n == "xterm_forced_refresh_skipped":
        skipped += 1
    elif n == "block":
        blocks += 1
        g = p.get("gap_ms")
        if isinstance(g, (int, float)) and g > bmax:
            bmax = g
    elif n == "frame_gap":
        frames.append((ts, p.get("rows_painted"), p.get("rows")))
print(f"SUMMARY forced_refresh={forced} skipped={skipped} ui_block={blocks} "
      f"ui_block_max_gap_ms={bmax} frame_gaps={frames}")
PY
cat "$OUT/window-trace.txt"

phase "dwell leg: content burst -> ${DWELL}s dwell -> one real click"
OTHER=$(python3 - "$OUT/switch.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
its = (d.get("actions", {}).get("switch", {}) or {}).get("iterations", [])
targets = list(dict.fromkeys(i["target"] for i in its if i.get("target")))
last = its[-1]["target"] if its else None
others = [t for t in targets if t != last]
print(others[0] if others else (last or ""))
PY
)
if [ -z "$OTHER" ]; then
  echo "RIG VACUOUS: no switch iterations — the dwell leg needs the pair"
  ls -la "$OUT/"; echo "RIG DONE (vacuous) label=$LABEL"; exit 3
fi
echo "dwell target row: $OTHER"
"$YGG" server app terminal send "$OTHER" --data $'seq 1 60\n' --allow-multiline \
  > "$OUT/send.json" 2>&1 || echo "send failed (see send.json)"
sleep 1
"$YGG" server app terminal read-buffer "$OTHER" --mode screen > "$OUT/burst-screen.txt" 2>&1 || true
echo "burst screen lines: $(grep -c . "$OUT/burst-screen.txt" 2>/dev/null || echo 0)"
"$YGG" server app tree select "$OTHER" > "$OUT/tree-select.json" 2>&1
sleep 1
RECTJSON=$("$YGG" server app dom-eval "const p = $(python3 -c "import json;print(json.dumps('$OTHER'))"); let el=null; for (const n of document.querySelectorAll('[data-sidebar-row-path]')) { if (n.getAttribute('data-sidebar-row-path') === p) { el=n; break; } } if (!el) { return 'MISSING'; } const r = el.getBoundingClientRect(); return JSON.stringify({x:r.x,y:r.y,w:r.width,h:r.height});")
echo "rect json: $RECTJSON"
XY=$(echo "$RECTJSON" | python3 -c "
import json, sys
d = json.load(sys.stdin)
raw = d['data']['result']
if raw in (None, 'MISSING'):
    print('MISS'); raise SystemExit
r = json.loads(raw)
print(int(r['x'] + min(r['w'] / 2, 120)), int(r['y'] + r['h'] / 2))
" 2>/dev/null)
if [ "$XY" = "MISS" ] || [ -z "$XY" ]; then
  echo "RIG VACUOUS: dwell row rect missing — virtualized out?"
  ls -la "$OUT/"; echo "RIG DONE (vacuous) label=$LABEL"; exit 3
fi
X=${XY%% *}; Y=${XY##* }
echo "dwell ${DWELL}s…"
sleep "$DWELL"
T2=$(date +%s%3N); echo "$T2" > "$OUT/t2"
"$YGG" server app pointer press --x "$X" --y "$Y" > /dev/null 2>&1
"$YGG" server app pointer release > /dev/null 2>&1
echo "clicked ($X,$Y); settle 2.5s"
sleep 2.5
T3=$(date +%s%3N); echo "$T3" > "$OUT/t3"
phase "counters AFTER DWELL SWITCH"
counters dwell
"$YGG" server app screenshot "$OUT/dwell-full.png" > "$OUT/shot.log" 2>&1
ls -la "$OUT/dwell-full.png" 2>/dev/null || cat "$OUT/shot.log"
phase "dwell-switch trace slice"
T2V=$(cat "$OUT/t2"); T3V=$(cat "$OUT/t3")
python3 - "$YGGTERM_HOME/ytrace.jsonl" "$T2V" "$T3V" > "$OUT/dwell-trace.txt" <<'PY'
import json, sys
path, t0f, t1f = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
for line in open(path, errors="replace"):
    try:
        e = json.loads(line)
    except Exception:
        continue
    ts = e.get("ts_ms", 0)
    if not (t0f <= ts <= t1f):
        continue
    n = e.get("name", "")
    c = e.get("category", "")
    p = e.get("payload") or {}
    if n in ("reveal_served", "reveal_raise_refused", "frame_gap", "block",
             "xterm_forced_refresh", "xterm_forced_refresh_skipped") or \
       (c == "xterm_paint" and n == "reveal"):
        keep = {k: p.get(k) for k in (
            "session_path", "host_id", "rows_painted", "rows", "gap_ms",
            "visible", "content_rows", "atlas_cleared",
            "heal_dedupe_age_ms") if k in p}
        print(f"{ts} {c}/{n} {json.dumps(keep)[:220]}")
PY
cat "$OUT/dwell-trace.txt"

phase "evidence index"
ls -la "$OUT/"
echo "RIG DONE label=$LABEL"
