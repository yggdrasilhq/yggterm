#!/usr/bin/env bash
# Enforce the docs SSOT law (docs/docs-ssot.md).
#
# The bug queue must list ONLY open items, every entry must declare exactly one
# status from the vocabulary, and no second file may advertise itself as a list
# of open bugs. Exits non-zero with the offending lines; no output means clean.
set -uo pipefail

cd "$(dirname "$0")/.." || exit 2
QUEUE="docs/pending-bugs.md"
fail=0

note() { echo "docs-ssot: $*" >&2; fail=1; }

[ -f "$QUEUE" ] || { note "$QUEUE is missing"; exit 1; }

# 1. No entry is CLOSED. Judged on the heading, because a live entry may well
#    say "this half is fixed" in its body and that is honest reporting, not a
#    dead entry: what must never happen is a heading that announces a fix.
closed=$(grep -nE '^## .*(✅|~~|\bCLOSED\b|FIXED AND VERIFIED|FIXED AND PROVEN|FOUND AND FIXED|\bSHIPPED\b)' "$QUEUE" || true)
if [ -n "$closed" ]; then
  note "these entries announce their own fix in the heading — delete them, git remembers:"
  echo "$closed" | head -20 >&2
fi

# 2. Every entry declares exactly one status from the vocabulary.
entries=$(grep -cE '^## ' "$QUEUE")
statuses=$(grep -cE '^\*\*Status:\*\* (OPEN|FIXED IN CODE — LIVE PROOF OWED|AWAITING A DECISION)$' "$QUEUE")
if [ "$entries" -ne "$statuses" ]; then
  note "$entries entries but $statuses valid status lines — every entry needs exactly one"
  grep -nE '^\*\*Status:\*\*' "$QUEUE" | grep -vE '\*\*Status:\*\* (OPEN|FIXED IN CODE — LIVE PROOF OWED|AWAITING A DECISION)$' | head -10 >&2
fi

# 3. No second file claims the queue.
# A file that POINTS at the queue is correct and expected (CLAUDE.md must). A
# file that reproduces one is the failure. Pointing = it names the queue's path.
rivals=$(grep -rlniE '^#+ .*(pending bugs|open bugs|bug list|what is left|what.s left)' \
  --include='*.md' docs/ . 2>/dev/null \
  | grep -vE "^(\./)?(docs/pending-bugs\.md|docs/docs-ssot\.md|docs/archive/|docs/triage-queue\.md|CHANGELOG\.md)" \
  | grep -vE '\.claude/worktrees/' \
  | while IFS= read -r f; do grep -q 'docs/pending-bugs\.md' "$f" || echo "$f"; done \
  | sort -u || true)
if [ -n "$rivals" ]; then
  note "these files also advertise a bug/status list — point at $QUEUE instead:"
  echo "$rivals" | head -10 >&2
fi

if [ "$fail" -eq 0 ]; then
  echo "docs-ssot: ok — $entries open entries, all statused, one owner"
fi
exit $fail
