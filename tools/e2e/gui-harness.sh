#!/usr/bin/env bash
# gui-harness.sh — THE DEV-LOCAL GUI HARNESS (charter §INFRA; built sitting 38)
#
# A PERSISTENT yggterm GUI on dev's Xvfb :78 — hermetic (private dbus session,
# scratch YGGTERM_HOME), running the DEPLOYED build by absolute path with an
# md5 exe-proof recorded at boot. This is the harness the [11.32] live leg,
# the [11.238] red-2-by-GUI bar, and the (c) live-proof lane all run on.
# First live proof delivered on it: [11.32]'s cold-resume paint-without-ladder
# (sitting 38, remote-agy row owned by oc).
#
# Usage:
#   gui-harness.sh start   boot (or report already-up); prints the EXE-PROOF line
#   gui-harness.sh status  aliveness + scratch home + exe proof + log tails
#   gui-harness.sh stop    reap GUI + daemon matched by YGGTERM_HOME environ
#                          ONLY (never a broad pkill — other seats' daemons)
#   gui-harness.sh env     print the probe env prefix (eval-able)
#
# Laws baked in (each paid for on the first boot, sitting 38):
#   • fence-rig run_boot envs verbatim: DISPLAY=:78 GDK_BACKEND=x11
#     YGGTERM_FORCE_X11_BACKEND=1 under dbus-run-session (the GUI refuses a
#     second yggterm on the owner's session bus — adopt-or-refuse).
#   • XDG_DATA_HOME under the scratch home ([11.231] class: never let a rig
#     resolve installs through the production share).
#   • The scratch home is seeded with config/machines.json (aliases only) AND
#     the machine records (ssh_targets + remote_machines) grafted from the
#     production server-state.json — a fresh home knows NO machines and every
#     remote row dies with "remote machine not found".
#   • The seeded PersistedDaemonState MUST carry stored_sessions +
#     live_sessions (the only fields without serde defaults — missing either
#     = daemon exit 1 at load, GUI supervisor crash-loop). Empty = no
#     restored rows, no surprise spawns.
#   • Reaps match /proc/<pid>/environ YGGTERM_HOME (the sitting-7 law).
#   • A re-deploy of ~/.local/bin/yggterm between boots is FINE (the
#     exe-proof line changes; read it, don't assume).
set -u

SCRATCH=${YGG_GUIHARNESS_HOME:-/tmp/ygg-guiharness-home}
BIN=${YGG_GUIHARNESS_BIN:-$HOME/.local/bin/yggterm}
GUI_LOG="$SCRATCH/gui.log"

alive_pids() {  # pids whose environ YGGTERM_HOME == SCRATCH (GUI or daemon)
  local p
  for p in $(pgrep -u "$USER" yggterm 2>/dev/null); do
    if tr '\0' '\n' < "/proc/$p/environ" 2>/dev/null | grep -qx "YGGTERM_HOME=$SCRATCH"; then
      echo "$p"
    fi
  done
}

seed_state() {
  # Graft the production machine records into a minimal-valid persisted
  # state. sed is WRONG here — SCRATCH contains slashes; and the state doc
  # is not line-shaped. Python by argument path, no heredoc-eaten quoting.
  python3 - "$HOME/.yggterm/server-state.json" "$SCRATCH/server-state.json" <<'PYEOF'
import json, sys
prod_path, scratch_path = sys.argv[1], sys.argv[2]
try:
    p = json.load(open(prod_path))
except Exception:
    sys.exit(0)
try:
    s = json.load(open(scratch_path))
except Exception:
    # Minimal-valid PersistedDaemonState: stored_sessions/live_sessions are
    # the only fields WITHOUT serde defaults (missing either = daemon exit 1
    # at load, measured sitting 38). Empty = no restored rows, no surprises.
    s = {"stored_sessions": [], "live_sessions": []}
for k in ("ssh_targets", "remote_machines"):
    if p.get(k) and not s.get(k):
        s[k] = p[k]
json.dump(s, open(scratch_path, "w"))
PYEOF
}

case "${1:-start}" in
env)
  echo "export YGGTERM_HOME=$SCRATCH"
  echo "export YGG_GUIHARNESS_BIN=$BIN"
  ;;
status)
  PIDS=$(alive_pids | tr '\n' ' ')
  if [ -n "$PIDS" ]; then echo "ALIVE pids: $PIDS"; else echo "DOWN"; fi
  echo "scratch: $SCRATCH"
  [ -f "$SCRATCH/exe-proof.txt" ] && echo "exe-proof: $(tr '\n' ' ' < "$SCRATCH/exe-proof.txt")"
  [ -f "$GUI_LOG" ] && echo "--- gui.log tail ---" && tail -3 "$GUI_LOG"
  ;;
stop)
  PIDS=$(alive_pids)
  if [ -z "$PIDS" ]; then echo "nothing to reap"; exit 0; fi
  for p in $PIDS; do kill "$p" 2>/dev/null; done
  sleep 2
  for p in $(alive_pids); do kill -9 "$p" 2>/dev/null; done
  echo "reaped: environ-matched harness processes"
  ;;
start)
  if [ -n "$(alive_pids)" ]; then echo "harness already up"; exit 0; fi
  [ -x "$BIN" ] || { echo "NO BINARY at $BIN"; exit 2; }
  mkdir -p "$SCRATCH/config"
  cp -f "$HOME/.yggterm/config/machines.json" "$SCRATCH/config/machines.json" 2>/dev/null
  seed_state
  md5sum "$BIN" | awk '{print $1, $2}' > "$SCRATCH/exe-proof.txt"
  "$BIN" --version 2>/dev/null | head -1 >> "$SCRATCH/exe-proof.txt"
  if ! [ -S "/tmp/.X11-unix/X78" ]; then
    pgrep -x Xvfb >/dev/null || { Xvfb :78 -screen 0 1600x1000x24 -ac > /tmp/guiharness-xvfb.log 2>&1 & sleep 1.5; }
    [ -S "/tmp/.X11-unix/X78" ] || { Xvfb :78 -screen 0 1600x1000x24 -ac > /tmp/guiharness-xvfb.log 2>&1 & sleep 1.5; }
  fi
  dbus-run-session -- bash -c "
    export DISPLAY=:78 GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
    export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
    exec $BIN > $GUI_LOG 2>&1
  " >/dev/null 2>&1 </dev/null &
  READY=0
  for i in $(seq 1 90); do
    sleep 1
    if YGGTERM_HOME=$SCRATCH "$BIN" server app clients >/dev/null 2>&1; then READY=1; break; fi
  done
  if [ "$READY" = 1 ]; then
    echo "HARNESS UP — exe-proof: $(tr '\n' ' ' < "$SCRATCH/exe-proof.txt")"
    echo "scratch: $SCRATCH"
  else
    echo "HARNESS FAILED to become ready in 90s — gui.log:"
    tail -20 "$GUI_LOG" 2>/dev/null
    exit 3
  fi
  ;;
*)
  echo "usage: gui-harness.sh {start|status|stop|env}"; exit 2;;
esac
