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
print("OK")
print("MINE="+mine["full_path"])
print("MINE_LABEL="+(mine.get("session_title") or mine.get("label") or ""))
print("NUM="+n)
print("PRED="+(pred["full_path"] if pred else ""))
print("PRED_LABEL="+((pred.get("session_title") or pred.get("label") or "") if pred else ""))
')"
case "$PLAN" in ERR*) echo "ygg-claim: ${PLAN#ERR }" >&2; exit 2 ;; esac
eval "$(printf '%s\n' "$PLAN" | grep -E '^(MINE|MINE_LABEL|NUM|PRED|PRED_LABEL)=' | sed 's/=/="/; s/$/"/')"

# THE SEAT GOES IN THE TITLE **AS WELL AS** IN `outline` — belt and braces.
#
# The sidebar builder re-composes `outline_prefix` onto the row's label as its
# last act, precisely so a CLI re-titling itself cannot drop the number, and the
# sidebar draws that composed label. So the API and the screen agree BY DESIGN,
# and seat/title separation is the better architecture.
#
# ⚠ But a stored prefix has been observed to VANISH between two reads (2026-08-08),
# leaving the row unnumbered. Until that durability defect is closed, also compose
# the number into the title — the field the watch below defends.
#
# ⛔ A CORRECTION WORTH KEEPING: I first read a composed label from the API, saw an
# unnumbered row in a screenshot taken 40 minutes later, and concluded the field
# was lying about what the sidebar renders. FALSE — the seat had evaporated in
# between. An API read taken at a DIFFERENT MOMENT from the screenshot is not a
# verification of the screen: sample both at once, or the difference you find may
# be TIME rather than disagreement.
case "$NUM" in
  *.*) FINAL_TITLE="${NUM} ${TITLE}" ;;   # sub-seat: "5.1 topic"
  *)   FINAL_TITLE="${NUM}. ${TITLE}" ;;  # top-level: "4. topic"
esac
log "row      : $MINE"
log "was      : $MINE_LABEL"
log "claiming : $FINAL_TITLE"
[ -n "${PRED:-}" ] && log "replacing: $PRED_LABEL ($PRED)"

if [ "$DRY" = 1 ]; then log "dry run — nothing changed"; exit 0; fi

# --- apply, then READ THE TITLE BACK. Never trust the verb's own field. -----
read_state() {  # -> "<outline_prefix>\t<session_title>", straight from the row table
  rows_json | MINE="$MINE" python3 -c '
import json,os,sys
rows=json.load(sys.stdin)["data"]["rows"]
r=next((x for x in rows if x.get("full_path")==os.environ["MINE"]),None)
print(((r.get("outline_prefix") or "") + "\t" + (r.get("session_title") or "")) if r else "\t")' 2>/dev/null
}
assert_state() {
  ygg server app session outline "$MINE" "$NUM"          >/dev/null 2>&1
  ygg server app session rename  "$MINE" "$FINAL_TITLE"  >/dev/null 2>&1
  read_state
}
GOT=""
for attempt in 1 2 3; do
  GOT="$(assert_state)"
  [ "$GOT" = "$(printf '%s\t%s' "$NUM" "$FINAL_TITLE")" ] && break
  sleep 3
done
[ "$GOT" = "$(printf '%s\t%s' "$NUM" "$FINAL_TITLE")" ] || {
  echo "ygg-claim: claim never verified (row reads: $(printf '%s' "$GOT" | tr '\t' '|'))" >&2; exit 3; }
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
if [ "${WATCH:-0}" -gt 0 ]; then
  WANT="$(printf '%s\t%s' "$NUM" "$FINAL_TITLE")"
  ( for _ in $(seq 1 $((WATCH/10)) ); do
      sleep 10
      [ "$(read_state)" = "$WANT" ] || assert_state >/dev/null
    done ) >/dev/null 2>&1 &
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

# `verified:false` with the row already delisted means the ROW is gone but the
# agent PROCESS survived — typically because only the local transport was reaped.
# Identify by cmdline, requiring BOTH a plausible agent binary and the uuid;
# a bare `pgrep -cf <uuid>` matches the querying shell and lies in both directions.
agent_pids() {   # every process that is genuinely THIS agent, and nothing else
  for p in $(pgrep -f -- "$PUUID" 2>/dev/null); do
    [ "$p" = "$$" ] && continue
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
log "done"
