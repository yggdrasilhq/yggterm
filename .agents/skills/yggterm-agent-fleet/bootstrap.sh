#!/usr/bin/env bash
# One-time wiring for an agent memory system. Idempotent: it creates what is
# missing and touches nothing that exists, so re-running it can never overwrite
# a memory someone wrote.
#
# The point is that this costs ZERO agent turns. Wiring a memory directory by
# hand, every time a new machine or a new user starts, is turns spent on
# scaffolding instead of work.
#
#   ./bootstrap.sh                 # default location, derived from the cwd
#   ./bootstrap.sh --dir <path>    # somewhere else
#   ./bootstrap.sh --campaign <slug>
set -euo pipefail

DIR=""
CAMPAIGN=""
while [ $# -gt 0 ]; do
  case "$1" in
    --dir)      DIR="${2:?--dir needs a path}"; shift 2 ;;
    --campaign) CAMPAIGN="${2:?--campaign needs a slug}"; shift 2 ;;
    -h|--help)  sed -n '2,12p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

# The default mirrors the convention most agent CLIs already use: one directory
# per project, named for the project's absolute path with separators flattened.
if [ -z "$DIR" ]; then
  DIR="$HOME/.claude/projects/$(pwd | tr '/' '-')/memory"
fi
: "${CAMPAIGN:=$(basename "$(pwd)")}"

created=0
note() { printf '  %s\n' "$1"; }

mkdir -p "$DIR"

INDEX="$DIR/MEMORY.md"
if [ ! -e "$INDEX" ]; then
  cat > "$INDEX" <<EOF
# Memory Index

> Doors, not rooms. One line per memory: a link and a hook short enough to
> decide relevance from. The body of a memory lives in its own file.
>
> ⛔ This index must never hold a second copy of what is OPEN. One question, one
> owner — a status list in two places rots in the one nobody reads.

| question | the one owner |
|---|---|
| What is OPEN? | the repo's queue file (e.g. \`docs/pending-bugs.md\`) |
| What SHIPPED? | git log + CHANGELOG |
| Why was a call made? | the campaign file below |
| What did the human settle? | a \`feedback-*\` memory |

## Campaign

- [The $CAMPAIGN campaign](campaign-$CAMPAIGN.md) — state, laws, handover log

## Findings

<!-- - [★ Title](finding-slug.md) — the hook, in one clause -->
EOF
  note "created $INDEX"; created=$((created + 1))
fi

CFILE="$DIR/campaign-$CAMPAIGN.md"
if [ ! -e "$CFILE" ]; then
  cat > "$CFILE" <<EOF
---
name: campaign-$CAMPAIGN
description: "Live ledger for the $CAMPAIGN campaign — standing laws, current state, and the handover log. Read §STATE first."
metadata:
  type: project
---

# The $CAMPAIGN campaign

## §LAWS

1. **One session grinds at a time.** A session spawns its successor, hands it
   one load-bearing subset, and is killed by it.
2. **Verify by the effect, never by the request.** A reply field that echoes
   what you asked for is not evidence that it happened.
3. **A test is proven only when you have watched it go RED.** Break the fix,
   see the failure, restore.
4. **Delete a queue entry in the same commit as its verified fix.**

## §STATE

<!-- What is true right now. Replace this, do not append to it. -->

Nothing yet — this campaign has not started.

## §HANDOVERS

<!-- Newest FIRST. Each: what shipped, what was measured, what was left, and
     the next load-bearing subset. This is the campaign's real output. -->
EOF
  note "created $CFILE"; created=$((created + 1))
fi

if [ "$created" -eq 0 ]; then
  echo "memory already wired at $DIR — nothing to do"
else
  echo "bootstrapped $created file(s) under $DIR"
fi
