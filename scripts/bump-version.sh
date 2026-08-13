#!/usr/bin/env bash
# Allocate the next version number — from `origin/main`, and claim it immediately.
#
# ⛔ THIS EXISTS BECAUSE A NUMBER TAKEN FROM A STALE FILE IS THE WHOLE DEFECT.
# On 2026-08-13, `3.0.117`–`3.0.120` were each allocated TWICE within minutes.
# The mechanism is not exotic: a cluster reads the version out of its working
# `Cargo.toml`, adds one, and carries that number for the length of a build; a
# second cluster that read the same file before the first pushed takes the same
# number. Four consecutive numbers each meant two builds, so every "is my fix
# live?" check written against `--version` was answering a different question.
#
# Two changes make the number an identifier again, and both are in here:
#
#   1. THE NUMBER COMES FROM `origin/main`, NEVER FROM THE LOCAL FILE. The local
#      file is exactly as old as the last time this checkout pulled, which on a
#      three-host fleet is a coin flip.
#   2. THE BUMP IS PUSHED ALONE, FIRST, BEFORE THE BUILD. The race window shrinks
#      from "the length of a build and a deploy" to "the length of one push", and
#      a lost race is *detected* — the push is rejected, and this script takes the
#      next number and tries again rather than spending one twice.
#
# ⚠ It deliberately does NOT bump as part of a work commit, which is what the
# repo did before: a number that rides along with the code it releases is only
# claimed when that work lands, which is the whole race.
#
#   scripts/bump-version.sh [--dry-run] [--no-push] [--attempts N]
#
# Prints the allocated version on stdout. Diagnostics go to stderr, so
# `VERSION=$(scripts/bump-version.sh)` is safe to substitute.
set -uo pipefail

DRY=0
PUSH=1
ATTEMPTS=3
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run)  DRY=1; shift;;
    --no-push)  PUSH=0; shift;;
    --attempts) ATTEMPTS="$2"; shift 2;;
    *) echo "unknown argument: $1" >&2; exit 2;;
  esac
done

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO" || exit 1
note() { echo "bump-version: $*" >&2; }

git rev-parse --git-dir >/dev/null 2>&1 || { note "⛔ not a git checkout"; exit 1; }

# `version = "X.Y.Z"` from a Cargo.toml given on stdin. Anchored to the first
# match so a dependency's own `version =` further down cannot answer instead.
read_version() {
  awk -F'"' '/^version = "/ { print $2; exit }'
}

attempt=1
while [ "$attempt" -le "$ATTEMPTS" ]; do
  # ── the number's one source ────────────────────────────────────────────────
  git fetch --quiet origin main 2>/dev/null || {
    note "⛔ cannot reach origin — refusing to allocate a number from a local file,"
    note "   which is the exact failure this script exists to prevent."
    exit 1; }
  UPSTREAM="$(git rev-parse FETCH_HEAD)"

  # ⛔ The bump must go up ALONE, so this checkout may not be carrying anything
  # else. Behind is fine and gets fast-forwarded; ahead is not, because the push
  # would drag unrelated commits along under a `chore(release)` message.
  HEAD_SHA="$(git rev-parse HEAD)"
  if [ "$HEAD_SHA" != "$UPSTREAM" ]; then
    if git merge-base --is-ancestor "$HEAD_SHA" "$UPSTREAM"; then
      note "this checkout is behind origin/main — fast-forwarding before allocating"
      git merge --ff-only "$UPSTREAM" >/dev/null 2>&1 || {
        note "⛔ fast-forward failed; run: git pull --rebase origin main"; exit 1; }
    else
      note "⛔ REFUSING: this checkout has commits that are not on origin/main."
      note "   A version bump is pushed ALONE — push or rebase your work first:"
      git log --oneline "$UPSTREAM..HEAD" | head -10 | sed 's/^/     /' >&2
      exit 1
    fi
  fi

  CURRENT="$(git show "$UPSTREAM:Cargo.toml" | read_version)"
  [ -n "$CURRENT" ] || { note "⛔ no version line in origin/main's Cargo.toml"; exit 1; }
  MAJOR="${CURRENT%%.*}"; REST="${CURRENT#*.}"
  MINOR="${REST%%.*}";    PATCH="${REST##*.}"
  case "$MAJOR$MINOR$PATCH" in *[!0-9]*)
    note "⛔ origin/main's version $CURRENT is not X.Y.Z"; exit 1;; esac
  NEXT="$MAJOR.$MINOR.$((PATCH + 1))"

  LOCAL="$(read_version < Cargo.toml)"
  note "origin/main is at $CURRENT (local file says $LOCAL) → allocating $NEXT"

  if [ "$DRY" = 1 ]; then
    echo "$NEXT"
    exit 0
  fi

  # ── claim it ───────────────────────────────────────────────────────────────
  # ⛔ The two files this commit will carry must be exactly what origin has, or
  # the bump publishes someone else's uncommitted edit under a release message.
  # This is a shared checkout; that is a normal state, not a freak one. (Checked
  # after the dry run above, so "what number would I get?" is always answerable.)
  for f in Cargo.toml Cargo.lock; do
    [ -e "$f" ] || continue
    if ! git diff --quiet -- "$f" || ! git diff --cached --quiet -- "$f"; then
      note "⛔ REFUSING: $f has uncommitted changes, and the bump commit would"
      note "   carry them. Commit or restore it first — the bump goes up alone."
      exit 1
    fi
  done

  # Only the version line, and only the first one: `sed -i` over the whole file
  # would rewrite any dependency pinned to the old number too.
  perl -0pi -e "s/^version = \"\Q$CURRENT\E\"/version = \"$NEXT\"/m unless \$done++" Cargo.toml
  [ "$(read_version < Cargo.toml)" = "$NEXT" ] || {
    note "⛔ the version line did not take the new value"; exit 1; }

  # The lock records every workspace member's version, so it moves with the
  # bump. `cargo metadata` resolves and rewrites it without compiling anything.
  LOCKED=""
  if [ -f Cargo.lock ] && command -v cargo >/dev/null 2>&1; then
    if cargo metadata --format-version 1 --offline >/dev/null 2>&1 ||
       cargo metadata --format-version 1 >/dev/null 2>&1; then
      LOCKED="Cargo.lock"
    else
      note "⚠ could not refresh Cargo.lock — commit it yourself before building"
    fi
  fi

  # ⛔ Staged BY PATH. This is a shared checkout: another session's in-flight
  # work is routinely sitting in the tree, and `commit -a` would publish it
  # under a release message.
  git add -- Cargo.toml $LOCKED
  git commit --quiet --only -m "chore(release): $NEXT" -- Cargo.toml $LOCKED || {
    note "⛔ the bump commit failed"; exit 1; }

  if [ "$PUSH" = 0 ]; then
    note "committed $NEXT locally; --no-push, so the number is NOT yet claimed"
    echo "$NEXT"
    exit 0
  fi

  if git push --quiet origin HEAD:main 2>/dev/null; then
    note "✅ $NEXT is claimed on origin/main — build and deploy against it"
    echo "$NEXT"
    exit 0
  fi

  # ── a lost race is the SUCCESS case for this design ────────────────────────
  # Someone else pushed between the fetch and the push, so this number is spent.
  # Undo our own commit and restore ONLY the two files we touched — never
  # `reset --hard`, which would take another session's uncommitted work with it.
  note "push rejected — another cluster claimed $NEXT first; taking the next one"
  git reset --quiet --soft HEAD~1
  git restore --staged --worktree -- Cargo.toml $LOCKED 2>/dev/null ||
    git checkout -- Cargo.toml $LOCKED 2>/dev/null
  attempt=$((attempt + 1))
done

note "⛔ gave up after $ATTEMPTS attempts — origin/main is moving faster than this"
note "   script can claim a number. Re-run, or allocate by hand and push it alone."
exit 1
