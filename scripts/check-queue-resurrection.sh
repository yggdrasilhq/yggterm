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
#     --against <ref>  also scan deletions carried by <ref> — use BEFORE merging
#                      it, because an unmerged side is invisible to a HEAD scan
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2

SINCE="3 days ago"
STRICT=0
AGAINST=""
while [ $# -gt 0 ]; do
  case "$1" in
    --since)  SINCE="${2:-}"; shift 2 ;;
    --strict) STRICT=1; shift ;;
    # ⛔ Check BEFORE merging: name the ref you are about to merge, so a deletion
    #    it carries is visible while you can still honour it.
    --against) AGAINST="${2:-}"; shift 2 ;;
    -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
    *) echo "check-queue-resurrection: unknown argument: $1" >&2; exit 64 ;;
  esac
done

QUEUE=docs/pending-bugs.md
[ -f "$QUEUE" ] || { echo "queue-resurrection: no $QUEUE here" >&2; exit 2; }

SINCE="$SINCE" QUEUE="$QUEUE" STRICT="$STRICT" AGAINST="$AGAINST" python3 - <<'PY'
import collections, os, subprocess, sys

QUEUE = os.environ["QUEUE"]
run = lambda c: subprocess.run(c, shell=True, capture_output=True, text=True).stdout

current = {l for l in run(f"grep '^## ' {QUEUE}").splitlines() if l.strip()}

# ⛔⛔ THIS CHECK WAS BLIND AT THE ONE MOMENT IT MATTERS: DURING A MERGE.
#
# `git log -- QUEUE` lists only commits reachable from HEAD. While a merge is
# unresolved, the INCOMING side's commits are not reachable yet — they arrive
# only when the merge commits. So every deletion made upstream was invisible,
# and the check reported a clean tree over a resurrected entry.
#
# ⇒ That is not an edge case, it is the whole case. A resurrection happens when
#   a conflict hunk is settled by "keep my side" and the entry upstream CLOSED
#   sits inside it. The check runs, sees nothing, and blesses it.
#
# Measured 2026-08-14: an entry closed by an upstream fix was carried back into a
# lane twice in one hour. Both times this check exited 0. The closing commit was
# simply not an ancestor of the branch being checked, so nothing in the scan
# could have named it — and afterwards, once merged, the same check saw it fine,
# which is what makes the failure so easy to mistake for a false alarm.
#
# ⇒ Scan the UNION of both sides. `MERGE_HEAD` exists exactly while a merge is in
#   flight; `--against <ref>` covers checking BEFORE starting one.
rev_range = "HEAD"
extra = run("git rev-parse -q --verify MERGE_HEAD").strip()
if not extra and os.environ.get("AGAINST"):
    extra = run(f"git rev-parse -q --verify {os.environ['AGAINST']}").strip()
if extra:
    rev_range = f"HEAD {extra}"

# ⛔ AND DIFF MERGES AGAINST THEIR FIRST PARENT. `git show <merge>` prints NO
#   diff by default, so a deletion that reached a branch THROUGH a merge is
#   invisible a second way — the same trap this file already warns about for
#   `git log -S`, which it had while being subject to it.
shas = run(
    f"git log --since='{os.environ['SINCE']}' --format=%H {rev_range} -- {QUEUE}"
).split()

# ⛔⛔ A CHECK THAT CANNOT SEE ITS OWN PRESCRIBED REMEDY BECOMES NOISE, AND NOISE
# IS WHY NOBODY RUNS IT. This told the reader to "say so in the entry itself" and
# then had no way to observe that they had — so a correctly re-opened entry was
# reported forever, identically to a stale whole-file write. Measured 2026-08-14:
# one entry had been re-opened DELIBERATELY, said so in its own body at length,
# and this check had been crying about it "ever since, and nobody ran it" — the
# entry's own words. The check trained the neglect that then hid a real one.
# ⇒ Honour an explicit declaration. `**Re-opened:**` inside the entry body means
#   a human decided; report it as acknowledged and keep the exit clean.
body = open(QUEUE, encoding="utf-8").read().split("\n## ")
reopened = set()
for chunk in body:
    head = "## " + chunk.split("\n", 1)[0] if not chunk.startswith("## ") else chunk.split("\n", 1)[0]
    if "**Re-opened:**" in chunk:
        reopened.add(head.strip())

# A heading is 'removed' by a commit whose diff drops its '## ' line.
removed = collections.defaultdict(list)
for sha in shas:
    for line in run(
        f"git show --diff-merges=first-parent {sha} --format= -- {QUEUE}"
    ).splitlines():
        if line.startswith("-## "):
            removed[line[1:]].append(sha)

all_hits = [(h, c) for h, c in removed.items() if h in current]
hits = [(h, c) for h, c in all_hits if h.strip() not in reopened]
acked = [h for h, _ in all_hits if h.strip() in reopened]

for h in acked:
    print(f"queue-resurrection: ✅ acknowledged re-open — {h[3:70]}")

if not hits:
    print(f"queue-resurrection: ok — {len(current)} open entries, "
          f"{len(acked)} declared re-open(s) ({len(shas)} queue commits scanned)")
    sys.exit(0)

print(f"⛔ queue-resurrection: {len(hits)} entry(ies) deleted earlier are present again\n",
      file=sys.stderr)
for h, c in hits:
    print(f"  {h}", file=sys.stderr)
    print(f"      deleted by: {' '.join(x[:8] for x in c)}", file=sys.stderr)
    # ⛔⛔ `git log -S` SKIPS MERGE COMMITS BY DEFAULT, and a deletion that
    #    reached main THROUGH a merge is therefore invisible to it — so it
    #    answers "this was never here", which is precisely the answer that tells
    #    a lane to keep its own side and resurrect the entry. Measured
    #    2026-08-14: a lane settled three empty-upstream conflicts with the bare
    #    form, concluded "never had it" for all three, and brought one back.
    print(f"      settle it:  git log --full-history -m -S {h[3:40]!r} -- {QUEUE}",
          file=sys.stderr)
    print(f"      ⚠ --full-history -m is NOT optional: without them a deletion "
          f"that arrived via a MERGE reads as 'never existed'.\n", file=sys.stderr)
print("If the entry was deliberately RE-OPENED, add a line `**Re-opened:** <why>` to\n"
      "the entry body — this check reads it and will stop reporting that entry.\n"
      "⛔ Say it in the entry, not in a commit message: the next reader has the file,\n"
      "not the history. Otherwise it is a stale whole-file write — delete it again\n"
      "and check what else that commit reverted.", file=sys.stderr)
if os.environ["STRICT"] != "1":
    # ⛔⛔ SAY THAT THIS EXIT IS A LIE ABOUT THE FINDING. The default is
    #    report-and-pass, which is right for a human reading the output and
    #    WRONG for every automated caller — and the campaign's own standing
    #    instruction ("run this after any queue merge") does not mention the
    #    flag. Measured 2026-08-14: a resurrection sat on main through several
    #    push loops, each of which ran this check and treated exit 0 as clean.
    #    A gate that reports a violation and exits 0 is decoration.
    print("\n⚠ EXITING 0 ANYWAY — this is the default. Findings above are REAL.\n"
          "  ⛔ In a script or a push loop, run `--strict` or this check gates nothing.",
          file=sys.stderr)
sys.exit(1 if os.environ["STRICT"] == "1" else 0)
PY
