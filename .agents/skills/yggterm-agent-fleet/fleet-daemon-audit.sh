#!/usr/bin/env bash
# SessionStart: REPORT the yggterm daemon version on every fleet host.
#
# ⛔ WHY THIS EXISTS. fleet-binary-sync.sh deliberately DENYs yggterm and
# yggterm-headless (its line 18: deploys are versioned and daemon-aware and must
# stay deliberate, never a background mtime race). That exclusion is CORRECT and
# must stay. But nothing ever replaced it with a VISIBILITY step, so a daemon
# could sit 26 versions behind for a week with no instrument saying so — and one
# did: dev ran 2.12.19 from Jul 30 to Aug 6 owning 12 of the owner's sessions,
# and it was noticed by a person, not by a tool.
#
# ⛔ THIS REPORTS. IT NEVER DEPLOYS, RESTARTS, OR KILLS ANYTHING. The whole
# reason binaries are excluded from the sync is that a daemon owns live PTYs; an
# audit that "helpfully" fixed a version would be that background race wearing a
# different hat.
#
# It must never fail a session: it always exits 0.

set -uo pipefail

# ⛔ THE FLEET ROSTER IS CONFIGURATION, NOT A CONSTANT (the ygg-memory law):
# $YGG_FLEET_MESH (comma list) > ~/.yggterm/auth/.fleet-hosts > none (skip).
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
SELF="$(hostname)"
report() { printf '%s\n' "$*"; }

# ⛔ REPORT DISAGREEMENT, NOT A VERSION.
#
# This hook used to read `yggterm-headless` at exactly two paths and print one
# number per host. It was therefore structurally blind to the case that actually
# bit, measured on dev 2026-08-07:
#
#   ~/.local/bin/yggterm            3.0.40      <- what `which yggterm` resolves
#   ~/.local/bin/yggterm-headless   3.0.44
#   ~/.yggterm/bin/yggterm          3.0.44
#   ~/.yggterm/bin/yggterm-headless 3.0.44
#
# Three copies updated, the one on PATH missed — the mirror image of edcc4927
# ("the refresh was updating a binary no session ever runs"), now updating the
# binaries no session TYPES while missing the one it does. The hook said "all
# audited hosts on 3.0.44" while every delegate typing `yggterm` on dev ran
# 3.0.40 and bound a 3.0.40 socket. **A version audit that reads ONE path cannot
# see a split install**, so this one enumerates BOTH names in BOTH directories
# plus whatever PATH actually resolves, and its headline is whether they AGREE.
#
# ⚠ `--version` is the right instrument HERE and nowhere else: it is a pure
# builtin exempt from the exec handoff, so it reports the file you named rather
# than the file that would run a real verb. That makes it useless for "what code
# will execute" (use a behavioural discriminator) and exactly right for "what
# version is sitting at this path", which is this hook's only question.
probe_host() {
  local host="$1"
  # ⛔ THE CONTRACT THIS MUST HONOUR: one line, `installs|running`.
  # `installs` is space-separated `path=version` for every copy that exists;
  # `running` is a human string naming the daemons actually serving. The caller
  # splits on the FIRST `|`, so neither half may contain one.
  #
  # ⛔⛔ RESTORED 2026-08-22. This function's body had been overwritten with a
  # COPY OF THE BOOTER-STALENESS PROBE — the "identify, do not pattern-match"
  # correction was written into the wrong function. Two things followed, and
  # neither one failed loudly: the version audit stopped existing (a clean fleet
  # returns nothing from a staleness probe, and the caller reads empty as
  # UNREACHABLE, so every host was reported unreachable including the one the
  # hook was running ON), and the headline printed `agree at .` — an empty
  # version, agreeing about nothing. ⇒ A probe whose empty answer is indistinguishable
  # from a failure has no negative result, only silence wearing two hats.
  local script='
    installs=""
    seen=""
    for f in "$HOME/.local/bin/yggterm" "$HOME/.local/bin/yggterm-headless" \
             "$HOME/.yggterm/bin/yggterm" "$HOME/.yggterm/bin/yggterm-headless" \
             "$(command -v yggterm 2>/dev/null)" \
             "$(command -v yggterm-headless 2>/dev/null)"; do
      [ -n "$f" ] || continue
      [ -x "$f" ] || continue
      case " $seen " in *" $f "*) continue ;; esac
      seen="$seen $f"
      v=$(timeout 5 "$f" --version 2>/dev/null | head -1 | tr -d " \t")
      v=$(printf "%s" "$v" | sed "s/^[A-Za-z_-]*//")
      installs="$installs $f=${v:-?}"
    done
    bin=""
    for c in "$(command -v yggterm-headless 2>/dev/null)" \
             "$HOME/.local/bin/yggterm-headless" "$HOME/.yggterm/bin/yggterm-headless"; do
      [ -n "$c" ] && [ -x "$c" ] && { bin="$c"; break; }
    done
    running=""
    if [ -n "$bin" ]; then
      # ⛔ PARSE A DAEMON LISTING BY FIELD NAME OR NOT AT ALL. `server daemons`
      # renders a table for a human and its BUILD column is only the fourth
      # field while the first daemon listed is the starred one — which it is
      # not, in exactly the window that matters (after a swap the outgoing
      # daemon lists first, unstarred, and every column shifts). The roll loop
      # read an UPTIME as a build id that way for weeks. The JSON carries
      # `build_commit` and `is_default_endpoint` and always did.
      running=$(timeout 8 "$bin" server daemons --json 2>/dev/null \
        | tr -d " \",|" \
        | awk -F: "
            /^endpoint:/            { ep=\$2; sub(/.*\//, \"\", ep) }
            /^build_commit:/        { bc=\$2 }
            /^is_default_endpoint:/ { df=\$2 }
            /^owned_terminal_session_count:/ { oc=\$2 }
            /^}/ { if (ep != \"\") {
                     printf \"%s%s build=%s owned=%s%s\", (n++ ? \"; \" : \"\"), ep,
                            (bc == \"\" ? \"?\" : bc), (oc == \"\" ? \"?\" : oc),
                            (df == \"true\" ? \" (default)\" : \"\")
                   }
                   ep=\"\"; bc=\"\"; df=\"\"; oc=\"\" }
          ")
    fi
    printf "%s|%s\n" "${installs# }" "$running"
  '
  if [ "$host" = "$SELF" ]; then
    bash -lc "$script" 2>/dev/null
  else
    ssh -o ConnectTimeout=6 -o BatchMode=yes "$host" "$script" 2>/dev/null
  fi
}

newest=""
declare -A DAEMON_BIN
declare -A SPLIT
for host in "${MESH[@]}"; do
  raw="$(probe_host "$host")" || raw=""
  [ -z "$raw" ] && { report "fleet-daemon-audit: $host unreachable — not audited."; continue; }
  installs="${raw%%|*}"
  running="${raw#*|}"

  # Every DISTINCT version present in any install on this host. One entry means
  # the host agrees with itself; more than one is a SPLIT INSTALL, and that is
  # the finding — the specific numbers matter less than the fact of the split.
  versions="$(printf '%s\n' $installs | sed 's/.*=//' | grep -v '^?$' | sort -Vu)"
  count="$(printf '%s\n' "$versions" | grep -c . )"
  # The host's own newest install is its reference for the fleet comparison.
  hostnewest="$(printf '%s\n' "$versions" | tail -1)"
  [ -n "$hostnewest" ] && DAEMON_BIN["$host"]="$hostnewest"
  if [ -z "$newest" ] || { [ -n "$hostnewest" ] && \
       [ "$(printf '%s\n%s\n' "$newest" "$hostnewest" | sort -V | tail -1)" = "$hostnewest" ]; }; then
    [ -n "$hostnewest" ] && newest="$hostnewest"
  fi

  if [ "$count" -gt 1 ]; then
    SPLIT["$host"]=1
    report "fleet-daemon-audit: ⛔ $host SPLIT INSTALL — $(printf '%s' "$versions" | tr '\n' '/' | sed 's|/$||')"
    # Name the exact paths, because "which copy is stale" is the whole question
    # and a bare version cannot answer it.
    for entry in $installs; do
      report "fleet-daemon-audit:     ${entry%%=*} = ${entry##*=}"
    done
  else
    report "fleet-daemon-audit: $host installs agree at ${hostnewest:-?}"
  fi
  running="$(printf '%s' "$running" | tr -d '\n')"
  [ -n "${running// /}" ] && report "fleet-daemon-audit:     daemons: $running"
done

# The whole point: name the hosts that are BEHIND, in one line a person reads.
behind=()
for host in "${!DAEMON_BIN[@]}"; do
  v="${DAEMON_BIN[$host]}"
  [ "$v" = "?" ] && continue
  [ "$v" != "$newest" ] && behind+=("$host($v)")
done
if [ ${#behind[@]} -gt 0 ]; then
  report "fleet-daemon-audit: ⚠ BEHIND $newest — ${behind[*]}. Deploy is DELIBERATE (field guide §4); this hook never does it for you."
fi
# ⛔ The split is the headline, and it OUTRANKS "everyone is current": a host can
# hold the newest build at three paths and still run an old one at the fourth,
# which is exactly how "all audited hosts on 3.0.44" was printed over a dev that
# ran 3.0.40 for every `yggterm` a delegate typed.
# ⛔ `${#SPLIT[@]}` ON A DECLARED-BUT-NEVER-ASSIGNED ASSOCIATIVE ARRAY IS AN
# `unbound variable` UNDER `set -u`, so this line crashed the hook on exactly one
# input: the one where there is nothing to report. It ran for weeks without
# hitting it because some host WAS split every time; the first clean fleet broke
# it. ⇒ The success path of a checker is its least-exercised path, and a checker
# that dies when it passes reads as a broken fleet.
#
# ⚠ `${!SPLIT[@]}` (KEYS) is safe on the same empty array where `${#SPLIT[@]}`
# (COUNT) is not — measured, not assumed. Do NOT "harden" it to `${!SPLIT[*]-}`:
# combining `!` with a default turns key expansion into INDIRECT expansion, which
# then tries to use the array's VALUES as a variable name and dies with
# `1 1: invalid variable name` on precisely the split case this branch exists for.
# ─── THE SCRIPTS HAVE THE SAME DISEASE, AND NOTHING WAS LOOKING ────────────
#
# ⛔ Everything above audits the BINARIES. The fleet scripts
# (`.agents/skills/yggterm-agent-fleet/*.py`) live INSIDE the repo, so every git
# WORKTREE ships its own copy, and which copy runs is decided per invocation.
#
# ⛔⛔ CORRECTED 2026-08-21 20:55 — THIS CHECK WAS REPORTING ITSELF. It derived the
# file from the matched process's CWD, on the stated belief that the watcher is
# invoked by a RELATIVE path. The live watcher's argv is ABSOLUTE
# (`python3 /home/user/gh/<tree>/.agents/skills/.../ygg-booter.py watch …`), so cwd
# decided nothing — and worse, `pgrep -f "ygg-booter.py watch"` matches any shell
# whose COMMAND STRING contains that text, which includes THIS HOOK'S OWN
# `bash -lc` subshell. So the probe found its own session, read that session's
# worktree as the watcher's, and reported "the watcher runs from here" to whatever
# tree the reader happened to be sitting in. Two sessions started at once and it
# named both. The `(detached)` in the message was not a measurement either: `br`
# was never assigned, so every row printed the `:-detached` default.
# ⇒ **Identify, do not pattern-match.** A process IS the booter only if argv[0] is
# a python and argv[1] is the script itself; the path in ARGV is then the copy that
# is running, and nothing else has to be inferred.
#
# Measured 2026-08-21: a booter fix was committed and 1 of 5 worktrees had it;
# the running watcher sat in a worktree that did not. These scripts are the
# fleet's INSTRUMENTS — a lane running a stale copy gets stale answers ABOUT
# OTHER LANES and cannot tell, because the script reports success in its own
# terms either way.
#
# ⚠ WHAT IS DELIBERATELY *NOT* REPORTED: worktrees disagreeing with EACH OTHER.
# They sit on different branches by design, so that fires every session on a
# healthy fleet, and an audit that cries every time is one nobody reads. The
# finding is narrower and actually actionable: **the copy the live watcher is
# EXECUTING is behind `origin/main`** — superseded instrument code still running,
# which persists indefinitely because a worktree only updates when someone
# rebases it. Unmerged work on a lane branch is NOT a finding; that is just
# unmerged work.
#
# ⛔ REPORTS ONLY, like everything else here. Never rebases, never copies a file
# into a worktree — those are live checkouts holding other lanes' uncommitted
# work, and writing into one is how you destroy a session's work by helping.
probe_watcher_script() {
  local host="$1"
  local script='
    for d in /proc/[0-9]*; do
      [ -r "$d/cmdline" ] || continue
      argv=$(tr "\0" "\n" < "$d/cmdline" 2>/dev/null) || continue
      [ -n "$argv" ] || continue
      a0=$(printf "%s\n" "$argv" | sed -n 1p)
      a1=$(printf "%s\n" "$argv" | sed -n 2p)
      # ⛔ THE PROCESS MUST *BE* THE BOOTER, NOT MERELY MENTION IT. A substring
      # match on the command string finds this hook own subshell and every agent
      # shell that ever echoed the name.
      case "${a0##*/}" in python|python2|python3|python3.*) ;; *) continue ;; esac
      case "${a1##*/}" in ygg-booter.py) ;; *) continue ;; esac
      printf "%s\n" "$argv" | grep -qx "watch" || continue
      # The path in ARGV is the copy that is executing. Only fall back to cwd when
      # argv actually carries a relative path.
      case "$a1" in
        /*) f="$a1" ;;
        *)  f="$(readlink "$d/cwd" 2>/dev/null)/$a1" ;;
      esac
      [ -f "$f" ] || continue
      tree=$(git -C "$(dirname "$f")" rev-parse --show-toplevel 2>/dev/null) || continue
      [ -n "$tree" ] || continue
      rel=${f#"$tree"/}
      br=$(git -C "$tree" branch --show-current 2>/dev/null)
      # ⛔ `origin/main` IS A LOCAL CACHE until fetched — a remote-tracking ref is
      # a note of where main was when THIS checkout last spoke to the remote, so
      # without this the comparison silently answers a stale question. Bounded
      # and silent: a startup hook may not hang on a network and may never fail a
      # session, so an offline remote just leaves the cached ref in place.
      timeout 8 git -C "$tree" fetch -q --no-tags origin main 2>/dev/null || true
      # ⛔ "DIFFERS FROM MAIN" IS NOT "BEHIND MAIN". A watcher running out of a
      # LANE worktree legitimately carries unmerged work, so a hash comparison
      # flags it every session for being AHEAD — which is how an audit teaches
      # people to ignore it. This asks only whether main holds commits FOR THIS
      # FILE that the worktree lacks, and stays silent for a branch merely ahead.
      behindlog=$(git -C "$tree" log --oneline origin/main --not HEAD -- "$rel" 2>/dev/null | head -3)
      [ -n "$behindlog" ] || continue
      missing=$(printf "%s" "$behindlog" | head -1 | cut -c1-72)
      nmiss=$(printf "%s\n" "$behindlog" | grep -c .)
      echo "${tree}|${br:-detached}|${nmiss}|${missing}"
    done | sort -u
  '
  if [ "$host" = "$SELF" ]; then
    bash -lc "$script" 2>/dev/null
  else
    ssh -o ConnectTimeout=6 -o BatchMode=yes "$host" "$script" 2>/dev/null
  fi
}

for host in "${MESH[@]}"; do
  drift="$(probe_watcher_script "$host")" || drift=""
  [ -z "${drift// /}" ] && continue
  while IFS="|" read -r wcwd wbr wnmiss wnewest; do
    [ -n "$wcwd" ] || continue
    report "fleet-daemon-audit: ⚠ $host watcher runs a ygg-booter.py BEHIND main — $wcwd ($wbr) is missing $wnmiss commit(s), newest: $wnewest"
    report "fleet-daemon-audit:     that path is the copy in the watcher own argv; rebase that worktree (never copy the file in)."
  done <<< "$drift"
done

split_hosts="${!SPLIT[@]}"
if [ -n "$split_hosts" ]; then
  report "fleet-daemon-audit: ⛔ $(set -- $split_hosts; echo $#) host(s) hold DISAGREEING copies ($split_hosts). \
Do NOT trust one path's --version as the host's version; fix the deploy to write every copy."
elif [ ${#behind[@]} -eq 0 ]; then
  report "fleet-daemon-audit: all audited hosts agree at $newest."
fi
exit 0
