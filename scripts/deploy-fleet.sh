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
#                           [--allow-behind] [--preflight]
#
# ⭐ `--preflight` runs ONLY the ancestry check and exits, without needing build
# products. Run it BEFORE the release build. The ancestry gate is correct and is
# not weakened, but it used to be asked only after the caller had already paid
# 2-3 minutes for a build — so on a busy evening main advances mid-build and the
# deploy refuses on a race that did not exist when the build began. One lane lost
# it SIX TIMES IN A ROW, about fifteen minutes of pure rebuild, and succeeded on
# the seventh only because main happened to go quiet. ⚠ The failure rate scales
# with how busy the fleet is, so it is worst exactly when the most lanes are
# shipping, and a single-lane evening never sees it.
#
#   scripts/deploy-fleet.sh --preflight && cargo build --release && scripts/deploy-fleet.sh
#
# `--from` defaults to target/release. The tree it was built from must be a
# DESCENDANT OF `origin/main` — see the refusal below — and a dirty checkout is
# named in a warning, because a shared checkout routinely holds another
# session's in-flight work.
set -uo pipefail

FROM="target/release"
# ⛔⛔ THE GUI HOST IS RESOLVED, NEVER SPELLED. A literal placeholder here does not
# resolve, so the DEFAULT invocation deployed to the two headless hosts, failed
# four copies with an unresolvable name, exited non-zero — and silently skipped
# the one host the GUI actually runs on, which is the only host a UI change can
# be proven on. Measured 2026-08-13; the workaround in use was to pass --hosts by
# hand, which is exactly the hand-assembly this script exists to end.
# ⇒ `scripts/ygg-live-host.sh` is the repo's single owner of "where is the live
#   GUI". Ask it. If it cannot answer, say so loudly rather than deploying to a
#   short list that looks complete.
HOSTS="dev $("$(dirname "$0")/ygg-live-host.sh" 2>/dev/null || true) oc"
HOSTS="$(printf '%s\n' $HOSTS | awk 'NF && !seen[$0]++' | tr '\n' ' ')"
if [ "$(printf '%s\n' $HOSTS | awk 'NF' | wc -l)" -lt 3 ]; then
  echo "deploy-fleet: ⛔ could not resolve the live GUI host — ygg-live-host.sh gave nothing." >&2
  echo "  Deploying to '$HOSTS' would SKIP the only host a UI change can be proven on." >&2
  echo "  Pass --hosts explicitly if that is really what you want." >&2
  exit 2
fi
DRY=0
ALLOW_BEHIND=0
PREFLIGHT=0
while [ $# -gt 0 ]; do
  case "$1" in
    --from) FROM="$2"; shift 2;;
    --hosts) HOSTS="$2"; shift 2;;
    --dry-run) DRY=1; shift;;
    --allow-behind) ALLOW_BEHIND=1; shift;;
    --preflight) PREFLIGHT=1; shift;;
    *) echo "unknown argument: $1" >&2; exit 2;;
  esac
done

# ⛔⛔ A DEPLOY FROM A PRE-REBASE TREE SILENTLY REVERTS ANOTHER CLUSTER'S FIX.
# Measured 2026-08-13: `3.0.117`–`3.0.120` were each allocated TWICE within
# minutes by clusters working in parallel, because nothing arbitrates a version
# number — read `Cargo.toml`, add one, push. A cluster that built before
# rebasing then deploys a binary lacking the other's commit; this script does
# its job perfectly and prints "every copy on every host reads back at 3.0.118"
# while two different builds wear that string. The GUI hot-restarts onto the
# older one by re-exec, so the pid is unchanged and `/proc/<pid>/exe` still
# reads clean, and the first cluster's live probe comes back RED against a
# binary that never carried its fix. Reading that as "my root cause was wrong"
# is the single most expensive wrong conclusion available.
#
# ⇒ The version cannot detect this and never could. Ancestry can, before the
# damage: if `origin/main` is not an ancestor of HEAD, this build is missing
# commits that are already shared, and shipping it un-ships them.
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD_COMMIT="unstamped"
if git -C "$REPO" rev-parse --git-dir >/dev/null 2>&1; then
  BUILD_COMMIT=$(git -C "$REPO" rev-parse --short=12 HEAD)
  DIRT=$(git -C "$REPO" status --porcelain | head -5)
  if [ -n "$DIRT" ]; then
    echo "⚠ the source checkout is dirty — the build may carry work that is not in any commit:"
    echo "$DIRT" | sed 's/^/    /'
    git -C "$REPO" status --porcelain | tail -n +6 | head -1 | grep -q . && echo "    … and more"
  fi
  if git -C "$REPO" fetch --quiet origin main 2>/dev/null; then
    UPSTREAM=$(git -C "$REPO" rev-parse FETCH_HEAD)
  else
    UPSTREAM=$(git -C "$REPO" rev-parse origin/main 2>/dev/null || echo "")
    [ -n "$UPSTREAM" ] && echo "⚠ could not reach origin — ancestry is checked against the LAST FETCH, which may be stale"
  fi
  if [ -n "$UPSTREAM" ] && ! git -C "$REPO" merge-base --is-ancestor "$UPSTREAM" HEAD; then
    echo "⛔ REFUSING: HEAD ($BUILD_COMMIT) is not a descendant of origin/main." >&2
    echo "   This build LACKS commits that are already on main, and deploying it" >&2
    echo "   reverts them on every host — under a version number that will look" >&2
    echo "   correct in the census and in --version. Missing:" >&2
    git -C "$REPO" log --oneline "HEAD..$UPSTREAM" | head -20 | sed 's/^/     /' >&2
    echo "   Fix: git pull --rebase origin main && rebuild && re-run this." >&2
    echo "   ⭐ To not pay for a doomed build again: run --preflight BEFORE building." >&2
    echo "   (--allow-behind exists for bisecting an old build onto a host, and" >&2
    echo "    for nothing else; the census names the commit either way.)" >&2
    [ "$ALLOW_BEHIND" = 1 ] || exit 1
    echo "⚠ --allow-behind: proceeding with a build that is behind origin/main" >&2
  fi
fi

# ⭐ --preflight answers the ancestry question and stops, so a caller can ask it
#    in one second instead of after a three-minute build. Deliberately BEFORE the
#    build-product check: at preflight time those products do not exist yet, and
#    requiring them is what forced the question to be asked too late.
if [ "$PREFLIGHT" = 1 ]; then
  echo "deploy-fleet: preflight ok — HEAD ($BUILD_COMMIT) is a descendant of origin/main; safe to build."
  exit 0
fi

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
echo "deploy-fleet: $VERSION ($BUILD_COMMIT) from $FROM → $HOSTS"

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
# ⛔⛔ "IS THIS HOST ME?" IS NOT A STRING COMPARISON. The test used to be
# `[ "$host" = "$(hostname -s)" ]`, and on this fleet the ssh alias and the
# kernel hostname differ — so a deploy run ON a host tried to ssh to itself,
# failed all four copies with `Could not resolve hostname`, and printed ⛔ for
# the very machine doing the deploying while the other two landed. The operator
# reads three-quarters success and moves on, which is exactly the split this
# script exists to prevent.
#
# ⛔⛔ AND `/etc/machine-id` IS THE WRONG IDENTITY — it nearly shipped here.
# It answers *"is this the same machine image?"*, and this deploy's real
# question is *"do these two paths name the same FILE?"*. Measured 2026-08-13
# on this fleet: two hosts report a byte-identical `/etc/machine-id` (cloned
# from one image) and have **different filesystems**. Using it would have
# written the second host's four copies into the first host's disk, read them
# back through the same wrong door, and printed four ✅ for a host that was
# never touched — a total deploy failure wearing a green census, which is
# strictly worse than the ⛔ storm being fixed.
#
# ⇒ Ask the question the writes actually depend on: drop a unique token in
# `$HOME` — the filesystem the four copies land in — and see whether the
# candidate channel can see it. That is self-verifying, it cannot be fooled by
# a shared image, and when it says "self" the four copies skip ssh entirely, so
# the probe pays for itself.
#
# ⚠ And when the alias cannot be reached AT ALL, that is reported ONCE, by name,
# with the remedy — never as four identical failures that look like a partial
# deploy. An unreachable name may well BE this machine (that is the reported
# case), and no local signal can tell: the alias is the fleet's, `hostname -s`
# is the kernel's, and neither knows about the other. `$YGG_FLEET_SELF` is how
# an operator settles it permanently, in one word, without this public repo ever
# naming a machine.
SELF_TOKEN=$(mktemp "$HOME/.ygg-deploy-self-XXXXXX" 2>/dev/null || true)
[ -n "$SELF_TOKEN" ] && trap 'rm -f "$SELF_TOKEN"' EXIT
declare -A HOST_IS_SELF=()
declare -A HOST_UNREACHABLE=()

classify_host() {  # host
  local host="$1" probe
  if [ "$host" = "local" ] || [ "$host" = "$(hostname -s)" ] ||
     { [ -n "${YGG_FLEET_SELF:-}" ] && [ "$host" = "$YGG_FLEET_SELF" ]; }; then
    HOST_IS_SELF["$host"]=1
    return 0
  fi
  [ -n "$SELF_TOKEN" ] || return 0  # no token: treat every named host as remote
  probe=$(ssh -o BatchMode=yes -o ConnectTimeout=8 "$host" \
    "test -e '$SELF_TOKEN' && echo SELF || echo REMOTE" 2>/dev/null < /dev/null)
  case "$probe" in
    SELF) HOST_IS_SELF["$host"]=1;;
    REMOTE) ;;
    # No answer at all: the alias did not resolve, the host is down, or ssh was
    # refused. ⛔ Never guess "remote" here — that is the path that produced the
    # four-⛔ storm; and never guess "self" either, which would write this
    # machine's copies twice and call the fleet current.
    *) HOST_UNREACHABLE["$host"]=1;;
  esac
}

is_self() { [ "${HOST_IS_SELF[$1]:-0}" = 1 ]; }

run_on() {  # host, command…  (stdin is NOT forwarded)
  local host="$1"; shift
  if is_self "$host"; then bash -c "$*" < /dev/null
  else ssh "$host" "$*" < /dev/null; fi
}

push_one() {  # host, local_file, remote_path, expected_md5
  local host="$1" src="$2" dest="$3" want="$4" got
  local write="d=\$(eval echo $dest); mkdir -p \$(dirname \$d); cat > \$d.new && chmod 755 \$d.new && mv -f \$d.new \$d"
  if is_self "$host"; then bash -c "$write" < "$src"
  else ssh "$host" "$write" < "$src"; fi
  got=$(run_on "$host" "md5sum \$(eval echo $dest)")
  got=${got%% *}
  if [ "$got" = "$want" ]; then printf "  ✅ %-14s %s\n" "$host" "$dest"
  else printf "  ⛔ %-14s %s  READ BACK %s WANTED %s\n" "$host" "$dest" "${got:-<none>}" "$want"; return 1; fi
}

FAILED=0
for host in $HOSTS; do classify_host "$host"; done

# ⛔ SAY IT ONCE, AND SAY WHAT IT MIGHT MEAN. Four identical copy failures read
# as a partial deploy; one named refusal reads as what it is.
for host in $HOSTS; do
  [ "${HOST_UNREACHABLE[$host]:-0}" = 1 ] || continue
  echo "  ⛔ $host: cannot be reached over ssh, so its four copies are SKIPPED, not failed." >&2
  echo "     If this is the machine you are standing on, its fleet alias and its" >&2
  echo "     kernel hostname ($(hostname -s)) differ and nothing local can bridge them." >&2
  echo "     Fix it for good with:  export YGG_FLEET_SELF=$host" >&2
  echo "     Or for this run:       --hosts local" >&2
  FAILED=1
done

for host in $HOSTS; do
  [ "${HOST_UNREACHABLE[$host]:-0}" = 1 ] && continue
  for dest in "${!COPY[@]}"; do
    if [ "${COPY[$dest]}" = "GUI" ]; then src="$GUI"; want="$GUI_SUM"; else src="$HL"; want="$HL_SUM"; fi
    if [ "$DRY" = 1 ]; then
      printf "  · %-14s %s ← %s%s\n" "$host" "$dest" "$(basename "$src")" \
        "$(is_self "$host" && echo "  (this machine — no ssh)")"
      continue
    fi
    push_one "$host" "$src" "$dest" "$want" || FAILED=1
  done
done

[ "$DRY" = 1 ] && exit 0

# ⛔ VERIFY BY READ-BACK, NOT BY THE COPY REPORTING SUCCESS. Every row verb on
# this project reports the REQUEST rather than the EFFECT; a deploy that trusts
# its own `mv` is the same mistake with worse blast radius.
# ⚠ THE VERSION COLUMN CANNOT IDENTIFY A BUILD — the md5 column is what does.
# Two clusters spend the same version number routinely, so a copy whose md5 is
# not one of the two below is a DIFFERENT build wearing the same string, and
# that is the whole defect this verb was taught to see.
# ⛔⛔ A HOST HAS TWO PLANES AND THIS DEPLOY ONLY TOUCHES ONE OF THEM. The four
# copies above are files; the daemons are PROCESSES that keep executing the code
# they were loaded with until each one retires on its own terms — which is the
# constitution working, not a fault. Reporting only the disk therefore prints a
# host as fully current while every daemon on it runs something else: measured on
# the desktop host 2026-08-13, where the GUI was ten versions ahead of the daemon
# serving it, and the running GUI's own build could not be named at all because a
# deploy had already replaced the file its `/proc/<pid>/exe` pointed at.
#
# ⇒ Census BOTH planes, side by side, and let the running one name its commit
# rather than inferring it: a running process is the only party that still knows.
# ⚠ Asking a running daemon is also the one interrogation that is SAFE to make
# fleet-wide — `--build-commit` on a binary built before that flag existed falls
# through to LAUNCHING THE GUI, so a census that ran it on every copy would open
# windows on exactly the hosts that are behind.
echo "deploy-fleet: census — this build is $VERSION from commit $BUILD_COMMIT"
echo "              gui=${GUI_SUM:0:10}  headless=${HL_SUM:0:10}  (any other md5 is another build)"
for host in $HOSTS; do
  if [ "${HOST_UNREACHABLE[$host]:-0}" = 1 ]; then
    echo "  == $host ==  (unreachable — nothing was written here, and nothing is claimed about it)"
    continue
  fi
  echo "  == $host =="
  # ⛔ ASK `--version` FIRST AND LET IT DECIDE WHETHER THE OTHER QUESTION IS SAFE.
  # `--build-commit` on a binary built before the flag existed is an UNKNOWN
  # ARGUMENT, and an unknown argument to the GUI binary falls through to
  # LAUNCHING THE GUI — so a census that asked every copy would open windows
  # across the fleet on precisely the hosts that are behind, which are the ones
  # it exists to find. `--version` is safe on every build ever shipped, so it is
  # the gate: below the floor the copy is named `(pre-flag)` and never asked.
  # The floor is the version whose deployed copy was OBSERVED answering the flag,
  # not the version whose source first contained it — those differ, and only the
  # first one is evidence.
  cen='MINV=3.0.125
       for p in $HOME/.local/bin/yggterm $HOME/.local/bin/yggterm-headless \
                $HOME/.yggterm/bin/yggterm $HOME/.yggterm/bin/yggterm-headless; do
         v=$("$p" --version 2>/dev/null || echo ERR)
         c="(pre-flag)"
         case "$v" in
           ERR|"") c="(no answer)";;
           *) [ "$(printf "%s\n%s\n" "$MINV" "$v" | sort -V | head -1)" = "$MINV" ] &&
                c=$("$p" --build-commit 2>/dev/null || echo "(refused)");;
         esac
         printf "    on disk  %-42s %-9s %-14s %s\n" "$p" "$v" "$c" "$(md5sum "$p" 2>/dev/null | cut -c1-10)"
       done
       $HOME/.yggterm/bin/yggterm-headless server daemons 2>/dev/null |
         sed "s/^/    running  /" || echo "    running  <no census: this host has no reachable daemon>"'
  if is_self "$host"; then bash -c "$cen"; else ssh "$host" bash -c "'$cen'"; fi
done

if [ "$FAILED" != 0 ]; then echo "⛔ deploy-fleet: at least one copy did not read back"; exit 1; fi
echo "deploy-fleet: every copy on every host reads back at $VERSION ($BUILD_COMMIT)"
echo "  Ask a FILE which source it is:    yggterm --build-commit"
echo "  Ask the RUNNING processes:        yggterm-headless server daemons   (BUILD column)"
echo "⚠ The daemons do NOT swap here. Each retires onto the new binary on its own"
echo "  poll once its own sessions allow it — that is the constitution, not a bug."
echo "  ⇒ The running plane is EXPECTED to trail the disk for a while. What must"
echo "    never happen is being unable to say by how much, which is why the BUILD"
echo "    column names a commit instead of leaving a version to stand for two."
