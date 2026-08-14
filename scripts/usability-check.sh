#!/usr/bin/env bash
# The hourly usability check. `docs/usability-contract.md` is the contract; this
# is its executable form, and the two are meant to be read together.
#
# It answers ONE question: is yggterm usable on the desktop host right now, and
# if not, what is the WORST thing wrong? Levels are ordered load-bearing first
# and the check STOPS at the first failure, because a ranked list of six is how
# the load-bearing item gets buried.
#
# ⛔ It never touches a human's session, and it never writes to a tmpfs. Level 4
#    creates its OWN ephemeral probe row with --no-activate, so the owner's
#    viewport never moves, and removes it with a read-back.
#
# Usage:
#   scripts/usability-check.sh            # levels 1, 1b, 1c, 2, 3, 4, 6
#   scripts/usability-check.sh --json     # one JSON object, for the relay/booter
#
# Exit code == the number of the first failing level, 0 if all pass.

set -uo pipefail

DEEP=0
JSON=0
for a in "$@"; do
  case "$a" in
    --deep) DEEP=1 ;;
    --json) JSON=1 ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
  esac
done

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# The live host is resolved by its one owner. There is deliberately NO fallback:
# a guessed hostname is both wrong on every other checkout and a private name in
# a public repo.
HOST="$("$REPO_ROOT/scripts/ygg-live-host.sh" 2>/dev/null)"
if [ -z "$HOST" ]; then
  echo "cannot resolve the live desktop host - scripts/ygg-live-host.sh is its one owner" >&2
  exit 64
fi
SSH="ssh -o BatchMode=yes -o ConnectTimeout=10 $HOST"
WORKDIR="${TMPDIR:-/tmp}/ygg-usability.$$"
mkdir -p "$WORKDIR"
trap 'rm -rf "$WORKDIR"' EXIT

FAIL_LEVEL=0
FAIL_WHAT=""
declare -a NOTES

note() { NOTES+=("$1"); }

fail() {
  # fail <level> <one-line what is wrong>
  if [ "$FAIL_LEVEL" -eq 0 ]; then FAIL_LEVEL="$1"; FAIL_WHAT="$2"; fi
}

emit() {
  if [ "$JSON" -eq 1 ]; then
    printf '{"host":"%s","ts":"%s","first_failing_level":%s,"what":"%s","notes":[' \
      "$HOST" "$(date -Iseconds)" "$FAIL_LEVEL" "${FAIL_WHAT//\"/\'}"
    local first=1
    for n in ${NOTES+"${NOTES[@]}"}; do
      [ $first -eq 1 ] || printf ','
      printf '"%s"' "${n//\"/\'}"
      first=0
    done
    printf ']}\n'
  else
    echo "=== yggterm usability check - host=$HOST $(date -Iseconds)"
    for n in ${NOTES+"${NOTES[@]}"}; do echo "  $n"; done
    if [ "$FAIL_LEVEL" -eq 0 ]; then
      # ⛔ "PASS" MUST NOT BE READ AS "THE TERMINAL RENDERS". When no session is
      # open, levels 2 and 3 never saw a canvas, and the open render faults are
      # untested rather than absent. Saying "all checked levels green" without
      # this qualifier is how a green tick comes to stand for a surface nobody
      # looked at.
      if [ "${TERMINAL_EXERCISED:-unknown}" = "yes" ]; then
        echo "PASS - all checked levels green"
      else
        echo "PASS - all checked levels green, but THE TERMINAL CANVAS WAS NOT EXERCISED (no session open): the render faults are untested this tick, not absent"
      fi
    else
      echo "FAIL at level $FAIL_LEVEL: $FAIL_WHAT"
    fi
  fi
  exit "$FAIL_LEVEL"
}

# ------------------------------------------------------------- level 0: armed?
# ⛔ IS THE WATCH ITSELF RUNNING? A successor inherits this seat by claiming it,
#    not by remembering a setup step, so the one thing that must never depend on
#    memory is the arming. This level is FIRST because every level below it is
#    worthless if nothing runs them when no session is awake.
#
# ⚠ It checks for a NEXT ELAPSE, not for "enabled". The first version of that
#    timer reported `enabled` and `active (running)` with `Trigger: n/a` — no
#    next run at all. A watchdog in that state reports healthy and never fires.
PANIC_NEXT="$(systemctl --user show ygg-panic.timer -p NextElapseUSecRealtime 2>/dev/null | cut -d= -f2-)"
PANIC_SUBS="$(ls "$HOME/.yggterm/relay/panic/subs"/*.json 2>/dev/null | wc -l)"
note "L0 panic_timer_next='${PANIC_NEXT:-none}' subscribers=${PANIC_SUBS:-0}"
if [ -z "${PANIC_NEXT:-}" ] || [ "${PANIC_NEXT}" = "n/a" ]; then
  note "L0 ⚠ THE RESOURCE WATCH IS NOT ARMED on this host - install it:"
  note "L0   cp .agents/systemd/ygg-panic.* ~/.config/systemd/user/ && systemctl --user daemon-reload && systemctl --user enable --now ygg-panic.timer"
fi
[ "${PANIC_SUBS:-0}" -eq 0 ] && note "L0 ⚠ no seat is subscribed - a breach would wake nobody (ygg-panic.py subscribe --row <addr>)"

# ---------------------------------------------------------------- level 1
# THE WINDOW IS THE PRODUCT. Exactly one GUI, running a live binary that
# matches what is installed. Everything below is meaningless if this fails,
# because a second GUI paints a window the user cannot escape by restarting.
#
# ⛔ NOT `pgrep -x yggterm`. THE CLI AND THE GUI ARE THE SAME BINARY, so counting
#    by process name counts every `yggterm server …` verb any agent on the fleet
#    happens to be running. Measured: baseline 2, **3** during one deliberate
#    `yggterm --version`, 1 once it exited. This check FAILED on that false
#    positive within its first two hours, and a level-1 alarm that cries wolf on
#    a busy fleet is worse than no alarm — it is how a real duplicate GUI gets
#    waved through.
#
# ⇒ Ask the product's own client registry, which is the SSOT for "which GUIs are
#   live" and which a CLI invocation never enters. Verified: it answers 1 while a
#   concurrent CLI call is in flight. Shadows are excluded deliberately — an
#   agent's read-only client is not a second window (same rule the launch guard
#   uses), and counting one would make every agent probe look like this failure.
L1="$($SSH 'set -o pipefail
  pids=$(~/.local/bin/yggterm server app clients 2>/dev/null \
    | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
except Exception:
    sys.exit(9)                      # unreadable != none; the caller must know
cl = d.get(\"clients\") or (d.get(\"data\") or {}).get(\"clients\") or []
for c in cl:
    role = (c.get(\"client_role\") or \"active\").lower()
    if role == \"active\":            # legacy record with no role reads as Active
        print(c.get(\"pid\"))
" | tr "\n" " ")
  if [ $? -eq 9 ]; then echo "count=unreadable"; exit 0; fi
  n=$(echo $pids | wc -w)
  echo "count=$n"
  for p in $pids; do
    exe=$(readlink /proc/$p/exe 2>/dev/null)
    age=$(ps -o etimes= -p $p 2>/dev/null | tr -d " ")
    echo "pid=$p age_s=${age:-0} exe=${exe:-unknown}"
  done
  echo "installed_md5=$(md5sum ~/.local/bin/yggterm 2>/dev/null | cut -d" " -f1)"
  for p in $pids; do echo "running_md5_$p=$(md5sum /proc/$p/exe 2>/dev/null | cut -d" " -f1)"; done
' 2>/dev/null)"

GUI_COUNT="$(sed -n 's/^count=//p' <<<"$L1")"
GUI_COUNT="${GUI_COUNT:-unreadable}"
note "L1 active_gui_clients=$GUI_COUNT"

# ⛔ "I could not ask" is not "there are none", and the two demand opposite
#    reactions: none means the user has no window, unreadable means this check
#    is blind. Reporting the second as the first would send someone to relaunch
#    a GUI that is running perfectly well.
if [ "$GUI_COUNT" = "unreadable" ]; then
  fail 1 "the client registry could not be read - this check is BLIND, not clear"
elif [ "$GUI_COUNT" -eq 0 ]; then
  fail 1 "no GUI process is running - the user has no window at all"
elif [ "$GUI_COUNT" -gt 1 ]; then
  # This is the failure that produced this whole contract. Name the oldest,
  # because that is the one owning the window the user is looking at.
  oldest="$(grep '^pid=' <<<"$L1" | sort -t= -k3 -rn | head -1)"
  fail 1 "$GUI_COUNT GUI processes are running; a restart ADDS one rather than replacing it. Oldest: $oldest"
fi

if grep -q '(deleted)' <<<"$L1"; then
  fail 1 "a GUI is running a DELETED binary - it cannot be the build that is installed"
fi

INSTALLED="$(sed -n 's/^installed_md5=//p' <<<"$L1")"
while IFS= read -r line; do
  [ -z "$line" ] && continue
  rmd5="${line#*=}"
  rpid="${line%%=*}"; rpid="${rpid#running_md5_}"
  if [ -n "$INSTALLED" ] && [ -n "$rmd5" ] && [ "$rmd5" != "$INSTALLED" ]; then
    fail 1 "GUI pid $rpid runs a build that differs from the installed binary"
  fi
done < <(grep '^running_md5_' <<<"$L1")

# ---------------------------------------------------------------- level 1b
# HAS IT CRASHED? Invisible to every other level, because a crash is followed
# by a relaunch that looks healthy. Four SIGSEGVs in 24h is what "janky as
# hell" actually was, and nothing in the app reports it.
CRASHES="$($SSH 'coredumpctl list --since "1 hour ago" -q --no-pager 2>/dev/null | grep -c yggterm' 2>/dev/null | tr -d ' ')"
CRASHES="${CRASHES:-0}"
note "L1b crashes_last_hour=$CRASHES"
[ "$CRASHES" -gt 0 ] && fail 1 "the GUI has crashed $CRASHES time(s) in the last hour (see coredumpctl)"

[ "$FAIL_LEVEL" -ne 0 ] && emit

# ---------------------------------------------------------------- level 1c
# ⛔ THE CHROME IS FROZEN BUT STILL PAINTED. This is the one that caught nothing
#    for three hours while the owner sat in front of it.
#
#    Edits reach the webview in batches over a websocket, acked per batch. A
#    batch that throws is acked ANYWAY (deliberately — a withheld ack starves
#    the whole VirtualDom). The host then records those mutations as landed and
#    diffs against that model forever, so nothing re-sends them: one subtree is
#    frozen at whatever it held, and every state field keeps confidently
#    reporting what it SHOULD be showing.
#
#    ⇒ The screenshot cannot catch this — a frozen sidebar paints perfectly, it
#    just paints the past. `capture_faithful` is true throughout. The owner
#    reported it as a "ghost" sidebar: visible, inert, buttons dead.
#    ⛔ Non-zero INVALIDATES THE SCREENSHOT, not the state. Only a GUI restart
#    clears it, so this level's remedy is a restart, not an investigation.
FAULTS="$($SSH "timeout 25 ~/.local/bin/yggterm server app state 2>/dev/null" \
  | python3 -c "
import sys, json
try:
    print(json.load(sys.stdin)['data'].get('webview_edit_faults', 'unknown'))
except Exception:
    print('unknown')
")"
note "L1c webview_edit_faults=${FAULTS:-unknown}"
# ⛔ UNREADABLE IS BLIND, NOT CLEAR — the same trap as level 1, and it was
#    reintroduced here within the hour of being fixed there, which is why it is
#    spelled out at both sites rather than assumed.
if [ "${FAULTS:-unknown}" = "unknown" ]; then
  fail 1 "could not read webview_edit_faults - the frozen-chrome check is BLIND, not clear"
elif [ "${FAULTS:-0}" -gt 0 ] 2>/dev/null; then
  fail 1 "webview_edit_faults=$FAULTS - the chrome is FROZEN and still painting; the screenshot is a lie. Restart the GUI, which is the only thing that clears it"
fi

[ "$FAIL_LEVEL" -ne 0 ] && emit

# ---------------------------------------------------------------- levels 2+3
# BOTH SIDEBARS RENDER, and THE VIEWPORT IS FAITHFUL. A visual symptom needs a
# faithful pixel: capture_faithful=false means the frame is a lie about the
# terminal, and telemetry has never once settled a rendering question here.
# ⛔ NOT /tmp. On the desktop host /tmp is a tmpfs, so every hourly screenshot
#    would be written into RAM on a machine already under memory pressure. A
#    check that costs the user memory to run is not a health check.
$SSH "mkdir -p ~/.yggterm/usability" >/dev/null 2>&1
SHOT_JSON="$($SSH "~/.local/bin/yggterm server app screenshot ~/.yggterm/usability/shot.png" 2>/dev/null)"
FAITHFUL="$(grep -o '"capture_faithful": *[a-z]*' <<<"$SHOT_JSON" | head -1 | grep -o '[a-z]*$')"
note "L2/3 capture_faithful=${FAITHFUL:-unknown}"
# ⛔⛔ `capture_faithful` DOES NOT MEAN "THE TERMINAL WAS CAPTURED".
#
# It means "this frame is an honest picture of what was on screen". With no
# session open the GUI shows the start page, the capture falls back to a
# compositor grab that never touches the xterm canvas, and it still reports
# `capture_faithful: true`. Measured 2026-08-14 17:30:
#   active_session_path = null
#   capture_backend     = linux_wayland_spectacle
#   capture_faithful    = true      <- on a frame with no terminal in it
#
# ⇒ Levels 2 and 3 exist to catch the three open RENDER faults, and all three
#   live in the xterm canvas. So this check could go green on a frame where the
#   canvas was never drawn — structurally blind to the only thing it is for.
#   ⛔ Blind is not clear, for the third time in this file.
BACKEND="$(grep -o '"capture_backend": *"[^"]*"' <<<"$SHOT_JSON" | head -1 | sed 's/.*"\([^"]*\)"$/\1/')"
SESSION_PATH="$(grep -o '"active_session_path": *[^,}]*' <<<"$SHOT_JSON" | head -1 | sed 's/.*: *//')"
# ⭐ THE BACKEND ALONE DECIDES, and deliberately so. Only the canvas-composite
# backend draws the xterm surface, so its presence is sufficient evidence that
# the canvas was exercised. `active_session_path` is carried as CONTEXT only —
# gating on it as well would turn a field that is merely absent, rather than
# null, into a false "not exercised" on a frame that genuinely did draw the
# terminal. An instrument built to report blindness must not invent it.
case "${BACKEND:-unknown}" in
  xterm_canvas_composite*) TERMINAL_EXERCISED=yes ;;
  *)                       TERMINAL_EXERCISED=no  ;;
esac
note "L2/3 terminal_exercised=$TERMINAL_EXERCISED backend=${BACKEND:-unknown} active_session_path=${SESSION_PATH:-absent}"
if [ "$TERMINAL_EXERCISED" = "no" ]; then
  note "L2/3 ⛔ THE CANVAS WAS NOT CAPTURED - the render faults are UNTESTED this tick, not absent. Open a session and re-run to exercise them."
fi
if [ "$FAITHFUL" != "true" ]; then
  fail 2 "screenshot is not faithful (capture_faithful=${FAITHFUL:-unknown}) - cannot verify the surface, so treat as broken"
else
  scp -q "$HOST:.yggterm/usability/shot.png" "$WORKDIR/shot.png" 2>/dev/null
  if [ -s "$WORKDIR/shot.png" ]; then
    note "L2/3 screenshot=$WORKDIR/shot.png (READ IT - levels 2 and 3 are an EYE check, not a field check)"
    cp "$WORKDIR/shot.png" "${TMPDIR:-/tmp}/ygg-usability-latest.png" 2>/dev/null
    note "L2/3 stable copy=${TMPDIR:-/tmp}/ygg-usability-latest.png"
  else
    fail 2 "could not retrieve the screenshot"
  fi
fi

[ "$FAIL_LEVEL" -ne 0 ] && emit

# ---------------------------------------------------------------- level 6
# COST AT REST. Reported in CORES, never a share, and measured as a RATE over a
# window - `ps %CPU` is a LIFETIME average and will quietly tell you a process
# that idled for 12 hours is busy now.
COST="$($SSH 'p=$(pgrep -x yggterm | head -1); [ -z "$p" ] && exit 0
  pids="$p $(ps --ppid $p -o pid= 2>/dev/null)"
  s1=0; for q in $pids; do v=$(awk "{print \$14+\$15}" /proc/$q/stat 2>/dev/null); s1=$((s1+${v:-0})); done
  t1=$(awk "/^cpu /{print \$2+\$3+\$4+\$5+\$6+\$7+\$8}" /proc/stat)
  sleep 10
  s2=0; for q in $pids; do v=$(awk "{print \$14+\$15}" /proc/$q/stat 2>/dev/null); s2=$((s2+${v:-0})); done
  t2=$(awk "/^cpu /{print \$2+\$3+\$4+\$5+\$6+\$7+\$8}" /proc/stat)
  n=$(nproc)
  echo "gui_subtree_cores=$(echo "scale=3; ($s2-$s1)/(($t2-$t1)/$n)" | bc)"
  echo "daemons=$(pgrep -fc "yggterm-headless server daemon")"
  echo "zombies=$(ps -eo stat --no-headers | grep -c "^Z")"
' 2>/dev/null)"

GUI_CORES="$(sed -n 's/^gui_subtree_cores=//p' <<<"$COST")"
DAEMONS="$(sed -n 's/^daemons=//p' <<<"$COST")"
ZOMBIES="$(sed -n 's/^zombies=//p' <<<"$COST")"
note "L6 gui_subtree_cores=${GUI_CORES:-?} daemons=${DAEMONS:-?} zombies=${ZOMBIES:-?}"

# A single spot sample is NOT the operating point - the GUI's idle CPU swings
# 11.5%-57.9% of a core on one build with nothing changed, and lengthening the
# window barely helps. So this threshold is deliberately loose: it exists to
# catch a runaway, not to measure the cost.
if [ -n "${GUI_CORES:-}" ] && [ "$(echo "${GUI_CORES} > 1.5" | bc 2>/dev/null)" = "1" ]; then
  fail 6 "GUI subtree is burning ${GUI_CORES} cores at rest"
fi
if [ -n "${ZOMBIES:-}" ] && [ "${ZOMBIES:-0}" -gt 20 ]; then
  fail 6 "${ZOMBIES} zombie processes - something is spawning children and never reaping them"
fi

# ---------------------------------------------------------------- level 4
# INPUT LANDS. ⛔ THIS IS THE LEVEL THAT MAKES THIS A USABILITY CHECK AT ALL.
#
# Levels 1-3 and 6 are structural: a process exists, it has not crashed, a frame
# is faithful, CPU is sane. **All of them passed while the owner was looking at
# an app he could not use.** A check built only from those reports PASS on a
# broken product, which is worse than no check, because it is quotable.
#
# ⛔ It never touches a live session. The probe is its own ephemeral shell row,
#    created with --no-activate so the owner's viewport does not move, carrying
#    an idle TTL so an abandoned probe reaps itself, and removed with
#    `server app session remove` (⛔ NOT `session remove`) with a read-back.
if [ "$FAIL_LEVEL" -eq 0 ]; then
  MARK="yggusab$$"
  PROBE="$($SSH "timeout 30 ~/.local/bin/yggterm server app terminal new --kind shell \
      --no-activate --purpose 'usability level-4 input probe' \
      --ephemeral --ephemeral-idle-ttl-secs 180 2>/dev/null" \
    | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin); d = d.get('data') or d
    print(d.get('session_path') or d.get('path') or '')
except Exception:
    print('')
")"
  if [ -z "$PROBE" ]; then
    fail 4 "could not create a probe session at all - the app cannot open a new terminal"
  else
    # ⛔ The Enter key is a SEPARATE write of a lone newline, after a beat. A
    #    command and its terminator sent as one write is not what a keyboard does.
    $SSH "timeout 25 ~/.local/bin/yggterm server app terminal send '$PROBE' --data 'echo $MARK'" >/dev/null 2>&1
    sleep 1
    $SSH "timeout 25 ~/.local/bin/yggterm server app terminal send '$PROBE' --data '
'" >/dev/null 2>&1
    sleep 2
    HITS="$($SSH "timeout 25 ~/.local/bin/yggterm server app terminal read-buffer '$PROBE' --mode screen 2>/dev/null" \
      | grep -c "$MARK" || true)"
    # ⛔ BLIND IS NOT BROKEN — and this level got it wrong once, loudly.
    #
    # A row created with --no-activate starts its PTY LAZILY, on first open.
    # That is sound design (685 rows must not each hold a shell), but it means
    # the probe has no shell to echo anything, and the write is QUEUED against
    # a session that has not started. Reading zero back then says nothing about
    # whether input lands — it says the probe never ran.
    #
    # ⚠ Reporting that as "INPUT DOES NOT LAND" produced a false level-4 alarm
    # that sent this session chasing a nonexistent regression, and rolling a
    # deploy back on suspicion of causing it. The rollback was right (restore
    # known-good first) and the diagnosis was wrong.
    #
    # ⇒ The honest states are three, not two: it echoed, it demonstrably did
    # not, or it could not be tested. Only the middle one is a failure, and it
    # requires the PTY to have actually started.
    STARTED="$($SSH "timeout 20 ~/.local/bin/yggterm server app terminal read-buffer '$PROBE' --mode screen 2>/dev/null" | wc -c)"
    note "L4 probe=$PROBE marker_hits=${HITS:-0} buffer_bytes=${STARTED:-0}"
    if [ "${HITS:-0}" -ge 1 ]; then
      : # input landed and echoed - the strong pass
    elif [ "${STARTED:-0}" -lt 200 ]; then
      # An empty buffer means no shell ever ran in it: lazy start, not a fault.
      note "L4 ⚠ probe PTY never started (lazy activation) - input path UNVERIFIED this tick, not failed"
    else
      fail 4 "the probe session IS running and a keystroke sent to it never came back - INPUT DOES NOT LAND"
    fi
    # Cleanup runs whether or not the assertion passed: a failed probe that
    # leaves a row behind turns one bad hour into a growing pile.
    $SSH "timeout 25 ~/.local/bin/yggterm server app session remove '$PROBE'" >/dev/null 2>&1
    LEFT="$($SSH "timeout 20 ~/.local/bin/yggterm server app rows 2>/dev/null" | grep -c "${PROBE##*/}" || true)"
    [ "${LEFT:-0}" -gt 0 ] && note "L4 ⚠ probe row SURVIVED removal ($PROBE) - remove it by hand"
  fi
fi

# ---------------------------------------------------------------- level 5
if [ "$DEEP" -eq 1 ] && [ "$FAIL_LEVEL" -eq 0 ]; then
  note "L5 click-open timing is still unimplemented - saying so beats implying it ran"
fi

emit
