#!/usr/bin/env bash
# ygg-roll-watch — the CI half of the input-block/heat lane: land what the lane
# ships, hourly, without anyone having to remember.
#
# ⛔⛔ WHY THIS EXISTS. The fixes for the defect that makes this product unusable
# were sitting ON MAIN, built and merged, while the running fleet went on
# executing older code — twice in one morning, and once for nine hours. Nothing
# was blocked: a version bump is a release act every lane's build rides on, so
# lanes correctly refuse to allocate one, and it falls to an orchestrator seat
# that may be mid-task, out of context, or gone. The gap is not technical, it is
# that "somebody decides to run it" is not a mechanism.
#
#   scripts/ygg-roll-watch.sh [--interval 3600] [--once] [--dry-run]
#
# ⭐ WHAT IT DELIBERATELY WILL NOT DO: restart the GUI. Every other step here is
# invisible to the person using the machine — a build on a build host, binaries
# written to disk, and a daemon that adopts on its own terms when its own
# sessions allow it, which is the constitution working rather than a compromise.
# Restarting the GUI is the one step a human FEELS, it is the step that has cost
# this owner a blank viewport and a lost composer more than once, and a timer is
# the worst possible chooser of its moment. So the GUI half is reported and left
# to a human or an orchestrator. ⇒ A CI job that can only ever improve things
# silently is one nobody has to supervise.
set -uo pipefail

INTERVAL=3600
ONCE=0
DRY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --interval) INTERVAL="$2"; shift 2;;
    --once) ONCE=1; shift;;
    --dry-run) DRY=1; shift;;
    *) echo "unknown argument: $1" >&2; exit 2;;
  esac
done

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STATE="$HOME/.yggterm/relay"
LOG="$STATE/roll-watch.log"
DEPLOY_TREE="$HOME/gh/yggterm--deploy"
mkdir -p "$STATE"

say() { printf '%s ygg-roll-watch %s\n' "$(date +%H:%M:%S)" "$*" | tee -a "$LOG"; }

tick() {
  # ⛔ ASK THE BUILD HOST, NOT THIS ONE. The deploy tree lives where the fleet
  # builds; running this from the desktop must not turn into a desktop build.
  local build_host; build_host="dev"
  local live_host;  live_host="$("$REPO/scripts/ygg-live-host.sh" --quiet 2>/dev/null)"
  if [ -z "$live_host" ]; then
    say "⛔ cannot resolve the live GUI host — doing nothing. Blind is not clear."
    return 0
  fi

  # What is on main, and what is the fleet actually RUNNING? The second question
  # is the one that has been going unasked: a version string is not a build id,
  # so compare commits, never versions.
  local main_sha; main_sha="$(ssh -n "$build_host" "cd $DEPLOY_TREE && git fetch -q origin && git rev-parse --short=12 origin/main" 2>/dev/null)"
  if [ -z "$main_sha" ]; then say "⛔ could not read origin/main from $build_host — skipping"; return 0; fi

  local running; running="$(ssh -n "$live_host" '~/.local/bin/yggterm-headless server daemons 2>/dev/null' \
                            | awk '/^ *\*?[ ]*[0-9]+ /{print $4; exit}')"
  if [ -z "$running" ]; then say "⛔ no daemon answered on $live_host — skipping"; return 0; fi

  if [ "$running" = "$main_sha" ]; then
    say "current: fleet daemon and origin/main are both $main_sha — nothing to roll"
    reconcile_client "$live_host" "" "current build"
    return 0
  fi

  say "main is $main_sha, the live daemon runs $running — rolling"
  [ "$DRY" = 1 ] && { say "(dry run: stopping here)"; return 0; }

  # ⛔ PREFLIGHT BEFORE THE BUILD, ALWAYS. The ancestry gate is correct and it
  # used to be asked only after 2-3 minutes of compiling had already been spent;
  # on a busy fleet main advances mid-build and the deploy then refuses on a race
  # that did not exist when the build began.
  ssh -n "$build_host" "cd $DEPLOY_TREE && git reset --hard -q origin/main && scripts/deploy-fleet.sh --preflight" >>"$LOG" 2>&1 || {
    say "⛔ preflight refused — not building. See $LOG"; return 0; }

  local ver; ver="$(ssh -n "$build_host" "cd $DEPLOY_TREE && scripts/bump-version.sh 2>/dev/null")"
  [ -z "$ver" ] && { say "⛔ could not allocate a version — skipping"; return 0; }
  say "allocated $ver"

  ssh -n "$build_host" "cd $DEPLOY_TREE && cargo build --release" >>"$LOG" 2>&1 || {
    say "⛔ build failed — nothing deployed. See $LOG"; return 0; }

  ssh -n "$build_host" "cd $DEPLOY_TREE && scripts/deploy-fleet.sh" >>"$LOG" 2>&1
  say "deployed $ver to disk fleet-wide"

  # ⚠ THE DAEMON ADOPTS ON ITS OWN TERMS AND THAT IS NOT A FAILURE. It defers
  # while its own sessions are active, which on a machine that is always active
  # can be a long time. Report the gap; never force it, and never wait for it.
  local after; after="$(ssh -n "$live_host" '~/.local/bin/yggterm-headless server daemons 2>/dev/null' \
                        | awk '/^ *\*?[ ]*[0-9]+ /{print $4; exit}')"
  say "after deploy: live daemon runs $after (disk is at $ver)"

  # ⛔⛔ NOTIFY, THEN RESTART THE CLIENT. THIS FILE USED TO REFUSE, AND THAT WAS
  # MY CALL AND IT WAS WRONG. The original reasoning was that a GUI restart is the
  # one step a human FEELS, so a timer is the worst chooser of its moment. True as
  # far as it goes — and it optimised the wrong thing.
  #
  # ⇒ OWNER RULING 2026-08-21, and it is a product decision rather than an
  #   engineering preference: *"if any daemon or client is new all should get the
  #   newest version. In case of client auto restart after showing a notification.
  #   This is the safest and default behaviour because on a user machine daemon
  #   updated and client backdated will cause unnecessary issues that I do not want
  #   them to face."*
  #
  # The skew is not hypothetical and the product already SEES it: the metadata
  # panel renders "daemon is on <newer>" and "<version> · newer than this client"
  # from `DaemonVersionRank::Newer`. It detects the mismatch, tells the user about
  # it, and then does nothing — so the user carries a known-inconsistent pair for
  # as long as they happen not to restart. Measured today: a client sat two
  # versions behind its daemon for twenty minutes while the panel said so.
  #
  # ⚖ Weighed honestly: a restart costs seconds and is recoverable. A mismatched
  # client/daemon pair costs confusing failures nobody can attribute, on someone
  # else's machine, and the daemon owns the PTYs so the sessions survive the
  # restart by construction. The notification goes FIRST so the restart is never a
  # surprise, and the grace window is long enough to read it.
  reconcile_client "$live_host" "" "$ver"
}

#: Bring the CLIENT onto whatever is on disk: notify, pause, restart, verify.
#:
#: ⛔⛔ CALLED ON EVERY TICK, ROLLED OR NOT. This logic used to live only at the
#: tail of the roll path, so when the daemon was already current the function
#: returned before ever looking at the window — and "daemon current, client two
#: versions behind" was the ONE state this watcher could not see. That is exactly
#: the state the owner reported. ⇒ "If any daemon or client is new, all should get
#: the newest version" makes the client check UNCONDITIONAL, never a tail step of
#: a roll that may not happen.
reconcile_client() {  # host disk_md5(optional) label
  local live_host="$1" gui_disk="$2" ver="$3" gui_running
  [ -n "$gui_disk" ] || gui_disk="$(ssh -n "$live_host" 'md5sum ~/.local/bin/yggterm 2>/dev/null | cut -c1-10')"
  gui_running="$(ssh -n "$live_host" 'for g in $(pgrep -x yggterm); do x=$(readlink /proc/$g/exe 2>/dev/null); case "$x" in "$HOME/.local/bin/"*|"$HOME/.yggterm/bin/"*) md5sum /proc/$g/exe | cut -c1-10;; esac; done' 2>/dev/null | head -1)"
  if [ -z "$gui_running" ]; then
    say "no installed GUI process on $live_host — nothing to restart"
    return 0
  fi
  if [ "$gui_running" = "$gui_disk" ]; then
    say "GUI on $live_host already runs this build ($gui_disk)"
    return 0
  fi
  say "GUI on $live_host runs $gui_running, disk is $gui_disk — notifying, then restarting it"
  ssh -n "$live_host" "~/.local/bin/yggterm-headless server app notify 'yggterm $ver — restarting the window' \
    'The daemon is already on this build. Restarting the window now so client and daemon match; your sessions are owned by the daemon and survive it.' --tone info" >/dev/null 2>&1
  sleep "$GUI_RESTART_GRACE_SECS"
  restart_gui "$live_host" "$gui_disk"
}

#: Long enough that the notification is READ before the window goes, short enough
#: that the mismatched pair is not carried for meaningful time.
GUI_RESTART_GRACE_SECS=12

restart_gui() {  # host expected_md5
  local host="$1" want="$2"
  # ⛔ COPY THE WHOLE ENVIRONMENT, NEVER A HAND-PICKED LIST. The YGGTERM_ prefix
  # carries this host's presentation policy — canvas, under-glass, GL, webkit
  # memory bounds — and hand-listing silently drops whichever flag nobody thought
  # of, which changes how the terminal renders on the user's machine. There is a
  # standing prohibition on setting any presentation variable against a live GUI;
  # copying the set it already has is how this obeys it.
  # ⚠ The supervisor exits with the child, so the relaunch must be explicit.
  # ⛔⛔ EVERY INSTALLED GUI PROCESS, NEVER `head -1`. The first cut of this took
  # the first pid and it FAILED LIVE on its first run: the host had TWO yggterm
  # processes, one was killed, the survivor kept running the old image, and the
  # relaunch then hit the single-instance guard and exited — so the window
  # "restarted" onto exactly the build it was already on. ⇒ Identify the whole
  # set and take all of it. This is the same law this repo states about `pgrep -c`
  # and about counting rather than identifying, arriving by a third route.
  # ⚠ Sandbox GUIs running out of a worktree target/ are left alone: a lane
  # driving its own build is the point of a sandbox, not a host that is behind.
  ssh -n "$host" 'set -e
    PIDS=""
    for g in $(pgrep -x yggterm); do
      x=$(readlink /proc/$g/exe 2>/dev/null)
      case "$x" in "$HOME/.local/bin/"*|"$HOME/.yggterm/bin/"*) PIDS="$PIDS $g";; esac
    done
    [ -n "$PIDS" ] || exit 0
    FIRST=$(echo $PIDS | awk "{print \$1}")
    tr "\0" "\n" < /proc/$FIRST/environ | grep -E "^(DISPLAY|WAYLAND_DISPLAY|XDG_RUNTIME_DIR|DBUS_SESSION_BUS_ADDRESS|XAUTHORITY|XDG_SESSION_TYPE|XDG_CURRENT_DESKTOP|HOME|PATH|LANG|USER|YGGTERM_)" > /tmp/ygg-gui-env
    kill -TERM $PIDS 2>/dev/null || true
    for i in $(seq 1 30); do
      alive=""
      for g in $PIDS; do kill -0 $g 2>/dev/null && alive="yes"; done
      [ -z "$alive" ] && break
      sleep 1
    done
    for g in $PIDS; do kill -0 $g 2>/dev/null && kill -KILL $g 2>/dev/null || true; done
    sleep 2
    ( set -a; while IFS= read -r l; do export "$l"; done < /tmp/ygg-gui-env; set +a
      setsid nohup "$HOME/.local/bin/yggterm" >/tmp/ygg-gui-relaunch.log 2>&1 </dev/null & )
    rm -f /tmp/ygg-gui-env' >/dev/null 2>&1
  sleep 15
  # ⛔ READ THE EFFECT. The relaunch reports nothing useful; the running image does.
  local now
  now="$(ssh -n "$host" 'for g in $(pgrep -x yggterm); do x=$(readlink /proc/$g/exe); case "$x" in "$HOME/.local/bin/"*) md5sum /proc/$g/exe | cut -c1-10;; esac; done' 2>/dev/null | head -1)"
  if [ "$now" = "$want" ]; then
    say "✅ GUI on $host restarted onto $now — client and daemon now match"
  else
    say "⛔ GUI on $host reads $now, expected $want — RESTART DID NOT TAKE. Left for a human."
    ssh -n "$host" "~/.local/bin/yggterm-headless server app notify 'yggterm: automatic restart failed' \
      'The window could not be restarted onto the new build. Restart it by hand; client and daemon are mismatched until then.' --tone warning" >/dev/null 2>&1
  fi
}

say "roll-watch up (interval ${INTERVAL}s, dry=$DRY)"
while :; do
  tick
  [ "$ONCE" = 1 ] && break
  sleep "$INTERVAL"
done
