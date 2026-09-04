#!/usr/bin/env bash
# One-time wiring for an agent memory system + fleet timers. Idempotent: it creates
# what is missing and touches nothing that exists, so re-running it can never
# overwrite a memory someone wrote or clobber a tuned timer.
#
# The point is that this costs ZERO agent turns. Wiring a memory directory by
# hand, every time a new machine or a new user starts, is turns spent on
# scaffolding instead of work.
#
#   ./bootstrap.sh                 # default location, derived from the cwd
#   ./bootstrap.sh --dir <path>    # somewhere else
#   ./bootstrap.sh --campaign <slug>
#   ./bootstrap.sh --with-timers   # also install ygg-memory/tick timers (Linux systemd --user)
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

> 🌐 **UNIFIED FLEET MEMORY**: Before deep memory recall or after campaign handovers, consult \`ygg-memory status --harness <me>\` or \`ygg-memory diff\` to catch updates from Claude, Grok, Codex, or Gemini. Ingest full or partial diffs as needed.
> ⛔ **Doors, not rooms.** One line per memory: a link and a hook short enough to
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

# --- Fleet timers (opt-in via --with-timers, or auto on first bootstrap) ---
WITH_TIMERS=false
for arg in "$@"; do
  case "$arg" in --with-timers) WITH_TIMERS=true ;; esac
done
# Also auto-install timers on first-ever bootstrap in this repo (created>0)
# so a new user gets fleet sync without asking. Re-runs are no-ops.
if [ "$WITH_TIMERS" = true ] || [ "$created" -gt 0 ]; then
  if command -v systemctl >/dev/null 2>&1 && [ -d "$HOME/.config/systemd/user" ] || mkdir -p "$HOME/.config/systemd/user" 2>/dev/null; then
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    # Install ygg-memory fleet/harness + booter/monitor timers if not already enabled
    for pair in "ygg-memory-fleet" "ygg-memory-harness" "ygg-booter-tick" "ygg-monitor-tick"; do
      if [ -f "$SCRIPT_DIR/../yggterm-agent-fleet/ygg-memory.py" ] || [ -f "$HOME/.local/bin/ygg-memory" ]; then
        # Timers are created by the skill's timer definitions; ensure they exist and are enabled
        if [ -f "$HOME/.config/systemd/user/${pair}.timer" ]; then
          systemctl --user daemon-reload >/dev/null 2>&1 || true
          systemctl --user enable --now "${pair}.timer" >/dev/null 2>&1 || true
          note "enabled ${pair}.timer (kernel-coalesced, idle-nice)"
        fi
      fi
    done
  fi
  # Ensure ygg-memory and ygg-memory-sync are on PATH for hooks — and keep the
  # installed copies fresh. The checkout is the SSOT for the verb's code the same
  # way the hub is the SSOT for memory content: every other CLI's memory is a
  # cache the sync refreshes unconditionally, and this install once guarded the
  # installed base so hard that the zcode adapter drifted for days existing only
  # in ~/.local/bin (a reinstall would have silently regressed fleet-memory
  # sync). Refresh unconditionally; hot-patching an installed copy is now
  # correctly impossible — land it in the checkout and re-run bootstrap.
  if [ -f "$SCRIPT_DIR/ygg-memory" ]; then
    mkdir -p "$HOME/.local/bin"
    cp "$SCRIPT_DIR/ygg-memory" "$HOME/.local/bin/ygg-memory" 2>/dev/null || true
    cp "$SCRIPT_DIR/ygg-memory.py" "$HOME/.local/bin/ygg-memory.py" 2>/dev/null || true
    cp "$SCRIPT_DIR/ygg-memory-sync" "$HOME/.local/bin/ygg-memory-sync" 2>/dev/null || true
    chmod +x "$HOME/.local/bin/ygg-memory"* 2>/dev/null || true
    note "installed/refreshed ygg-memory to ~/.local/bin"
  fi
  # Backfill Muse/Gemini/Codex from unified if they are empty (new user with only Claude)
  if command -v ygg-memory >/dev/null 2>&1 || [ -x "$HOME/.local/bin/ygg-memory" ]; then
    YM="$HOME/.local/bin/ygg-memory"; [ -x "$YM" ] || YM="ygg-memory"
    for h in muse gemini codex grok; do
      # Count local projects for this harness
      cnt=$(find "$HOME/.${h}/projects" -maxdepth 2 -name "memory" -type d 2>/dev/null | wc -l)
      if [ "$cnt" -le 1 ]; then
        $YM sync-harness --harness "$h" --all >/dev/null 2>&1 || true
        note "backfilled $h from unified fleet memory ($cnt -> all)"
      fi
    done
  fi
fi
