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
#                           [--allow-behind] [--preflight] [--no-pin]
#                           [--allow-downgrade] [--no-direct-restart]
#
# ⛔⛔ A DEPLOY MAY NEVER TAKE A HOST BACKWARDS — 2026-08-27, MEASURED TWICE IN
# ONE HOUR. The hourly roll put 3.1.61 fleet-wide and restarted the GUI host;
# minutes later a lane deploying from a STALE checkout wrote 3.1.60 back over
# all four flat paths (mtimes 01:47:59 and 02:01:40), clobbered the fresh GUI's
# binary under three running windows, and left every host one version behind —
# silently, with four green ✅ rows, because nothing in this script asked
# "older than what is already there?". The roll's own commit compare then saw
# the live daemon still current and skipped, so the regression persisted.
# ⇒ Per host, before any write: read the version the host already carries and
#   refuse a strictly-older incoming build. Same version and newer proceed;
#   a fresh host has nothing to compare and proceeds. `--allow-downgrade`
#   exists for the deliberate rollback and says so in the log.
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
LIVE_HOST="$("$(dirname "$0")/ygg-live-host.sh" 2>/dev/null || true)"
HOSTS="dev $LIVE_HOST oc"
HOSTS="$(printf '%s\n' $HOSTS | awk 'NF && !seen[$0]++' | tr '\n' ' ')"
DRY=0
ALLOW_BEHIND=0
PREFLIGHT=0
HOSTS_EXPLICIT=0
NO_PIN=0
ALLOW_DOWNGRADE=0
# The direct-channel restart door fires by default (staged, unforced — see the
# DIRECT CHANNEL block for why default-on is the honest behavior). `--no-direct-restart`
# stages the build and flips the install-state but leaves the live stack alone.
DIRECT_RESTART=1
DIRECT_TOUCHED=0
while [ $# -gt 0 ]; do
  case "$1" in
    --from) FROM="$2"; shift 2;;
    --hosts) HOSTS="$2"; HOSTS_EXPLICIT=1; shift 2;;
    --no-pin) NO_PIN=1; shift;;
    --dry-run) DRY=1; shift;;
    --allow-behind) ALLOW_BEHIND=1; shift;;
    --preflight) PREFLIGHT=1; shift;;
    --allow-downgrade) ALLOW_DOWNGRADE=1; shift;;
    --no-direct-restart) DIRECT_RESTART=0; shift;;
    *) echo "unknown argument: $1" >&2; exit 2;;
  esac
done

# ⛔ THIS REFUSAL USED TO RUN **BEFORE** ARGUMENT PARSING, so it exited on the
# DEFAULT list and `--hosts` could never be read — while the refusal's own last
# line told you to pass `--hosts`. **Advice the code made unreachable**, and it
# is the same shape as a caution whose callee can never return the error it
# guards: the words were right and nothing could act on them.
# ⇒ Measured cost: `deploy_fleet_guard`'s two ancestry tests passed `--hosts local`,
# were refused before it was parsed, and sat RED long enough to be filed as a
# known-failing pair — a real gate whose tests nobody could read.
# ⚠ The refusal itself is CORRECT and stays: failing to resolve the live host
# silently skips the only host a UI change can be proven on. Count is not the
# invariant: the live host may legitimately be one of the fixed fleet hosts
# (for example `dev`), leaving two unique names after deduplication. Preserve the
# resolver's answer and prove that exact answer remains in the deployment set.
if [ "$HOSTS_EXPLICIT" = 0 ] && { [ -z "$LIVE_HOST" ] || ! printf '%s\n' $HOSTS | grep -Fxq "$LIVE_HOST"; }; then
  echo "deploy-fleet: ⛔ could not resolve the live GUI host — ygg-live-host.sh gave nothing." >&2
  echo "  Deploying to '$HOSTS' would SKIP the only host a UI change can be proven on." >&2
  echo "  Pass --hosts explicitly if that is really what you want." >&2
  exit 2
fi

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

# ⛔⛔ A DAEMON BUMP TYPES. THE GUI RELAUNCH DOES NOT. THAT IS THE OPPOSITE OF HOW
# IT IS USUALLY TREATED, AND IT IS WHY THIS CHECK IS HERE RATHER THAN IN A DOC.
#
# The hot-restart repair submits `continue` to rows after a handover. On a build
# without the draft guard that submit has no draft check, so a deploy can splice
# `continue` and its retry barrage into a half-typed sentence a human is holding.
# The ordering rule was written down — inside a bug entry — and a release was still
# cut over it, because the person cutting a release does not read the bug queue
# first. A rule is only load-bearing where the harmful act happens; this is it.
#
# ⚠ THE NARROWING, and it is what keeps this from becoming a freeze: the danger is
# a handover that FAILS TO CONVERGE, not a bump as such. `continue` is submitted
# only to sessions named in the interrupted-sessions record, and that record is
# written on a FORCED cold shutdown. A handover that settles gracefully interrupts
# nothing, records nothing and submits nothing. So:
#   - the record EXISTS  ⇒ the last handover did not converge ⇒ REFUSE
#   - attended rows exist ⇒ WARN, name them, and continue
# ⛔ This never probes a row to find out whether a draft is open. The readiness
# probe TYPES — probing for the hazard would BE the hazard, wearing a lab coat.
ATTENDED_TSV="${YGGTERM_RELAY_DIR:-$HOME/.yggterm/relay}/never-arm.tsv"
ATTENDED_N=0
if [ -r "$ATTENDED_TSV" ]; then
  ATTENDED_N=$(grep -cve '^[[:space:]]*\(#\|$\)' "$ATTENDED_TSV" || true)
else
  # An unreadable list is NOT an empty one. Say which, and treat it as attended.
  echo "deploy-fleet: ⚠ cannot read $ATTENDED_TSV — attendance NOT verified, assuming attended." >&2
  ATTENDED_N=unknown
fi

STRANDED=""
for h in $HOSTS; do
  if [ "$h" = "$(hostname -s 2>/dev/null)" ] || [ "$h" = dev ]; then
    [ -e "${YGGTERM_HOME:-$HOME/.yggterm}/hot-restart-interrupted.json" ] && STRANDED="$STRANDED $h:local"
  else
    ssh -o BatchMode=yes -o ConnectTimeout=5 "$h" \
      'test -e "${YGGTERM_HOME:-$HOME/.yggterm}/hot-restart-interrupted.json"' 2>/dev/null \
      && STRANDED="$STRANDED $h:remote"
  fi
done

# The gate's hazard is a `continue` that will be TYPED. But the record's own
# law already bounds that hazard to a 10-minute window
# (crates/yggterm-server/src/hot_restart_repair.rs REPAIR_WINDOW_MS): past it,
# take_repairable drops the record "loudly, rather than honoured" and dispatch
# can never act on it. Refusing on an EXPIRED record blocks every deploy on a
# repair that is already a dead letter — and the deadlock is self-reinforcing,
# because the drop only runs when a daemon looks, and the gate stands in front
# of every daemon that could look. Measured live on dev 2026-09-02, twice the
# same day: a stranded record (writer was the daemon that SURVIVED its own
# aborted swap) blocked deploys from 21:31 to past 23:00 until resolved by
# hand, the second hand resolution of the day. So: FRESH record ⇒ refuse.
# EXPIRED record ⇒ proceed over a loud warning (the successor's sweeper drops
# it on arrival). UNREADABLE record ⇒ refuse — an unreadable list is not an
# empty one.
REPAIR_WINDOW_MS=600000
record_age_ms() {
  # $1 = "host:local|remote"; prints age in ms, or returns 2 if unreadable.
  local host="${1%%:*}" kind="${1##*:}" path recorded now
  if [ "$kind" = local ]; then
    path="${YGGTERM_HOME:-$HOME/.yggterm}/hot-restart-interrupted.json"
    recorded=$(grep -o '"recorded_at_ms":[0-9]*' "$path" 2>/dev/null | head -1 | cut -d: -f2)
  else
    recorded=$(ssh -o BatchMode=yes -o ConnectTimeout=5 "$host" \
      'grep -o "\"recorded_at_ms\":[0-9]*" "${YGGTERM_HOME:-$HOME/.yggterm}/hot-restart-interrupted.json" 2>/dev/null | head -1' 2>/dev/null \
      | cut -d: -f2)
  fi
  case "$recorded" in ''|*[!0-9]*) return 2 ;; esac
  now=$(date +%s%3N)
  echo $((now - recorded))
}

for entry in $STRANDED; do
  age=$(record_age_ms "$entry")
  rc=$?
  if [ "$rc" -eq 2 ]; then
    echo "⛔ REFUSING: interrupted-sessions record on ${entry%%:*} is UNREADABLE —" >&2
    echo "   an unreadable list is not an empty one; resolve it by hand first." >&2
    [ -n "${YGG_DRAFT_CLEARED:-}" ] || exit 1
    echo "   ⚠ overridden by YGG_DRAFT_CLEARED=$YGG_DRAFT_CLEARED" >&2
    continue
  fi
  if [ "$age" -gt "$REPAIR_WINDOW_MS" ]; then
    echo "deploy-fleet: ⚠ EXPIRED interrupted-sessions record on ${entry%%:*} " >&2
    echo "   (age ${age}ms > window ${REPAIR_WINDOW_MS}ms). Its \`continue\`s are dropped by" >&2
    echo "   the daemon's own expiry law and will never be typed; proceeding. The successor" >&2
    echo "   drops the record on arrival. If this repeats, the swapping daemon's poll thread" >&2
    echo "   is dying mid-linger — that is the defect to fix, not this gate." >&2
    continue
  fi
  echo "⛔ REFUSING: a LIVE interrupted-sessions record (${age}ms old) is present on ${entry%%:*}." >&2
  echo "   That record is written on a FORCED cold shutdown and is the list of rows the" >&2
  echo "   repair will TYPE \`continue\` into. Deploying now types into every one of them." >&2
  echo "   Resolve the stranded handover first; do not delete the record to get past this." >&2
  [ -n "${YGG_DRAFT_CLEARED:-}" ] || exit 1
  echo "   ⚠ overridden by YGG_DRAFT_CLEARED=$YGG_DRAFT_CLEARED" >&2
done

if [ "$ATTENDED_N" != 0 ]; then
  echo "deploy-fleet: ⚠ $ATTENDED_N human-attended row(s) on the never-arm list." >&2
  echo "   No interrupted-sessions record exists, so a graceful handover types nothing" >&2
  echo "   and this is a warning, not a refusal. ⛔ But one graceful observation does not" >&2
  echo "   license the general case: if the outgoing daemons cannot be shown to converge," >&2
  echo "   hold the bump until the draft is sent or cleared." >&2
fi

# ⭐ --preflight answers the ancestry question and stops, so a caller can ask it
#    in one second instead of after a three-minute build. Deliberately BEFORE the
#    build-product check: at preflight time those products do not exist yet, and
#    requiring them is what forced the question to be asked too late.
if [ "$PREFLIGHT" = 1 ]; then
  echo "deploy-fleet: preflight ok — HEAD ($BUILD_COMMIT) is a descendant of origin/main; safe to build."
  exit 0
fi

# ⛔⛔⛔ ONE DEPLOY AT A TIME, AND IT MUST NAME WHO HOLDS IT.
#
# The fleet runs a dozen lanes in parallel, each in its own worktree with its own
# `target/release`. Nothing stopped two of them reaching this point at once, and
# the owner reported hitting exactly that: two agents pushing builds. The damage
# is not a failed copy — it is that the two interleave. Each allocates its own
# version, each writes the same three binaries on the same three hosts, and the
# census then names a commit that is a mixture of two builds no tree ever held.
# Every "is my fix live?" check afterwards answers about a binary that never
# existed as a whole.
#
# ⚠ A LEASE, NOT A MUTEX QUEUE. A second deploy must be REFUSED and told who has
# it, not silently queued behind a build it knows nothing about — the caller can
# then decide, which is the whole point of the orchestrator owning the roll.
#
# ⭐ The lease is taken AFTER preflight (which is read-only and must stay free) and
# BEFORE any binary is written. It is released on every exit path by trap, and a
# lease older than the ceiling is taken over WITH A LOUD LINE rather than
# deadlocking the fleet on a killed process.
DEPLOY_LEASE="${YGG_DEPLOY_LEASE:-$HOME/.yggterm/relay/deploy.lease}"
DEPLOY_LEASE_CEILING_S=1800
# ⛔⛔ THE FALLBACK BELOW CANNOT RUN IF THE EXPANSION ABOVE IT DIES FIRST. Under
# `set -u`, `${VAR##*/}` on an UNSET variable is an unbound-variable error, not an
# empty string — so the `[ -n ... ] ||` default was unreachable in exactly the
# environment it was written for. The hourly roll runs with no session id, so
# every roll from the moment this lease landed refused the deploy at this line and
# reported it as a refusal rather than a crash. Nothing reached any host for four
# hours while `main` went on advancing, and the fleet read as "up to date at the
# last version that shipped".
#
# ⇒ A guard that fails CLOSED in the one environment that matters is worse than no
#   guard. Default first, then transform.
HOLDER="${YGGTERM_SESSION_ID:-}"
HOLDER="${HOLDER##*/}"
[ -n "$HOLDER" ] || HOLDER="$(whoami)@$(hostname)/$$"

mkdir -p "$(dirname "$DEPLOY_LEASE")"
if ! (set -o noclobber; printf '%s
%s
%s
' "$HOLDER" "$$" "$(date +%s)" > "$DEPLOY_LEASE") 2>/dev/null; then
  LEASE_WHO="$(sed -n 1p "$DEPLOY_LEASE" 2>/dev/null)"
  LEASE_PID="$(sed -n 2p "$DEPLOY_LEASE" 2>/dev/null)"
  LEASE_AT="$(sed -n 3p "$DEPLOY_LEASE" 2>/dev/null)"
  AGE=$(( $(date +%s) - ${LEASE_AT:-0} ))
  if [ "${LEASE_AT:-0}" -gt 0 ] && [ "$AGE" -lt "$DEPLOY_LEASE_CEILING_S" ]      && { [ "$LEASE_PID" = "$$" ] || kill -0 "${LEASE_PID:-0}" 2>/dev/null || [ -z "$LEASE_PID" ]; }; then
    echo "⛔ deploy-fleet: another deploy holds the lease." >&2
    echo "   holder : ${LEASE_WHO:-unknown} (pid ${LEASE_PID:-?}, ${AGE}s ago)" >&2
    echo "   lease  : $DEPLOY_LEASE" >&2
    echo "   ⇒ Two deploys interleaving write a fleet no tree ever held, and every" >&2
    echo "     version check afterwards answers about a binary that never existed." >&2
    echo "     Wait, or ask that holder. Do NOT delete the lease to get past this." >&2
    exit 75
  fi
  echo "⚠ deploy-fleet: taking over a STALE lease from ${LEASE_WHO:-unknown} (${AGE}s old," >&2
  echo "  pid ${LEASE_PID:-?} is gone). If that deploy is in fact running, stop now." >&2
  printf '%s
%s
%s
' "$HOLDER" "$$" "$(date +%s)" > "$DEPLOY_LEASE"
fi
# ⛔ Released on EVERY exit path. A lease that outlives its holder is a fleet that
# cannot deploy until a human deletes a file nobody documented.
trap 'rm -f "$DEPLOY_LEASE"' EXIT
echo "deploy-fleet: lease held by $HOLDER"

GUI="$FROM/yggterm"
HL="$FROM/yggterm-headless"
YNPM="$FROM/ynpm"
for f in "$GUI" "$HL" "$YNPM"; do
  [ -x "$f" ] || { echo "⛔ missing build product: $f" >&2; exit 1; }
done

# ⛔ ONE BUILD, NOT TWO. Two binaries at different versions is the split this
# script exists to end, and shipping them together is the only way the fleet can
# ever be checked with a single number.
GUI_V=$("$GUI" --version 2>/dev/null)
HL_V=$("$HL" --version 2>/dev/null)
[ "$GUI_V" = "$HL_V" ] && [ "$GUI_V" = "$("$YNPM" --version 2>/dev/null)" ] || {
  YNPM_V=$("$YNPM" --version 2>/dev/null || echo unknown)
  echo "⛔ REFUSING: workspace binaries disagree — yggterm=$GUI_V yggterm-headless=$HL_V ynpm=$YNPM_V" >&2
  echo "   Build all three from the same tree in one command." >&2
  exit 1; }
VERSION="$GUI_V"
echo "deploy-fleet: $VERSION ($BUILD_COMMIT) from $FROM → $HOSTS"

GUI_SUM=$(md5sum "$GUI" | awk '{print $1}')
HL_SUM=$(md5sum "$HL" | awk '{print $1}')
YNPM_SUM=$(md5sum "$YNPM" | awk '{print $1}')

# The eight canonical copies, and which build product belongs in each. `~/.local/bin` is the
# GUI's home; `~/.yggterm/bin` is the install root the daemon actually runs from
# AND the path remote sessions invoke. All eight are real, so all eight are written.
#
# ⛔⛔ AND THE MANAGED-VERSIONS PAIR IS NOT OPTIONAL — ITS ABSENCE DEADLOCKED THE
# GUI UPDATE RESTART. The GUI's convergence finder (`installed_gui_executable_for_version`)
# and `install_path_declared_version` trust exactly one layout statement:
# `~/.yggterm/versions/<VERSION>/<binary>`. This script wrote the flat four and
# walked past that directory, whose newest entry was 3.1.59 while the fleet ran
# 3.1.60+ — so the finder answered "no matching GUI binary on disk" for a binary
# THIS DEPLOY HAD JUST WRITTEN, convergence returned None, and a GUI that was
# closed (by a deploy, a crash, or the draft-hold retry) had nothing to relaunch
# onto: measured on the GUI host 2026-08-27 as the live "Version Skew — Binary
# Missing" notification and a dead window the owner relaunched by hand. The
# version directory IS the declaration — nothing can bump it without a move —
# so writing it is what makes convergence true on every host.
declare -A COPY=(
  ["\$HOME/.local/bin/yggterm"]="GUI"
  ["\$HOME/.local/bin/yggterm-headless"]="HL"
  ["\$HOME/.local/bin/ynpm"]="YNPM"
  ["\$HOME/.local/bin/ynpx"]="YNPM"
  ["\$HOME/.yggterm/bin/yggterm"]="GUI"
  ["\$HOME/.yggterm/bin/yggterm-headless"]="HL"
  ["\$HOME/.yggterm/bin/ynpm"]="YNPM"
  ["\$HOME/.yggterm/bin/ynpx"]="YNPM"
  ["\$HOME/.yggterm/versions/$VERSION/yggterm"]="GUI"
  ["\$HOME/.yggterm/versions/$VERSION/yggterm-headless"]="HL"
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
  # ⛔ SAME BYTES, SAME INODE. Renaming a fresh copy over a byte-identical
  # file is not a deploy — it is a re-arm of every running binary's "my exe
  # was deleted" signal. Measured 2026-09-03 on the GUI host: sixteen
  # same-version daemon takeovers in one hour, because every deploy rewrote
  # four identical copies, every running daemon saw its exe deleted, spawned
  # a successor (from the same mutable path), and every attached client
  # dropped. The md5 is already in hand; when the host already holds these
  # exact bytes, keep the inode and report the copy as done. A genuine new
  # build has a different md5 and writes exactly as before.
  got=$(run_on "$host" "md5sum \$(eval echo $dest) 2>/dev/null")
  got=${got%% *}
  if [ "$got" = "$want" ]; then
    printf "  ✅ %-14s %s (same bytes, inode kept)\n" "$host" "$dest"
    return 0
  fi
  local write="d=\$(eval echo $dest); mkdir -p \$(dirname \$d); cat > \$d.new && chmod 755 \$d.new && mv -f \$d.new \$d"
  if is_self "$host"; then bash -c "$write" < "$src"
  else ssh "$host" "$write" < "$src"; fi
  got=$(run_on "$host" "md5sum \$(eval echo $dest)")
  got=${got%% *}
  if [ "$got" = "$want" ]; then printf "  ✅ %-14s %s\n" "$host" "$dest"
  else printf "  ⛔ %-14s %s  READ BACK %s WANTED %s\n" "$host" "$dest" "${got:-<none>}" "$want"; return 1; fi
}

# ⛔⛔ THE CANONICAL FOUR ARE A FLOOR, NOT THE SET — DISCOVER THE REST.
#
# This table used to BE the answer to "which copies exist", and a host that had
# picked up copies outside it was invisible to the deploy forever. Measured
# 2026-08-20: one fleet host held SIX copies across three roots at FOUR
# different versions — `~/.cargo/bin` at 3.0.162 and 3.1.0, `~/.local/bin` at
# 3.1.3, `~/.yggterm/bin` at 3.1.4 — because `cargo install` had once put a pair
# in a root this script does not know about, and every deploy since wrote four
# copies and walked past the other two.
#
# ⇒ A hardcoded destination list cannot notice a destination it does not name.
# Ask the HOST which copies it actually has, write the union, and the split
# cannot re-open: the next unexpected root is discovered rather than missed.
#
# ⚠ Home-rooted only. A copy under /usr needs privilege this deploy does not
# take, so one found there is REPORTED by name and left alone — an unattended
# deploy that starts using sudo is a worse failure than a stale binary.
discover_copies() {  # host  → prints "abs_path KIND" per line
  run_on "$1" '''for d in "$HOME/.local/bin" "$HOME/.yggterm/bin" "$HOME/.cargo/bin" \
                        "$HOME/bin" "$HOME/.bun/bin" "$HOME/go/bin"; do
                   for n in yggterm yggterm-headless ynpm ynpx; do
                     [ -f "$d/$n" ] && echo "$d/$n"
                   done
                 done
                 for n in yggterm yggterm-headless ynpm ynpx; do
                   p=$(command -v "$n" 2>/dev/null) && [ -f "$p" ] && echo "$p"
                 done' 2>/dev/null | sort -u | while read -r path; do
    [ -n "$path" ] || continue
    case "$(basename "$path")" in
      yggterm-headless) echo "$path HL" ;;
      ynpm|ynpx)       echo "$path YNPM" ;;
      yggterm)          echo "$path GUI" ;;
    esac
  done
}

FAILED=0
for host in $HOSTS; do classify_host "$host"; done

# Like run_on, but THIS function's stdin is forwarded — the direct-channel
# block pipes the flip program and JSON through it. run_on deliberately closes
# stdin ("< /dev/null"); a forwarded stdin must be a separate verb or every
# existing call site would have to re-close it by hand.
pipe_run() {  # host, command words…
  local host="$1"; shift
  if is_self "$host"; then "$@"
  else ssh "$host" "$*"; fi
}

# The direct install-state flip, fed on stdin: `python3 - <ver> <new_exe>`.
# Empty <ver> keeps active_version (same-version rebuild adoption) and only
# repoints active_executable. Atomic (tmp + os.replace) and field-preserving
# (channel/repo/asset_label ride through) — the binary's own
# write_direct_install_state does tmp+rename the same way.
FLIP_PY='import json, os, sys
# argv[3] (optional) names the state file: the channel-default one, or a
# SUPERVISED PIN (see the pin pass below) — the same flip, different file.
p = os.path.expandvars(sys.argv[3]) if len(sys.argv) > 3 else os.path.expanduser("~/.local/share/yggterm/direct/install-state.json")
with open(p) as f:
    s = json.load(f)
ver, exe = os.path.expandvars(sys.argv[1]), os.path.expandvars(sys.argv[2])
if ver == "KEEP":
    ver = ""  # same-version rebuild: the sentinel rode through "$*" where an empty arg cannot
if ver:
    s["active_version"] = ver
    s["icon_revision"] = ver
if s.get("active_executable") == exe and (not ver or s.get("active_version") == ver):
    print("already names this build")
else:
    s["active_executable"] = exe
    tmp = p + ".deploy-tmp"
    with open(tmp, "w") as f:
        json.dump(s, f, indent=2)
        f.write("\n")
    os.replace(tmp, p)
    print("flipped")'


# ⛔ SAY IT ONCE, AND SAY WHAT IT MIGHT MEAN. Four identical copy failures read
# as a partial deploy; one named refusal reads as what it is.
for host in $HOSTS; do
  [ "${HOST_UNREACHABLE[$host]:-0}" = 1 ] || continue
  echo "  ⛔ $host: cannot be reached over ssh, so its eight copies are SKIPPED, not failed." >&2
  echo "     If this is the machine you are standing on, its fleet alias and its" >&2
  echo "     kernel hostname ($(hostname -s)) differ and nothing local can bridge them." >&2
  echo "     Fix it for good with:  export YGG_FLEET_SELF=$host" >&2
  echo "     Or for this run:       --hosts local" >&2
  FAILED=1
done

# ⛔ PIN THE TRACE BEFORE THE SWAP, BECAUSE RETENTION WILL NOT KEEP IT.
# Sessions have died 166-464 s after a release, and every investigation but one
# arrived to find the trace covering the death already pruned: the byte budget is
# per home while the write rate is per daemon, so the measured window was ~3 h,
# not the 3 days the constant advertised. Pinning costs nothing (hard links) and
# is the difference between the next death being explainable and being another
# structural absence. ⚠ Deliberately NOT gated on success — a diagnostic that can
# fail a deploy is a diagnostic that gets switched off.
if [ "$NO_PIN" = 0 ] && [ "$DRY" = 0 ]; then
  "$(dirname "$0")/pin-trace-window.sh" --label "$VERSION" --hosts "$HOSTS" \
    --follow-mins 15 2>&1 | sed 's/^/  /' || \
    echo "  ⚠ trace pin failed — the deploy continues, but a death in the next" \
         "15 min may not be explainable afterwards." >&2
fi

for host in $HOSTS; do
  [ "${HOST_UNREACHABLE[$host]:-0}" = 1 ] && continue

  # ⛔ THE DOWNGRADE GUARD. Ask the host what it already carries — both flat
  # roots, since a split past deploy can leave them disagreeing — and refuse a
  # strictly-older incoming build for that host. Refusal is PER HOST: the rest
  # of the fleet still receives the release, and the refusal names both
  # versions so the operator knows exactly which checkout is stale.
  if [ "$ALLOW_DOWNGRADE" = 0 ]; then
    host_versions=$(run_on "$host" '"$HOME/.local/bin/yggterm" --version 2>/dev/null; "$HOME/.yggterm/bin/yggterm-headless" --version 2>/dev/null' 2>/dev/null)
    stale_ref=""
    while read -r hv; do
      [ -n "$hv" ] || continue
      oldest=$(printf '%s\n%s\n' "$VERSION" "$hv" | sort -V | head -1)
      if [ "$oldest" = "$VERSION" ] && [ "$VERSION" != "$hv" ]; then
        stale_ref="$hv"
        break
      fi
    done <<EOF
$host_versions
EOF
    if [ -n "$stale_ref" ]; then
      echo "  ⛔ $host: REFUSING DOWNGRADE — host already runs $stale_ref, this build is $VERSION." >&2
      echo "     A checkout older than the fleet is trying to deploy. Rebase it, or pass" >&2
      echo "     --allow-downgrade if taking this host backwards is deliberate." >&2
      FAILED=1
      continue
    fi
  fi

  # The canonical eight are written whether or not they exist yet; anything else
  # this host already carries is written because it exists. Union, deduped by
  # the resolved path so a symlinked root is not written twice.
  declare -A DEST_KIND=()
  for dest in "${!COPY[@]}"; do DEST_KIND["$dest"]="${COPY[$dest]}"; done
  canon=$(run_on "$host" 'echo "$HOME/.local/bin/yggterm $HOME/.local/bin/yggterm-headless $HOME/.yggterm/bin/yggterm $HOME/.yggterm/bin/yggterm-headless"' 2>/dev/null)
  while read -r path kind; do
    [ -n "$path" ] || continue
    case " $canon " in *" $path "*) continue ;; esac
    DEST_KIND["$path"]="$kind"
    echo "  ⚠ $host: $path is an EXTRA copy this deploy would once have skipped — writing it too."
  done < <(discover_copies "$host")

  for dest in "${!DEST_KIND[@]}"; do
    case "${DEST_KIND[$dest]}" in
      GUI)  src="$GUI";  want="$GUI_SUM" ;;
      HL)   src="$HL";   want="$HL_SUM" ;;
      YNPM) src="$YNPM"; want="$YNPM_SUM" ;;
      *) echo "  ⛔ $host: unknown deployment kind ${DEST_KIND[$dest]} for $dest" >&2; FAILED=1; continue ;;
    esac
    if [ "$DRY" = 1 ]; then
      printf "  · %-14s %s ← %s%s\n" "$host" "$dest" "$(basename "$src")" \
        "$(is_self "$host" && echo "  (this machine — no ssh)")"
      continue
    fi
    push_one "$host" "$src" "$dest" "$want" || FAILED=1
  done

  # ⛔ PRUNE ONLY WHAT NO LIVE PROCESS STILL EXECUTES. The versions/ layout now
  # grows one pair per release (~86 MB); left alone it accrues forever. But a
  # version directory is also where a RUNNING daemon binary may live (the
  # managed-versions install model), and deleting a live binary's path makes its
  # /proc/<pid>/exe read "(deleted)" — the exact cold-kill class the .old.*
  # warning above exists for. So: semver-older than THIS deploy only, never the
  # running version, and never a path any /proc/*/exe still points into.
  run_on "$host" 'cd "$HOME/.yggterm/versions" 2>/dev/null || exit 0
    open_exe_paths=$(for f in /proc/[0-9]*/exe; do readlink "$f" 2>/dev/null; done)
    for d in [0-9]*; do
      case "$d" in
        '"$VERSION"') continue ;;
        *[!0-9.]*) continue ;;
      esac
      newer=$(printf "%s\n%s\n" "$d" "'$VERSION'" | sort -V | tail -1)
      [ "$newer" = "'$VERSION'" ] || continue
      case "
$open_exe_paths
" in *"$HOME/.yggterm/versions/$d/"*) continue ;; esac
      rm -rf "$HOME/.yggterm/versions/$d" && echo "  · pruned versions/$d (older, not executed)"
    done' 2>/dev/null | sed 's/^/  /'

  unset DEST_KIND

  # ⛔⛔ THE DIRECT CHANNEL IS A SECOND INSTALL PLANE, AND EVERYTHING ABOVE IS
  # A DEAD WRITE FOR IT — [11.121], measured 2026-09-15 on the GUI host. A host
  # whose live stack runs the DIRECT channel (~/.local/share/yggterm/direct/
  # install-state names the executable the daemon and GUI actually exec) never
  # reads a byte of the managed layout this script writes: the roll printed
  # four ✅ and claimed "deployed" while three landed UX fixes sat
  # executed-never on disk, and both restart doors were blind to it — GUI
  # convergence restarts only on a version-STRING bump (equal versions never
  # restart, per the 2026-09-04 loop fix), and `server app update restart`
  # derives its pending update from the direct install-state, whose
  # active_executable the deploy never touched, so preferred == current → None.
  #
  # ⇒ ADOPT THE CHANNEL'S OWN SHAPE. The direct channel updates itself by
  # writing versions/<v>/ and flipping install-state; that flip IS the pending
  # update — every launched binary re-execs to active_executable, and the GUI
  # update workflow + `server app update restart` restart into it through the
  # guarded machinery (agent-lease refusal, draft defers, session-preserving
  # daemon handover). A same-version rebuild (the roll reuses one VERSION
  # inside a release window) stages under builds/<commit>/: a path OUTSIDE
  # versions/ makes no version claim (install_path_declared_version), so the
  # record's word stands and the pending-restart door opens — and the door is
  # one-shot by construction (preferred == current once adopted), exactly the
  # property the 2026-09-04 same-version convergence loop taught this repo to
  # demand before anything may restart a GUI on a rebuild. NEVER repoint
  # active_executable back into the RUNNING versions/<v>/ directory: the path
  # would not change, the pending update would stay invisible, and the defect
  # would reopen under a green report line.
  direct_state=$(run_on "$host" 'cat "$HOME/.local/share/yggterm/direct/install-state.json" 2>/dev/null' 2>/dev/null || true)
  case "$direct_state" in
    *'"channel": "direct"'*|*'"channel":"direct"'*) ;;
    *) continue ;;  # managed-channel host: the write + convergence machinery above already owns it
  esac
  # ⛔ THE PARSE IS LOCAL, ON PURPOSE. direct_state is already a shell variable
  # on THIS machine; a `python3 -c` program cannot cross ssh inside "$*" (the
  # remote shell strips its quoting and python reads `import` as the program —
  # measured: jojo answered "unreadable" while the identical probe in a bare
  # ssh line matched). Local sed on the flat machine-written JSON, no second
  # round trip.
  dactive=$(printf '%s' "$direct_state" | sed -n 's/.*"active_version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)
  dexe=$(printf '%s' "$direct_state" | sed -n 's/.*"active_executable"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)
  if [ -z "$dactive" ] || [ -z "$dexe" ]; then
    echo "  ⚠ $host: direct install-state unreadable — the direct channel is left untouched rather than guessed at." >&2
    continue
  fi
  # The channel may have self-updated PAST this roll. A deploy never takes a
  # host backwards — the same law as the managed downgrade guard above, applied
  # to the plane that guard cannot see.
  if [ "$VERSION" != "$dactive" ] &&
     [ "$(printf '%s\n%s\n' "$VERSION" "$dactive" | sort -V | head -1)" = "$VERSION" ]; then
    echo "  · $host: direct channel is at $dactive, NEWER than this roll ($VERSION) — direct install left untouched."
    continue
  fi
  need_flip=1
  if [ "$VERSION" = "$dactive" ]; then
    # SAME-VERSION REBUILD — stage by build identity, never into the running dir.
    bid=${BUILD_COMMIT:-}
    case "$bid" in ""|*[!A-Za-z0-9._-]*) bid=${GUI_SUM:0:12} ;; esac
    ddest="\$HOME/.local/share/yggterm/direct/builds/$bid"
    dnewexe='$HOME/.local/share/yggterm/direct/builds/'"$bid"'/yggterm'
    dver=""
    activemd5=$(run_on "$host" "md5sum '$dexe' 2>/dev/null" | awk '{print $1}')
    if [ "$activemd5" = "$GUI_SUM" ]; then
      # The state file already names THIS build's bytes. Nothing to stage or
      # flip — but the RUNNING stack may still be on older bytes, so the door
      # decision below still runs: rotation is a process fact, not a file fact.
      need_flip=0
      label="direct: state already names this build ($dexe)"
    else
      label="direct: same-version rebuild staged at builds/$bid"
    fi
  else
    ddest="\$HOME/.local/share/yggterm/direct/versions/$VERSION"
    dnewexe='$HOME/.local/share/yggterm/direct/versions/'"$VERSION"'/yggterm'
    dver="$VERSION"
    label="direct: release $VERSION staged (channel was $dactive)"
  fi
  if [ "$DRY" = 1 ]; then
    drypins=$(run_on "$host" 'for p in $(pgrep -x yggterm 2>/dev/null) $(pgrep -f "yggterm-headless server daemon" 2>/dev/null); do
           [ -r "/proc/$p/environ" ] && tr "\000" "\n" < "/proc/$p/environ" 2>/dev/null \
             | sed -n "s/^YGGTERM_DIRECT_INSTALL_ROOT=//p"
         done | sort -u' 2>/dev/null || true)
    drypins=$(printf '%s' "$drypins" | grep -v "^$HOME/.local/share/yggterm/direct$" | tr '\n' ' ')
    echo "  · $host: DRY — $label; flip needed: $need_flip; supervised pins: ${drypins:-none}; restart door: $([ "$DIRECT_RESTART" = 1 ] && echo 'would fire if live bytes differ, unforced' || echo 'skipped (--no-direct-restart)')"
    continue
  fi
  DIRECT_TOUCHED=1
  if [ "$need_flip" = 1 ]; then
    dstage_fail=0
    for pair in "yggterm:GUI" "yggterm-headless:HL" "ynpm:YNPM" "ynpx:YNPM"; do
      bin=${pair%%:*}; kind=${pair##*:}
      case "$kind" in
        GUI)  src="$GUI";  want="$GUI_SUM"  ;;
        HL)   src="$HL";   want="$HL_SUM"   ;;
        YNPM) src="$YNPM"; want="$YNPM_SUM" ;;
      esac
      push_one "$host" "$src" "$ddest/$bin" "$want" || { dstage_fail=1; break; }
    done
    # ⛔ FAILED is the fleet-wide accumulator; a copy that failed on ANY host
    # must not silently skip the direct block of every host after it — a
    # per-host local flag decides the skip, FAILED only decides the exit code.
    if [ "$dstage_fail" != 0 ]; then FAILED=1; continue; fi
    # dnewexe carries an expanded /home/pi path ON PURPOSE: it was read from
    # the host's own install-state-shaped layout and this fleet is single-user
    # — the same expansion the activemd5 probe above already relies on.
    fb64=$(printf '%s' "$FLIP_PY" | base64 | tr -d '\n')
    # ⛔ the flip command is a PIPELINE, so it goes to ssh as ONE command
    # string — pipe_run's "$*" join would let the remote shell re-split
    # `bash -c` around the pipes. Same explicit is_self/ssh split the census
    # below uses.
    fcmd="echo $fb64 | base64 -d | python3 - '${dver:-KEEP}' '$dnewexe'"
    flip_ok=0
    if is_self "$host"; then flip_out=$(bash -c "$fcmd" 2>&1) && flip_ok=1
    else flip_out=$(ssh "$host" "$fcmd" 2>&1) && flip_ok=1; fi
    if [ "$flip_ok" != 1 ]; then
      echo "  ⛔ $host: direct install-state flip FAILED — staged bytes at builds/versions are NOT adopted; the live stack still runs $dexe" >&2
      FAILED=1
      continue
    fi
    echo "  ✅ $host: $label — install-state flipped ($flip_out); the guarded restart doors can now see the update"
  else
    echo "  · $host: $label — nothing to stage or flip"
    fb64=$(printf '%s' "$FLIP_PY" | base64 | tr -d '\n')
  fi
  # ⛔ SUPERVISED PINS: THE FILE ABOVE MAY NOT BE THE ONE THE LIVE STACK READS.
  # A supervised install (YGGTERM_SUPERVISED=1) is LAUNCHED with
  # YGGTERM_DIRECT_INSTALL_ROOT naming another root, and
  # direct_install_state_for_executable checks that env var FIRST — every
  # install-state read inside the pinned processes resolves there, and
  # YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF=1 means the binary will not even
  # self-upgrade on launch. Measured 2026-09-15 on the GUI host: both live
  # planes pinned to ~/.yggterm whose state still named the OLD build, so the
  # channel-root flip above was invisible to them — the door no-op'd, the
  # startup workflow answered installed:false, and the owner's manual restart
  # landed on the pinned file's OLD target ([11.122]). The supervisor reads
  # the pinned file too, so flipping IT is what the next launch adopts.
  # Discover the pins from the live processes' OWN environment — a pin nobody
  # carries needs no flip, and one somebody carries must not be guessed.
  # Runs on EVERY direct host pass (need_flip or not): a pin missed by an
  # earlier roll is exactly the state this pass exists to heal.
  pins=$(run_on "$host" 'for p in $(pgrep -x yggterm 2>/dev/null) $(pgrep -f "yggterm-headless server daemon" 2>/dev/null); do
           [ -r "/proc/$p/environ" ] && tr "\000" "\n" < "/proc/$p/environ" 2>/dev/null \
             | sed -n "s/^YGGTERM_DIRECT_INSTALL_ROOT=//p"
         done | sort -u' 2>/dev/null || true)
  for proot in $pins; do
    [ -n "$proot" ] || continue
    case "$proot" in "$HOME/.local/share/yggterm/direct") continue ;; esac
    pfile="$proot/install-state.json"
    if ! run_on "$host" "test -f '$pfile'" 2>/dev/null; then
      echo "  · $host: pinned root $proot has no install-state — not guessed at"
      continue
    fi
    fcmd="echo $fb64 | base64 -d | python3 - '${dver:-KEEP}' '$dnewexe' '$pfile'"
    pin_ok=0
    if is_self "$host"; then pin_out=$(bash -c "$fcmd" 2>&1) && pin_ok=1
    else pin_out=$(ssh "$host" "$fcmd" 2>&1) && pin_ok=1; fi
    if [ "$pin_ok" = 1 ]; then
      echo "  ✅ $host: supervised pin $pfile: $pin_out (live stack reads it via YGGTERM_DIRECT_INSTALL_ROOT)"
    else
      echo "  ⛔ $host: supervised-pin flip FAILED for $pfile — the pinned live stack still reads the old target" >&2
      FAILED=1
    fi
  done
  # Fire the restart door UNFORCED: it refuses on its own while an agent holds
  # a live web-surface lease, and defers are honest — the staged build is
  # adopted by the next deploy or the GUI's own next start. Fired only when a
  # live process still runs different bytes; after adoption the door is a
  # no-op and re-firing it every roll would be noise wearing a ✅.
  if [ "$DIRECT_RESTART" = 1 ]; then
    livemd5s=$(run_on "$host" 'for p in $(pgrep -x yggterm 2>/dev/null) $(pgrep -f "yggterm-headless server daemon" 2>/dev/null); do
                  [ -r "/proc/$p/exe" ] && md5sum "/proc/$p/exe" 2>/dev/null | cut -d" " -f1
                done | sort -u' 2>/dev/null || true)
    if [ -z "$livemd5s" ]; then
      echo "  · $host: no live yggterm process to rotate — the staged build is what the next launch execs"
    else
      case "
$livemd5s
" in
        *"
$GUI_SUM
"**"
$HL_SUM
"*) echo "  ✅ $host: direct stack already executes this build — restart door not fired"; continue ;;
      esac
      resp=$(run_on "$host" '"$HOME/.yggterm/bin/yggterm-headless" server app update restart --timeout-ms 20000' 2>&1 || true)
      case "$resp" in
        *'"error": null'*)
          echo "  ✅ $host: direct restart requested through the guarded door — sessions ride the handover" ;;
        "")
          echo "  ⚠ $host: no GUI answered the restart door — build stays staged; the next start adopts it" ;;
        *)
          echo "  ⚠ $host: restart door did not adopt ($(printf '%s' "$resp" | tr '\n' ' ' | head -c 220)) — build stays staged; the next deploy or GUI start retries" ;;
      esac
    fi
  else
    echo "  · $host: direct build staged; restart door skipped (--no-direct-restart)"
  fi
    # Keep the last three staged builds; never the one install-state points at.
  run_on "$host" 'cd "$HOME/.local/share/yggterm/direct/builds" 2>/dev/null || exit 0
    active=$(sed -n "s/.*\"active_executable\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p" "$HOME/.local/share/yggterm/direct/install-state.json" 2>/dev/null | head -n1)
    activedir=$(dirname "$active" 2>/dev/null)
    n=0
    for d in $(ls -1t 2>/dev/null); do
      [ -d "$d" ] || continue
      [ "$PWD/$d" = "$activedir" ] && continue
      n=$((n+1))
      [ "$n" -gt 3 ] || continue
      rm -rf "$d" && echo "  · pruned direct builds/$d (older staged build, not active)"
    done' 2>/dev/null | sed 's/^/  /'
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
  # ⛔ THE CENSUS ENUMERATES WHAT THE HOST HAS, NOT WHAT THIS SCRIPT EXPECTS.
  # It listed the same hardcoded four as the write loop, so a host holding a
  # copy in an unknown root reported ✅ across the board while running a build
  # from a path nobody was looking at. Same defect, same file, twice — a
  # hardcoded path list is the bug, and repeating it in the verification step is
  # what made the split survive every deploy that was supposed to catch it.
  cen='MINV=3.0.125
       for p in $(for d in "$HOME/.local/bin" "$HOME/.yggterm/bin" "$HOME/.cargo/bin" \
                           "$HOME/bin" "$HOME/.bun/bin" "$HOME/go/bin"; do
                    for n in yggterm yggterm-headless; do
                      [ -f "$d/$n" ] && echo "$d/$n"
                    done
                  done | sort -u); do
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
         sed "s/^/    running  /" || echo "    running  <no census: this host has no reachable daemon>"
       # ⛔⛔ THE GUI PROCESS IS THE THIRD PLANE AND IT WAS INVISIBLE HERE.
       # `server daemons` censuses DAEMONS. The GUI is a separate long-lived
       # process that goes on executing the image it was loaded with, and it is
       # the only plane a UI fix can be proven on — so a roll could report every
       # copy ✅ and every daemon current while a host kept rendering a build
       # from before the roll. Measured 2026-08-21: one host ran a NINE-HOUR-OLD
       # pre-roll GUI, with three new recovery counters simply absent, and the
       # audit called the fleet clean because it checks INSTALLS, not PROCESSES.
       # ⇒ The comment above this block used to say the running GUI could not be
       #   named because the deploy had replaced the file its /proc/<pid>/exe
       #   pointed at. It can: the kernel keeps that inode reachable through
       #   /proc for the life of the process, deleted or not, so it hashes fine
       #   and the answer is exact rather than inferred.
       # ⛔ A SANDBOX GUI IS NOT A STALE GUI, AND CONFLATING THEM MAKES THIS
       # WARNING WORTHLESS. A build host runs several yggterm GUIs at once —
       # each lane drives one out of its OWN worktree target/release, which is
       # the entire point of a sandbox and must never be reported as behind.
       # Only a GUI running from an INSTALLED path is claiming to be this
       # deploy. Measured 2026-08-21: one host had six GUI processes, four of
       # them lane sandboxes; warning on all six would have trained the reader
       # to ignore the two that mattered.
       for g in $(pgrep -x yggterm 2>/dev/null); do
         x=$(readlink /proc/$g/exe 2>/dev/null)
         m=$(md5sum /proc/$g/exe 2>/dev/null | cut -c1-10)
         case "$x" in
           "$HOME/.local/bin/"*|"$HOME/.yggterm/bin/"*|"$HOME/.cargo/bin/"*|"$HOME/bin/"*|"$HOME/.local/share/yggterm/direct/"*) k="installed";;
           *) k="sandbox";;
         esac
         printf "    gui      pid %-8s %-11s %-10s %s\n" \
           "$g" "${m:-(unreadable)}" "$k" "${x:-?}"
       done
       [ -z "$(pgrep -x yggterm 2>/dev/null)" ] && echo "    gui      (no GUI process on this host)"'
  if is_self "$host"; then bash -c "$cen"; else ssh "$host" bash -c "'$cen'"; fi
done

# ⛔ AND SAY IT OUT LOUD RATHER THAN LEAVING IT IN A COLUMN. A stale GUI is the
# one staleness that reads as a working system: the window is up, the rows are
# there, and only the fix you shipped is missing. The daemons are EXPECTED to
# trail (that is the constitution); the GUI is not, because nothing retires it
# on its own terms — somebody has to restart it.
for host in $HOSTS; do
  [ "${HOST_UNREACHABLE[$host]:-0}" = 1 ] && continue
  # Only INSTALLED GUIs are claiming to be this deploy; see the note in the census.
  probe='for g in $(pgrep -x yggterm 2>/dev/null); do
           x=$(readlink /proc/$g/exe 2>/dev/null)
           case "$x" in "$HOME/.local/bin/"*|"$HOME/.yggterm/bin/"*|"$HOME/.cargo/bin/"*|"$HOME/bin/"*|"$HOME/.local/share/yggterm/direct/"*)
             md5sum /proc/$g/exe 2>/dev/null | cut -c1-10;; esac
         done'
  if is_self "$host"; then running=$(bash -c "$probe"); else running=$(ssh "$host" bash -c "'$probe'"); fi
  [ -z "$running" ] && continue
  for m in $running; do
    if [ "$m" != "${GUI_SUM:0:10}" ]; then
      echo "⚠ deploy-fleet: $host is RUNNING a GUI built from $m, not this build (${GUI_SUM:0:10})."
      echo "   Nothing retires a GUI on its own terms — restart it or the UI half of this"
      echo "   roll is not live on that host, however green every other line above reads."
    fi
  done
done

if [ "$FAILED" != 0 ]; then echo "⛔ deploy-fleet: at least one copy did not read back"; exit 1; fi
echo "deploy-fleet: every copy on every host reads back at $VERSION ($BUILD_COMMIT)"
[ "${DIRECT_TOUCHED:-0}" = 1 ] && echo "  Direct-channel hosts staged this build into their own install plane (their lines above say adopted / deferred / untouched — a deferred one is honest, not green)."
echo "  Ask a FILE which source it is:    yggterm --build-commit"
echo "  Ask the RUNNING processes:        yggterm-headless server daemons   (BUILD column)"
echo "⚠ The daemons do NOT swap here. Each retires onto the new binary on its own"
echo "  poll once its own sessions allow it — that is the constitution, not a bug."
echo "  ⇒ The running plane is EXPECTED to trail the disk for a while. What must"
echo "    never happen is being unable to say by how much, which is why the BUILD"
echo "    column names a commit instead of leaving a version to stand for two."
