#!/usr/bin/env bash
# ygg-procfind — answer "which process is that?" without accusing the asker.
#
# ⛔⛔ THE DEFECT THIS EXISTS FOR, and it is endemic rather than one tool's slip:
#
#     sup=$(pgrep -f "yggterm --supervise" | head -1)
#     gui=$(pgrep -P "$sup" | head -1)
#
# **`pgrep -f` matches the caller's own command line.** Any shell whose argv
# contains the pattern — which is every shell that ran this line — is itself a
# hit. So the search finds the ASKING SHELL, `pgrep -P` walks to some unrelated
# child of it, and the tool then reports a specific, plausible, entirely wrong
# diagnosis about a pid that was never the thing it was looking for.
#
# ⚠ **AND IT FAILS TOWARD A FALSE ACCUSATION, NOT TOWARD SILENCE.** That is what
# makes it expensive. A search that finds nothing makes you look again; a search
# that confidently names the wrong process sends you to debug innocent code. It
# happened on this campaign: a GUI-repair tool announced *"GUI <pid> has no edit
# socket; not the flush-gate freeze"* about a pid that was not the GUI, and the
# freeze it was denying was real.
#
# ⭐ THE GUARD IS NOT A NAME BLACKLIST. Tools that noticed this wrote
# `case "$c" in *pgrep*|*bash*) skip ;;` — which is a list of the shells someone
# thought of, and quietly fails for `sh`, `zsh`, `python3 -c`, `xargs`, a make
# recipe, or an ssh command string.
#
# The rule that IS exact is below `match`, and it is not lineage: a self-match is
# any process whose command line IS one of ours, because it was forked from
# something invoked with the pattern in its argv. Two more obvious rules were
# tried first and both leaked — the reasoning is recorded there, since each
# wrong answer looked right until a control disproved it.
#
# ⛔ AND FOR ANYTHING WITH A REGISTRY, DO NOT PATTERN-MATCH AT ALL. `gui` below
# asks the daemon's own client registry, because "which process is the GUI" is a
# question something already knows the answer to. A pattern is what you reach
# for when nothing is keeping the record — not a substitute for reading it.
#
# Usage:
#   ygg-procfind.sh gui [--host <h>]   # the GUI's pid, from the client registry
#   ygg-procfind.sh match <pattern>    # pids matching, minus every self-match
#   ygg-procfind.sh ancestors          # the chain that `match` excludes (debug)
#
# Exit: 0 found · 1 nothing found · 2 asked wrongly.
set -euo pipefail

YGGTERM_BIN_DEFAULT="$HOME/.local/bin/yggterm"

# Every pid from here up to init. These are exactly the processes that can carry
# a search pattern *because* the search is running, and nothing else.
ancestors() {
    local pid=$$
    while [ -n "$pid" ] && [ "$pid" -gt 1 ]; do
        echo "$pid"
        pid=$(awk '/^PPid:/{print $2}' "/proc/$pid/status" 2>/dev/null || true)
        [ -n "$pid" ] || break
    done
}

# ⚠ The redirect is INSIDE the subshell, not on `tr`: a pid can exit between
# pgrep listing it and us reading it, and the shell reports a failed redirect
# before the command it belongs to ever runs — so `tr … 2>/dev/null` does not
# silence it. A racing pid is normal, not an error worth printing.
cmdline_of() { ( tr '\0' ' ' < "/proc/$1/cmdline" ) 2>/dev/null || true; }

# ⛔⛔ THE EXACT RULE IS CMDLINE IDENTITY, NOT A POSITION IN THE PROCESS TREE —
# and it took two failed controls to get here, which is worth recording because
# both wrong answers looked reasonable:
#
#   1. "exclude my ancestors" — misses the SUBSHELL. A fork inherits its
#      parent's `/proc/<pid>/cmdline` verbatim, so every command substitution
#      inside a script whose argv holds the pattern is another copy of it,
#      BELOW us.
#   2. "…and my descendants" — misses the SIBLING. A pipeline member forked from
#      an ancestor (`./tool match "sleep 400" | sed …`) carries that ancestor's
#      whole command line while being neither above us nor below us.
#
# What every false positive actually has in common is not lineage: it is that
# **its command line IS one of ours**, because it was forked from a process that
# was invoked with the pattern in its argv. So compare the bytes. That catches
# ancestors, descendants and siblings in one test, needs no tree walk, and
# cannot be defeated by a shell nobody thought of.
#
# ⚠ THE HONEST LIMIT: a target whose command line is byte-identical to the
# caller's would be excluded too. For the things this helper is asked about — a
# GUI, a daemon, a supervisor — that cannot happen, because those are not
# invoked with the search pattern as an argument. A caller searching for
# something that IS spelled like its own invocation must not use this.
#
# ⛔⛔ AND A THIRD FALSE POSITIVE THE CMDLINE-IDENTITY RULE CANNOT SEE: THE CALL
# THAT ARRIVED OVER THE NETWORK. Run this over `ssh <host>` while standing ON
# that same host — which is the normal shape of a fleet query, and the shape
# every agent session here uses — and the `ssh` client process is on the target
# host too, carrying the pattern in its argv. It is not an ancestor of the
# remote-side script (sshd starts a fresh tree), it is not a descendant, and its
# bytes are nobody else's, so all three existing tests pass it through.
#
# ⇒ Measured 2026-08-21, and it gave a WRONG ANSWER IN BOTH DIRECTIONS in one
#   session: rows with no agent at all reported `pids=2` and rows with a live
#   agent reported `pids=4`, the constant 2 being the caller's own shell and its
#   ssh. A count inflated by a fixed amount is the worst kind of wrong, because
#   the differences still look right and the absolute numbers quietly are not —
#   "this row has processes" was read off a search that had found only itself.
#
# ⭐ The guard is NOT another shell blacklist: it is THIS SCRIPT'S OWN NAME. A
#   command line containing it can only be a copy of the search that is running,
#   because nothing this tool is ever asked to find is invoked by its name. That
#   is the same principle as the byte-identity rule — "it can only be there
#   because I am running" — applied where lineage cannot reach.
match() {
    local pattern="$1"
    local mine="" pid
    for pid in $(ancestors); do
        mine+="$(cmdline_of "$pid")"$'\n'
    done
    local self; self="$(basename "$0")"
    local found=1
    for pid in $(pgrep -f -- "$pattern" 2>/dev/null || true); do
        local cmd
        cmd="$(cmdline_of "$pid")"
        # An empty cmdline is a kernel thread; it cannot be what was searched for.
        [ -n "$cmd" ] || continue
        if grep -qxF -- "$cmd" <<<"$mine"; then
            continue
        fi
        # The network-delivered copy of this very query: an ssh client, a wrapper
        # shell, an xargs — whatever carried it, it names this script.
        case "$cmd" in *"$self"*) continue;; esac
        echo "$pid"
        found=0
    done
    return $found
}

# The GUI, from the registry that already knows.
#
# ⛔ VERIFIED AGAINST /proc IN THE SAME BREATH, because a registry can be stale
# and a pid can be REUSED. `process_start_ticks` is the kernel's own start time
# for that pid (field 22 of /proc/<pid>/stat); if a different process now holds
# the number, the ticks disagree and we say so instead of handing back a pid
# that names somebody else's program.
gui() {
    local host="${1:-}"
    local bin="${YGGTERM_BIN:-$YGGTERM_BIN_DEFAULT}"
    local reply
    if [ -n "$host" ]; then
        reply=$(ssh -o BatchMode=yes "$host" "$bin server app clients" 2>/dev/null || true)
    else
        reply=$("$bin" server app clients 2>/dev/null || true)
    fi
    [ -n "$reply" ] || {
        echo "the client registry did not answer — run this on the GUI host, or pass --host" >&2
        return 2
    }
    # The registry answers; a local /proc cross-check is only possible locally.
    local parsed
    parsed=$(printf '%s' "$reply" | python3 -c '
import json, sys
try:
    d = json.load(sys.stdin)
except Exception:
    sys.exit(1)
clients = d.get("clients") or []
# ⛔ "active" is the role that OWNS the surfaces. A shadow or an agent client is
#    also a client, and unwedging one of those would sever the wrong socket.
active = [c for c in clients if c.get("client_role") == "active" and c.get("pid")]
if not active:
    sys.exit(1)
if len(active) > 1:
    # Never silently pick one. Two active clients is a real state and the caller
    # must decide, not us.
    print("AMBIGUOUS " + " ".join(str(c["pid"]) for c in active))
    sys.exit(0)
c = active[0]
print("%s %s %s" % (c["pid"], c.get("process_start_ticks") or 0, c.get("executable_path") or ""))
' 2>/dev/null || true)
    [ -n "$parsed" ] || { echo "no active GUI client is registered" >&2; return 1; }
    case "$parsed" in
        AMBIGUOUS*)
            echo "more than one ACTIVE client is registered (${parsed#AMBIGUOUS }) — refusing to guess" >&2
            return 2
            ;;
    esac
    local pid ticks exe
    read -r pid ticks exe <<<"$parsed"
    if [ -z "$host" ]; then
        [ -d "/proc/$pid" ] || {
            echo "the registry names pid $pid but no such process exists — the registry is stale" >&2
            return 1
        }
        local live_ticks
        live_ticks=$(awk '{ n = split($0, f, ") "); split(f[n], g, " "); print g[20] }' \
            "/proc/$pid/stat" 2>/dev/null || true)
        if [ -n "$ticks" ] && [ "$ticks" != "0" ] && [ -n "$live_ticks" ] \
           && [ "$ticks" != "$live_ticks" ]; then
            echo "pid $pid started at $live_ticks but the registry recorded $ticks — \
the pid has been REUSED by another process; refusing to name it the GUI" >&2
            return 1
        fi
    fi
    echo "$pid"
}

case "${1:-}" in
    gui)
        shift
        host=""
        [ "${1:-}" = "--host" ] && { host="${2:-}"; }
        gui "$host"
        ;;
    match)
        shift
        [ -n "${1:-}" ] || { echo "match needs a pattern" >&2; exit 2; }
        match "$1"
        ;;
    ancestors) ancestors ;;
    *)
        sed -n '/^# Usage:/,/^# Exit:/p' "$0" | sed 's/^# \{0,1\}//'
        exit 2
        ;;
esac
