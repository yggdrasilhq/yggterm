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
#
# ⛔⛔ PAIRED PER ENTRY, NEVER BY TOTAL — AND THE TOTAL VERSION WAS GREEN OVER TWO
# DEFECTS AT ONCE. This compared `grep -c '^## '` against `grep -c '^\*\*Status'`
# and required the two numbers to match. Measured 2026-08-22: 351 against 351,
# passing, while ONE entry was a bare heading with no body and no status (a fixed
# item's headstone, left behind when its body was deleted) and ANOTHER carried
# TWO status lines (a merge that kept both halves of one topic). Each defect
# moved a total by one, in opposite directions, so they cancelled.
#
# ⇒ **A total cannot fail on an absence when a surplus elsewhere pays for it.**
#   The check has to ask the question once per entry, which is the level the rule
#   is actually written at. Same shape as every other counting instrument this
#   project has had to repair: the aggregate answers a different question than
#   the one its name suggests.
# ⚠ Kept for the summary line at the end: a COUNT is a fine thing to report and a
#    poor thing to check with, which is the whole point of the pairing above.
entries=$(grep -cE '^## ' "$QUEUE")
bad_entries=$(awk '
  /^## / {
    if (seen) report(head_line, head_text, n)
    seen = 1; n = 0; head_line = NR; head_text = $0; next
  }
  /^\*\*Status:\*\* (OPEN|FIXED IN CODE — LIVE PROOF OWED|AWAITING A DECISION)$/ { if (seen) n++ ; next }
  END { if (seen) report(head_line, head_text, n) }
  function report(ln, text, count) {
    if (count != 1) printf "%d: %d status line(s) — %.90s\n", ln, count, text
  }
' "$QUEUE")
if [ -n "$bad_entries" ]; then
  note "every entry needs exactly one status line from the vocabulary:"
  echo "$bad_entries" | head -10 >&2
fi
# A malformed status line is its own message: the loop above only counts VALID
# ones, so a typo shows up as "0 status line(s)" without saying what was typed.
grep -nE '^\*\*Status:\*\*' "$QUEUE" \
  | grep -vE '\*\*Status:\*\* (OPEN|FIXED IN CODE — LIVE PROOF OWED|AWAITING A DECISION)$' \
  | head -10 > /tmp/ygg-docs-ssot-badstatus.$$ || true
if [ -s /tmp/ygg-docs-ssot-badstatus.$$ ]; then
  note "status lines outside the vocabulary:"
  cat /tmp/ygg-docs-ssot-badstatus.$$ >&2
fi
rm -f /tmp/ygg-docs-ssot-badstatus.$$

# 2b. No paragraph appears twice.
#
# ⛔ A CLEAN MERGE CAN PRODUCE A SELF-CONTRADICTING DOCUMENT, AND EVERY OTHER GATE
# HERE PASSES ON IT. Measured 2026-08-13: merging a lane into main duplicated a
# 41-line block with no conflict, no markers and no warning, keeping BOTH a
# superseded paragraph and the text that replaced it — so one entry said "the
# private side is done" in one place and "Next: the private side" forty lines
# later. The heading count was exactly right, every status was valid, no other
# lane's entry was touched. Duplicating a block breaks none of the rules above,
# so nothing could see it.
#
# ⇒ This file is SEMANTICALLY ORDERED — supersession, "next steps", status
# lines — and git merges it as TEXT. Anything whose meaning depends on which
# paragraph came later is exposed, and many lanes merge into this one file.
# Cheap to check, and it fails that merge outright.
dupes=$(python3 - "$QUEUE" <<'PY' || true
import sys, hashlib
from collections import Counter
paras = [p.strip() for p in open(sys.argv[1]).read().split("\n\n")]
long  = [p for p in paras if len(p) > 80]
key   = lambda p: hashlib.sha1(" ".join(p.split()).encode()).hexdigest()
counts = Counter(key(p) for p in long)
seen = set()
for p in long:
    h = key(p)
    if counts[h] > 1 and h not in seen:
        seen.add(h)
        print(f"  x{counts[h]}  {' '.join(p.split())[:110]}")
PY
)
if [ -n "$dupes" ]; then
  note "these paragraphs appear more than once — a merge duplicated a block, and the entry may now contradict itself:"
  echo "$dupes" | head -10 >&2
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
