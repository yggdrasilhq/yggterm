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

  # ⛔ THE GUI IS THE STEP A HUMAN FEELS. Say what is owed and stop.
  local gui_running gui_disk
  gui_running="$(ssh -n "$live_host" 'for g in $(pgrep -x yggterm); do x=$(readlink /proc/$g/exe); case "$x" in "$HOME/.local/bin/"*) md5sum /proc/$g/exe | cut -c1-10;; esac; done' 2>/dev/null | head -1)"
  gui_disk="$(ssh -n "$live_host" 'md5sum ~/.local/bin/yggterm | cut -c1-10' 2>/dev/null)"
  if [ -n "$gui_running" ] && [ "$gui_running" != "$gui_disk" ]; then
    say "⚠ GUI OWED A RESTART on $live_host: running $gui_running, disk $gui_disk."
    say "   Not restarting it from a timer — that is the step the user feels."
    ssh -n "$live_host" "~/.local/bin/yggterm-headless server app notify 'yggterm $ver is on disk' \
      'The GUI is still running the previous build. Restart it when convenient to pick up the input-block fixes.' --tone info" >/dev/null 2>&1
  fi
}

say "roll-watch up (interval ${INTERVAL}s, dry=$DRY)"
while :; do
  tick
  [ "$ONCE" = 1 ] && break
  sleep "$INTERVAL"
done
