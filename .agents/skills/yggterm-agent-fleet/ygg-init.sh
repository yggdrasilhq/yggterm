#!/usr/bin/env bash
# ygg init / ygg doctor — the deterministic onboarding engine.
#
# The yggterm-onboarding SKILL explains; this script DOES. Tiers:
#   0  node sanity            python3, bun, flock, ssh, git — fix hints, no changes
#   1  verbs + memory hub     dispersal present, hub reachable, sync works
#   2  observability          row registry present, agent shims healthy
#   3  swarm services (OPT-IN) dream orchestrator config + units (spawner timer);
#                             never armed without an explicit `--arm`
#
# usage: ygg init [--tier N] [--arm] [--fix]
#        ygg doctor [--fix]          (same engine, tiers 0-2 only)
set -uo pipefail
YGGBIN="${YGG_BIN_DIR:-$HOME/.yggterm/bin/ygg}"
TIER=3 ARM=0 FIX=0
MODE="${1:-init}"
shift || true
while [ $# -gt 0 ]; do
  case "$1" in
    --tier) TIER="$2"; shift 2 ;;
    --arm) ARM=1; shift ;;
    --fix) FIX=1; shift ;;
    *) shift ;;
  esac
done
[ "$MODE" = "doctor" ] && TIER=2

pass=0; fail=0
ok()   { echo "  ok   $1"; pass=$((pass+1)); }
bad()  { echo "  FAIL $1"; fail=$((fail+1)); }
fix()  { [ "$FIX" = 1 ] && shift 0; }

echo "ygg $MODE — tier $TIER $( [ "$FIX" = 1 ] && echo '(fix mode)')"

# ---- tier 0: node sanity ----------------------------------------------------
echo "tier 0 — node sanity"
for bin in python3 git ssh flock; do
  command -v "$bin" >/dev/null 2>&1 && ok "$bin" || bad "$bin missing — install it (apt/brew: coreutils provides flock)"
done
if command -v bun >/dev/null 2>&1 || [ -x "$HOME/.bun/bin/bun" ]; then ok bun; else bad "bun missing — https://bun.sh (the worker CLI needs it)"; fi

# ---- tier 1: verbs + memory hub --------------------------------------------
echo "tier 1 — verbs + memory hub"
if [ -d "$YGGBIN" ] && [ -f "$YGGBIN/ygg-disperse.py" ]; then
  ok "dispersal tree at $YGGBIN"
else
  bad "dispersal tree missing — run the yggterm bootstrap, then ygg-disperse.py install"
fi
if [ -x "$HOME/.local/bin/ygg-memory" ] || [ -x "$YGGBIN/ygg-memory" ]; then
  ok "ygg-memory runner present"
  if python3 "$HOME/.local/bin/ygg-memory.py" now >/dev/null 2>&1 || python3 "$YGGBIN/ygg-memory.py" now >/dev/null 2>&1; then
    ok "memory hub answers (now)"
  else
    bad "memory hub not answering — run ygg-memory sync --harness <you> --ns=_global and read the error"
  fi
else
  bad "ygg-memory runner missing — bootstrap.sh installs it"
fi

# ---- tier 2: observability --------------------------------------------------
echo "tier 2 — observability"
[ -f "$YGGBIN/ygg_row.py" ] && ok "row registry shipped" || bad "ygg_row.py missing (gui tier)"
if [ "$(systemctl --user is-active yggterm-dream-spawner.timer 2>/dev/null || true)" = "active" ] && [ ! -e "$HOME/.yggterm/config/dream/enabled" ]; then
  bad "spawner timer active but NOT armed — harmless (passes no-op) but confusing; arm or disable"
fi

# ---- tier 3: swarm services (opt-in) ----------------------------------------
if [ "$TIER" -ge 3 ]; then
  echo "tier 3 — swarm services (opt-in; nothing arms without --arm or manual touch)"
  CFG="$HOME/.yggterm/config/dream/floors.json"
  if [ -f "$CFG" ]; then ok "dream config exists ($CFG)"; else
    if [ "$FIX" = 1 ]; then python3 "$YGGBIN/ygg-dream-orchestrator.py" status >/dev/null 2>&1 && ok "dream config generated" || bad "dream config generation failed"
    else bad "dream config missing — generated on first orchestrator run or --fix"; fi
  fi
  for u in yggterm-dream-spawner.service yggterm-dream-spawner.timer; do
    if [ -f "$HOME/.config/systemd/user/$u" ]; then ok "$u installed"
    elif [ "$FIX" = 1 ] && [ -f "$YGGBIN/systemd/$u" ]; then
      mkdir -p "$HOME/.config/systemd/user"
      sed "s|%h|$HOME|g" "$YGGBIN/systemd/$u" > "$HOME/.config/systemd/user/$u"
      systemctl --user daemon-reload && ok "$u installed + daemon-reloaded"
    else bad "$u not installed — --fix installs it (systemd hosts)"
    fi
  done
  if [ "$ARM" = 1 ]; then
    mkdir -p "$HOME/.yggterm/config/dream" && touch "$HOME/.yggterm/config/dream/enabled"
    ok "DREAMING ARMED on this host (single-host law: arm exactly ONE host)"
  fi
fi

echo "ygg $MODE: $pass ok, $fail fail"
[ "$fail" -gt 0 ] && exit 1
exit 0
