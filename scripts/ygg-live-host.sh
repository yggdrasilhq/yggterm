#!/bin/bash
# ygg-live-host — the ONE answer to "which host is the live GUI on?"
#
# ⛔ THIS EXISTS BECAUSE THE FILE THAT NAMED THE LIVE HOST EXISTED ONLY ON THE
# LIVE HOST. Fifteen recipes in the yggui skill, one in the deeptest skill and
# one python script all opened with
#
#     LIVE_HOST=$(cat .agents/config/live-host)
#
# and `.gitignore` line 21 excludes `.agents/config/` — correctly, the alias is
# infrastructure and this repo is public. The consequence is backwards exactly
# where it matters: the standing directive is that sessions run on the headless
# hosts, never on the desktop host, so the machines that need to be TOLD which
# host is live are precisely the ones whose checkout does not carry the file,
# and the one machine that can read it is the one that does not need to. Every
# such recipe died on its first line with `No such file or directory`. Measured
# 2026-08-13: absent on BOTH headless checkouts, and `$YGG_GUI_HOST` unset on
# all three — so the recipe failed everywhere except the desktop.
#
# ⚠ AND DISCOVERY ALONE CANNOT REPLACE IT, which is why this is not a one-liner.
# `server app rows` on a headless host answers *"this daemon knows no remote
# machines, so it cannot name a candidate"*, and `server snapshot` agrees:
# `remote_machines: []`. The app plane has no candidate list to offer off the
# GUI host. The candidates have to come from the machine's own private ssh
# configuration — which every fleet host has, no checkout carries, and this
# public repo therefore never names.
#
# ⇒ Resolution order, each step answering what the next cannot:
#   1. $YGG_GUI_HOST     — an operator said so; nothing outranks that, no probe.
#   2. this machine      — one local call, and on the desktop host it is the
#                          whole answer.
#   3. the cached alias  — .agents/config/live-host, VERIFIED by one probe. A
#                          remembered name is a hint, not a fact; it is checked
#                          before it is trusted.
#   4. discovery         — every candidate probed in PARALLEL for a live GUI
#                          client, and the winner is written back to the cache
#                          so the next fifteen recipes cost one ssh, not twelve.
#   5. the cached alias, UNVERIFIED — the GUI is DOWN, and this is the only
#                          source that can still name the host you are ssh-ing
#                          to in order to START it. Discovery cannot see a GUI
#                          that is not running, and that case is common.
#
# Prints one line — an ssh-able alias — to stdout. Diagnostics go to stderr, so
# `LIVE_HOST=$(scripts/ygg-live-host.sh)` is always safe to substitute.
#
#   scripts/ygg-live-host.sh [--quiet] [--no-cache]
#
# Exit 2 when no host resolves, with stderr naming which sources were tried — a
# resolver that failed silently would put the empty-string bug back one layer
# down, and an empty $LIVE_HOST makes `ssh "$LIVE_HOST" cmd` run cmd LOCALLY.
set -uo pipefail

QUIET=0
CACHE=1
while [ $# -gt 0 ]; do
  case "$1" in
    --quiet) QUIET=1; shift;;
    --no-cache) CACHE=0; shift;;
    *) echo "unknown argument: $1" >&2; exit 2;;
  esac
done
note() { [ "$QUIET" = 1 ] || echo "ygg-live-host: $*" >&2; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CACHE_FILE="$REPO/.agents/config/live-host"

BIN=""
for cand in "$HOME/.yggterm/bin/yggterm" "$HOME/.local/bin/yggterm" "$(command -v yggterm 2>/dev/null)"; do
  [ -n "$cand" ] && [ -x "$cand" ] && { BIN="$cand"; break; }
done

# ⚠ `server app clients` answers with a TOP-LEVEL {clients,count}, unlike every
# other app verb, which wraps its payload in {data:{…}} — and it exits 0 with
# ZERO clients, so the exit code is not the signal. Accept both shapes and
# require a non-empty list. (Same reading as ygg-claim.sh's `has_gui`.)
has_gui() {  # host ("" = this machine)
  local host="$1" out
  if [ -n "$host" ]; then
    out="$(ssh -o BatchMode=yes -o ConnectTimeout=6 -o StrictHostKeyChecking=accept-new \
             "$host" "\$HOME/.yggterm/bin/yggterm server app clients" 2>/dev/null)" || return 1
  else
    [ -n "$BIN" ] || return 1
    out="$("$BIN" server app clients 2>/dev/null)" || return 1
  fi
  printf '%s' "$out" | python3 -c 'import json,sys
try: d=json.load(sys.stdin)
except Exception: sys.exit(1)
c=d.get("clients")
if c is None: c=(d.get("data") or {}).get("clients")
sys.exit(0 if (c and len(c)>0) else 1)' 2>/dev/null
}

remember() {  # host — cache the answer where the override already lives
  [ "$CACHE" = 1 ] || return 0
  mkdir -p "$(dirname "$CACHE_FILE")" 2>/dev/null || return 0
  printf '%s\n' "$1" > "$CACHE_FILE" 2>/dev/null || return 0
}

# 1 — the operator said so.
if [ -n "${YGG_GUI_HOST:-}" ]; then
  echo "$YGG_GUI_HOST"
  exit 0
fi

# 2 — the GUI is on this machine. Recipes ssh to whatever we print, so an alias
# beats a kernel hostname: the two differ on this fleet, and that difference has
# already made a deploy report ⛔ for the very host doing the deploying.
#
# ⛔⛔ BUT THE CACHE MAY NAME A DIFFERENT MACHINE, AND THIS STEP USED TO PRINT IT
# ANYWAY — a self-confirming wrong answer on the one host that cannot be wrong.
# `has_gui ""` has just PROVED the GUI is local; the cache holds whatever host
# was last DISCOVERED, which after any period of running headless is a peer. So
# on the desktop host the resolver answered with the name of a machine that has
# no GUI at all, and every recipe that asks it — deploy, screenshot, verify —
# was aimed one host sideways.
#
# ⇒ Measured 2026-08-21 on the GUI host: the resolver answered `dev` while the
#   GUI ran locally, and the fleet booter had been started `--host dev` from that
#   answer, so every boot it issued drove app-control on a machine with no GUI
#   and reached nobody. Rows it believed it was waking sat untouched.
#
# The alias is still preferred over the kernel hostname — that part was right.
# It simply has to be an alias for THIS machine, and that is one cheap check.
if has_gui ""; then
  ME="$(hostname -s)"
  CACHED_LOCAL=""
  [ -r "$CACHE_FILE" ] && CACHED_LOCAL="$(head -1 "$CACHE_FILE" | tr -d '[:space:]')"
  if [ -z "$CACHED_LOCAL" ] || [ "$CACHED_LOCAL" = "$ME" ]; then
    echo "$ME"
  elif [ "$(ssh -o BatchMode=yes -o ConnectTimeout=6 \
              -o StrictHostKeyChecking=accept-new \
              "$CACHED_LOCAL" hostname -s 2>/dev/null)" = "$ME" ]; then
    # The cached name is an ALIAS for this machine — the case the comment above
    # is about. Prefer it: it is what the rest of the fleet can ssh to.
    echo "$CACHED_LOCAL"
  else
    # It names somebody else. The GUI is HERE, so that answer is simply wrong;
    # say so and repair the cache rather than handing out a sideways host.
    note "cached live-host '$CACHED_LOCAL' is NOT this machine, but the GUI is running HERE — answering '$ME' and repairing the cache"
    remember "$ME"
    echo "$ME"
  fi
  exit 0
fi

CACHED=""
[ -r "$CACHE_FILE" ] && CACHED="$(head -1 "$CACHE_FILE" | tr -d '[:space:]')"

# 3 — the remembered alias, checked before it is trusted.
if [ -n "$CACHED" ] && has_gui "$CACHED"; then
  echo "$CACHED"
  exit 0
fi

# 4 — discovery. Candidates come from the operator, then the daemon (which knows
# none when it is headless), then this machine's own ssh configuration. Probed
# in parallel: a serial sweep pays the connect timeout once per unreachable host
# and that is the difference between a recipe and a wait.
CANDIDATES="${YGG_GUI_HOSTS:-}"
if [ -n "$BIN" ]; then
  CANDIDATES="$CANDIDATES $("$BIN" server app rows 2>&1 |
    grep -oE 'candidates this daemon knows: [a-z0-9, ]+' | sed 's/.*: //; s/,//g')"
fi
if [ -r "$HOME/.ssh/config" ]; then
  CANDIDATES="$CANDIDATES $(awk 'tolower($1)=="host"{for(i=2;i<=NF;i++) if ($i !~ /[*?]/) print $i}' \
                              "$HOME/.ssh/config")"
fi
CANDIDATES="$(printf '%s\n' $CANDIDATES | grep -v '^$' | awk '!seen[$0]++' | head -32)"

if [ -n "$CANDIDATES" ]; then
  PROBE_DIR="$(mktemp -d)"
  for h in $CANDIDATES; do
    ( has_gui "$h" && : > "$PROBE_DIR/$(printf '%s' "$h" | tr -c 'A-Za-z0-9._-' '_')" ) &
  done
  wait
  HIT=""
  for h in $CANDIDATES; do
    [ -e "$PROBE_DIR/$(printf '%s' "$h" | tr -c 'A-Za-z0-9._-' '_')" ] && { HIT="$h"; break; }
  done
  rm -rf "$PROBE_DIR"
  if [ -n "$HIT" ]; then
    [ "$HIT" = "$CACHED" ] || note "live GUI found on a host the cache did not name; remembering it"
    remember "$HIT"
    echo "$HIT"
    exit 0
  fi
fi

# 5 — no GUI is running anywhere we can see. The remembered alias is still the
# name you need in order to go and start one.
if [ -n "$CACHED" ]; then
  note "no live GUI client found on any candidate; using the remembered alias (the GUI may be down)"
  echo "$CACHED"
  exit 0
fi

echo "ygg-live-host: cannot resolve the live host." >&2
echo "  tried: \$YGG_GUI_HOST (unset), a live GUI client here, the cache at" >&2
echo "         $CACHE_FILE, and $(printf '%s' "$CANDIDATES" | wc -w) ssh/daemon candidates." >&2
echo "  Set \$YGG_GUI_HOST, or \$YGG_GUI_HOSTS to the aliases worth probing." >&2
exit 2
