#!/bin/bash
# ygg-claim — a session claims its own row: title, seat number, and (optionally)
# the row it is replacing.
#
# WHY THIS EXISTS
#   An agent session is born with a title its CLI invented and a seat at the head
#   of the sidebar. Both are wrong for campaign work: the row a human most needs
#   to find is the one running the campaign, and a launch that lands at the head
#   leaves the whole outline scrambled until someone repairs it by hand.
#
#   Every session that fixed this by hand re-derived the same five steps from
#   primitives, and the failure was never the rename — it was the parts around it:
#     · the CLI composes its OWN title when its first turn ends, CLOBBERING an
#       earlier rename, so a one-shot rename does not hold;
#     · the remove verb reports the REQUEST, not the EFFECT — it can delist a row
#       while the agent process keeps running on the remote host;
#     · `pgrep -cf <uuid>` counts the querying shell, so it reports a dead row
#       alive.
#   Those are three different lessons and a session should not have to learn them.
#
# USAGE
#   ygg-claim.sh --title "<topic>: <what this session is for>" [options]
#
#     --title T          required; the human-facing name for this row
#     --number N         seat number. Omit to derive one (see --campaign)
#     --campaign TOKEN   sibling-matching token used to derive the number and to
#                        find a predecessor; defaults to the first word of --title
#     --replace UUID     retire this predecessor row after claiming the seat
#     --inherit-number   take the predecessor's number instead of deriving one
#     --session UUID     claim a row other than your own (default: $YGGTERM_SESSION_ID)
#     --watch-secs N     keep re-asserting the title for N seconds (default 240)
#     --booter           subscribe this row to the booter, so that a STALL is
#                        woken from outside. ⛔ OFF by default — arm it only for
#                        relay/unattended work (see "ARM THE BOOTER" below)
#     --no-booter        accepted and ignored; kept so older call sites still run
#     --host H           GUI host; default: auto-detect, then $YGG_GUI_HOST
#     --dry-run          print what would happen, change nothing
#
# EXIT  0 claimed (and predecessor retired, if asked) · 2 no row found
#       3 rename never verified · 4 predecessor survived reaping
set -uo pipefail
STATE_DIR="$HOME/.yggterm/relay"

TITLE=""; NUMBER=""; CAMPAIGN=""; REPLACE=""; INHERIT=0
SESSION="${YGGTERM_SESSION_ID:-}"; WATCH=240; HOST="${YGG_GUI_HOST:-}"; DRY=0
# ⛔ OFF unless asked. A session that claims a row is not thereby unattended —
# see "ARM THE BOOTER" below for why this default was inverted 2026-08-10.
BOOTER="${YGG_BOOTER:-0}"

while [ $# -gt 0 ]; do
  case "$1" in
    --title)          TITLE="${2:-}"; shift 2 ;;
    --number)         NUMBER="${2:-}"; shift 2 ;;
    --campaign)       CAMPAIGN="${2:-}"; shift 2 ;;
    --replace)        REPLACE="${2:-}"; shift 2 ;;
    --inherit-number) INHERIT=1; shift ;;
    --booter)         BOOTER=1; shift ;;
    --no-booter)      BOOTER=0; shift ;;
    --session)        SESSION="${2:-}"; shift 2 ;;
    --watch-secs)     WATCH="${2:-}"; shift 2 ;;
    --host)           HOST="${2:-}"; shift 2 ;;
    --dry-run)        DRY=1; shift ;;
    -h|--help)        sed -n '2,40p' "$0"; exit 0 ;;
    *) echo "ygg-claim: unknown argument: $1" >&2; exit 64 ;;
  esac
done

[ -n "$TITLE" ]   || { echo "ygg-claim: --title is required" >&2; exit 64; }
[ -n "$SESSION" ] || { echo "ygg-claim: no session id (\$YGGTERM_SESSION_ID unset, and no --session)" >&2; exit 64; }
UUID="${SESSION##*/}"
[ -n "$CAMPAIGN" ] || CAMPAIGN="$(printf '%s' "$TITLE" | awk '{print $1}' | tr -d ':' )"

# ⛔⛔ NEVER REAP YOURSELF. A brief hands the successor a `PREDECESSOR TO REAP`
# uuid as a literal — and if the "successor" was started as an in-process helper
# rather than as its own PTY session, it INHERITS the parent's session id, so
# that literal is its own. The spawn succeeds, the transcript appears and the ACK
# token is present, because the brief really was delivered; it was simply
# delivered to something that is not a separate row. Reported by another campaign
# 2026-08-13 and survived only because the kill happened to be the first act.
# ⇒ The guard belongs in the TOOL, not in the operator: discipline resets every
#    session and a check does not.
if [ -n "${REPLACE:-}" ] && [ "${REPLACE##*/}" = "$UUID" ]; then
  echo "ygg-claim: ⛔ REFUSING — --replace names THIS session (${UUID}). A brief handed you" >&2
  echo "  your own uuid as its predecessor, which means you were spawned as an in-process" >&2
  echo "  helper rather than as a row. A helper is never a relay: it has no seat, no booter" >&2
  echo "  subscription, and dies with its parent. Spawn the successor as its own session." >&2
  exit 64
fi

log() { printf '%s ygg-claim %s\n' "$(date +%H:%M:%S)" "$*"; }

# --- locate a binary that can talk to the GUI ------------------------------
# App control is served by the GUI PROCESS, not the daemon, so it only answers on
# the host where the GUI runs. Probe for a NON-EMPTY client list: the verb exits 0
# with zero clients, so a zero exit code is not proof the GUI is here.
BIN=""
for cand in "$HOME/.yggterm/bin/yggterm" "$HOME/.local/bin/yggterm" "$(command -v yggterm 2>/dev/null)"; do
  [ -n "$cand" ] && [ -x "$cand" ] && { BIN="$cand"; break; }
done
[ -n "$BIN" ] || { echo "ygg-claim: no yggterm binary found" >&2; exit 64; }

ygg() {  # run an app-control verb wherever the GUI actually lives
  if [ -n "$HOST" ] && [ "$HOST" != "$(hostname)" ]; then
    # ⛔ QUOTE EVERY ARGUMENT FOR THE REMOTE SHELL. `ssh host "cmd $*"` hands the
    # far side ONE string which it re-splits on whitespace, so a multi-word title
    # arrives as several arguments and `rename` silently takes only the first —
    # a row asked for "topic: the long name" ends up titled "topic:". It looks
    # exactly like the CLI re-titling itself, which is the wrong diagnosis and
    # sends you hunting a defect in the app instead of in your own quoting.
    local q="" a
    for a in "$@"; do q="$q $(printf '%q' "$a")"; done
    ssh "$HOST" "\$HOME/.yggterm/bin/yggterm$q" 2>/dev/null
  else
    "$BIN" "$@" 2>/dev/null
  fi
}
# --- §2 of the hot-restart spec: DECLARE THE RELAY BOUNDARY ----------------
# A hand-off is the one moment this fleet produces that the daemon gate can
# actually use: a predecessor has finished and its successor has not started, so
# a swap owed by this host can run at an announced quiet point instead of
# waiting for a silence that never comes on a machine full of agents. The verb
# is a no-op on a converged host, which is the common case — it releases the
# retry floor for ONE attempt and returns.
#
# ⚠ Declared on BOTH planes and neither is redundant: the agent's own host owns
# the process that just died, and the GUI host owns the terminal runtime that
# just came free. A converged host answers "no swap is owed" either way.
# ⚠ Never allowed to fail the claim: a headless binary older than 3.0.129 does
# not know the verb, and a hand-off must not start failing because a boundary
# could not be announced.
boundary() {
  local why="${1:-ygg-claim}" hb
  for hb in "$HOME/.yggterm/bin/yggterm-headless" "$HOME/.local/bin/yggterm-headless"; do
    [ -x "$hb" ] && { "$hb" server relay-boundary --by "$why" >/dev/null 2>&1 || true; break; }
  done
  if [ -n "$HOST" ] && [ "$HOST" != "$(hostname)" ]; then
    ssh "$HOST" "\$HOME/.yggterm/bin/yggterm-headless server relay-boundary --by $(printf '%q' "$why")" \
      >/dev/null 2>&1 || true
  fi
}
has_gui() {
  local out; out="$(ygg server app clients 2>/dev/null)" || return 1
  # NOTE: `server app clients` answers with a TOP-LEVEL {clients,count}, unlike
  # every other app verb, which wraps its payload in {data:{...}}. Accept both —
  # and require a NON-EMPTY list, because the verb exits 0 with zero clients.
  printf '%s' "$out" | python3 -c 'import json,sys
try: d=json.load(sys.stdin)
except Exception: sys.exit(1)
c=d.get("clients")
if c is None: c=(d.get("data") or {}).get("clients")
sys.exit(0 if (c and len(c)>0) else 1)' 2>/dev/null
}
if ! has_gui; then
  found=""
  for h in ${YGG_GUI_HOSTS:-} $(ygg server app rows 2>&1 | grep -oE 'candidates this daemon knows: [a-z0-9, ]+' | sed 's/.*: //; s/,//g'); do
    HOST="$h"; has_gui && { found=1; break; }
  done
  [ -n "$found" ] || { echo "ygg-claim: could not find a host with a live GUI client (set --host or \$YGG_GUI_HOST)" >&2; exit 2; }
fi
log "GUI host: ${HOST:-$(hostname)} (local binary $BIN)"

rows_json() { ygg server app rows; }

# --- find my row, my siblings, and a free number ---------------------------
PLAN="$(rows_json | UUID="$UUID" CAMPAIGN="$CAMPAIGN" REPLACE="$REPLACE" \
        NUMBER="$NUMBER" INHERIT="$INHERIT" python3 -c '
import json,os,re,sys
d=json.load(sys.stdin)["data"]["rows"]
sess=[r for r in d if r.get("kind")=="Session"]
uuid,camp=os.environ["UUID"],os.environ["CAMPAIGN"].lower()
mine=next((r for r in sess if uuid in (r.get("full_path") or "")),None)
if not mine: print("ERR no row matches this session id"); sys.exit(0)
rep=os.environ.get("REPLACE","")
pred=next((r for r in sess if rep and rep in (r.get("full_path") or "") and uuid not in (r.get("full_path") or "")),None) if rep else None
# siblings = other rows already numbered, so we can pick a number that is free
def title(r): return (r.get("session_title") or r.get("label") or "")
def num(r):
    # Prefer the STORED seat. Falling back to the title is only for rows that
    # predate seat/title separation and still carry their number in the name —
    # and it is why this must not read the title first: once a row is seated
    # properly its title has no number at all, and a title-first parser would
    # conclude the row is unseated and hand out a duplicate.
    for src in (r.get("outline_prefix") or "", title(r)):
        m=re.match(r"^\s*(\d+)(?:\.(\d+))?",str(src))
        if m: return (int(m.group(1)), int(m.group(2)) if m.group(2) else None)
    return None
def isme(r):  return uuid in (r.get("full_path") or "")
def ispred(r):return bool(pred) and r.get("full_path")==pred.get("full_path")
others=[r for r in sess if not isme(r) and not ispred(r)]

n=os.environ.get("NUMBER","")
if not n:
    # PRECEDENCE, most specific first. Re-running claim must be a NO-OP, so a
    # number this row already carries outranks anything derived.
    sibs=[r for r in others if num(r) and camp in title(r).lower()]
    if os.environ.get("INHERIT")=="1" and pred and num(pred):
        n=str(num(pred)[0])                       # take the seat we are replacing
    elif pred and num(pred):
        n=str(num(pred)[0])                       # replacing implies inheriting the seat
    elif sibs:
        # the campaign already has rows: join it as a SUB-number (5 -> 5.3),
        # which is what keeps a multi-row campaign readable in a long sidebar.
        major=sorted(num(r)[0] for r in sibs)[0]
        used={num(r)[1] for r in sibs if num(r)[0]==major and num(r)[1] is not None}
        k=1
        while k in used: k+=1
        n=f"{major}.{k}"
    elif num(mine):
        n=(f"{num(mine)[0]}.{num(mine)[1]}" if num(mine)[1] is not None else str(num(mine)[0]))
    else:
        majors={num(r)[0] for r in others if num(r)}
        n=str((max(majors)+1) if majors else 1)

# ⛔⛔ WHATEVER WAS DERIVED, NEVER HAND OUT A SEAT SOMEONE ALREADY HOLDS.
# Reported 2026-08-13: a claim with --campaign and no --number derived 6.1, which
# 6.1 already held, verified by read-back and reported success. Two rows then wore
# one seat and could only be told apart by grepping their transcripts.
#
# The derivation above matches siblings by looking for the campaign token in a
# row TITLE — and titles are now stored CLEAN, so which rows mention the
# campaign word is arbitrary. Rather than make that heuristic cleverer, make the
# OUTCOME safe: a derived number is a suggestion, and a held seat is a fact.
# ⚠ Only for DERIVED numbers. An explicit --number is an instruction, and a
# caller re-running claim on their own row must be a no-op, not a renumber.
if not os.environ.get("NUMBER",""):
    held={}
    for r in others:
        p=str(r.get("outline_prefix") or "").strip()
        if p: held[p]=title(r)
    if n in held:
        head,_,tail = n.rpartition(".")
        k = int(tail) + 1 if tail.isdigit() else 1
        base = head if head else n
        while (f"{base}.{k}" if head else f"{base}.{k}") in held: k += 1
        was, n = n, (f"{base}.{k}")
        print(f"NOTE=derived seat {was} is already held by "
              f"{held[was][:40]}; taking {n} instead")
print("OK")
print("MINE="+mine["full_path"])
print("MINE_LABEL="+(mine.get("session_title") or mine.get("label") or ""))
print("NUM="+n)
print("PRED="+(pred["full_path"] if pred else ""))
print("PRED_LABEL="+((pred.get("session_title") or pred.get("label") or "") if pred else ""))
')"
case "$PLAN" in ERR*) echo "ygg-claim: ${PLAN#ERR }" >&2; exit 2 ;; esac
eval "$(printf '%s\n' "$PLAN" | grep -E '^(MINE|MINE_LABEL|NUM|PRED|PRED_LABEL)=' | sed 's/=/="/; s/$/"/')"
printf '%s\n' "$PLAN" | grep -E '^NOTE=' | sed 's/^NOTE=/ygg-claim: ⚠ /' || true

# ⛔⛔ THE SEAT NEVER GOES IN THE TITLE. It lives in `outline_prefix` and NOWHERE ELSE.
#
# The row's name is composed at RENDER time:  label = "<outline_prefix> <title>",
# and the shape the sidebar must read is:
#
#        N.x  [category]: what this row is for
#        ───  ────────────────────────────────
#         │                    └── the stored title, exactly as passed to --title
#         └── outline_prefix, stored apart and composed on
#
# ⚠ THIS USED TO WRITE THE NUMBER INTO THE TITLE AS WELL — "belt and braces" against
# a prefix observed to evaporate (2026-08-08). That belt caused TWO defects at once
# and is removed:
#
#   1. DOUBLE NUMBERING. The builder composes the prefix onto a title that already
#      carried it, so the sidebar drew "6.1 6.1 restore lifecycle: …". Once several
#      rows are wearing two numbers, nobody can tell a seat from a name.
#   2. ⛔ A CLAIM THAT WORKED REPORTED FAILURE, AND THE FAILURE SKIPPED THE REAP.
#      The server normalises the seat back out of the title, so the verifier below
#      compared its own composed string against a correctly-stored clean one, called
#      a good row bad, and `exit 3`-ed — ABOVE the booter arm and ABOVE `--replace`.
#      ⇒ every successor that ran this script left its predecessor alive and itself
#      unarmed, which is why duplicate seats kept appearing and were mistaken for
#      an agent declining to reap. The agent never got the chance.
#
# The durability worry the belt existed for is answered by the WATCH below, which
# re-asserts the prefix — the right instrument, because it defends the field that
# actually stores the seat instead of hiding a copy somewhere it must not be.
#
# ⛔ A CORRECTION WORTH KEEPING: an API read taken at a DIFFERENT MOMENT from a
# screenshot is not a verification of the screen. Sample both at once, or the
# difference you find may be TIME rather than disagreement.

# Defensive: strip a seat the caller composed in by hand. Callers keep doing this
# because the sidebar SHOWS a number, so a number looks like part of the name.
TITLE="$(printf '%s' "$TITLE" | sed -E 's/^[0-9]+(\.[0-9]+)*\.?[[:space:]]+//')"
case "$TITLE" in
  *:*) ;;
  *) log "note: --title has no '<category>: ' prefix; the scheme is \"N.x [category]: what it is for\"" ;;
esac
FINAL_TITLE="$TITLE"
log "row      : $MINE"
log "was      : $MINE_LABEL"
log "claiming : $FINAL_TITLE"
[ -n "${PRED:-}" ] && log "replacing: $PRED_LABEL ($PRED)"

if [ "$DRY" = 1 ]; then log "dry run — nothing changed"; exit 0; fi

# --- apply, then READ THE TITLE BACK. Never trust the verb's own field. -----
read_state() {  # -> "<outline_prefix>\t<session_title>", straight from the row table
  rows_json | MINE="$MINE" python3 -c '
import json,os,sys
# ⛔ ABSENT FROM THE LISTING IS NOT THE SAME FAILURE AS SEATED WRONG, AND THE OLD
# CODE COLLAPSED BOTH TO AN EMPTY PAIR. Measured 2026-08-13 by a sibling campaign:
# three consecutively-claimed rows reported `claim never verified (row reads: |)`
# and were ALL correctly seated — the read-back could not see them, so it denied an
# effect that was real, and a retry mechanism spawned duplicate workers on the
# strength of it. `server app rows` is known to have omitted live rows hidden by a
# collapsed set before 3.0.140, which is the listing this very check reads.
# ⇒ Say WHICH failure it is, and match on the UUID rather than on full_path
# equality, so a scheme/format difference cannot masquerade as a missing row.
rows=json.load(sys.stdin)["data"]["rows"]
mine=os.environ["MINE"]; uuid=mine.rstrip("/").split("/")[-1]
r=next((x for x in rows if x.get("full_path")==mine),None)
if r is None:
    r=next((x for x in rows if uuid and uuid in ((x.get("full_path") or "")+(x.get("session_id") or ""))),None)
if r is None:
    # ⚠ Not "the claim failed" — "this instrument cannot see the row".
    print("\t\x00ABSENT")
else:
    print((r.get("outline_prefix") or "") + "\t" + (r.get("session_title") or "")
          + ("\t\x00HIDDEN" if r.get("hidden_by_collapsed_set") else ""))' 2>/dev/null
}
assert_state() {
  ygg server app session outline "$MINE" "$NUM"          >/dev/null 2>&1
  ygg server app session rename  "$MINE" "$FINAL_TITLE"  >/dev/null 2>&1
  read_state
}
# ⛔ COMPARE AGAINST WHAT THE SERVER STORES, NOT AGAINST WHAT WE SENT.
# The seat lives in `outline_prefix`; the title is stored CLEAN. A comparison that
# expects the seat inside the title asserts a representation the server deliberately
# stopped keeping — it calls a correct row wrong, and the exit it takes is above the
# booter arm and above the predecessor reap. Normalise a legacy numbered title out of
# the read-back so an old row verifies too (and gets rewritten clean on the way).
matches() {
  local got_num="${1%%	*}" got_title="${1#*	}"
  # strip the diagnostic markers read_state may append before comparing
  got_title="${got_title%%	$'\x00'*}"
  got_title="$(printf '%s' "$got_title" | sed -E 's/^[0-9]+(\.[0-9]+)*\.?[[:space:]]+//')"
  [ "$got_num" = "$NUM" ] && [ "$got_title" = "$FINAL_TITLE" ]
}
GOT=""
for attempt in 1 2 3; do
  GOT="$(assert_state)"
  matches "$GOT" && break
  sleep 3
done
matches "$GOT" || {
  # ⛔ NAME WHICH FAILURE THIS IS. A caller that cannot tell "the claim did not take"
  # from "the instrument cannot see the row" will retry the claim — and a retry
  # mechanism acting on the second one has already spawned duplicate workers with a
  # byte-identical brief. Two agents in one working tree is the loss this averts.
  case "$GOT" in
    *$'\x00'ABSENT*)
      echo "ygg-claim: ⚠ THE ROW IS ABSENT FROM \`server app rows\` — this is NOT proof the claim failed." >&2
      echo "ygg-claim:   That listing has omitted live rows (hidden by a collapsed set, fixed 3.0.140)." >&2
      echo "ygg-claim:   Read the plane by session id before concluding anything, and ⛔ DO NOT re-spawn:" >&2
      echo "ygg-claim:     yggterm server app rows | grep ${MINE##*/}" >&2 ;;
    *) echo "ygg-claim: claim never verified (row reads: $(printf '%s' "$GOT" | tr '\t' '|' | tr -d '\000'))" >&2 ;;
  esac
  echo "ygg-claim: ⛔ the booter arm and the --replace reap below did NOT run" >&2; exit 3; }
case "$GOT" in *$'\x00'HIDDEN*)
  log "⚠ seated correctly, but the row is HIDDEN BY A COLLAPSED SET — it will not be on screen" ;;
esac
log "claimed and verified by read-back: seat=$NUM title=$FINAL_TITLE"

# --- ARM THE BOOTER (OPT-IN) ------------------------------------------------
# recorded 2026-08-09, said while he was hand-booting a stalled relay row:
# *"I have seen you stall sometimes, so arm a booter in a fleet."* A session that
# stalls cannot restart itself — the stall IS its turn ending — so something
# OUTSIDE it has to.
#
# ⛔ DEFAULT INVERTED 2026-08-10, recorded: *"When there is no relay mode
# the booter should not self arm. You should not be booted."* It fired on him in
# a session he had opened with "NOT like a relay, all agents contained in the
# session" — the row was claimed, so the row self-armed, and he was answered by a
# machine wake-up he had explicitly ruled out.
#
# ⚠ The bad inference, named so it is not re-derived: **claiming a row is not
# evidence that a session is unattended.** The old rationale — "claiming a row is
# the moment a session becomes long-running work" — conflated LONG-RUNNING with
# UNATTENDED. An interactive session at a keyboard is long-running too, and the
# one thing it never needs is to be woken by a robot. The scope the owner
# actually set was "in a fleet", i.e. relay/delegate work; the tool widened it to
# every claim.
#
# ⇒ Arm it where the unattendedness is KNOWN, not where a row is claimed:
#   • a DELEGATE is armed by its SPAWNER, explicitly, at spawn (SKILL.md §9) —
#     that path is untouched by this default and is the one that matters;
#   • a relay session arms itself with `--booter` (or `YGG_BOOTER=1`);
#   • a session a human is talking to arms nothing.
#
# ⛔ Unsubscribing is the SUBSCRIBER's job when the work is done
# (`ygg-booter.py unsubscribe --row <path>`); the booter retires a row only on
# facts it can see for itself — the row is gone, or the subscription expired.
if [ "${BOOTER:-0}" = 1 ] && [ -x "$(dirname "$0")/ygg-booter.py" ]; then
  "$(dirname "$0")/ygg-booter.py" subscribe \
      ${CAMPAIGN:+--campaign "$CAMPAIGN"} --note "$FINAL_TITLE" 2>&1 \
    | sed 's/^/  /' || log "⚠ booter subscribe failed — this row is NOT watched"
fi

# The CLI composes its own title when its first turn ends and will clobber this.
# Re-assert in the background for a while rather than assuming one write holds.
#
# ⛔⛔ THIS WATCHER FOUGHT A CORRECTIVE CLAIM FOR 240 s AND WON (2026-08-13).
# Two defects, both mine, and the first is the interesting one:
#
#  1. It compared `read_state` against "$NUM\t$FINAL_TITLE" — the RAW comparison
#     that was replaced in the verifier above and never here. The server stores
#     the title CLEAN, so the watcher saw a mismatch on every pass and re-asserted
#     every 10 s forever. A half-applied fix is worse than none: the verify
#     started passing, so nothing looked wrong, while a background loop kept
#     writing.
#  2. It OUTLIVED the script. A session that claimed twice — normal, when the
#     first claim derived a wrong seat — left the first watcher re-asserting the
#     old seat against the second claim's new one. `server app rows` showed the
#     old number while the claim's own read-back showed the new one, and the row
#     could only be settled by killing the watcher tree by hand.
#
# ⇒ Use the same predicate as the verify, and retire this session's previous
#   watcher before starting another. One session, at most one watcher.
WATCHPID="$STATE_DIR/claim-watcher-${MINE##*/}.pid"
mkdir -p "$STATE_DIR" 2>/dev/null || true
if [ -f "$WATCHPID" ]; then
  OLD="$(cat "$WATCHPID" 2>/dev/null || true)"
  if [ -n "$OLD" ] && kill -0 "$OLD" 2>/dev/null; then
    kill "$OLD" 2>/dev/null && log "retired this session's previous claim watcher (pid $OLD)"
  fi
  rm -f "$WATCHPID"
fi
if [ "${WATCH:-0}" -gt 0 ]; then
  ( for _ in $(seq 1 $((WATCH/10)) ); do
      sleep 10
      matches "$(read_state)" || assert_state >/dev/null
    done ) >/dev/null 2>&1 &
  echo $! > "$WATCHPID"
  log "watching seat+title for ${WATCH}s (the CLI self-titles at first-turn end)"
fi

# --- retire the predecessor, and REAP IT YOURSELF --------------------------
[ -n "${PRED:-}" ] || { log "done"; exit 0; }
PUUID="${PRED##*/}"

log "retiring predecessor $PUUID"
OUT="$(ygg server app session remove "$PRED")"
LISTED="$(printf '%s' "$OUT" | python3 -c 'import json,sys
try: print(json.load(sys.stdin)["data"].get("row_still_listed"))
except Exception: print("unknown")' 2>/dev/null)"
VERIF="$(printf '%s' "$OUT" | python3 -c 'import json,sys
try: d=json.load(sys.stdin)["data"]; print(d.get("verified"), d.get("verified_refusal"))
except Exception: print("unknown")' 2>/dev/null)"
log "remove: row_still_listed=$LISTED verified=$VERIF"

# --- move the SUBSCRIBERS with the seat ------------------------------------
# ⛔ RETIRING A ROW DOES NOT TOUCH THE SUPERVISION PLANE, AND THE ORPHANS THEN
# ESCALATE INTO A CORPSE. `escalate()` addresses `remote-cc://<host>/<uuid>`
# unconditionally and logs "escalated to orchestrator" whether or not the row
# exists — so a handover silently leaves every cluster escalating into nothing,
# while the plane reports itself healthy. Measured at a seat-6.0 handover
# 2026-08-13: five cluster rows, orphaned ninety seconds after the reap, found
# only because the successor happened to run `list`.
#
# ⇒ Run it HERE, before the process-reap gate below can `exit 4`. The rows are
#   orphaned the moment the row is removed, so the repair must not sit behind a
#   check that might skip it — the same lesson the verifier above was fixed for.
if [ -x "$(dirname "$0")/ygg-monitor.py" ]; then
  "$(dirname "$0")/ygg-monitor.py" succeed --from "$PUUID" --to "${MINE##*/}" 2>&1 \
    | sed 's/^/  /' || log "⚠ monitor succession failed — re-point subscribers by hand"
fi

# `verified:false` with the row already delisted means the ROW is gone but the
# agent PROCESS survived — typically because only the local transport was reaped.
# Identify by cmdline, requiring BOTH a plausible agent binary and the uuid;
# a bare `pgrep -cf <uuid>` matches the querying shell and lies in both directions.
agent_pids() {   # every process that is genuinely THIS agent, and nothing else
  for p in $(pgrep -f -- "$PUUID" 2>/dev/null); do
    [ "$p" = "$$" ] && continue
    # A pid from `pgrep` can exit before we read it — and the failure is the
    # SHELL's redirection, not tr's, so `2>/dev/null` on the command never
    # suppressed it. A clean reap printed "/proc/N/cmdline: No such file" three
    # times at the handover that found this, which reads as breakage in the one
    # output a successor studies most closely.
    [ -r "/proc/$p/cmdline" ] || continue
    c="$(tr '\0' ' ' < "/proc/$p/cmdline" 2>/dev/null)" || continue
    case "$c" in *pgrep*|*ygg-claim*|*terminate-cc*) continue ;; esac
    case "$c" in *claude*"$PUUID"*|*codex*"$PUUID"*|*"--session-id $PUUID"*) printf '%s ' "$p" ;; esac
  done
}
VICTIMS="$(agent_pids)"
if [ -n "${VICTIMS// /}" ]; then
  log "reaping surviving agent pids:$VICTIMS"
  # shellcheck disable=SC2086
  kill -TERM $VICTIMS 2>/dev/null
  for _ in 1 2 3 4 5 6; do
    sleep 1
    [ -z "$(agent_pids)" ] && break
  done
else
  log "no surviving agent process for $PUUID"
fi
SURV=0
for p in $(agent_pids); do SURV=$((SURV+1)); log "SURVIVED: pid $p"; done
[ "$SURV" = 0 ] || { echo "ygg-claim: predecessor processes survived — reap them by hand" >&2; exit 4; }
log "predecessor retired and reaped clean"
# ⇒ HERE is the boundary, and only here. The rename above is not a quiet point;
# a reaped predecessor is. Declared after the reap is verified, so the boundary
# names a moment that actually happened rather than one that was requested.
boundary "ygg-claim-reap"
log "done"
