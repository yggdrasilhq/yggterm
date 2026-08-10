#!/usr/bin/env bash
# A SETTLED CALL IS LAW. No other document may narrow it, re-scope it, or
# reintroduce a permission gate it removed.
#
# Why this exists as a SCRIPT and not a paragraph: the paragraph already existed.
# On 2026-08-10 the owner had settled "always restart the GUI, do not ask" in
# `docs/settled-calls.md`; a session quoted that rule correctly, obeyed it once,
# and then stopped restarting anyway — because a NEIGHBOURING note in
# `pending-bugs.md` said "one deploy per session" and the session read a
# daemon-binary rule onto a GUI action. He had to ask "why did that steer not
# work?". Re-stating the rule is not a fix for a rule that was already stated;
# only something that FAILS can be.
#
# Deliberately narrow. A guard that cries wolf gets bypassed, which is worse than
# no guard, so this checks two things it can check exactly.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SETTLED="$ROOT/docs/settled-calls.md"
status=0
note() { printf 'settled-calls: %s\n' "$1" >&2; status=1; }

[ -f "$SETTLED" ] || { note "docs/settled-calls.md is missing — it is the law file"; exit 1; }

# The docs a session actually reads for "what am I allowed to do". Scoped to
# these rather than the whole tree so a historical narrative in docs/archive/
# cannot fail the build for quoting an old rule.
mapfile -t DOCS < <(
  find "$ROOT/docs" -maxdepth 1 -name '*.md' ! -name 'settled-calls.md' -print
  printf '%s\n' "$ROOT/CLAUDE.md" "$ROOT/AGENTS.md"
)

# ── 1. "deploy per session" MUST say which kind of deploy ────────────────────
# The rule is about DAEMON binaries, whose blast radius is the cold-shutdown
# cascade and a mass re-resume of live rows. A GUI restart has none of that, and
# the owner has settled that it needs no permission. An unqualified phrasing is
# exactly what swallowed the GUI case once.
for doc in "${DOCS[@]}"; do
  [ -f "$doc" ] || continue
  while IFS=: read -r lineno line; do
    [ -n "$lineno" ] || continue
    printf '%s' "$line" | grep -qi 'daemon' && continue
    note "${doc#"$ROOT"/}:$lineno says 'deploy per session' without naming DAEMON — a GUI restart is not a deploy, and an unqualified rule will be read onto it"
  done < <(grep -nEi 'deploy[s]? per session' "$doc" || true)
done

# ── 2. No doc outside the law file may gate a GUI RESTART on permission ──────
# The single sanctioned exception (an ACTIVE ychrome row) lives in
# settled-calls.md and nowhere else. A line that both names a GUI restart and
# asks for permission is either that exception restated in the wrong file, or a
# new gate the owner never set.
for doc in "${DOCS[@]}"; do
  [ -f "$doc" ] || continue
  while IFS=: read -r lineno line; do
    [ -n "$lineno" ] || continue
    # An explicit "no permission needed" phrasing is the rule, not a breach.
    printf '%s' "$line" | grep -qiE 'no permission|without (asking|permission)|need[s]? no permission|never gates' && continue
    # Quoting the sanctioned exception is allowed when it names ychrome.
    printf '%s' "$line" | grep -qi 'ychrome' && continue
    note "${doc#"$ROOT"/}:$lineno appears to gate a GUI restart on permission — only docs/settled-calls.md may, and only for an ACTIVE ychrome row"
  done < <(grep -nEi '(gui|app) restart.{0,80}(permission|ask (the )?(user|owner|him)|wait for (his|the owner))|((permission|ask (the )?(user|owner|him)|wait for (his|the owner)).{0,80}(gui|app) restart)' "$doc" || true)
done

if [ "$status" -eq 0 ]; then
  echo "settled-calls: ok — no document re-scopes a settled call"
fi
exit "$status"
