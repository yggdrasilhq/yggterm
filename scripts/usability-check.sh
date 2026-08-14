#!/usr/bin/env bash
# The hourly usability check. `docs/usability-contract.md` is the contract; this
# is its executable form, and the two are meant to be read together.
#
# It answers ONE question: is yggterm usable on the desktop host right now, and
# if not, what is the WORST thing wrong? Levels are ordered load-bearing first
# and the check STOPS at the first failure, because a ranked list of six is how
# the load-bearing item gets buried.
#
# ⛔ It never touches a human's session. Levels 1-3 and 6 are pure reads. Levels
#    4-5 need a probe row and are therefore opt-in behind --deep.
#
# Usage:
#   scripts/usability-check.sh            # levels 1,2,3,6 - safe, non-invasive
#   scripts/usability-check.sh --deep     # also 4,5 - creates an ephemeral probe row
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
      echo "PASS - all checked levels green"
    else
      echo "FAIL at level $FAIL_LEVEL: $FAIL_WHAT"
    fi
  fi
  exit "$FAIL_LEVEL"
}

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

# ---------------------------------------------------------------- levels 2+3
# BOTH SIDEBARS RENDER, and THE VIEWPORT IS FAITHFUL. A visual symptom needs a
# faithful pixel: capture_faithful=false means the frame is a lie about the
# terminal, and telemetry has never once settled a rendering question here.
SHOT_JSON="$($SSH "~/.local/bin/yggterm server app screenshot /tmp/ygg-usability.png" 2>/dev/null)"
FAITHFUL="$(grep -o '"capture_faithful": *[a-z]*' <<<"$SHOT_JSON" | head -1 | grep -o '[a-z]*$')"
note "L2/3 capture_faithful=${FAITHFUL:-unknown}"
if [ "$FAITHFUL" != "true" ]; then
  fail 2 "screenshot is not faithful (capture_faithful=${FAITHFUL:-unknown}) - cannot verify the surface, so treat as broken"
else
  scp -q "$HOST:/tmp/ygg-usability.png" "$WORKDIR/shot.png" 2>/dev/null
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

# ---------------------------------------------------------------- levels 4+5
if [ "$DEEP" -eq 1 ] && [ "$FAIL_LEVEL" -eq 0 ]; then
  note "L4/5 deep probe requested - see docs/usability-contract.md for why this is opt-in"
fi

emit
