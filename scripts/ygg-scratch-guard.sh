#!/usr/bin/env bash
# Scratch-space guard: keep agent scratch OFF tmpfs, and keep it BOUNDED.
#
#   scripts/ygg-scratch-guard.sh            # report; non-zero if anything is abusive
#   scripts/ygg-scratch-guard.sh --enforce  # also reap: oldest-first, until under budget
#   scripts/ygg-scratch-guard.sh --host H   # run against a remote host over ssh
#   scripts/ygg-scratch-guard.sh --json
#
# ⛔ WHY THIS EXISTS. `/tmp` on the desktop host is a **tmpfs**, which is RAM
#    wearing a filesystem's clothes. Anything staged there is charged to memory
#    and then to swap. Measured 2026-08-14: a CLI provisioner had leaked 51
#    staging dirs totalling 2.85 GB there, and the machine sat at 11 GB of 15 GB
#    swap while its owner reported it burning. Nothing was holding those files
#    open. They were simply never deleted, and nothing was watching.
#
# ⇒ THE RULE: agent scratch belongs in `~/.yggterm/scratchpad/<whatever>`, which
#   is disk-backed. Not `/tmp`, not `/dev/shm`, not `$XDG_RUNTIME_DIR`.
#
# ⚠ And a disk-backed scratch is not a licence either — that is the second half
#   of this guard. An unbounded directory on disk is a slower leak, not a fixed
#   one, so every root below carries a budget and the oldest entries are reaped
#   first when it is exceeded.

set -uo pipefail

ENFORCE=0
JSON=0
HOST=""
while [ $# -gt 0 ]; do
  case "$1" in
    --enforce) ENFORCE=1 ;;
    --json)    JSON=1 ;;
    --host)    shift; HOST="${1:-}" ;;
    -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
  esac
  shift
done

# Budgets in MB. Deliberately generous: this guard exists to catch a RUNAWAY,
# not to police normal work. A budget tight enough to trip on ordinary use is a
# budget people learn to ignore.
BUDGET_SCRATCHPAD_MB=2048
BUDGET_STAGING_MB=512
BUDGET_TMPFS_OURS_MB=256   # ⛔ our footprint on ANY tmpfs. Small on purpose.

run() {
  if [ -n "$HOST" ]; then ssh -o BatchMode=yes -o ConnectTimeout=10 "$HOST" "$1"; else bash -c "$1"; fi
}

REPORT=""
STATUS=0
add() { REPORT="${REPORT}$1"$'\n'; }
bad() { STATUS=1; }

# ---------------------------------------------------------------- the roots
# `is_tmpfs` is the whole point: a root that is SUPPOSED to be disk-backed but
# has been mounted on tmpfs is the abuse this guard is named for, and it is
# invisible to a size check.
PROBE='
scratch="$HOME/.yggterm/scratchpad"
staging="$HOME/.yggterm/cli-staging"
mkdir -p "$scratch" "$staging" 2>/dev/null
for d in "$scratch" "$staging"; do
  fstype=$(stat -f -c %T "$d" 2>/dev/null)
  mb=$(du -sm "$d" 2>/dev/null | cut -f1)
  echo "root|$d|${fstype:-unknown}|${mb:-0}"
done
# Our own footprint on every tmpfs mount, whoever wrote it.
for m in $(awk "\$3==\"tmpfs\"{print \$2}" /proc/mounts 2>/dev/null | sort -u); do
  [ -d "$m" ] || continue
  mb=$(find "$m" -maxdepth 2 \( -name "ygg*" -o -name "*yggterm*" -o -name "codex-litellm-*" \) -print0 2>/dev/null \
       | du -sm --files0-from=- 2>/dev/null | awk "{s+=\$1} END{print s+0}")
  tot=$(df -m "$m" 2>/dev/null | tail -1 | awk "{print \$3}")
  echo "tmpfs|$m|${mb:-0}|${tot:-0}"
done
'
OUT="$(run "$PROBE" 2>/dev/null)"

while IFS='|' read -r kind a b c; do
  [ -z "${kind:-}" ] && continue
  case "$kind" in
    root)
      dir="$a"; fstype="$b"; mb="${c:-0}"
      case "$dir" in
        *scratchpad) budget=$BUDGET_SCRATCHPAD_MB ;;
        *)           budget=$BUDGET_STAGING_MB ;;
      esac
      # ⛔ A scratch root ON a tmpfs defeats the entire purpose, and no amount of
      #    reaping fixes it — the bytes are RAM while they exist.
      if [ "$fstype" = "tmpfs" ] || [ "$fstype" = "ramfs" ]; then
        add "ABUSE  $dir is on $fstype - scratch MUST be disk-backed"
        bad
      fi
      if [ "${mb:-0}" -gt "$budget" ]; then
        add "OVER   $dir ${mb}MB > ${budget}MB budget"
        bad
      else
        add "ok     $dir ${mb}MB / ${budget}MB ($fstype)"
      fi
      ;;
    tmpfs)
      mount="$a"; ours="${b:-0}"; used="${c:-0}"
      if [ "${ours:-0}" -gt "$BUDGET_TMPFS_OURS_MB" ]; then
        add "ABUSE  ${ours}MB of OUR files on tmpfs $mount (limit ${BUDGET_TMPFS_OURS_MB}MB, mount holds ${used}MB)"
        bad
      elif [ "${ours:-0}" -gt 0 ]; then
        add "ok     ${ours}MB of ours on tmpfs $mount (limit ${BUDGET_TMPFS_OURS_MB}MB)"
      fi
      ;;
  esac
done <<< "$OUT"

# ---------------------------------------------------------------- enforcement
# Oldest-first, and only inside roots we own. ⛔ It never deletes from a tmpfs
# mount it does not own a subtree of: reaping another program's files to make
# our own numbers look good is not this tool's business, and the report already
# names them.
if [ "$ENFORCE" -eq 1 ] && [ "$STATUS" -ne 0 ]; then
  REAP='
for pair in "$HOME/.yggterm/scratchpad:'"$BUDGET_SCRATCHPAD_MB"'" "$HOME/.yggterm/cli-staging:'"$BUDGET_STAGING_MB"'"; do
  d="${pair%:*}"; budget="${pair##*:}"
  [ -d "$d" ] || continue
  # Oldest first, stopping as soon as the root is back under budget, so a busy
  # scratch loses its stale end rather than its current work.
  while [ "$(du -sm "$d" 2>/dev/null | cut -f1)" -gt "$budget" ]; do
    victim=$(find "$d" -mindepth 1 -maxdepth 1 -printf "%T@ %p\n" 2>/dev/null | sort -n | head -1 | cut -d" " -f2-)
    [ -z "$victim" ] && break
    rm -rf "$victim" || break
    echo "reaped $victim"
  done
done
# ⛔ AND THE tmpfs ITSELF, which is the abuse this guard is named for.
#
# The scratch roots above are ours to relocate; an agent harness that picks its
# own `/tmp/<tool>-<uid>` session directory is not, and telling people to stop
# has never once bounded anything. So the protection that actually holds is to
# REAP what is stale there. Measured 2026-08-14: 613 MB of agent scratch and
# 452 MB of abandoned deploy binaries, all on a RAM-backed mount, none of it
# held open by any process.
#
# ⚠ Only entries untouched for over a day, and only shapes we can identify: a
#   live session keeps its scratch warm, and reaping a running agent out from
#   under itself would trade a memory leak for lost work.
for m in $(awk "\$3==\"tmpfs\"{print \$2}" /proc/mounts | sort -u); do
  [ -d "$m" ] || continue
  case "$m" in /run/user/*|/tmp) ;; *) continue ;; esac
  find "$m" -mindepth 2 -maxdepth 2 -type d -mtime +1 \
       \( -path "*/claude-*/*" -o -path "*/codex-*/*" \) \
       -exec rm -rf {} + 2>/dev/null && echo "reaped stale agent scratch under $m"
  find "$m" -mindepth 1 -maxdepth 1 -type d -mtime +1 -name "codex-litellm-*" \
       -exec rm -rf {} + 2>/dev/null && echo "reaped stale provisioner dirs under $m"
done'
  ENF="$(run "$REAP" 2>/dev/null)"
  [ -n "$ENF" ] && add "$ENF"
  add "(re-run without --enforce to confirm)"
fi

if [ "$JSON" -eq 1 ]; then
  printf '{"host":"%s","abusive":%s,"report":"%s"}\n' \
    "${HOST:-local}" "$([ $STATUS -eq 0 ] && echo false || echo true)" \
    "$(printf '%s' "$REPORT" | tr '\n' ';' | sed 's/"/\\"/g')"
else
  printf '=== scratch guard %s\n%s' "${HOST:+($HOST) }" "$REPORT"
  [ "$STATUS" -eq 0 ] && echo "clean - nothing abusive" || echo "⛔ abusive scratch found (see above)"
fi
exit "$STATUS"
