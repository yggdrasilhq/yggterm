#!/usr/bin/env bash
# Resolve a conflicted merge of the status docs as a UNION, then honour every
# entry the upstream side DELETED.
#
# ⛔ THE UNION ALONE IS THE RESURRECTION TRAP, AND IT IS NOT THEORETICAL.
# Every lane adds at the TOP of `docs/pending-bugs.md`, so a busy `main` collides
# there on essentially every merge. Resolving "keep both sides" is right for the
# additions and WRONG for anything upstream closed: an entry that main deleted
# with its fix sits INSIDE the conflict hunk, so keeping our side silently
# re-opens it. That happened twice in one session, and
# `scripts/check-queue-resurrection.sh` passed both times — it inspects committed
# history, not a working tree mid-merge, so it cannot see a resurrection that has
# not been committed yet.
#
# ⇒ The two halves have to be done separately and in this order:
#   1. union the hunk (drop only the marker lines, keep both sides)
#   2. delete every `## ` heading present in the MERGE BASE but absent from
#      MERGE_HEAD — i.e. everything the upstream side closed while we worked
#
# ⚠ Step 2 is keyed on the merge BASE, not on our side. An entry WE added is not
# in the base, so it is never a deletion candidate; an entry upstream closed is.
# That asymmetry is what makes this safe to run unattended.
#
# Run it while the merge is in progress, then check the gates and commit:
#   git merge origin/main || scripts/resolve-queue-merge.sh
#   ./scripts/check-docs-ssot.sh && ./scripts/check-queue-resurrection.sh
#   git commit --no-edit
#
# ⛔ It stages ONLY the files it resolved. It never runs `git add -A`, because a
# host with a duplicated agent can have another session's uncommitted work in the
# same tree — see the duplicate-agent entry in `docs/pending-bugs.md`.
set -uo pipefail

if ! git rev-parse -q --verify MERGE_HEAD >/dev/null 2>&1; then
  echo "resolve-queue-merge: no merge in progress" >&2
  exit 2
fi

BASE="$(git merge-base HEAD MERGE_HEAD)"
CONFLICTED="$(git diff --name-only --diff-filter=U)"
if [ -z "$CONFLICTED" ]; then
  echo "resolve-queue-merge: nothing conflicted" >&2
  exit 0
fi

for f in $CONFLICTED; do
  case "$f" in
    *.md) ;;
    *)
      echo "resolve-queue-merge: ⛔ $f is not a status doc — resolve it by hand" >&2
      exit 3
      ;;
  esac
  # union: drop the marker lines only
  sed -i '/^<<<<<<< /d;/^=======$/d;/^>>>>>>> /d' "$f"
done

for f in $CONFLICTED; do
  base_h="$(mktemp)"; theirs_h="$(mktemp)"
  git show "$BASE:$f"       2>/dev/null | grep '^## ' | sort -u > "$base_h"
  git show "MERGE_HEAD:$f"  2>/dev/null | grep '^## ' | sort -u > "$theirs_h"
  while IFS= read -r heading; do
    [ -z "$heading" ] && continue
    grep -Fxq "$heading" "$f" || continue
    start="$(grep -Fxn "$heading" "$f" | head -1 | cut -d: -f1)"
    end="$(awk -v s="$start" 'NR>s && /^## /{print NR-1; exit}' "$f")"
    [ -z "$end" ] && end="$(wc -l < "$f")"
    sed -i "${start},${end}d" "$f"
    echo "resolve-queue-merge: honoured upstream deletion — ${heading:0:72}"
  done < <(comm -23 "$base_h" "$theirs_h")
  rm -f "$base_h" "$theirs_h"
  git add -- "$f"
done

echo "resolve-queue-merge: resolved $(echo "$CONFLICTED" | wc -w) file(s); run the gates before committing"
