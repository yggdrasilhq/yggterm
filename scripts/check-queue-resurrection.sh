#!/usr/bin/env bash
# check-queue-resurrection — has a CLOSED queue entry come back?
#
# ⛔ THE DEFECT THIS EXISTS FOR (measured 2026-08-13)
#   An entry was closed, live-proven and deleted from docs/pending-bugs.md. A
#   later commit re-added it verbatim — a whole-file write to the queue from a
#   stale copy, in a checkout several clusters share. The work was done; the
#   queue said it was not.
#
# ⚠ check-docs-ssot.sh CANNOT catch this and never will. It asks whether every
#   entry is well-formed, and a resurrected entry is PERFECTLY well-formed — it
#   was well-formed when it was written the first time. The question "should this
#   entry still be here at all" is not answerable from the file's own contents;
#   it is only answerable from history.
#
# ⇒ THE RULE: a heading that a commit DELETED and that is present again was
#   resurrected, unless someone deliberately re-opened it. Deliberate re-opening
#   is real and legitimate, so this REPORTS rather than fails by default, and
#   `git log -S "<heading>" -- docs/pending-bugs.md` settles who did what.
#
# ⭐ WHY IT WILL RECUR: the cause is a shared checkout plus a whole-file write,
#   not carelessness about queues. As long as several clusters edit one repo, the
#   queue is a merge surface like any other file — and it is the one file whose
#   silent regression costs the most, because it decides what anybody works on.
#
# USAGE
#   scripts/check-queue-resurrection.sh [--since <git-date>] [--strict]
#     --strict   exit 1 on any finding (for CI); default reports and exits 0
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2

SINCE="3 days ago"
STRICT=0
while [ $# -gt 0 ]; do
  case "$1" in
    --since)  SINCE="${2:-}"; shift 2 ;;
    --strict) STRICT=1; shift ;;
    -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
    *) echo "check-queue-resurrection: unknown argument: $1" >&2; exit 64 ;;
  esac
done

QUEUE=docs/pending-bugs.md
[ -f "$QUEUE" ] || { echo "queue-resurrection: no $QUEUE here" >&2; exit 2; }

SINCE="$SINCE" QUEUE="$QUEUE" STRICT="$STRICT" python3 - <<'PY'
import collections, os, subprocess, sys

QUEUE = os.environ["QUEUE"]
run = lambda c: subprocess.run(c, shell=True, capture_output=True, text=True).stdout

current = {l for l in run(f"grep '^## ' {QUEUE}").splitlines() if l.strip()}
shas = run(f"git log --since='{os.environ['SINCE']}' --format=%H -- {QUEUE}").split()

# A heading is 'removed' by a commit whose diff drops its '## ' line.
removed = collections.defaultdict(list)
for sha in shas:
    for line in run(f"git show {sha} --format= -- {QUEUE}").splitlines():
        if line.startswith("-## "):
            removed[line[1:]].append(sha)

hits = [(h, c) for h, c in removed.items() if h in current]

if not hits:
    print(f"queue-resurrection: ok — {len(current)} open entries, "
          f"none previously deleted ({len(shas)} queue commits scanned)")
    sys.exit(0)

print(f"⛔ queue-resurrection: {len(hits)} entry(ies) deleted earlier are present again\n",
      file=sys.stderr)
for h, c in hits:
    print(f"  {h}", file=sys.stderr)
    print(f"      deleted by: {' '.join(x[:8] for x in c)}", file=sys.stderr)
    print(f"      settle it:  git log -S {h[3:40]!r} -- {QUEUE}\n", file=sys.stderr)
print("If the entry was deliberately RE-OPENED, say so in the entry itself so the\n"
      "next reader is not left guessing. Otherwise it is a stale whole-file write —\n"
      "delete it again and check what else that commit reverted.", file=sys.stderr)
sys.exit(1 if os.environ["STRICT"] == "1" else 0)
PY
