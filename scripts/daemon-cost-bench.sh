#!/usr/bin/env bash
# What a daemon costs, per request and at rest, at a row count you choose.
#
# ⛔ THIS EXISTS BECAUSE THE CHORE WAS HAND-ASSEMBLED AND GOT IT WRONG THREE
# TIMES IN A ROW. Seeding a daemon with N rows and driving it needs a state file
# whose enum spellings and field TYPES are exact, a daemon proved alive before
# any request is sent (a CLI call against a dead socket cheerfully starts a
# replacement, and the arm then measures a daemon with no rows), and a final
# request past the flush boundary because the cost window flushes LAZILY. Each
# of those was a wasted run. An agent's discipline resets every session; a
# verb's does not.
#
#   scripts/daemon-cost-bench.sh --served 0 100 250 1000
#   scripts/daemon-cost-bench.sh --idle   0 100 1000
#   scripts/daemon-cost-bench.sh --served 0 250 --bin /path/to/yggterm-headless
#
# --served drives `status` as fast as it can for --dwell seconds and reports the
#          daemon's own `status_cost` record: CPU µs per reply against ROWS, and
#          the share of process CPU the path accounted for.
# --idle   sends NOTHING and reports the process CPU burned anyway. This is the
#          control the served run needs: if the cost is request serving, a
#          daemon nobody polls costs ~0, and if it is not, this is where the
#          rest shows up.
#
# ⚠ Each arm runs in its OWN yggterm home, so the daemon under test is polled by
# nobody and serves only what this script sends. That is the point — it is also
# why these numbers are a per-request cost and NOT a prediction of a live
# daemon's total, which additionally carries its sessions.
#
# ⚠ Read the numbers as an UNDER-read of the live path, in three known ways: the
# seeded rows carry short synthetic strings, the drive rate keeps caches warmer
# than a real poll rate does, and the seeded state has no PTY grids, ssh targets
# or remote machines — three tables the live reply path touches.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$REPO/target/release/yggterm-headless"
MODE=""
DWELL=70
ROWS=()

while [ $# -gt 0 ]; do
  case "$1" in
    --served|--idle) MODE="${1#--}"; shift ;;
    --bin) BIN="$2"; shift 2 ;;
    --dwell) DWELL="$2"; shift 2 ;;
    --help|-h) sed -n '2,40p' "${BASH_SOURCE[0]}" | sed 's/^# \?//'; exit 0 ;;
    -*) echo "unknown flag: $1" >&2; exit 2 ;;
    *) ROWS+=("$1"); shift ;;
  esac
done

[ -n "$MODE" ] || { echo "pick --served or --idle (--help for the difference)" >&2; exit 2; }
[ "${#ROWS[@]}" -gt 0 ] || { echo "give at least one row count" >&2; exit 2; }
[ -x "$BIN" ] || { echo "no daemon binary at $BIN (build it, or pass --bin)" >&2; exit 2; }

BASE="$(mktemp -d "${TMPDIR:-/tmp}/ygg-cost-bench.XXXXXX")"
trap 'rm -rf "$BASE"' EXIT
TICK="$(getconf CLK_TCK)"

seed_home() {  # $1=home $2=rows
  mkdir -p "$1"
  python3 - "$1" "$2" <<'PY'
import json, sys, uuid
home, k = sys.argv[1], int(sys.argv[2])
# Invented rows: nothing here names a real path, host or session.
live = [{"key": f"local://{uuid.uuid4()}", "id": str(uuid.uuid4()),
         "title": f"/srv/example/lane-{i:04d}", "kind": "shell",
         "keep_alive": False, "ssh_target": "localhost",
         "prefix": None, "cwd": f"/srv/example/lane-{i:04d}"} for i in range(k)]
stored = [{"path": f"/srv/example/store-{i:04d}", "kind": "codex",
           "session_id": f"seed-{i:04d}", "cwd": f"/srv/example/store-{i:04d}",
           "title_hint": f"/srv/example/store-{i:04d}"} for i in range(k // 4)]
# ⚠ The enum spellings and the LIST types below are load-bearing: "terminal"
# and an object-valued session_pty_grids both make the daemon refuse to start,
# and it refuses by exiting rather than by complaining loudly.
json.dump({"active_session_path": None, "active_view_mode": "Terminal",
           "ssh_targets": [], "remote_machines": [],
           "stored_sessions": stored, "live_sessions": live,
           "session_pty_grids": []}, open(f"{home}/server-state.json", "w"))
PY
}

LAUNCHER=""
start_daemon() {  # $1=home -> echoes the DAEMON's pid, or empty on failure
  local home="$1"
  YGGTERM_HOME="$home" "$BIN" server daemon >"$home/daemon.log" 2>&1 &
  LAUNCHER=$!
  for _ in $(seq 1 40); do
    if YGGTERM_HOME="$home" timeout 5 "$BIN" server ping >/dev/null 2>&1; then
      # ⛔ Ask the daemon who it is. The pid the shell just backgrounded is NOT
      # reliably the daemon's, and a harness that assumes it reads an EMPTY
      # second sample later and reports a parse error where a measurement
      # should have been.
      YGGTERM_HOME="$home" timeout 10 "$BIN" server daemons --json 2>/dev/null |
        python3 -c "import json,sys; d=json.load(sys.stdin)['daemons']; print(d[0]['pid'] if d else '')"
      return 0
    fi
    sleep 0.5
  done
  kill "$LAUNCHER" 2>/dev/null || true
  return 1
}

proc_cpu_us() { awk -v t="$TICK" '{print int((($14+$15)/t)*1000000)}' "/proc/$1/stat" 2>/dev/null || echo 0; }

for K in "${ROWS[@]}"; do
  HOME_DIR="$BASE/k$K"
  seed_home "$HOME_DIR" "$K"
  if ! DPID="$(start_daemon "$HOME_DIR")"; then
    echo "rows=$K FAILED to start: $(tail -2 "$HOME_DIR/daemon.log" | tr '\n' ' ')"
    continue
  fi

  if [ "$MODE" = served ]; then
    # ⛔ THE ZERO-REQUEST BASELINE, TAKEN IN THE SAME RUN. A daemon burns CPU
    # whether or not anyone sends it a request, and dividing a process-wide
    # delta by your own request count charges that background to the requests
    # you sent — a denominator you caused against a numerator you did not. That
    # error was worth 29x on a neighbouring arm. Measure it; never assume zero.
    BG_A="$(proc_cpu_us "$DPID")"; BG_T0="$(date +%s)"
    sleep 20
    BG_B="$(proc_cpu_us "$DPID")"; BG_T1="$(date +%s)"
    BG_CORES="$(python3 -c "print(f'{(${BG_B:-0}-${BG_A:-0})/1e6/max(${BG_T1}-${BG_T0},1):.5f}')")"

    END=$(( $(date +%s) + DWELL ))
    FAILED=0
    while [ "$(date +%s)" -lt "$END" ]; do
      # ⚠ Count failures, never break on one. A single transient refusal used to
      # end the arm silently, and the only symptom was "no window flushed" —
      # a truncated run wearing the costume of an instrument that did not fire.
      YGGTERM_HOME="$HOME_DIR" timeout 10 "$BIN" server status >/dev/null 2>&1 || FAILED=$((FAILED + 1))
      [ "$FAILED" -gt 20 ] && { echo "rows=$K ABORTED after $FAILED failed requests"; break; }
    done
    [ "$FAILED" -gt 0 ] && echo "      ⚠ $FAILED requests failed during this arm"
    echo "      background (zero-request) = $BG_CORES cores"
    # The cost window flushes lazily, by the NEXT reply — without one more
    # request past the boundary the window this run paid for is never emitted.
    sleep 1
    YGGTERM_HOME="$HOME_DIR" timeout 10 "$BIN" server status >/dev/null 2>&1 || true
    python3 - "$HOME_DIR/event-trace.jsonl" "$K" <<'PY'
import json, sys
path, k = sys.argv[1], sys.argv[2]
status, handler = [], []
for line in open(path, errors="replace"):
    if '"status_cost"' in line:
        status.append(json.loads(line)["payload"])
    elif '"client_handler_cost"' in line:
        handler.append(json.loads(line)["payload"])
if not status:
    print(f"rows={k} no window flushed — raise --dwell above the flush interval")
else:
    p = status[-1]
    share = 100 * p["cpu_us_total"] / max(p["proc_cpu_us_delta"], 1)
    print(f"rows={p['rows_mean']:<6} cpu_us/reply={p['cpu_us_mean']:<7}"
          f" wall_us/reply={p['wall_us_mean']:<7}"
          f" proc_cpu_us/reply={p['proc_cpu_us_delta'] // max(p['replies'], 1):<7}"
          f" replies/s={p['replies_per_s']:<7.0f} share={share:.1f}%")
# ⚠ The LAST window, not the first: the first covers process start, so it
# reports a control that was never warm.
for i, h in enumerate(handler):
    ks = h["kernel_share"]
    warm = "warm " if i else "COLD "
    print(f"      {warm}handler: cpu_us/conn={h['cpu_us_mean']:<7}"
          f" wall_us/conn={h['wall_us_mean']:<7} max={h['cpu_us_max']:<8}"
          f" kernel_share={'n/a' if ks is None else f'{100*ks:.1f}%'}"
          f" ticks={h['cpu_ticks_sampled']:<5} conns/s={h['handlers_per_s']:.0f}")
PY
  else
    # Startup work — state restore, scans — is not the idle cost. Settle first.
    sleep 20
    A="$(proc_cpu_us "$DPID")"; T0="$(date +%s)"; LAST="$A"; LAST_T="$T0"
    END=$(( T0 + DWELL )); DIED_AT=""
    # ⛔ Sampled at intervals, not just at the ends: an unpolled daemon may
    # RETIRE ITSELF mid-window, and that is a finding, not an error. A harness
    # that only reads the ends turns it into an empty variable.
    while [ "$(date +%s)" -lt "$END" ]; do
      sleep 15
      NOW="$(proc_cpu_us "$DPID")"
      if [ -z "$NOW" ] || [ "$NOW" = 0 ] && ! kill -0 "$DPID" 2>/dev/null; then
        DIED_AT=$(( $(date +%s) - T0 )); break
      fi
      LAST="$NOW"; LAST_T="$(date +%s)"
    done
    RSS="$(awk '/VmRSS/{print $2}' "/proc/$DPID/status" 2>/dev/null || echo 0)"
    THREADS="$(awk '/Threads/{print $2}' "/proc/$DPID/status" 2>/dev/null || echo 0)"
    python3 -c "
a,b,e = $A,$LAST,$(( LAST_T - T0 ))
died = '$DIED_AT'
note = f'  RETIRED ITSELF after {died}s with nobody polling it' if died else ''
print(f'rows=$K'.ljust(12) + f'idle_cores={(b-a)/1e6/max(e,1):<10.5f}'
      f' cpu_us={b-a:<10} over={e}s rss_kb=${RSS:-0} threads=${THREADS:-0}{note}')
"
    [ -n "$DIED_AT" ] && tail -3 "$HOME_DIR/daemon.log" | sed 's/^/        log: /'
  fi

  YGGTERM_HOME="$HOME_DIR" "$BIN" server shutdown >/dev/null 2>&1 || true
  sleep 1
  kill "$LAUNCHER" 2>/dev/null || true
done
