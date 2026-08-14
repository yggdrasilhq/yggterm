#!/usr/bin/env bash
# Resource panic watch — the standing alarm for seat 6.7.
#
#   scripts/ygg-resource-panic.sh              # measure + report; exit 1 on PANIC
#   scripts/ygg-resource-panic.sh --notify     # also raise a toast on breach
#   scripts/ygg-resource-panic.sh --json
#
# Owner directive 2026-08-14: *measure the fan, and over a threshold give panic
# prompts — likewise resource utilisation — so the seat stays vigilant on the
# growing resource and jank problem.* Priority order, his: **MEMORY first, CPU
# second, SPACE third.** The thresholds below are ordered and reported that way.
#
# ⛔ THERE IS NO FAN RPM ON THIS MACHINE, AND PRETENDING OTHERWISE WOULD BE THE
#    WORST KIND OF INSTRUMENT. `acpi_fan` exposes `fan1_input = 0` permanently
#    and there is no `thinkpad_acpi`; a check reading that field would report a
#    silent fan while the machine screams. So the fan is measured by its two
#    CAUSES, which are readable and real:
#      * **package power (PPT)** — sustained watts is what a fan curve tracks
#      * **die temperature (Tctl)**
#    ⇒ If a real tachometer ever appears, use it and delete this paragraph.
#
# ⚠ And the remedy is NOT the power profile. Capping the platform to `balanced`
#   does cut temperature — and it is a CHEAT FIX, ruled out by the owner because
#   it buys the heat back by making the whole UI ~20% more sluggish. See
#   `docs/settled-calls.md`. A remedy that degrades the product is not a remedy.

set -uo pipefail

NOTIFY=0
JSON=0
HOST=""
while [ $# -gt 0 ]; do
  case "$1" in
    --notify) NOTIFY=1 ;;
    --json)   JSON=1 ;;
    --host)   shift; HOST="${1:-}" ;;
    -h|--help) sed -n '2,28p' "$0"; exit 0 ;;
  esac
  shift
done

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [ -z "$HOST" ]; then
  HOST="$("$REPO_ROOT/scripts/ygg-live-host.sh" 2>/dev/null)"
  [ -z "$HOST" ] && { echo "cannot resolve the live host" >&2; exit 64; }
fi
SSH="ssh -o BatchMode=yes -o ConnectTimeout=10 $HOST"

# ---- thresholds -------------------------------------------------------------
# PANIC is "wake the seat", not "something is slightly high". A threshold that
# fires on ordinary work trains its reader to ignore it, which is how the real
# breach gets missed.
SWAP_USED_PANIC_GB=6          # memory: the owner's report was 11 of 15 GB
MEM_AVAIL_PANIC_GB=2
GUI_COMMITTED_PANIC_MB=2500   # rss+swap. ⛔ NEVER rss alone: it FALLS while the
                              #   footprint climbs, which is how the leak hid.
MEM_PRESSURE_PANIC=5.0        # PSI `some avg60`, %. Above this, tasks are
                              #   actually stalling on memory — which is what a
                              #   human feels. Swap USED is not that; see below.
PPT_AVG_PANIC_W=25            # fan proxy: sustained package watts
TCTL_PANIC_C=85
YGG_CORES_PANIC=2.0           # cpu: our whole family, as a rate not a lifetime avg
RENDER_STORM_PANIC=20         # renders/s. Deliberately the SAME number as the
                              #   app's own `app_render_storm` arm rate, so the
                              #   watch and the app cannot disagree about the
                              #   word. Rest is 0.7-1.2/s; storms run 54-64/s.
TMPFS_OURS_PANIC_MB=256       # space: our footprint on any RAM-backed mount

M="$($SSH '
sw_t=$(awk "/^SwapTotal/{print \$2}" /proc/meminfo); sw_f=$(awk "/^SwapFree/{print \$2}" /proc/meminfo)
echo "swap_used_gb=$(echo "scale=2; ($sw_t-$sw_f)/1048576" | bc)"
echo "mem_avail_gb=$(awk "/^MemAvailable/{printf \"%.2f\", \$2/1048576}" /proc/meminfo)"
# ⛔ SWAP USED IS AN ACCUMULATION, NOT A PRESSURE, AND THE TWO NEED DIFFERENT
# ANSWERS. Measured 2026-08-14: 6.35 GB of swap with 9.8 GB available, no
# process near a limit and the GUI cgroup at memory.events high=0 — cold pages
# the kernel parked at swappiness=60 and never needed back. That is not the
# same event as a machine paging to keep up, and reporting one number for both
# makes the alarm fire hardest when nothing is wrong. PSI is the discriminator:
# `some avg60` is the share of the last minute in which at least one task
# STALLED waiting for memory. An empty file (no PSI in this kernel) reports
# blind rather than 0 — BLIND IS NOT CALM.
psi=$(awk "/^some /{for(i=1;i<=NF;i++) if (\$i ~ /^avg60=/) {sub(/avg60=/,\"\",\$i); print \$i; exit}}" /proc/pressure/memory 2>/dev/null)
echo "mem_pressure_avg60=${psi:-blind}"
ppt=$(sensors 2>/dev/null | grep -m1 "PPT:" | grep -oE "avg =[ ]*[0-9.]+" | grep -oE "[0-9.]+")
echo "ppt_avg_w=${ppt:-0}"
tctl=$(sensors 2>/dev/null | grep -m1 "Tctl:" | grep -oE "[0-9.]+" | head -1)
echo "tctl_c=${tctl:-0}"
gui=$(~/.local/bin/yggterm server app clients 2>/dev/null | python3 -c "
import sys,json
try:
    d=json.load(sys.stdin)
    a=[c[\"pid\"] for c in (d.get(\"clients\") or []) if (c.get(\"client_role\") or \"active\")==\"active\"]
    print(a[0] if a else \"\")
except Exception: print(\"\")")
if [ -n "$gui" ]; then
  r=$(awk "/^VmRSS/{print \$2}" /proc/$gui/status 2>/dev/null)
  s=$(awk "/^VmSwap/{print \$2}" /proc/$gui/status 2>/dev/null)
  echo "gui_committed_mb=$(( (${r:-0}+${s:-0})/1024 ))"
  # the web child is the actual leaker; report it beside the GUI
  for c in $(ps --ppid $gui -o pid= 2>/dev/null); do
    if grep -q WebKitWebProcess /proc/$c/comm 2>/dev/null || [ "$(cat /proc/$c/comm 2>/dev/null)" = "WebKitWebProces" ]; then
      wr=$(awk "/^VmRSS/{print \$2}" /proc/$c/status 2>/dev/null)
      ws=$(awk "/^VmSwap/{print \$2}" /proc/$c/status 2>/dev/null)
      echo "web_committed_mb=$(( (${wr:-0}+${ws:-0})/1024 ))"
      cg=$(awk -F: "{print \$3}" /proc/$gui/cgroup 2>/dev/null | head -1)
      echo "gui_memory_high=$(cat /sys/fs/cgroup$cg/memory.high 2>/dev/null || echo unknown)"
    fi
  done
else
  echo "gui_committed_mb=0"; echo "web_committed_mb=0"; echo "gui_memory_high=no-gui"
fi
# our CPU as a RATE over 8s (ps %CPU is a LIFETIME average and would lie)
#
# ⛔⛔ DIFFERENCE PER PID, NEVER TWO SUMS. A process that exits between the two
# samples takes its whole lifetime of ticks OUT of the second sum, so a
# sum-to-sum difference goes negative — once seen as "-.06 cores" while the
# family was busy. That was cured by clamping at zero, and the clamp is a WORSE
# instrument than the bug: on this host the pattern also matches every
# short-lived `yggterm` CLI invocation, of which there are dozens a minute, so
# an exit inside the window is the common case and the clamp turns it into a
# confident `0 cores` — a reading that can never trip its own threshold and
# reads as "the GUI is free" while it burns a core. Measured 2026-08-14: this
# printed `0` in the same run in which the GUI own sampler reported 0.39 cores.
# ⇒ Intersecting the two samples costs nothing and loses only the share of
#   the process that exited, which is the honest amount to lose.
sample() { for q in $(pgrep -f "yggterm|WebKitWebProcess" 2>/dev/null); do
    v=$(awk "{print \$14+\$15}" /proc/$q/stat 2>/dev/null) && echo "$q $v"; done; }
a=$(sample); t1=$(awk "/^cpu /{print \$2+\$3+\$4+\$5+\$6+\$7+\$8}" /proc/stat); sleep 8
b=$(sample); t2=$(awk "/^cpu /{print \$2+\$3+\$4+\$5+\$6+\$7+\$8}" /proc/stat)
d=$(awk "NR==FNR{first[\$1]=\$2; next} (\$1 in first){s+=\$2-first[\$1]} END{print s+0}" \
      <(echo "$a") <(echo "$b"))
echo "ygg_cores=$(echo "scale=2; d=$d; if (d<0) d=0; d/(($t2-$t1)/$(nproc))" | bc)"
# ⛔ du COUNTS AN UNREADABLE DIRECTORY AS ZERO, AND THAT IS THE FAILURE THIS
# BLOCK EXISTS TO SURVIVE. The pattern below already matched the CLI staging
# dirs; they were owned by root, `du` running as us could not descend into them,
# and the total came back as a confident small number rather than an error.
# Measured 2026-08-14: this reported 269 MB while the true figure was 1,323 MB,
# of which ~1,079 MB was ours. The instrument named the right files and called
# them empty, which is worse than not finding them.
# ⇒ Prefer passwordless sudo. Where it is unavailable, COUNT what we cannot read
# and let the caller report BLIND instead of a number.
DU="du"; sudo -n true 2>/dev/null && DU="sudo -n du"
ours=0; blind=0
for m in $(awk "\$3==\"tmpfs\"{print \$2}" /proc/mounts | sort -u); do
  [ -d "$m" ] || continue
  while IFS= read -r -d "" e; do
    [ "$DU" = "du" ] && [ ! -r "$e" ] && blind=$((blind+1))
  done < <(find "$m" -maxdepth 2 \( -name "ygg*" -o -name "*yggterm*" -o -name "codex-litellm-*" -o -name "claude-*" \) -print0 2>/dev/null)
  v=$(find "$m" -maxdepth 2 \( -name "ygg*" -o -name "*yggterm*" -o -name "codex-litellm-*" -o -name "claude-*" \) -print0 2>/dev/null | $DU -sm --files0-from=- 2>/dev/null | awk "{s+=\$1} END{print s+0}")
  ours=$((ours+${v:-0}))
done
echo "tmpfs_ours_mb=$ours"
echo "tmpfs_blind_entries=$blind"
# ⛔ THE FAN AND THE UNRESPONSIVE MINUTES AFTER A RESTART ARE A RENDER STORM, AND
# THIS WATCH COULD NOT SEE IT. The Dioxus root renders at 0.7-1.2/s at rest and
# has been measured at 54-64/s for nine unbroken minutes, pinning exactly one
# core. `ygg_cores` above does catch that as CPU — but only while it is
# happening, and it cannot say the cost is RENDERING rather than work. The GUI
# already publishes the rate every 60s whether or not anything is wrong; this
# just reads the last one it wrote.
tf=$(ls -t ~/.yggterm/event-trace*.jsonl 2>/dev/null | head -1)
if [ -n "$tf" ]; then
  rr=$(tail -c 400000 "$tf" 2>/dev/null | grep -o "\"renders_per_sec\":[0-9.]*" | tail -1 | cut -d: -f2)
fi
# ⛔ An empty tail is not a calm GUI — it is a window with no sample in it.
echo "renders_per_sec=${rr:-blind}"
' 2>/dev/null)"

g() { sed -n "s/^$1=//p" <<<"$M" | head -1; }
gt() { awk -v a="${1:-0}" -v b="$2" 'BEGIN{exit !(a+0 > b+0)}'; }
lt() { awk -v a="${1:-0}" -v b="$2" 'BEGIN{exit !(a+0 < b+0)}'; }

PANICS=""
p() { PANICS="${PANICS}  PANIC $1"$'\n'; }

SWAP=$(g swap_used_gb);   AVAIL=$(g mem_avail_gb); PSI=$(g mem_pressure_avg60)
GUIC=$(g gui_committed_mb); WEBC=$(g web_committed_mb); HIGH=$(g gui_memory_high)
PPT=$(g ppt_avg_w);       TCTL=$(g tctl_c)
CORES=$(g ygg_cores);     TMPO=$(g tmpfs_ours_mb); RENDERS=$(g renders_per_sec)
TMPBLIND=$(g tmpfs_blind_entries)

# 1 MEMORY
gt "$SWAP" "$SWAP_USED_PANIC_GB"       && p "swap ${SWAP}GB used (> ${SWAP_USED_PANIC_GB}GB)$(
  case "$PSI" in
    blind) echo " — ⚠ memory pressure UNREADABLE, cannot say whether this is felt";;
    *) if gt "$PSI" "$MEM_PRESSURE_PANIC"; then
         echo " — AND pressure avg60=${PSI}%: the machine is paging to keep up"
       else
         echo " — but pressure avg60=${PSI}%, ${AVAIL}GB available: COLD swap, not a stall"
       fi;;
  esac)"
# The other half, and the one that is actually felt: stalling. It can fire with
# swap far below the threshold above, which is why it is its own line.
case "$PSI" in
  blind) ;;
  *) gt "$PSI" "$MEM_PRESSURE_PANIC" && p "memory pressure avg60=${PSI}% — tasks are STALLING on memory";;
esac
lt "$AVAIL" "$MEM_AVAIL_PANIC_GB"      && p "only ${AVAIL}GB available (< ${MEM_AVAIL_PANIC_GB}GB)"
gt "$GUIC" "$GUI_COMMITTED_PANIC_MB"   && p "GUI committed ${GUIC}MB (> ${GUI_COMMITTED_PANIC_MB}MB)"
gt "$WEBC" "$GUI_COMMITTED_PANIC_MB"   && p "web process committed ${WEBC}MB — the known unbounded leak"
[ "$HIGH" = "max" ]                    && p "memory.high = max — THE GUI IS UNBOUNDED, the cgroup cap did not arm"
# 2 CPU / the fan's two causes
gt "$PPT" "$PPT_AVG_PANIC_W"           && p "package power ${PPT}W sustained (> ${PPT_AVG_PANIC_W}W) — this is what spins the fan"
gt "$TCTL" "$TCTL_PANIC_C"             && p "die temperature ${TCTL}C (> ${TCTL_PANIC_C}C)"
gt "$CORES" "$YGG_CORES_PANIC"         && p "yggterm family ${CORES} cores (> ${YGG_CORES_PANIC})"
# The render storm, by its own always-on instrument rather than by inference
# from CPU. Its threshold matches the app's own `app_render_storm` arm rate, so
# the two agree on what the word means instead of disagreeing quietly.
case "$RENDERS" in
  blind) p "the GUI's render rate is UNREADABLE (no app_render_rate sample in the trace tail) — the storm cannot be ruled out, only unmeasured";;
  *) gt "$RENDERS" "$RENDER_STORM_PANIC"  && p "the GUI is rendering ${RENDERS}/s (rest is ~1/s) — this is the render storm: one core, the fan, and the minutes of unresponsiveness after a restart";;
esac
# 1b MEMORY, ON THIS HOST — TWO AGENT PROCESSES ON ONE SESSION UUID
#
# ⛔ A version bump re-resumes a live row into a NEW process and does not reap
# the old one. They land on different ttys, sids and pgids, so they are wholly
# independent and nothing ever notices. Measured 2026-08-14 after the 3.0.157
# deploy: 4 of 4 armed rows had a twin, ~3.2 GB held between them.
#
# ⛔⛔ THIS REPORTS. IT MUST NEVER KILL, AND THE REASON IS NOT CAUTION.
# The obvious cleanup is inverted in the dangerous direction:
#   - the `--session-id` process is the ORIGINAL; the `--resume` process is the
#     one the re-resume created, and the RE-RESUME IS THE LIVE AGENT. A recipe
#     that "kills the twin" kills the session that is running it.
#   - "the duplicate is idle" is NOT reliable. One measured here moved 0.490s of
#     CPU in 5s with a tool call in flight and its children cycling — a working
#     agent, in a shared worktree, whose death would have destroyed real work.
# ⇒ A duplicate is an ALARM. Only the row that OWNS the uuid may act, and only
#   after resolving its own pid FROM INSIDE (walk `ppid` up from `$$` until
#   `comm=claude`) and measuring the other's CPU and children. ⛔ Never
#   `pkill -f <uuid>`: the pattern is in the killing shell's own command line.
#
# ⚠ And the memory is the smaller half. Two agents on one uuid also share one
# worktree and one cargo `target/`, which is the shared-checkout clobber hazard.
DUPES=""
for p in $(pgrep -x claude 2>/dev/null); do
  u=$(tr '\0' ' ' < "/proc/$p/cmdline" 2>/dev/null \
    | grep -oE '[0-9a-f]{8}-([0-9a-f]{4}-){3}[0-9a-f]{12}' | head -1)
  [ -n "$u" ] && DUPES="$DUPES$u"$'\n'
done
DUPE_UUIDS=$(printf '%s' "$DUPES" | sort | uniq -d | wc -l)
[ "${DUPE_UUIDS:-0}" -gt 0 ] && p "${DUPE_UUIDS} session uuid(s) have MORE THAN ONE live claude process on $(hostname) — duplicate agents, ~900MB each. ⛔ REPORT ONLY: the re-resumed process is the LIVE one and a 'twin' may be busy; only the owning row may act"
# 3 SPACE
gt "$TMPO" "$TMPFS_OURS_PANIC_MB"      && p "${TMPO}MB of ours on tmpfs (> ${TMPFS_OURS_PANIC_MB}MB) — that is RAM"
# BLIND IS NOT CLEAR. An unreadable entry makes the figure above a FLOOR, not a
# measurement, so say so rather than letting a small number read as safety.
gt "$TMPBLIND" 0                       && p "${TMPBLIND} tmpfs entr(ies) of ours are UNREADABLE — ${TMPO}MB is a FLOOR, not a measurement (no passwordless sudo here)"

SUMMARY="mem: swap ${SWAP}GB, avail ${AVAIL}GB, pressure ${PSI}%, gui ${GUIC}MB, web ${WEBC}MB, cap ${HIGH} | cpu: ${CORES} cores, ${PPT}W, ${TCTL}C, render ${RENDERS}/s | space: ${TMPO}MB on tmpfs"

if [ "$JSON" -eq 1 ]; then
  printf '{"host":"%s","panic":%s,"summary":"%s","panics":"%s"}\n' \
    "$HOST" "$([ -z "$PANICS" ] && echo false || echo true)" "$SUMMARY" \
    "$(printf '%s' "$PANICS" | tr '\n' ';' | sed 's/"/\\"/g')"
else
  echo "=== resource panic watch ($HOST)"
  echo "  $SUMMARY"
  [ -n "$PANICS" ] && printf '%s' "$PANICS"
  [ -z "$PANICS" ] && echo "  all thresholds clear"
fi

if [ -n "$PANICS" ] && [ "$NOTIFY" -eq 1 ]; then
  # ⛔ A notification the seat raises for ITSELF still lands on the owner's
  #    screen if untargeted, because mutating verbs prefer the Active client.
  #    This one is FOR him — the machine is in trouble — so it is deliberate.
  $SSH "~/.local/bin/yggterm server app notify 'yggterm resource panic' \
        $(printf '%q' "$(printf '%s' "$PANICS" | tr '\n' ' ')")" >/dev/null 2>&1
fi

[ -z "$PANICS" ]
