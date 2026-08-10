#!/usr/bin/env bash
# Put ONE build on every copy, on every host, and prove it landed.
#
# ⛔ THIS EXISTS BECAUSE "DEPLOY" WAS A CHORE EVERY SESSION HAND-ASSEMBLED FROM
# `scp` + `mv`, AND EACH ONE PICKED A DIFFERENT SUBSET OF THE PATHS.
# The result is the recurring split the session-start audit has been reporting
# for days, and on 2026-08-10 it reached the owner as a symptom: "client on 100th
# and daemon on 101" — a session had shipped a daemon-side fix and deployed only
# the daemon binary, because nothing said the GUI was part of the same deploy.
# An agent's discipline resets every session; a verb's does not.
#
#   scripts/deploy-fleet.sh [--from <dir>] [--hosts "dev guihost oc"] [--dry-run]
#
# `--from` defaults to target/release. Build it from a CLEAN tree: this script
# refuses a dirty source checkout, because a shared checkout routinely holds
# another session's in-flight work and a deploy must never ship it.
set -uo pipefail

FROM="target/release"
HOSTS="dev guihost oc"
DRY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --from) FROM="$2"; shift 2;;
    --hosts) HOSTS="$2"; shift 2;;
    --dry-run) DRY=1; shift;;
    *) echo "unknown argument: $1" >&2; exit 2;;
  esac
done

GUI="$FROM/yggterm"
HL="$FROM/yggterm-headless"
for f in "$GUI" "$HL"; do
  [ -x "$f" ] || { echo "⛔ missing build product: $f" >&2; exit 1; }
done

# ⛔ ONE BUILD, NOT TWO. Two binaries at different versions is the split this
# script exists to end, and shipping them together is the only way the fleet can
# ever be checked with a single number.
GUI_V=$("$GUI" --version 2>/dev/null)
HL_V=$("$HL" --version 2>/dev/null)
[ "$GUI_V" = "$HL_V" ] || {
  echo "⛔ REFUSING: the two binaries disagree — yggterm=$GUI_V yggterm-headless=$HL_V" >&2
  echo "   Build both from the same tree in one command." >&2
  exit 1; }
VERSION="$GUI_V"
echo "deploy-fleet: $VERSION from $FROM → $HOSTS"

GUI_SUM=$(md5sum "$GUI" | awk '{print $1}')
HL_SUM=$(md5sum "$HL" | awk '{print $1}')

# The four copies, and which build product belongs in each. `~/.local/bin` is the
# GUI's home; `~/.yggterm/bin` is the install root the daemon actually runs from
# AND the path remote sessions invoke. All four are real, so all four are written.
declare -A COPY=(
  ["\$HOME/.local/bin/yggterm"]="GUI"
  ["\$HOME/.local/bin/yggterm-headless"]="HL"
  ["\$HOME/.yggterm/bin/yggterm"]="GUI"
  ["\$HOME/.yggterm/bin/yggterm-headless"]="HL"
)

# ⛔ NEVER `rm -f *.old.*`. A past deploy renamed a live binary while a daemon
# held it, so that daemon's /proc/<pid>/exe followed the rename onto the backup
# path; deleting the backup makes its exe link read "(deleted)", fires the retire,
# and cold-kills its PTYs. In-place `mv` over the canonical path writes no
# `.old.*` at all, so it disarms nothing and arms nothing.
#
# ⚠ The WRITE and the READ-BACK are two separate calls on purpose. Folding the
# checksum into the same command as the `cat` means quoting a `cut -d' '` or an
# `awk '{print $1}'` through two shells, and both get mangled — which on the
# first run of this script made it print ⛔ for twelve copies that had all landed
# correctly. A deploy verb that cries failure on success is worse than no verb.
run_on() {  # host, command…  (stdin is NOT forwarded)
  local host="$1"; shift
  if [ "$host" = "$(hostname -s)" ] || [ "$host" = "local" ]; then bash -c "$*" < /dev/null
  else ssh "$host" "$*" < /dev/null; fi
}

push_one() {  # host, local_file, remote_path, expected_md5
  local host="$1" src="$2" dest="$3" want="$4" got
  local write="d=\$(eval echo $dest); mkdir -p \$(dirname \$d); cat > \$d.new && chmod 755 \$d.new && mv -f \$d.new \$d"
  if [ "$host" = "$(hostname -s)" ] || [ "$host" = "local" ]; then bash -c "$write" < "$src"
  else ssh "$host" "$write" < "$src"; fi
  got=$(run_on "$host" "md5sum \$(eval echo $dest)")
  got=${got%% *}
  if [ "$got" = "$want" ]; then printf "  ✅ %-14s %s\n" "$host" "$dest"
  else printf "  ⛔ %-14s %s  READ BACK %s WANTED %s\n" "$host" "$dest" "${got:-<none>}" "$want"; return 1; fi
}

FAILED=0
for host in $HOSTS; do
  for dest in "${!COPY[@]}"; do
    if [ "${COPY[$dest]}" = "GUI" ]; then src="$GUI"; want="$GUI_SUM"; else src="$HL"; want="$HL_SUM"; fi
    if [ "$DRY" = 1 ]; then printf "  · %-14s %s ← %s\n" "$host" "$dest" "$(basename "$src")"; continue; fi
    push_one "$host" "$src" "$dest" "$want" || FAILED=1
  done
done

[ "$DRY" = 1 ] && exit 0

# ⛔ VERIFY BY READ-BACK, NOT BY THE COPY REPORTING SUCCESS. Every row verb on
# this project reports the REQUEST rather than the EFFECT; a deploy that trusts
# its own `mv` is the same mistake with worse blast radius.
echo "deploy-fleet: census"
for host in $HOSTS; do
  echo "  == $host =="
  cen='for p in $HOME/.local/bin/yggterm $HOME/.local/bin/yggterm-headless \
                $HOME/.yggterm/bin/yggterm $HOME/.yggterm/bin/yggterm-headless; do
         printf "    %-42s %-9s %s\n" "$p" "$($p --version 2>/dev/null || echo ERR)" "$(md5sum "$p" 2>/dev/null | cut -c1-10)"
       done'
  if [ "$host" = "$(hostname -s)" ] || [ "$host" = "local" ]; then bash -c "$cen"; else ssh "$host" bash -c "'$cen'"; fi
done

if [ "$FAILED" != 0 ]; then echo "⛔ deploy-fleet: at least one copy did not read back"; exit 1; fi
echo "deploy-fleet: every copy on every host reads back at $VERSION"
echo "⚠ The daemons do NOT swap here. Each retires onto the new binary on its own"
echo "  poll once its own sessions allow it — that is the constitution, not a bug."
