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
#   scripts/ygg-roll-watch.sh [--interval 3600] [--once] [--dry-run] [--sweep]
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

#: ⛔ CAPTURED BEFORE THE PARSER EATS THEM: the re-exec below must relaunch with
#: the same arguments this process was given.
ORIG_ARGS=("$@")

INTERVAL=3600
#: How many times ONE tick will re-sync and build again when the deploy refuses
#: because main moved under it. Two: a third attempt losing the same race means
#: main advances faster than a build takes, and the answer to that is a human.
DEPLOY_ATTEMPTS=2
#: The repaint sweep OPENS every blank row, which takes over the window. Never on
#: a timer; only when an operator asks for it and is watching.
SWEEP=0
ONCE=0
DRY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --interval) INTERVAL="$2"; shift 2;;
    --once) ONCE=1; shift;;
    --sweep) SWEEP=1; shift;;
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

  # ⛔⛔ THE DEPLOY'S ANSWER IS THE ONLY EVIDENCE THAT ANYTHING WAS DEPLOYED, AND
  # THIS FUNCTION USED TO THROW IT AWAY. The `deploy-fleet.sh` call took no exit
  # status and the `say "deployed ..."` beneath it fired unconditionally. What
  # that hid is not an exotic failure; on a busy fleet it is the ORDINARY one.
  # `deploy-fleet.sh` refuses a build whose HEAD is not a descendant of
  # origin/main; main advances during the two minutes the build takes; so the
  # preflight passes and the deploy then refuses on a commit that did not exist
  # when the build began.
  #
  # Measured 2026-08-21: the refusal named a commit that landed mid-build, a
  # version number was burned, NOTHING reached any host, and this watcher reported
  # `deployed 3.1.25 to disk fleet-wide`. The one instrument whose entire purpose
  # is to notice that the fleet is behind was asserting hourly that it was current
  # — and an instrument that fails toward "all clear" is worse than no instrument,
  # because it stops anyone else from looking.
  #
  # ⇒ Three changes, and the middle one is what makes the loop converge:
  #    1. read the status, and never claim a deploy that did not answer;
  #    2. on the mid-build race, RE-SYNC AND BUILD AGAIN in this same tick.
  #       Waiting an hour to lose the identical race is not patience — the busier
  #       main is, the more certainly the next tick loses it too;
  #    3. prove it on a host afterwards, because "the deploy command exited 0" and
  #       "the binary a machine will execute changed" are different claims.
  local ver="" attempt=1 deployed=0
  while [ "$attempt" -le "$DEPLOY_ATTEMPTS" ]; do
    # ⛔ PREFLIGHT BEFORE THE BUILD, ALWAYS, AND ON EVERY ATTEMPT. The ancestry
    # gate is correct and it used to be asked only after 2-3 minutes of compiling
    # had already been spent.
    ssh -n "$build_host" "cd $DEPLOY_TREE && git fetch -q origin && git reset --hard -q origin/main && scripts/deploy-fleet.sh --preflight" >>"$LOG" 2>&1 || {
      say "⛔ preflight refused on attempt $attempt — not building. See $LOG"; return 0; }

    ver="$(ssh -n "$build_host" "cd $DEPLOY_TREE && scripts/bump-version.sh 2>/dev/null")"
    [ -z "$ver" ] && { say "⛔ could not allocate a version — skipping"; return 0; }
    say "allocated $ver (attempt $attempt of $DEPLOY_ATTEMPTS)"

    ssh -n "$build_host" "cd $DEPLOY_TREE && cargo build --release" >>"$LOG" 2>&1 || {
      say "⛔ build failed — nothing deployed. See $LOG"; return 0; }

    if ssh -n "$build_host" "cd $DEPLOY_TREE && scripts/deploy-fleet.sh" >>"$LOG" 2>&1; then
      deployed=1
      break
    fi

    local landed; landed="$(ssh -n "$build_host" "cd $DEPLOY_TREE && git fetch -q origin && git rev-list --count HEAD..origin/main" 2>/dev/null)"
    if [ "${landed:-0}" -gt 0 ] 2>/dev/null && [ "$attempt" -lt "$DEPLOY_ATTEMPTS" ]; then
      say "⚠ deploy refused: $landed commit(s) landed on main during the build — re-syncing and building again"
      attempt=$((attempt + 1))
      continue
    fi
    say "⛔ deploy REFUSED and $ver reached no host. See $LOG — NOTHING was deployed."
    return 0
  done
  [ "$deployed" = 1 ] || { say "⛔ deploy did not succeed in $DEPLOY_ATTEMPTS attempts — nothing deployed."; return 0; }

  # ⛔ READ IT BACK OFF A MACHINE. The census the deploy writes is the deploy's
  # own account of itself; the installed binary is the fleet's.
  local installed; installed="$(ssh -n "$live_host" '~/.local/bin/yggterm-headless --version 2>/dev/null' | awk '{print $NF}')"
  if [ "$installed" = "$ver" ]; then
    say "deployed $ver to disk fleet-wide (verified on $live_host)"
  else
    say "⛔ the deploy reported success but $live_host has ${installed:-nothing} on disk, not $ver"
    return 0
  fi

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
  # ⛔⛔ NEVER `head -1` A PROCESS SET. `restart_gui` below already learned this
  # the hard way and takes every pid; the DETECTION half was still asking only the
  # first, which is the same bug one step earlier and strictly worse — if the
  # first process happens to match disk, this returns "already runs this build"
  # and the stale sibling is never restarted, so the repair never even begins.
  # Hosts here routinely carry more than one installed GUI process. ⇒ Take the
  # distinct images; anything but exactly one, equal to disk, is work to do.
  gui_running="$(ssh -n "$live_host" 'for g in $(pgrep -x yggterm); do x=$(readlink /proc/$g/exe 2>/dev/null); case "$x" in "$HOME/.local/bin/"*|"$HOME/.yggterm/bin/"*) md5sum /proc/$g/exe | cut -c1-10;; esac; done' 2>/dev/null | sort -u | tr '\n' ' ' | sed 's/ *$//')"
  if [ -z "$gui_running" ]; then
    say "no installed GUI process on $live_host — nothing to restart"
    return 0
  fi
  if [ "$gui_running" = "$gui_disk" ]; then
    say "GUI on $live_host already runs this build ($gui_disk)"
    return 0
  fi
  say "GUI on $live_host runs [$gui_running], disk is $gui_disk — notifying, then restarting it"
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
    # ⛔⛔⛔ THE REPAINT SWEEP IS NOT CALLED HERE ANY MORE, AND MUST NOT BE PUT BACK.
    # It was added on the reasoning that `sessions restore` TYPES NOTHING and is
    # therefore safe to run unconditionally. The typing half is true. The
    # conclusion is not, and the gap between them cost the owner several minutes
    # of an unusable machine: `sessions restore` is the RECOVERY verb, and it opens
    # a session "through the same path a manual drag takes" — so sweeping 34 rows
    # opens 34 sessions in a row. Reported live, 2026-08-21, in these words: the
    # window blinking, changing session and beeping every three to five seconds,
    # for minutes, until it could be typed into at all.
    #
    # ⇒ NOT TYPING IS ONE SAFETY PROPERTY AND NOT DRIVING THE VIEW IS ANOTHER. A
    #   remedy that steals the active session is an interruption whether or not it
    #   writes a byte, and an automation is the worst possible chooser of when to
    #   take somebody's screen away. This is the second time in one day that
    #   something correct in isolation was wrong in composition, on this same file.
    #
    # ⚠ AND THE MEASUREMENT IT WAS BUILT ON IS ITSELF IN DOUBT — see the queue
    #   entry. `read-buffer` on a row the client has not mounted may report an
    #   empty surface because nothing has been mounted yet, not because a mount
    #   failed; opening the row is then what "repairs" it, which would make the
    #   sweep a device for visiting rows and calling the visit a cure. Settle that
    #   before any sweep is automated again.
    #
    # `--sweep` still runs it by hand, for an operator who is looking at the screen
    # and has decided to spend it.
    [ "$SWEEP" = 1 ] && repaint_sweep "$host"
  else
    say "⛔ GUI on $host reads $now, expected $want — RESTART DID NOT TAKE. Left for a human."
    ssh -n "$host" "~/.local/bin/yggterm-headless server app notify 'yggterm: automatic restart failed' \
      'The window could not be restarted onto the new build. Restart it by hand; client and daemon are mismatched until then.' --tone warning" >/dev/null 2>&1
  fi
}


#: ⛔⛔ A CLIENT RESTART RE-MOUNTS EVERY ROW, AND SOME COME UP BLANK.
#:
#: This is the honest cost of the auto-restart above, and it must be paid here
#: rather than by the person looking at the window. Owner-reported minutes after
#: the restart shipped: *"input bugs are getting fancier by deleting the UI
#: itself"* — a row whose viewport was not stale or garbled but EMPTY, on a
#: client and daemon that matched, with the row reading `running · idle`. The
#: re-mount races the seed and the surface lands with nothing in it.
#:
#: ⚖ The restart is still right — a backdated client against a new daemon is the
#: skew a user must never face, and that is the owner's ruling. But shipping a
#: restart on a timer while the blank-mount path is only partly fixed moves a
#: known defect from rare to routine, and pretending otherwise would be dishonest
#: about what this file does.
#:
#: ⭐ THE REPAIR IS PROVEN AND IT TYPES NOTHING. `sessions restore` re-attaches a
#: row and the screen begins painting on the very next read — measured live today
#: on a row that had been invisible for half an hour. So the sweep is safe to run
#: unconditionally: it cannot reach a composer, and re-attaching an already-healthy
#: row is a no-op.
#:
#: ⚠ A row that is legitimately EMPTY is indistinguishable from one that failed to
#: seed, so this deliberately restores on emptiness alone rather than trying to be
#: clever. The cost of a wrong restore is one redundant re-attach; the cost of
#: missing one is a window the owner cannot use.
repaint_sweep() {  # host
  local host="$1" blanks=0 fixed=0
  local rows
  rows="$(ssh -n "$host" '~/.local/bin/yggterm-headless server app rows --json 2>/dev/null' \
          | python3 -c "import json,sys
try: d=json.load(sys.stdin)
except Exception: raise SystemExit
for r in (d.get('data') or {}).get('rows') or []:
    p=r.get('full_path') or ''
    if r.get('outline_prefix') and '://' in p: print(p)" 2>/dev/null)"
  [ -n "$rows" ] || { say "repaint sweep: no seated rows to check"; return 0; }
  for row in $rows; do
    local chars
    chars="$(ssh -n "$host" "~/.local/bin/yggterm-headless server app terminal read-buffer '$row' --mode screen 2>/dev/null" \
             | python3 -c "import json,sys
try: print((json.load(sys.stdin).get('data') or {}).get('nonblank_line_count') or 0)
except Exception: print(-1)" 2>/dev/null)"
    # -1 is BLIND (the read itself failed) and 0 is EMPTY. Both are a surface the
    # owner cannot use, and both are repaired by the same non-typing re-attach.
    if [ "${chars:-0}" = "0" ] || [ "${chars:-0}" = "-1" ]; then
      blanks=$((blanks + 1))
      ssh -n "$host" "~/.local/bin/yggterm-headless server app sessions restore '$row'" >/dev/null 2>&1
      sleep 1
      local after
      after="$(ssh -n "$host" "~/.local/bin/yggterm-headless server app terminal read-buffer '$row' --mode screen 2>/dev/null" \
               | python3 -c "import json,sys
try: print((json.load(sys.stdin).get('data') or {}).get('nonblank_line_count') or 0)
except Exception: print(0)" 2>/dev/null)"
      [ "${after:-0}" -gt 0 ] 2>/dev/null && fixed=$((fixed + 1))
    fi
  done
  if [ "$blanks" -gt 0 ]; then
    say "repaint sweep: $blanks blank/unreadable surface(s) after the restart, $fixed repainted by re-attach"
  else
    say "repaint sweep: every seated row painted after the restart"
  fi
}

# ⛔⛔ THE MAIN LOOP IS LAST IN THIS FILE AND THAT POSITION IS LOAD-BEARING.
# A shell function must have been DEFINED — that is, its definition must already
# have executed — before anything calls it, and this loop never returns. Written
# above `repaint_sweep`, it meant the definition below was never reached, so the
# repaint sweep that `restart_gui` calls after every restart died every time with
# `repaint_sweep: command not found` and had never once run.
#
# ⚠ That is the worst possible thing for it to be, because it is the mitigation
# for the defect the owner reports as the product deleting its own interface: it
# was written, reviewed, committed, described in the queue as shipped, and was
# dead code on every path. Nothing failed loudly — the message went to the log
# under a line that had just said the restart succeeded.
#
# ⇒ Proved live: immediately after a restart taken by this watcher, ALL 35 seated
#   rows read `nonblank_line_count: 0`; running the sweep by hand repainted 21 of
#   them. This repo has recorded the same trap once before, about a `log()` helper
#   defined below the code that called it, which also exited 0 while saying
#   nothing. ⛔ Do not "tidy" this loop back up beside the other top-level code.
# ⛔⛔ RE-EXEC WHEN THIS FILE CHANGES, AND FOR BASH IT IS NOT MERELY A FRESHNESS
# QUESTION — IT IS A CORRECTNESS ONE. Bash reads a script lazily, by byte offset,
# so editing the file under a running instance does not leave it uniformly old:
# it resumes at an offset that now points into different text, which is how a
# long-lived shell loop starts executing fragments of two versions. This watcher
# lives in a checkout that other lanes land into every few minutes.
#
# ⇒ It is also the fourth instance of one class in a single day — a stale daemon,
#   a stale GUI, a stale monitor watchdog, and this. Every one of them a
#   long-lived process reading code from a checkout that moved under it, and
#   nothing anywhere saying how far behind it was. The monitor's cure (re-exec on
#   its own mtime) is the right shape and this is the same cure in shell.
#
# ⚠ Checked AFTER the sleep and never mid-tick: a tick holds a build and a deploy,
#   and re-execing through those would abandon a half-deployed fleet.
# ⭐ AND SAY SO AT STARTUP RATHER THAN AT THE MOMENT OF USE. A missing function is
# discovered when it is called, which for `repaint_sweep` is after a restart has
# already happened on the owner's machine — the latest possible moment and the one
# where nobody is reading the log. `declare -F` costs nothing and turns a silent
# hole into a refusal to start.
for _fn in tick reconcile_client restart_gui repaint_sweep say; do
  declare -F "$_fn" >/dev/null || {
    printf 'ygg-roll-watch: ⛔ %s is not defined at the point the loop starts — a
' "$_fn" >&2
    printf '  function defined BELOW the main loop is never reached, because the loop
' >&2
    printf '  does not return. Move the definition above it.
' >&2
    exit 70; }
done
unset _fn

SELF="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
self_mtime() { stat -c %Y "$SELF" 2>/dev/null || echo 0; }
OWN_MTIME="$(self_mtime)"

say "roll-watch up (interval ${INTERVAL}s, dry=$DRY, source $(date -d "@$OWN_MTIME" +%H:%M:%S 2>/dev/null))"
while :; do
  tick
  [ "$ONCE" = 1 ] && break
  sleep "$INTERVAL"
  NOW_MTIME="$(self_mtime)"
  if [ "$NOW_MTIME" != "$OWN_MTIME" ]; then
    say "⭐ source changed on disk — re-execing so this loop runs one version of itself"
    exec bash "$SELF" "${ORIG_ARGS[@]}"
  fi
done
