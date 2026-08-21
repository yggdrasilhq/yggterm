#!/usr/bin/env bash
# verify-input-block-fixes — the owed live proof for the input-block strike,
# as a checklist a roller can run instead of rediscovering it.
#
# ⛔ WHY THIS EXISTS AND WHY IT IS A SCRIPT, NOT A PARAGRAPH. Five fixes shipped
# in code with "LIVE PROOF OWED" against them, and each is observable only
# through a query that is not obvious: a probe name nobody would guess, a state
# field whose name does not say what it means, a counter that must be read
# BESIDE another one or it misleads. Prose falsifiers in the queue say what to
# look for; they do not say where. The gap between those two is where owed proof
# goes to die, because the person who rolls the build is never the person who
# wrote the fix.
#
# ⚠ IT PROVES NOTHING BY ITSELF. Four of the five need a human to make the thing
# happen (raise a picker, open a row mid-adoption). This script tells you what to
# do, then reads the instrument that answers — it is a checklist with the queries
# pre-loaded, not an oracle.
#
# USAGE
#   scripts/verify-input-block-fixes.sh [--host <gui-host>]
#     --host H   the host whose GUI/daemon to read; default: scripts/ygg-live-host.sh
#
# EXIT  0 every automatic check passed (the manual ones are printed, not judged)
#       1 an automatic check failed · 2 could not reach the host
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2

HOST=""
while [ $# -gt 0 ]; do
  case "$1" in
    --host) HOST="${2:-}"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ -n "$HOST" ] || HOST="$(scripts/ygg-live-host.sh 2>/dev/null)"
[ -n "$HOST" ] || { echo "verify: no GUI host — pass --host" >&2; exit 2; }

fail=0
say()  { printf '\n== %s\n' "$*"; }
ok()   { printf '   ✅ %s\n' "$*"; }
bad()  { printf '   ❌ %s\n' "$*"; fail=1; }
todo() { printf '   ⌛ BY HAND: %s\n' "$*"; }

run() { ssh "$HOST" "$@" 2>/dev/null; }

# ── 0. The build under test ──────────────────────────────────────────────────
#
# ⛔ FIRST, ALWAYS. A same-version rebuild is never adopted (its own queue
# entry), so every reading below can describe code that is not running. Compare
# the daemon's own answer against the binary on disk before believing anything.
say "0. which build is answering"
# ⛔ VERB NAMES ARE MEASURED, NOT GUESSED. `server status --json` answers
# "unsupported server command: status" — the flag mis-routes the parse — and the
# field is `server_version`, not `version`. Both were wrong in this script's
# first draft and it reported "no daemon answered" against a perfectly healthy
# daemon: blind read as broken, which is the failure this repo keeps paying for.
running="$(run '~/.local/bin/yggterm-headless server status' | python3 -c \
  'import json,sys
try: d=json.load(sys.stdin)
except Exception: raise SystemExit
d=d.get("data",d)
print(d.get("server_version",""))' 2>/dev/null)"
ondisk="$(run '~/.local/bin/yggterm-headless --version' | awk '{print $NF}')"
if [ -z "$running" ]; then
  bad "no daemon answered on $HOST — nothing below can be trusted"
elif [ "$running" = "$ondisk" ]; then
  ok "daemon $running matches the binary on disk"
else
  bad "daemon is $running but the binary on disk is $ondisk — roll it first"
fi

# ── 1. The seed re-asks instead of leaving a black canvas ────────────────────
say "1. blank viewport: the seed re-asks, and refuses in words if it cannot"
# ⛔ THE "A ROLL ALREADY STAGES THIS" ADVICE WAS WRONG AND IS WITHDRAWN (measured
# 2026-08-21, 11.14). The 3.1.16 roll produced ZERO retained_rehydrate events of
# any kind, and so did a deliberate GUI restart afterwards. Two reasons, both
# measured:
#   1. THE ROW MUST BE A REMOTE **RESUME**, NOT MERELY A REMOTE ROW. The gate is
#      `is_remote_resume_agent_session`; `terminal_live_host_connected` is seeded
#      `!is_remote_resume_session`, so a freshly CREATED remote row (start-cc)
#      starts "connected" and the seed never arms. A revealed resume row armed it
#      on the first try.
#   2. THE HEALTHY ATTACH IS ~300 ms (`request_to_ready_ms`), so there is no
#      window to race by hand.
# ⇒ STAGE IT AS: reveal a retained remote RESUME row (`remote-cc://<host>/<uuid>`
#   whose Remote Launch Action is a resume). That reliably produces
#   begin → daemon_ready_wait → end. It does NOT produce `empty`: an empty answer
#   needs the daemon to hold no paintable screen, which a healthy fleet will not
#   do on demand.
todo "reveal a retained remote RESUME row (NOT a freshly created one — see the note above)."
echo "      ⚠ a converged run (begin → end) proves the path RUNS; it does NOT prove the retry"
echo "        ladder, which needs a real empty answer and cannot be manufactured on a healthy fleet."
echo "      the shapes, in order, on the row that came up blank:"
echo "        ytrace ... terminal_mount/retained_rehydrate_empty       (the answer that used to end it)"
echo "        ytrace ... terminal_mount/retained_rehydrate_retry_scheduled  (delay_ms 2500 → 60000)"
echo "        ytrace ... terminal_mount/retained_rehydrate_end          (converged — the good ending)"
echo "      only if it never converges:"
echo "        ytrace ... terminal_mount/retained_rehydrate_refused      (attempts, recovery_lane_armed)"
echo "      ⛔ A BLANK CANVAS WITH NO retry_scheduled BESIDE THE empty IS THE BUG UNFIXED."
seed_probe="$(run '~/.local/bin/ytrace tail --lines 4000' \
  | grep -c 'retained_rehydrate_retry_scheduled\|retained_rehydrate_refused' 2>/dev/null | tail -1)"
[ -n "$seed_probe" ] || seed_probe=0
echo "      (this window already holds $seed_probe retry/refuse events)"

# ── 2. A question picker is its own state ────────────────────────────────────
say "2. picker: neither working nor idle, and the GUI names the mode"
# ⭐ MEASURED 2026-08-21 (11.14): stage this on a THROWAWAY row you spawn with
# `terminal new --no-activate --ephemeral`, never on an owner row — the readers
# below are all daemon-side or GUI-state, so nothing needs the owner's viewport.
# ⚠ Two of the six readers cannot be read back by an agent at all: the right-rail
# Status string is exposed by no app-control verb, and a row's title cannot be
# blanked, so the empty-title card branch stays code-only.
# ⚠ Allow ~40-50 s: the dot and the card lag the daemon flag on a GUI reporting
# `background_refresh_suspended: true`.
todo "raise an owner question on any Claude Code row, then read all three:"
echo "        server gate-screen <row> --tail 60   → screen_shows_question_picker: true"
echo "        server snapshot                      → that row's awaiting_user_choice: true"
echo "        the row's dot stays lit, and the metadata Status reads 'asking you a question'"
echo "      and a card must appear naming the keys that answer it."
echo "      ⛔ MEASURED 2026-08-21: the two working readers DISAGREE and that is expected."
echo "         server snapshot  → working: FALSE (this CLI's descriptor; the phrase left the screen)"
echo "         server gate-screen → screen_text_shows_agent_working: TRUE (kind-agnostic union)"
echo "         Trust the snapshot for a known CLI. And note the misread the fourth state"
echo "         prevents is 'this row looks IDLE/DONE', not 'this row looks busy'."

# ── 3. The webview ack ladder ────────────────────────────────────────────────
say "3. webview edit plane: read acks_late FIRST or the numbers mislead"
# ⛔ THE STATE DOCUMENT GOES IN ON STDIN, NEVER AS AN ARGUMENT. It is far past
# the argv limit, and passing it as one produced `Argument list too long` — a
# failure that printed beside the other checks and did not fail the script.
if ! run '~/.local/bin/yggterm-headless server app state' > "/tmp/ygg-verify-state.$$" 2>/dev/null \
   || [ ! -s "/tmp/ygg-verify-state.$$" ]; then
  bad "no app state — is the GUI up? (the verb is 'server app state')"
else
  # ⛔ THE PATH GOES IN ARGV AND THE PROGRAM ON STDIN — they cannot share fd 0.
  # `python3 - < state.json <<PY` looks like it feeds both; the heredoc wins, the
  # program reads an exhausted stdin, and the failure reads as "could not parse",
  # i.e. as a broken payload rather than as a broken invocation.
  python3 - "/tmp/ygg-verify-state.$$" <<'PYEOF' || fail=1
import json, sys
try:
    with open(sys.argv[1]) as handle:
        d = json.load(handle)
except Exception:
    print("   ❌ could not parse app state"); raise SystemExit(1)
d = d.get("data", d)
keys = ("webview_edit_faults", "webview_edit_flush_timeouts", "webview_edit_acks_late",
        "webview_edit_gate_bypasses", "webview_edit_resync_requests")
missing = [k for k in keys if d.get(k) is None]
if missing:
    print("   ❌ recovery counters missing — this GUI predates the fix:", ", ".join(missing))
    raise SystemExit(1)
for k in keys:
    print(f"      {k}: {d[k]}")
t, late = d["webview_edit_flush_timeouts"], d["webview_edit_acks_late"]
if t == 0:
    print("   ✅ no flush-gate timeouts in this GUI's life")
elif late > 0:
    print("   ✅ timeouts WITH late acks = a slow webview whose DOM caught up, not a stale UI")
else:
    print("   ⚠ timeouts and NO late ack — the ack plane may be dead; expect a resync request")
if d["webview_edit_faults"]:
    print("   ⚠ edit_faults is the one that really is divergence, and it is restart-only")
PYEOF
  rm -f "/tmp/ygg-verify-state.$$"
fi

# ── 4 & 5. The two 1 Hz ticks ────────────────────────────────────────────────
#
# The only honest instrument here is the render-attribution window: both fixes
# REMOVE a cause, so the proof is a cause that stops appearing while the
# condition that used to arm it is held.
say "4+5. the 1 Hz ticks: a held condition that no longer buys renders"
todo "focus a terminal row, leave a restore/refusal card up, and keep the window FOCUSED"
echo "      (focus matters: the old leak was cleared by unfocusing, so an unfocused"
echo "       window hides the very bug you are checking)"
echo "      then read the attribution window:"
echo "        ytrace tail --lines 2000 | grep dioxus_render/component_window"
echo "      ⛔ 'input_gate_deadline_tick' must NOT appear once per second while the"
echo "         card sits still. A stage advance may take one — that is the bar moving."

printf '\n'
if [ "$fail" -eq 0 ]; then
  echo "verify: automatic checks passed; the ⌛ items still need a human."
else
  echo "verify: an automatic check FAILED — see ❌ above." >&2
fi
exit "$fail"
