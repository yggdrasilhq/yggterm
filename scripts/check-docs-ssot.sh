#!/usr/bin/env bash
# Enforce the docs SSOT law (docs/docs-ssot.md).
#
# The bug queue must list ONLY open items, every entry must declare exactly one
# status from the vocabulary, no deleted entry may come back through a merge,
# and no second file may advertise itself as a list of open bugs. Exits non-zero with the offending lines; no output means clean.
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
    if (count != 1) printf "%d: %d status line(s) — %s\n", ln, count, text
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
paras = [p.strip() for p in open(sys.argv[1], encoding="utf-8").read().split("\n\n")]
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

# 2c. No deleted entry comes back through a merge.
#
# ⛔ A GREEN INTEGRATION TICK CAN RESURRECT A DEAD ENTRY. Measured 2026-09-29
# ([11.207], the [11.203] merge ghost): a24d5d9a deleted [11.203] on main with
# its live proof; a lane based on PRE-deletion main still carried the entry
# text; delete-on-main vs carried-text-on-lane merged TEXTUALLY CLEAN (different
# hunks), so a green tick would have published the dead entry. Nothing else in
# this gate can see it: the resurrected text carries one valid status line, no
# duplicate paragraphs, no closed-marker heading.
#
# ⇒ The question is per id, at the plane the merge runs on: an id present in
#   this tree but ABSENT from origin/main, whose `## ⛔ [id]` heading line
#   origin/main's history ever carried (`git log -G` on the heading line), was
#   DELETED on main — refuse, naming the id and the newest heading-touching
#   commit. A stale origin/main weakens nothing at commit time (the id is still
#   in the stale blob, so the question is merely skipped); the integration tick
#   fetches first, and the tick is the plane that publishes. Non-numeric ids
#   ([CLI], [PASS N]) are topic buckets reused by design and are scoped out.
#   Re-filing under a fresh id is the doctrine; the same id coming back needs
#   the deletion reversed on main, not a quiet merge.
tombstones=$(python3 - "$QUEUE" <<'PY' || true
import re, subprocess, sys

HEAD = re.compile(r"^## ⛔ \[([0-9]+(?:\.[0-9]+)*)\]")

def heading_ids(text):
    found = {}
    for n, line in enumerate(text.splitlines(), 1):
        m = HEAD.match(line)
        if m:
            found.setdefault(m.group(1), n)
    return found

def git(args):
    return subprocess.run(["git"] + args, capture_output=True, text=True)

now = heading_ids(open(sys.argv[1], encoding="utf-8").read())
blob = git(["show", "origin/main:docs/pending-bugs.md"])
if blob.returncode != 0:
    sys.exit(0)  # no origin/main to ask — the tick's fresh fetch is the enforcement plane
was = heading_ids(blob.stdout)

for ident in sorted(set(now) - set(was)):
    hist = git(["log", "-G", r"^## ⛔ \[%s\]" % re.escape(ident),
                "--format=%h %s", "origin/main", "--", sys.argv[1]])
    if hist.stdout.strip():
        print(f"{ident}: re-added here, but origin/main DELETED this heading — "
              f"newest heading-touching commit on main: {hist.stdout.strip().splitlines()[0]}; "
              f"a merge carried a dead entry back. Refile under a fresh id.")
PY
)
if [ -n "$tombstones" ]; then
  note "these entry ids were deleted on origin/main and must not return through a merge:"
  echo "$tombstones" | head -10 >&2
fi

# 2d. Reused ids are REPORTED, not gated.
#
# The id-uniqueness dream (ACK-aad3de4a80 — the double [11.214] shipped through
# the gate) measured against the live queue 2026-09-29: id reuse is an
# ESTABLISHED PRACTICE here — the 99.x class is a topic bucket ([99.1] x5,
# [99.0] x2) and [11.0]/[11.49]/[11.97] each sit on two live entries. A hard
# uniqueness gate would red the plane on ten entries and fight the numbering
# scheme the owner has not ruled on. So: count, surface in the ok line, decide
# later. The dream's real target — one id one defect — stays open for that
# ruling; this line is the instrument that keeps it visible meanwhile.
dup_ids=$(python3 - "$QUEUE" <<'PY' || true
import re, sys
from collections import Counter
ids = Counter(re.findall(r"^## ⛔ \[([0-9]+(?:\.[0-9]+)*)\]",
                         open(sys.argv[1], encoding="utf-8").read(), re.M))
dups = {i: c for i, c in sorted(ids.items()) if c > 1}
if dups:
    print("; ".join(f"{i} x{c}" for i, c in dups.items()))
PY
)

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
  if [ -n "$dup_ids" ]; then
    echo "docs-ssot: ok — $entries open entries, all statused, one owner (reused ids: $dup_ids — informational, not gated)"
  else
    echo "docs-ssot: ok — $entries open entries, all statused, one owner"
  fi
fi
exit $fail
