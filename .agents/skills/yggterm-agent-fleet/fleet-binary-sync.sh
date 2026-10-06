#!/usr/bin/env bash
# SessionStart: converge the libyggterm app binaries across the fleet
# (user directive 2026-07-23: "all the libyggterm binaries should be in our
# working fleet … synced up should be the default case").
#
# Sibling of fleet-memory-sync.sh — same mesh, same doctrine: every host runs
# this same script and self-excludes; pull from every peer then push to every
# peer; newest mtime wins per binary; nothing is ever deleted. rsync installs
# via temp-file + rename, so replacing a RUNNING binary is safe (no ETXTBSY,
# the running process keeps its inode).
#
# Roster: DISCOVERED, not hand-listed. A hardcoded roster silently strands every
# app nobody remembered to add — it was `(yedit ychrome)` until 2026-08-02, so
# one tool existed on two hosts but never reached the third, and another
# existed on exactly one host. The roster is now the union across the mesh of ~/.local/bin
# entries matching the libyggterm naming convention (y*), minus DENY.
#
# yggterm/yggterm-headless are NOT synced — their deploys are versioned and
# daemon-aware and must stay deliberate (field guide §4), never a background
# mtime race. Backup/rollback siblings are never synced either.
#
# This hook must never fail a session: it always exits 0.

set -uo pipefail

# ⛔ THE FLEET ROSTER IS CONFIGURATION, NOT A CONSTANT (the ygg-memory law):
# resolution order — $YGG_FLEET_MESH (comma list) > ~/.yggterm/auth/.fleet-hosts
# (one alias or comma list per line) > none (skip loudly). A hardcoded roster
# in a PUBLIC repo would carry the fleet's private host names, and a second
# copy would drift from the one resolver that already owns them.
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
  report "fleet-binary-sync: no fleet roster (env or ~/.yggterm/auth/.fleet-hosts); skipped."
  exit 0
fi

# Never synced, even though they match y*.
DENY=(yggterm yggterm-headless yggterm-mock-cli)

# A name is a roster candidate if it (a) is DECLARED by a launcher manifest in
# ~/.yggterm/apps/*.json — an app that wrote its manifest has
# declared itself, whatever it is spelled — or (b) starts with y (the naming
# convention, kept as a fallback so convention-only tools are not stranded).
# Either way it must be a regular file with no backup/rollback/staging suffix.
roster_here() {
  local f b m
  {
    for f in "$HOME/.local/bin"/y*; do printf '%s\n' "$f"; done
    for m in "$HOME/.yggterm/apps"/*.json; do
      [ -f "$m" ] || continue
      b=$(basename "$m" .json)
      printf '%s\n' "$HOME/.local/bin/$b"
    done
  } | sort -u | while read -r f; do
    [ -f "$f" ] || continue
    b=$(basename "$f")
    case "$b" in
      # ⛔ trailing-anything: `yggterm.old.522814` and `.new2` slipped past the
      # anchored forms and ~100 MB of dead rollback binary replicated fleet-wide.
      *.bak*|*.prev*|*.rollback*|*.new*|*.old*|*.orig*|*~) continue ;;
    esac
    local skip=0 d
    for d in "${DENY[@]}"; do [ "$b" = "$d" ] && skip=1; done
    [ "$skip" = 1 ] && continue
    printf '%s\n' "$b"
  done
}
BIN="$HOME/.local/bin"
APP_DIR="$HOME/.yggterm/apps"
LOCK="$HOME/.yggterm/fleet-binary-sync.lock"
SSH_OPTS=(-o BatchMode=yes -o ConnectTimeout=8 -o LogLevel=ERROR)

report() { printf '%s\n' "$*"; }

# SELF is derived, not mapped: every roster entry is treated as a peer and
# rsync -au is a natural no-op against this host's own alias (same mtimes),
# so an alias/hostname mismatch (oc's hostname is not its alias) can never
# silently exclude the local host from convergence.
SELF="$(hostname)"
PEERS=("${MESH[@]}")

if command -v flock >/dev/null 2>&1; then
  exec 9>"$LOCK"
  flock -w 30 9 || { report "fleet-binary-sync: another session holds the lock; skipped."; exit 0; }
fi

mkdir -p "$BIN" "$APP_DIR"

LIVE=()
for peer in "${PEERS[@]}"; do
  ssh "${SSH_OPTS[@]}" "$peer" true 2>/dev/null && LIVE+=("$peer") \
    || report "fleet-binary-sync: peer '$peer' unreachable — skipping it this run."
done

# The roster is the UNION over this host and every live peer, so an app that
# exists on exactly one machine still reaches the other two.
ROSTER=()
seen=" "
add() { case "$seen" in *" $1 "*) ;; *) ROSTER+=("$1"); seen="$seen$1 " ;; esac; }
while read -r b; do [ -n "$b" ] && add "$b"; done < <(roster_here)
for peer in "${LIVE[@]}"; do
  while read -r b; do [ -n "$b" ] && add "$b"; done < <(
    ssh "${SSH_OPTS[@]}" "$peer" "$(declare -f roster_here); DENY=(${DENY[*]}); roster_here" 2>/dev/null)
done
report "fleet-binary-sync: roster (${#ROSTER[@]}): ${ROSTER[*]}"

pulled=0
pushed=0
for peer in "${LIVE[@]}"; do
  for bin in "${ROSTER[@]}"; do
    # Pull (peer newer or local absent), then push (local newer or peer absent).
    # -u skips when the receiver is newer, which is exactly newest-wins.
    out=$(rsync -au -e "ssh ${SSH_OPTS[*]}" --out-format='%n' \
      "$peer:$BIN/$bin" "$BIN/$bin" 2>/dev/null)
    [ -n "$out" ] && { pulled=$((pulled+1)); report "fleet-binary-sync: pulled $bin from $peer"; }
    out=$(rsync -au -e "ssh ${SSH_OPTS[*]}" --out-format='%n' \
      "$BIN/$bin" "$peer:$BIN/$bin" 2>/dev/null)
    [ -n "$out" ] && { pushed=$((pushed+1)); report "fleet-binary-sync: pushed $bin to $peer"; }
  done
done

report "fleet-binary-sync: $SELF — ${#ROSTER[@]} binaries, $pulled pulled, $pushed pushed."

# A launcher manifest is part of an app installation: the binary alone is not
# reachable from yggterm's titlebar, start page, or cwd-tree context menu.  Sync
# manifests with the same newest-wins rule as their binaries.  Convention-only
# y* tools have no manifest and are intentionally left alone.
manifest_pulled=0
manifest_pushed=0
for peer in "${LIVE[@]}"; do
  for bin in "${ROSTER[@]}"; do
    out=$(rsync -au -e "ssh ${SSH_OPTS[*]}" --out-format='%n' \
      "$peer:$APP_DIR/$bin.json" "$APP_DIR/$bin.json" 2>/dev/null)
    [ -n "$out" ] && { manifest_pulled=$((manifest_pulled+1)); report "fleet-binary-sync: pulled $bin manifest from $peer"; }
    out=$(rsync -au -e "ssh ${SSH_OPTS[*]}" --out-format='%n' \
      "$APP_DIR/$bin.json" "$peer:$APP_DIR/$bin.json" 2>/dev/null)
    [ -n "$out" ] && { manifest_pushed=$((manifest_pushed+1)); report "fleet-binary-sync: pushed $bin manifest to $peer"; }
  done
done
report "fleet-binary-sync: $SELF — $manifest_pulled manifests pulled, $manifest_pushed manifests pushed."

# ── The privacy guard's ANSWER KEY, which the roster above cannot carry ────────
# ⛔ A TOOL SYNCED WITHOUT ITS ANSWER KEY IS A TOOL THAT AGREES ON NOTHING.
# ygg-privacy-guard rides the roster above, but its wordlist deliberately lives
# OUTSIDE ~/.local/bin and outside every repo (a guard whose wordlist is committed
# to the repo it guards has published the answer key). So nothing carried it, and
# it drifted for three days: one host held the full list, the others a strict
# subset. Names were unguarded on most of the fleet while every scan there printed
# a green tick — a guard reports CLEAN for a term it does not HAVE for exactly the
# same reason it reports CLEAN for a term that is genuinely absent.
#
# The merge is the guard's own `sync-terms`, deliberately NOT reimplemented here:
# it is a UNION, not the newest-mtime-wins doctrine used for binaries above.
# Newest-wins is right for a binary and catastrophic for a wordlist — a host whose
# file is merely newer would delete terms everywhere, dressed as a successful sync.
#
# Runs LAST so it uses whichever guard the sync above just converged on. It is
# silent when every host already agrees, and cannot fail the session.
if [ -x "$BIN/ygg-privacy-guard" ]; then
  "$BIN/ygg-privacy-guard" sync-terms --quiet 2>&1 | sed 's/^/fleet-binary-sync: /' || true
fi

exit 0
