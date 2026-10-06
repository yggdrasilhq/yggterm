#!/usr/bin/env python3
"""context-relay-gauge — tell a session how much of its window is gone, BEFORE the wall.

⛔⛔ THE DEFECT THIS EXISTS FOR — measured, not imagined (practice-rs, 2026-08-09/10).

Session `569e15eb` (row "8. practice") ran a relay for 8.5 hours on `opus[1m]` with
`autoCompactEnabled:false`. It grew 49k → **976,493** tokens and at 00:00:37 the API
began answering **"Prompt is too long"**. It never recovered. The booter then kicked
the corpse every ~10 minutes for the next TEN HOURS, and the owner found it by
looking at a screen.

★ THE THREE FACTS THAT MAKE THIS A TOOL AND NOT A REMINDER:

 1. **Nothing in the loop measured context.** The campaign door told the session HOW
    to spawn a successor and never WHEN. A rule with no trigger is a wish. Every
    other campaign survived only because a HUMAN stopped it in time — of 62 fleet
    sessions above 500k tokens, this is the only one that ever hit the wall, and it
    is the only one that was being driven by a machine instead of a person.

 2. **A dead session is INVISIBLE to an activity watchdog, because it answers
    FASTER than a live one.** `ygg-babysit.classify()` reads the transcript's
    *mtime*; "Prompt is too long" comes back in 5–66 ms and still writes three rows,
    so every boot reset the age to ~0 and the log reads `WORKING 0.1m` about a
    session that had been dead for two hours. ⇒ **An error returned faster than a
    success looks like health to anything that measures activity rather than
    outcome.** (Routed to the yggterm campaign, whose code that is. Not fixed here.)

 3. ⇒ So the check must run INSIDE the session, on the way IN, on every prompt —
    the one moment that is guaranteed to happen and cannot be forgotten by a fresh
    agent whose discipline resets every session. It fires on the booter's kick too,
    which is precisely the caller that has no judgement of its own.

★ WHY A THRESHOLD AND NOT COMPACTION. Compaction is lossy at exactly the moment the
  session is most loaded; a relay hands over a WRITTEN brief plus a repo state. The
  owner's design is relays, and `autoCompactEnabled:false` is deliberate. So this
  gauge exists to make the relay land with runway — ~300k tokens on a 1M window —
  never to squeeze the last 3% out of a context.

⛔ IT NEVER FAILS A SESSION. Every path exits 0, and anything unexpected exits 0
  silently. A gauge that can break the thing it measures is worse than no gauge.

⛔ IT IS SILENT BELOW `NOTICE`. A warning printed on every turn is read on none.

Usage:
    context-relay-gauge.py                 # hook mode: JSON on stdin (UserPromptSubmit)
    context-relay-gauge.py --report        # CLI mode: always print, even when green
    context-relay-gauge.py --session <uuid> [--report]

Env overrides (percent of window): CTX_NOTICE=55 CTX_LAND=70 CTX_CRITICAL=85
"""
import json
import os
import sys
from pathlib import Path

HOME = Path.home()
PROJECTS = HOME / ".claude" / "projects"
# ⭐ The gauge PUBLISHES its reading. An external watchdog cannot see a session's
#    token count — that number exists only inside the CLI — and fact (2) above is
#    what happens when it has to guess from file mtimes instead. One small file
#    per session turns "is it about to die" from an inference into a lookup.
PUBLISH = HOME / ".claude" / "context-gauge"

# zcode (taught 2026-09-04, owner-directed): the rollout dir holds one
# `model-io-<session>.jsonl` per session — the model I/O log, not a Claude
# transcript. Same publish/verdict plane; a different reader below.
ZCODE_ROLLOUT = HOME / ".zcode" / "cli" / "rollout"

NOTICE = int(os.environ.get("CTX_NOTICE", "55"))
LAND = int(os.environ.get("CTX_LAND", "70"))
CRITICAL = int(os.environ.get("CTX_CRITICAL", "85"))

# ⚠ Read from the END. These transcripts reach 6 MB and this runs on every prompt;
#   a full parse would put a visible pause in front of every thing the owner types.
TAIL_BYTES = 3_000_000


# ⛔⛔ THE 2026-08-20 INSTANCE THIS TABLE EXISTS FOR: a Fable session at 186k/1M (19%,
#   /context-verified) was told "93% of the 200k window — THE WALL IS CLOSE" and relayed
#   twice off the false wall; the owner killed the spun successors by hand. Fable's window
#   is NATIVELY 1M with no `[1m]` selector anywhere — env, settings and suffix heuristics
#   all miss it. The model id in the transcript's assistant records is the truth the API
#   actually answered with; map THAT, and when the model is not in the table say so
#   instead of asserting 200k.
# ⛔⛔ THE 2026-08-21 INSTANCE — THE SAME BUG, ONE MODEL LATER, AND THE FIX ABOVE WAS
#   SCOPED TO FABLE SO IT DID NOT TRANSFER. An Opus 5 session at 179.7k/1M (18%,
#   /context-verified) was told "90% of the 200k window — THE WALL IS CLOSE AND IT IS NOT
#   SURVIVABLE". It refused a 103-image reading job it had ~816k of room for, and was three
#   steps into retiring itself when the owner ran /context and caught it.
#   ⭐ THE DANGEROUS PART IS NOT THE NUMBER, IT IS THE CONFIDENCE: because "opus" sat in the
#   200k table, window_for returned assumed=False, which SUPPRESSES the "WINDOW ASSUMED —
#   verify with /context" banner. The one safety valve the Fable fix added was switched off
#   by the very table that was wrong. A bare-family token ("opus") cannot carry a window,
#   because the window changes BETWEEN GENERATIONS of the same family.
#   ⇒ Opus 5's window is NATIVELY 1M on this plan — settings.json reads plain
#   `claude-opus-5`, there is no `[1m]` suffix in env, settings or the transcript, so every
#   selector heuristic below misses it. Match the GENERATION, never the family, and let
#   anything unrecognised fall through to assumed=True rather than asserting 200k.
KNOWN_1M_MODEL_TOKENS = ("fable", "opus-5", "opus5")
# ⛔ "opus" alone is NOT in this table on purpose — opus-4 is 200k and opus-5 is 1M.
KNOWN_200K_MODEL_TOKENS = ("opus-4", "sonnet", "haiku")
# zcode/GLM (taught 2026-09-04, owner instrument): GLM-5.3-Flash variant
# `max` has a 2M window — the harness's own context indicator read 20.8%
# where this gauge's formula (input+cacheRead+cacheWrite = 423,902 tokens)
# lands 21.2% of 2M (sub-point agreement), while the 1M assumption read a
# The token matches the MODEL FAMILY on purpose: the [max]/[high] suffix is
# a reasoning-effort variant, not a window property (dev's [high] session
# reads the same model); the window is the model's. Measured on [max] against
# the owner's instrument; the effort variants share the model.
KNOWN_2M_MODEL_TOKENS = ("glm-5.3-flash",)


def window_for(observed_max, model=""):
    """(window_tokens, assumed) — `assumed` True when the model was not recognized
    and the number is a guess the message must direct the agent to VERIFY with the
    CLI's own instrument (/context) rather than act on.

    ⛔ For the `[1m]` variant do NOT trust `message.model` — it reads
      `claude-opus-5` even for the 1M variant, because the suffix is a client-side
      selector stripped before the request. Settings and env carry it. But for
      models whose window is natively 1M (Fable), the transcript model id is
      exactly right and nothing else knows. Observation stays as the backstop that
      cannot be wrong in the dangerous direction."""
    m = (model or "").lower()
    if any(tok in m for tok in KNOWN_2M_MODEL_TOKENS):
        return 2_000_000, False
    if any(tok in m for tok in KNOWN_1M_MODEL_TOKENS):
        return 1_000_000, False
    for src in (os.environ.get("ANTHROPIC_MODEL", ""),
                os.environ.get("CLAUDE_MODEL", "")):
        if "[1m]" in src:
            return 1_000_000, False
    try:
        s = json.loads((HOME / ".claude" / "settings.json").read_text())
        if "[1m]" in str(s.get("model", "")):
            return 1_000_000, False
    except Exception:
        pass
    if observed_max > 250_000:
        return 1_000_000, False
    known_200k = any(tok in m for tok in KNOWN_200K_MODEL_TOKENS)
    return 200_000, not known_200k


def read_usage_zcode(path):
    """(used, observed_max, dead, model) from a zcode `model-io-*.jsonl` rollout.

    Row shape (measured 2026-09-04): `type == "model_io"`, top-level
    `sessionId`, `model` is a DICT (`{modelId, providerId, variant, ...}`),
    and `response.usage` is camelCase
    (`inputTokens/outputTokens/totalTokens/cacheReadTokens/cacheWriteTokens`).
    ⛔ `totalTokens` is input+output only — it EXCLUDES the cache reads that
    are most of a live context (measured: inputTokens 781 + cacheRead 190464
    = next row's inputTokens 191245). Context used is therefore the Claude
    formula's camelCase twin: input + cacheRead + cacheWrite."""
    try:
        size = os.path.getsize(path)
        with open(path, "rb") as fh:
            if size > TAIL_BYTES:
                fh.seek(size - TAIL_BYTES)
                fh.readline()
            blob = fh.read().decode("utf-8", errors="ignore")
    except OSError:
        return 0, 0, False, ""

    used = 0
    observed_max = 0
    dead = False
    model = ""
    for line in blob.splitlines():
        if '"model_io"' not in line:
            continue
        try:
            row = json.loads(line)
        except Exception:
            continue
        if row.get("type") != "model_io":
            continue
        dead = dead or "Prompt is too long" in line
        u = (row.get("response") or {}).get("usage") or {}
        input_tok = u.get("inputTokens", 0)
        cache_read = u.get("cacheReadTokens", 0)
        cache_write = u.get("cacheWriteTokens", 0)
        # Convention auto-detect (instrument-vs-gauge fix, 2026-10-06):
        # INCLUSIVE rows (zcode) carry cacheRead INSIDE inputTokens —
        # 390546 = 390208 cache + 338 fresh — and input+cacheRead+cacheWrite
        # double-counted the cache (77% gauge vs 38% instrument). EXCLUSIVE
        # rows (the 2026-09-04 claude-style measurement above) need the sum.
        if cache_read and input_tok <= cache_read:
            total = input_tok + cache_read + cache_write
        else:
            total = input_tok
        if total <= 0:
            continue
        used = total
        observed_max = max(observed_max, total)
        m = row.get("model") or {}
        if isinstance(m, dict):
            # The variant rides along ([max] etc.) — window tables key on the
            # exact measured shape, not the model family (Issue: GLM 2M).
            model = (m.get("modelId") or "") + (
                f"[{m.get('variant')}]" if m.get("variant") else "")
        else:
            model = str(m) or model
    return used, observed_max, dead, model

def read_usage(path):
    """(used, observed_max, dead) from the tail of a transcript.

    `used` is the LAST assistant row carrying a non-zero token count — not the max.
    ⛔ The distinction is load-bearing twice over: a `/clear` or a compaction DROPS
      the count, and a max would keep reporting the pre-compaction peak forever; and
      a refused turn is written as a `<synthetic>` row whose usage is 0, which must
      not read as "context is empty now"."""
    try:
        size = os.path.getsize(path)
        with open(path, "rb") as fh:
            if size > TAIL_BYTES:
                fh.seek(size - TAIL_BYTES)
                fh.readline()          # discard the partial line we landed inside
            blob = fh.read().decode("utf-8", errors="ignore")
    except OSError:
        return 0, 0, False

    used = 0
    observed_max = 0
    dead = False
    model = ""
    for line in blob.splitlines():
        if '"assistant"' not in line:
            continue
        try:
            row = json.loads(line)
        except Exception:
            continue
        if row.get("type") != "assistant":
            continue
        msg = row.get("message", {})
        content = msg.get("content")
        text = content if isinstance(content, str) else ""
        if isinstance(content, list):
            text = " ".join(p.get("text", "") for p in content
                            if isinstance(p, dict) and p.get("type") == "text")
        # The exact string the API returns when the window is already exceeded.
        dead = "Prompt is too long" in text
        u = msg.get("usage") or {}
        input_tok = u.get("input_tokens", 0)
        cache_read = u.get("cache_read_input_tokens", 0)
        cache_write = u.get("cache_creation_input_tokens", 0)
        # Same convention auto-detect as the camelCase block above.
        if cache_read and input_tok <= cache_read:
            total = input_tok + cache_read + cache_write
        else:
            total = input_tok
        if total > 0:
            used = total
            observed_max = max(observed_max, total)
            model = msg.get("model") or model
    return used, observed_max, dead, model


def transcript_for(session_id):
    if not session_id:
        return None

    # zcode: `~/.zcode/cli/rollout/model-io-<session>.jsonl` (owner-directed
    # 2026-09-04). Session ids are `sess_<uuid>`; the rollout file is the only
    # per-session transcript zcode keeps.
    zcode = ZCODE_ROLLOUT / f"model-io-{session_id}.jsonl"
    return str(zcode) if zcode.exists() else None
    hits = list(PROJECTS.glob(f"*/{session_id}.jsonl"))
    return str(hits[0]) if hits else None


def publish(session_id, payload):
    try:
        PUBLISH.mkdir(parents=True, exist_ok=True)
        (PUBLISH / f"{session_id}.json").write_text(json.dumps(payload))
    except Exception:
        pass


def verdict(pct, notice=None, land=None, critical=None):
    """Verdict at optional per-CLI thresholds (defaults: the Claude Code set)."""
    n = NOTICE if notice is None else notice
    l = LAND if land is None else land
    c = CRITICAL if critical is None else critical
    if pct >= c:
        return "CRITICAL"
    if pct >= l:
        return "LAND"
    if pct >= n:
        return "NOTICE"
    return "OK"


# ⚠ The wording is deliberately an INSTRUCTION, not a statistic. "context 72%" is a
#   number an agent can note and ignore; "stop taking new work" is a thing it either
#   did or did not do, and the next session can tell which from the transcript.
MESSAGES = {
    "NOTICE": (
        "CONTEXT GAUGE — {pct}% of the {win} window used ({used:,} tokens).\n"
        "You are past the halfway mark. Do NOT open a new plane of work "
        "(a new subsystem, a large corpus, a long browser session) in THIS session. "
        "Finish the unit in flight, and DECIDE where the relay boundary goes.\n"
        "\u26d4 THE BOUNDARY IS YOURS TO EXECUTE, NOT HIS TO CHOOSE. If the next plane "
        "belongs to a fresh session, the default action is the HANDOVER, DONE: write the "
        "brief into the campaign's queue, spawn the successor, verify it is armed, stand "
        "down. A row spawn is reversible and the mechanism is documented \u21d2 the bar is "
        "REVERSIBILITY, not certainty.\n"
        "\u26d4 NEVER END THIS TURN ON A QUESTION \u2014 not \"shall I continue?\", not "
        "\"spawn or keep going?\". If you can name a recommendation you can act on it. "
        "Log the ambiguity in the owner-window file instead; he reads it at the END."
    ),
    "LAND": (
        "⛔ CONTEXT GAUGE — {pct}% of the {win} window used ({used:,} tokens). LAND THIS SESSION.\n"
        "Take no new work. In this order: (1) finish + commit + push the unit in flight; "
        "(2) update the campaign door memory and the repo queue with what is open; "
        "(3) spawn the successor with `terminal new --kind claude-code` and PROVE the brief "
        "arrived by grepping its transcript for your ACK token; (4) `ygg-booter.py unsubscribe`; "
        "(5) retire this row. You have runway to do all five — that is what this threshold is for."
    ),
    "CRITICAL": (
        "⛔⛔ CONTEXT GAUGE — {pct}% of the {win} window used ({used:,} tokens). "
        "THE WALL IS CLOSE AND IT IS NOT SURVIVABLE.\n"
        "Past ~98% every turn returns 'Prompt is too long' and this session is unrecoverable — "
        "no compaction is armed. Read NOTHING large. Commit what exists, write the handover to "
        "the campaign door memory, spawn the successor, unsubscribe from the booter, retire. "
        "If you must choose, the WRITTEN HANDOVER outranks the work in flight: unfinished work "
        "is recoverable from a brief, and a brief that was never written is not."
    ),
}

# ⛔ Owner-directed 2026-08-20: sessions grossly underestimate their own context, and this
#   gauge has been the amplifier. The CLI's own instrument outranks it, always.
RITUAL = (
    "\n⭐ THE CONTEXT RITUAL (owner-directed 2026-08-20): before ANY relay decision, verify "
    "with the CLI's own instrument — in Claude Code run /context and read the true window; "
    "for other CLIs read the PTY frame. A gauge is a hint; the instrument is the fact. If "
    "they disagree, the instrument wins and the gauge gets fixed IN THE SAME SESSION."
)


# ── opencode support (added 2026-08-27, first non-Claude-Code CLI) ─────────
# Sessions live in ~/.local/share/opencode/opencode.db (SQLite): table
# session_message, rows type='assistant' with a tokens JSON block
# {input, output, reasoning, cache:{read,write}}. The context window used
# is input + cache.read of the LAST assistant row carrying a non-zero
# block (identical semantics to read_usage's "last, not max" rule; the
# UI's own "tokens used" counter reports the same number).
OPENCODE_DB = HOME / ".local" / "share" / "opencode" / "opencode.db"
OPENCODE_WINDOW = 1_000_000
# Owner-directed 2026-08-27 (opencode, 1M window): 600-700k = consider
# packing up and spawning a fresh session; 900k = seal work and strongly
# consider a new session.
OPENCODE_NOTICE, OPENCODE_LAND, OPENCODE_CRITICAL = 60, 70, 90


def read_usage_opencode(session_id: str):
    """(used, model, dead) from opencode's session_message table."""
    import sqlite3

    con = sqlite3.connect(str(OPENCODE_DB), timeout=5)
    rows = con.execute(
        "SELECT data FROM session_message WHERE session_id = ? AND type = 'assistant' "
        "ORDER BY seq DESC LIMIT 40",
        (session_id,),
    ).fetchall()
    con.close()
    used = 0
    model = ""
    breakdown = {}
    for (data,) in rows:
        try:
            d = json.loads(data)
        except Exception:
            continue
        tok = d.get("tokens") or {}
        cache_read = (tok.get("cache") or {}).get("read") or 0
        total = (tok.get("input") or 0) + cache_read
        if total > 0:
            used = total
            model = d.get("modelID") or model
            breakdown = tok
            break  # newest non-empty block = current context
    return used, model, False, breakdown


def publish_opencode(session_id: str, payload: dict):
    PUBLISH.mkdir(parents=True, exist_ok=True)
    (PUBLISH / f"{session_id}.json").write_text(json.dumps(payload))


def run_opencode(session_id: str, report: bool, used_override=None, window_override=None):
    used, model, dead, breakdown = read_usage_opencode(session_id)
    if used_override:
        used, model, dead = used_override, "manual", False
    win = window_override or OPENCODE_WINDOW
    if used <= 0:
        if report:
            print(f"CONTEXT GAUGE — no token data found for opencode session {session_id}. "
                  "A missing or stale gauge is NO INFORMATION, never 'healthy'. "
                  "Feed the instrument value: --used N (opencode's own tokens-used counter).")
        return 0
    pct = round(100.0 * used / win)
    v = verdict(pct, OPENCODE_NOTICE, OPENCODE_LAND, OPENCODE_CRITICAL)
    publish_opencode(session_id, {
        "session": session_id, "cli": "opencode", "pct": pct, "used": used,
        "window": win, "model": model, "verdict": v, "dead": dead,
        "cwd": os.getcwd(), "breakdown": breakdown,
        "thresholds": {"notice": OPENCODE_NOTICE, "land": OPENCODE_LAND,
                       "critical": OPENCODE_CRITICAL},
    })
    win_label = f"{win // 1000}k"
    thresholds = (f"NOTICE {OPENCODE_NOTICE}% / LAND {OPENCODE_LAND}% / CRITICAL "
                  f"{OPENCODE_CRITICAL}%")
    if v != "OK":
        msg = MESSAGES[v].format(pct=pct, win=win_label, used=used)
        msg += (f"\n(opencode row: thresholds {thresholds} of the {win_label} window, "
                f"owner-directed 2026-08-27; relay = seal state + spawn a fresh "
                f"opencode session with the written handover.)")
        print(msg + RITUAL)
    elif report:
        print(f"CONTEXT GAUGE — {pct}% of the {win_label} window used ({used:,} tokens) "
              f"— opencode session {session_id}, model {model or '?'}. {v}; thresholds "
              f"{thresholds}.")
    return 0


# ── antigravity / gemini support ──────────────────────────────────────────
# Storage lives under ~/.gemini/antigravity-cli/ (or legacy ~/.gemini/).
# Transcripts: ~/.gemini/antigravity-cli/brain/<session_id>/.system_generated/logs/transcript_full.jsonl
# Presence: ~/.gemini/antigravity-cli/presence/<session_id>.lock
# Settings: ~/.gemini/antigravity-cli/settings.json
AGY_HOME = HOME / ".gemini" / "antigravity-cli"
AGY_BRAIN = AGY_HOME / "brain"
AGY_PRESENCE = AGY_HOME / "presence"
AGY_SETTINGS = AGY_HOME / "settings.json"
AGY_WINDOW = 1_000_000
AGY_NOTICE, AGY_LAND, AGY_CRITICAL = 60, 70, 90


def read_model_antigravity() -> str:
    try:
        if AGY_SETTINGS.exists():
            s = json.loads(AGY_SETTINGS.read_text())
            return str(s.get("model", ""))
    except Exception:
        pass
    return ""


def find_active_antigravity_session() -> str:
    try:
        if AGY_PRESENCE.exists():
            locks = list(AGY_PRESENCE.glob("*.lock"))
            if locks:
                locks.sort(key=lambda p: p.stat().st_mtime, reverse=True)
                return locks[0].stem
    except Exception:
        pass
    return ""


# Baseline system context in Antigravity (system prompt + 18 tool JSON schemas + active skills/subagents):
# Measured ~26.5k tokens (System prompt ~10.4k, System tools ~14.0k, Skills/Subagents ~2.1k)
AGY_BASE_SYSTEM_TOKENS = 26_500


def read_usage_antigravity(session_id: str):
    """(used, model, dead, breakdown) for antigravity session."""
    model = read_model_antigravity()
    dead = False
    breakdown = {}
    used = 0
    sid = session_id if (session_id and session_id != "unknown") else find_active_antigravity_session()

    if sid:
        t_path = AGY_BRAIN / sid / ".system_generated" / "logs" / "transcript_full.jsonl"
        if not t_path.exists():
            t_path = AGY_BRAIN / sid / ".system_generated" / "logs" / "transcript.jsonl"
        if t_path.exists():
            try:
                user_chars = 0
                agent_chars = 0
                tool_chars = 0
                with open(t_path, "r", encoding="utf-8", errors="ignore") as f:
                    for line in f:
                        try:
                            d = json.loads(line)
                            t = d.get("type")
                            if t == "USER_INPUT":
                                user_chars += len(d.get("content") or "")
                            elif t == "PLANNER_RESPONSE":
                                th = d.get("thinking") or ""
                                c = d.get("content") or ""
                                tc = json.dumps(d.get("tool_calls") or [])
                                agent_chars += len(th) + len(c)
                                tool_chars += len(tc)
                            elif t == "GENERIC":
                                c = d.get("content") or ""
                                tool_chars += len(c)
                        except Exception:
                            continue
                # Token density ratios: ~3.5 chars/tok for user text, ~3.1 chars/tok for agent/code/tool output
                user_tok = int(user_chars / 3.5) if user_chars else 0
                agent_tok = int(agent_chars / 3.1) if agent_chars else 0
                tool_tok = int(tool_chars / 3.1) if tool_chars else 0
                used = AGY_BASE_SYSTEM_TOKENS + user_tok + agent_tok + tool_tok
                breakdown = {
                    "base_system_tokens": AGY_BASE_SYSTEM_TOKENS,
                    "user_tokens": user_tok,
                    "agent_tokens": agent_tok,
                    "tool_tokens": tool_tok,
                    "total_tokens": used,
                }
            except Exception:
                pass
    return used, model, dead, breakdown


def publish_antigravity(session_id: str, payload: dict):
    PUBLISH.mkdir(parents=True, exist_ok=True)
    (PUBLISH / f"{session_id}.json").write_text(json.dumps(payload))


def run_antigravity(session_id: str, report: bool, used_override=None, window_override=None):
    used, model, dead, breakdown = read_usage_antigravity(session_id)
    if used_override:
        used, model, dead = used_override, model or "manual", False
    win = window_override or AGY_WINDOW
    if used <= 0:
        if report:
            print(f"CONTEXT GAUGE — no token data found for antigravity session {session_id}. "
                  "A missing or stale gauge is NO INFORMATION, never 'healthy'. "
                  "Feed the instrument value: --used N (from agy --output-format json or frame footer).")
        return 0
    pct = round(100.0 * used / win)
    v = verdict(pct, AGY_NOTICE, AGY_LAND, AGY_CRITICAL)
    publish_antigravity(session_id, {
        "session": session_id, "cli": "antigravity", "pct": pct, "used": used,
        "window": win, "model": model, "verdict": v, "dead": dead,
        "cwd": os.getcwd(), "breakdown": breakdown,
        "thresholds": {"notice": AGY_NOTICE, "land": AGY_LAND,
                       "critical": AGY_CRITICAL},
    })
    win_label = f"{win // 1000}k"
    thresholds = (f"NOTICE {AGY_NOTICE}% / LAND {AGY_LAND}% / CRITICAL "
                  f"{AGY_CRITICAL}%")
    if v != "OK":
        msg = MESSAGES[v].format(pct=pct, win=win_label, used=used)
        msg += (f"\n(antigravity row: thresholds {thresholds} of the {win_label} window; "
                f"relay = seal state + spawn a fresh antigravity session with the written handover.)")
        print(msg + RITUAL)
    elif report:
        print(f"CONTEXT GAUGE — {pct}% of the {win_label} window used ({used:,} tokens) "
              f"— antigravity session {session_id}, model {model or '?'}. {v}; thresholds "
              f"{thresholds}.")
    return 0


def main():
    argv = sys.argv[1:]
    report = "--report" in argv
    cli = "claude"
    if "--cli" in argv:
        try:
            cli = argv[argv.index("--cli") + 1]
        except IndexError:
            pass
    used_override = None
    if "--used" in argv:
        try:
            used_override = int(argv[argv.index("--used") + 1])
        except (IndexError, ValueError):
            pass
    window_override = None
    if "--window" in argv:
        try:
            window_override = int(argv[argv.index("--window") + 1])
        except (IndexError, ValueError):
            pass
    if cli == "opencode":
        session_id = None
        if "--session" in argv:
            try:
                session_id = argv[argv.index("--session") + 1]
            except IndexError:
                pass
        session_id = session_id or os.environ.get("OPENCODE_SESSION_ID") or "unknown"
        win_o = window_override or OPENCODE_WINDOW
        return run_opencode(session_id, report, used_override, win_o)
    if cli in ("antigravity", "gemini"):
        session_id = None
        if "--session" in argv:
            try:
                session_id = argv[argv.index("--session") + 1]
            except IndexError:
                pass
        session_id = (session_id or os.environ.get("ANTIGRAVITY_SESSION_ID") or
                      os.environ.get("GEMINI_SESSION_ID") or find_active_antigravity_session() or "unknown")
        win_o = window_override or AGY_WINDOW
        return run_antigravity(session_id, report, used_override, win_o)
    if used_override:
        session_id = None
        if "--session" in argv:
            try:
                session_id = argv[argv.index("--session") + 1]
            except IndexError:
                pass
        session_id = session_id or "unknown"
        win_o = window_override or 1_000_000
        return run_opencode(session_id, report, used_override, win_o)
    session_id = None
    if "--session" in argv:
        try:
            session_id = argv[argv.index("--session") + 1]
        except IndexError:
            pass

    path = None
    if not sys.stdin.isatty() and not session_id:
        try:
            payload = json.loads(sys.stdin.read() or "{}")
            session_id = payload.get("session_id")
            path = payload.get("transcript_path")
        except Exception:
            pass
    session_id = session_id or os.environ.get("CLAUDE_CODE_SESSION_ID")
    if not path or not os.path.exists(path):
        path = transcript_for(session_id)
    if not path:
        return 0

    # Dispatch by transcript shape: zcode rollouts carry `model_io` rows with
    # camelCase usage; Claude transcripts carry `assistant` rows. Filename
    # prefix is the cheap tell (both are JSONL).
    if Path(path).name.startswith("model-io-"):
        used, observed_max, dead, model = read_usage_zcode(path)
    else:
        used, observed_max, dead, model = read_usage(path)
    win, assumed = window_for(observed_max, model)
    if used <= 0:
        return 0
    pct = round(100.0 * used / win)
    v = verdict(pct)
    win_label = (f"{win // 1_000_000}M"
                 if win >= 1_000_000 and win % 1_000_000 == 0
                 else f"{win // 1000}k")

    publish(session_id or "unknown", {
        "session": session_id, "pct": pct, "used": used, "window": win,
        "window_assumed": assumed, "model": model,
        "verdict": v, "dead": dead, "cwd": os.getcwd(),
    })

    if v != "OK":
        msg = MESSAGES[v].format(pct=pct, win=win_label, used=used)
        if assumed:
            msg = (f"⚠ WINDOW ASSUMED — model '{model or 'unknown'}' is not in this gauge's "
                   f"table, so {win_label} is a GUESS. Before acting on ANY line below, run "
                   "/context and trust IT; this gauge has cried wall at 19% real usage before "
                   "(Fable 1M read as 200k, 2026-08-20).\n") + msg
        print(msg + RITUAL)
    elif report:
        print(f"CONTEXT GAUGE — {pct}% of the {win_label} window used "
              f"({used:,} tokens) — model {model or '?'}, window "
              f"{'ASSUMED' if assumed else 'known'}. OK; relay at {LAND}%.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception:
        # ⛔ Silence, and exit 0. See the header: this must never be able to break a
        #    session, and a traceback on stdout would be injected into the context.
        sys.exit(0)
