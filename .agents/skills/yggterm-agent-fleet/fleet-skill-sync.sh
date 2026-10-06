#!/usr/bin/env bash
# SessionStart: converge this host's hand-authored Claude skills with the fleet.
#
# Sibling of fleet-memory-sync.sh, same mesh and same algorithm: snapshot, pull
# from every peer, then push to every peer, newest mtime wins per file, nothing
# is ever auto-deleted on either side. Two passes means a skill that exists only
# on oc reaches dev via this host in one run.
#
# WHY: skills are shared fleet knowledge exactly like memory (data-fabric,
# an evidentiary-graph skill), and they were drifting silently — that skill
# lived on two hosts but never reached the third, so a session there ran the
# campaign without its own doctrine.
#
# WHAT IS NOT SYNCED: anything gstack owns. `gstack-upgrade` installs one dir
# per skill out of the ~/.claude/skills/gstack checkout, so a skill is
# gstack-owned iff a same-named dir exists inside that checkout — the exclusion
# list is DERIVED, never hardcoded, so new hand-written skills propagate on
# their own and new gstack skills stay out on their own. Two reasons to exclude:
#   1. gstack/ itself is ~767M with node_modules and a .git; rsyncing a git
#      checkout with newest-wins is both slow and destructive.
#   2. skill text is versioned with the checkout. Pushing a newer host's skill
#      text onto an older host's binaries yields a skill referencing flags that
#      host does not have — silent breakage. gstack-upgrade is that path.
# The exclusion list is the UNION of local and peer checkouts, so a host that
# has no gstack installed cannot accidentally inherit 57 orphaned skill dirs.
#
# CAVEAT (same as memory): newest-wins is per FILE. Editing one SKILL.md on two
# hosts inside the same window loses the older edit wholesale.
#
# This hook must never fail a session: it always exits 0.

set -uo pipefail

# ⛔ THE FLEET ROSTER IS CONFIGURATION, NOT A CONSTANT (the ygg-memory law):
# $YGG_FLEET_MESH (comma list) > ~/.yggterm/auth/.fleet-hosts > none (skip
# loudly). Never hardcode the fleet's private host names in a public repo.
MESH=()
if [ -n "${YGG_FLEET_MESH:-}" ]; then
  IFS=, read -r -a MESH <<< "$YGG_FLEET_MESH"
elif [ -f "$HOME/.yggterm/auth/.fleet-hosts" ]; then
  while IFS= read -r line; do
    line="${line%%#*}"; line="${line//,/ }"; line="$(echo $line)"
    [ -z "$line" ] && continue
    for entry in $line; do MESH+=("$entry"); done
  done < "$HOME/.yggterm/auth/.fleet-hosts"
fi
if [ "${#MESH[@]}" -eq 0 ]; then
  report() { printf '%s\n' "$*"; }
  report "fleet-skill-sync: no fleet roster (env or ~/.yggterm/auth/.fleet-hosts); skipped."
  exit 0
fi
SKILLS="$HOME/.claude/skills"
GSTACK="$SKILLS/gstack"
BACKUP_ROOT="$HOME/.claude/skill-backups"
KEEP_BACKUPS=10
LOCK="$HOME/.yggterm/fleet-skill-sync.lock"
SSH_OPTS="ssh -o BatchMode=yes -o ConnectTimeout=8 -o LogLevel=ERROR"
STAMP=$(date +%Y%m%d-%H%M%S)

report() { printf '%s\n' "$*"; }

# SELF is derived, not mapped: every roster entry is a peer and rsync's
# newest-wins is a natural no-op against this host's own alias, so an
# alias/hostname mismatch can never silently exclude the local host.
SELF="$(hostname)"
PEERS=("${MESH[@]}")

# Serialize concurrent session starts so two rsyncs never interleave.
if command -v flock >/dev/null 2>&1; then
  exec 9>"$LOCK"
  flock -w 30 9 || { report "fleet-skill-sync: another session holds the lock; skipped."; exit 0; }
fi

mkdir -p "$SKILLS"

LIVE=()
for peer in "${PEERS[@]}"; do
  if $SSH_OPTS "$peer" true 2>/dev/null; then LIVE+=("$peer"); else
    report "fleet-skill-sync: peer '$peer' unreachable — skipping it this run."
  fi
done

if [ ${#LIVE[@]} -eq 0 ]; then
  report "fleet-skill-sync: no peers reachable from $SELF — working from local skills only."
  exit 0
fi

# Derive the gstack-owned name list: union of every reachable checkout, so a
# host missing gstack still learns what to leave alone.
owned=$(
  { ls -1 "$GSTACK" 2>/dev/null
    for peer in "${LIVE[@]}"; do
      $SSH_OPTS "$peer" 'ls -1 ~/.claude/skills/gstack 2>/dev/null' 2>/dev/null
    done
  } | sed 's#/$##' | grep -v '^$' | sort -u
)

EXCL=(--exclude=/gstack/)
while IFS= read -r name; do
  [ -n "$name" ] || continue
  [ -d "$SKILLS/$name" ] && EXCL+=("--exclude=/$name/")
done <<<"$owned"

# A router skill can live outside the checkout yet symlink its SKILL.md back
# into it (_gstack-command does). That is gstack's too — same derived test, one
# level of indirection.
for dir in "$SKILLS"/*/ "$SKILLS"/.[!.]*/; do
  [ -d "$dir" ] || continue
  name=$(basename "$dir")
  [ "$name" = gstack ] && continue
  [ -L "$dir/SKILL.md" ] || continue
  target=$(readlink -f "$dir/SKILL.md" 2>/dev/null) || continue
  case "$target" in
    "$GSTACK"/*) EXCL+=("--exclude=/$name/"); owned="$owned"$'\n'"$name" ;;
  esac
done

# A skill dir that exists only on a peer is still gstack-owned if the checkout
# names it; exclude by name regardless of whether it exists here yet.
while IFS= read -r name; do
  [ -n "$name" ] || continue
  [ -d "$SKILLS/$name" ] || EXCL+=("--exclude=/$name/")
done <<<"$owned"

backup="$BACKUP_ROOT/$STAMP"
mkdir -p "$backup"
rsync -a "${EXCL[@]}" "$SKILLS/" "$backup/" 2>/dev/null

# Pass 1: collect the union from every peer.
pulled=0
for peer in "${LIVE[@]}"; do
  $SSH_OPTS "$peer" 'mkdir -p ~/.claude/skills' 2>/dev/null
  n=$(rsync -az -u --itemize-changes "${EXCL[@]}" -e "$SSH_OPTS" \
    "$peer:/home/user/.claude/skills/" "$SKILLS/" 2>/dev/null | grep -c '^[<>]')
  pulled=$((pulled + n))
done

# Pass 2: hand the union back out.
pushed=0
for peer in "${LIVE[@]}"; do
  n=$(rsync -az -u --itemize-changes "${EXCL[@]}" -e "$SSH_OPTS" \
    "$SKILLS/" "$peer:/home/user/.claude/skills/" 2>/dev/null | grep -c '^[<>]')
  pushed=$((pushed + n))
done

synced=$(find "$SKILLS" -maxdepth 2 -name SKILL.md 2>/dev/null | while read -r f; do
  d=$(basename "$(dirname "$f")")
  printf '%s\n' "$owned" | grep -qx "$d" || { [ "$d" = gstack ] || echo "$d"; }
done | wc -l)

report "fleet-skill-sync: ${pulled} in, ${pushed} out, ${synced} skills (peers: ${LIVE[*]})"
[ $((pulled + pushed)) -eq 0 ] && report "fleet-skill-sync: already converged."

if [ -d "$BACKUP_ROOT" ]; then
  ls -1d "$BACKUP_ROOT"/*/ 2>/dev/null | sort -r | tail -n +$((KEEP_BACKUPS + 1)) \
    | while read -r old; do rm -rf "$old"; done
fi

exit 0
