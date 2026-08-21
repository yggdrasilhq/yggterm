#!/usr/bin/env bash
# Unwedge a frozen yggterm GUI without killing anything.
#
# ⛔ NOT FOR AN UNBOOTABLE ROW. The name invites reaching for this when the
# booter refuses a row over composer residue ("holds boot-text residue PLUS
# other content") — this script severs the GUI's edit socket and touches no
# composer, so it does nothing for that. Residue recovery is the booter's own
# residue cleaner (ygg-booter.py, `_composer_is_boot_residue_only`); a row it
# still refuses holds something that may be the owner's and stays refused.
#
# The GUI freezes when its webview fails to acknowledge an edit batch: dioxus's
# poll_vdom then skips the ENTIRE VirtualDom -- renders, effects and every
# spawned task -- while the event loop stays healthy and idle. Severing the edit
# socket makes the blocked websocket.read() return, which fires the ack the
# connection thread was holding, which opens the gate. The webview reconnects on
# its own ~100ms later. No process, session or PTY is touched.
#
# Obsolete once a binary containing the edits.rs deadline + native.ts
# unconditional ack is deployed.
#
# ⛔⛔ HOW IT USED TO FIND THE GUI, AND WHY THAT WAS WORSE THAN NOT FINDING IT:
#
#     sup=$(pgrep -f "yggterm --supervise" | head -1)
#     gui=$(pgrep -P "$sup" | head -1)
#
# Two independent defects in two lines.
#
# **1. Nothing supervises the GUI any more.** `server app launch` detaches it, so
# it runs with PPID 1 and that search can never hit a real supervisor. Measured
# on the GUI host: the GUI is PPID 1, and no supervisor process exists at all.
#
# **2. `pgrep -f` matches the caller's own command line.** Every shell that runs
# that line carries the pattern in its argv, so the search finds the ASKING
# SHELL, `pgrep -P` walks to an unrelated child of it, and this script then
# announced a specific, plausible, entirely wrong diagnosis: *"GUI <pid> has no
# edit socket; not the flush-gate freeze"* — about a pid that was never the GUI,
# while the freeze it was denying was real.
#
# ⇒ The fix is not a better pattern. **Ask the registry that already knows.** The
# daemon records its clients, with the GUI's pid, its role and its start ticks;
# "which process is the GUI" is not a question anyone should be guessing at from
# a process table. Pattern matching is what you reach for when nothing keeps the
# record — see `ygg-procfind.sh`, which owns both halves of this now.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ⚠ TWO SPELLINGS, BECAUSE THE REPO AND THE INSTALL DISAGREE ON PURPOSE: the
# tracked copy keeps its `.sh` suffix, and `~/.local/bin` tools are installed
# bare like every other `ygg-*` verb. Looking for only one of them is a script
# that passes in the checkout and fails on the machine — which is exactly what
# happened here, and was caught only by running the DEPLOYED copy rather than
# the source.
PROCFIND=""
for candidate in "$HERE/ygg-procfind.sh" "$HERE/ygg-procfind" \
                 "$HOME/.local/bin/ygg-procfind"; do
    [ -x "$candidate" ] && { PROCFIND="$candidate"; break; }
done
[ -n "$PROCFIND" ] || {
    echo "ygg-procfind is not beside this script or on the usual path — it owns the"
    echo "process identification this tool refuses to do by pattern."
    exit 1
}

# ⭐ SO THE TOOL CAN BE EXERCISED WITHOUT BEING FIRED. Everything up to the
# sever runs; the sever does not. This is not a convenience — a repair tool
# whose only mode is "repair" can only ever be tested by breaking something, so
# in practice it is never tested at all, which is how it carried a broken
# process search for long enough to accuse innocent code.
DRY=""
ARGS=()
for a in "$@"; do
    case "$a" in
        --dry-run) DRY=1 ;;
        *) ARGS+=("$a") ;;
    esac
done
set -- "${ARGS[@]+"${ARGS[@]}"}"

# ⛔⛔ THIS TOOL RUNS ON THE GUI HOST, AND NOWHERE ELSE. Everything after the
# identification is local — `ss` reads THIS machine's sockets and `sudo ss -K`
# severs one of them — so a `--host` that only redirected the identification
# would diagnose one machine and act on another. That mixed reading is the exact
# shape of confident-wrong-answer this rewrite exists to delete, so it is
# refused rather than half-supported.
#
# ⚠ Caught by testing the rewrite itself: with `--host <gui-host>` it named the
# right GUI and then reported "no edit socket" — a true statement about the
# wrong machine's socket table.
for a in "$@"; do
    case "$a" in
        --host|--host=*)
            echo "⛔ ygg-unwedge acts on the LOCAL machine's sockets, so it must run ON"
            echo "  the GUI host. Identifying a remote GUI and then reading this"
            echo "  machine's socket table would diagnose one box and act on another."
            echo "  Run it there:  ssh <gui-host> ygg-unwedge"
            exit 2
            ;;
    esac
done

# ⛔ A REFUSAL HERE IS THE POINT, NOT A NUISANCE. If the registry cannot name an
#    active GUI, this tool has nothing to act on — and the whole reason it was
#    rewritten is that it used to invent an answer at exactly this moment.
gui=$("$PROCFIND" gui) || {
    echo "could not identify the GUI from the client registry (see the message above)."
    echo "⇒ Not proceeding: severing a socket on a pid nobody vouched for is how"
    echo "  this tool used to report a confident wrong diagnosis."
    exit 1
}

# ⛔⛔ THE SECOND CAUSE OF THE SAME WRONG SENTENCE, and fixing the pid alone would
# not have removed it. The old lookup read `split($3,a,":")` — but in this `ss`'s
# output `$3` is Send-Q, not an address:
#
#   ESTAB  0  0  127.0.0.1:37787  127.0.0.1:43696  users:(("yggterm",pid=…,fd=18))
#   $1     $2 $3 $4               $5               $6
#
# So it extracted nothing, and the script announced *"GUI <pid> has no edit
# socket; not the flush-gate freeze"* — which is the reassuring direction. A
# wedged GUI would be told its freeze was something else. Measured against the
# live GUI: it holds a LISTENING edit socket on 127.0.0.1:37787, and the old
# line reported none.
#
# ⇒ Do not index columns of a tool's human output at all. Match the port out of
# the LISTENING line by shape, which no column reshuffle can break.
port=$(ss -ltnp 2>/dev/null \
    | grep -F "pid=$gui," \
    | grep -oE '127\.0\.0\.1:[0-9]+' \
    | head -1 \
    | cut -d: -f2)
[ -n "$port" ] || {
    echo "GUI $gui holds no listening loopback socket; not the flush-gate freeze"
    exit 1
}

if [ -n "$DRY" ]; then
    echo "GUI pid $gui, edit socket port $port -- would sever (dry run, nothing done)"
    exit 0
fi

echo "GUI pid $gui, edit socket port $port -- severing"
sudo ss -K src 127.0.0.1 sport = "$port" >/dev/null
sleep 3

if timeout 25 ~/.local/bin/yggterm server app state >/dev/null 2>&1; then
  echo "GUI is alive again (nothing was killed)"
else
  echo "still wedged -- this is something else; capture a stack before restarting:"
  echo "  eu-stack -p $gui -n 40"
  exit 1
fi
