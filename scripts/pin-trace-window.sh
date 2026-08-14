#!/usr/bin/env bash
# Keep the event-trace around a deploy, because retention will not.
#
# ⛔ THIS EXISTS BECAUSE FOUR SESSION DEATHS WENT UNEXPLAINED FOR WANT OF
# EVIDENCE THAT HAD ALREADY BEEN DELETED. The trace's byte budget is per HOME
# while the write rate is per DAEMON, so on a host running ~20 daemons the
# retained window measured 2.98 h -- not the 3 days its own constant advertised.
# Every one of those deaths happened 166-464 s after a release and was looked at
# hours later, by which time the trace covering it was gone. The investigation
# could then only report an absence, and an absence that is STRUCTURAL is not
# evidence of anything.
#
#   scripts/pin-trace-window.sh --label 3.0.155 [--hosts "dev guihost oc"]
#                               [--follow-mins 15] [--dry-run]
#
# ⭐ IT LINKS, IT DOES NOT COPY. A hard link costs no bytes and no time, and --
# the point -- an unlink by the retention pruner then frees nothing, because the
# inode still has a name. The trace keeps rotating and pruning exactly as before;
# the window we asked for simply stops being reclaimable.
#
# ⚠ Rotation RENAMES the live file, so a link taken now follows that inode into
# its rotated life and keeps receiving the writes that other daemons' cached
# handles are still making to it. The follow loop exists for the other half: a
# generation created AFTER the pin has an inode nobody has linked yet, and the
# deaths land minutes after the swap, which is squarely in that half.
#
# The pinned window is never deleted automatically. That is deliberate: it is
# the only copy of a question somebody is going to ask, and a snapshot that
# prunes itself is the failure this script is named after. Remove one by hand
# when its release is settled: rm -rf ~/.yggterm/incident-snapshots/<label>
set -euo pipefail

LABEL=""
HOSTS=""
FOLLOW_MINS=15
DRY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --label) LABEL="$2"; shift 2;;
    --hosts) HOSTS="$2"; shift 2;;
    --follow-mins) FOLLOW_MINS="$2"; shift 2;;
    --dry-run) DRY=1; shift;;
    -h|--help) sed -n '2,32p' "$0"; exit 0;;
    *) echo "pin-trace-window: unknown argument $1" >&2; exit 2;;
  esac
done

if [ -z "$LABEL" ]; then
  echo "pin-trace-window: --label is required -- a pin nobody can name is a pin" >&2
  echo "   nobody will find. Use the release it is about, e.g. --label 3.0.155" >&2
  exit 2
fi
# ⛔ A label is a directory name. Refuse anything that could escape the snapshot
#    root rather than sanitising it quietly into something the caller did not ask
#    for and will not recognise later.
case "$LABEL" in
  */*|.*|"") echo "pin-trace-window: refusing label '$LABEL' -- no slashes, no leading dot" >&2; exit 2;;
esac

if [ -z "$HOSTS" ]; then
  HOSTS="dev $("$(dirname "$0")/ygg-live-host.sh" 2>/dev/null || true) oc"
  HOSTS="$(printf '%s\n' $HOSTS | awk 'NF && !seen[$0]++' | tr '\n' ' ')"
fi

# The whole per-host job, run locally or through ssh. Deliberately one string:
# the remote copy of this repo may be older, or absent, so nothing here may
# depend on the remote having this script.
remote_job() {
  cat <<'REMOTE'
set -u
YH="${YGGTERM_HOME:-$HOME/.yggterm}"
DEST="$YH/incident-snapshots/$LABEL"
mkdir -p "$DEST" || exit 1
linked_inodes() { [ -f "$DEST/.inodes" ] && cat "$DEST/.inodes" || true; }
pass() {
  local n=0
  for f in "$YH"/event-trace.jsonl "$YH"/event-trace.g*.jsonl; do
    [ -f "$f" ] || continue
    ino=$(stat -c %i "$f" 2>/dev/null) || continue
    case " $(linked_inodes) " in *" $ino "*) continue;; esac
    # Name by inode as well as basename: two generations can share a name
    # across a rotation, and the inode is what actually identifies the bytes.
    if ln "$f" "$DEST/$(basename "$f").i$ino" 2>/dev/null; then
      echo "$ino" >> "$DEST/.inodes"; n=$((n+1))
    fi
  done
  echo "$n"
}
first=$(pass)
{
  echo "label:        $LABEL"
  echo "host:         $(hostname -s 2>/dev/null)"
  echo "pinned_at:    $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "follow_mins:  $FOLLOW_MINS"
  echo "first_pass:   $first file(s)"
  echo "daemon_count: $(ls "$YH"/*.sock 2>/dev/null | wc -l) socket(s)"
} > "$DEST/manifest.txt"
# ⚠ Detached, so a deploy is never held up by a diagnostic. Its own log is the
#   read-back: a pin that reported success and linked nothing is the shape this
#   whole file exists to distrust -- and it caught exactly that on the first
#   run. `DEST` is computed here but was not EXPORTED, so the detached shell
#   built its paths from an empty string, wrote at `/`, and failed silently into
#   a log nobody had read yet. The first pass looked perfect; the half that
#   matters did nothing.
export DEST YH FOLLOW_SECS
nohup setsid sh -c '
  end=$(( $(date +%s) + FOLLOW_SECS ))
  while [ "$(date +%s)" -lt "$end" ]; do
    sleep 30
    for f in "$YH"/event-trace.jsonl "$YH"/event-trace.g*.jsonl; do
      [ -f "$f" ] || continue
      ino=$(stat -c %i "$f" 2>/dev/null) || continue
      grep -qx "$ino" "$DEST/.inodes" 2>/dev/null && continue
      ln "$f" "$DEST/$(basename "$f").i$ino" 2>/dev/null && echo "$ino" >> "$DEST/.inodes"
    done
  done
  echo "follow finished $(date -u +%H:%M:%SZ): $(ls "$DEST" | grep -c jsonl) file(s) pinned" >> "$DEST/manifest.txt"
' >> "$DEST/follow.log" 2>&1 &
echo "pinned $first file(s) now, following for $FOLLOW_MINS min -> $DEST"
REMOTE
}

FAILED=0
for host in $HOSTS; do
  script="LABEL='$LABEL'; FOLLOW_MINS='$FOLLOW_MINS'; FOLLOW_SECS=$((FOLLOW_MINS * 60)); YH=\"\${YGGTERM_HOME:-\$HOME/.yggterm}\"; export LABEL FOLLOW_MINS FOLLOW_SECS YH; $(remote_job)"
  if [ "$DRY" = 1 ]; then
    echo "pin-trace-window: DRY would pin on $host (label=$LABEL, follow=${FOLLOW_MINS}m)"
    continue
  fi
  if [ "$host" = "$(hostname -s 2>/dev/null)" ]; then
    out=$(printf '%s' "$script" | bash 2>&1) || FAILED=1
  else
    # ⛔ Feed the script on stdin rather than as an argument: an argument goes
    #    through two shells' quoting and this one contains both kinds of quote.
    out=$(ssh -o BatchMode=yes -o ConnectTimeout=8 "$host" bash -s <<<"$script" 2>&1) || FAILED=1
  fi
  echo "pin-trace-window: $host: ${out:-⛔ no output}"
done
exit $FAILED
