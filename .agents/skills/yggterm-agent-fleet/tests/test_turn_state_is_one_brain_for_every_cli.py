#!/usr/bin/env python3
"""One classifier decides a row's turn state, and the remote probe IS that classifier.

    python3 tests/test_turn_state_is_one_brain_for_every_cli.py

⛔ THE HOLE THIS PINS. `ygg-babysit` decided TURN_ENDED / MIDTURN / RATE_LIMITED from
the reference CLI's spellings, written out longhand a SECOND time inside a string that
gets shipped over ssh. Two consequences:

  · every other CLI came back `EMPTY`, which reads as "idle" and is not — a stalled
    codex lane was indistinguishable from a finished one;
  · the two copies were kept in step by a comment asking the reader to remember, and
    the cost of forgetting is stated in the code itself: *a local row and a remote row
    disagreeing about whether the account has quota is a fleet that boots half of
    itself into a wall.*

⛔⛔ **AND THE TRAP THAT MAKES THE OBVIOUS FIX WORSE THAN THE BUG.** Codex writes a
`rate_limits` block into a `token_count` event on essentially every turn — routine
remaining-quota telemetry, measured at 6,949 occurrences across 25 transcripts and
present in 39 of 40 files. A substring match for "rate_limit" would report almost
every codex session as rate limited and freeze the wake plane for all of them. That
case is pinned below, because it is the one a future shortcut will reintroduce.

Every fixture is invented. The record shapes are real.
"""
import importlib.util
import inspect
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(HERE))
spec = importlib.util.spec_from_file_location("ygg_transcript", HERE / "ygg_transcript.py")
T = importlib.util.module_from_spec(spec)
spec.loader.exec_module(T)

FAILURES = []


def check(name, got, want):
    if got != want:
        FAILURES.append(f"{name}: got {got!r}, wanted {want!r}")


# ── the reference CLI: behaviour must not move ──────────────────────────────
cc_spoke = {"type": "assistant",
            "message": {"content": [{"type": "text", "text": "an invented answer"}]}}
cc_tool = {"type": "assistant",
           "message": {"content": [{"type": "tool_use", "name": "invented_tool"}]}}
cc_user = {"type": "user", "message": {"content": [{"type": "text", "text": "ask"}]}}
cc_limit = {"type": "assistant", "isApiErrorMessage": True, "apiErrorStatus": 429,
            "message": {"content": [{"type": "text", "text": "limit reached"}]}}

check("reference: a finished turn", T.classify_records([cc_user, cc_spoke]), "TURN_ENDED")
check("reference: a tool call is mid-turn", T.classify_records([cc_user, cc_tool]), "MIDTURN")
check("reference: waiting on the model", T.classify_records([cc_spoke, cc_user]), "MIDTURN")
check("reference: a rate limit", T.classify_records([cc_spoke, cc_limit]), "RATE_LIMITED")

# ── codex: explicit turn boundaries, which the reference CLI does not have ───
def ev(payload):
    return {"type": "event_msg", "payload": payload}


cx_started = ev({"type": "task_started", "turn_id": "invented"})
cx_done = ev({"type": "task_complete", "turn_id": "invented", "last_agent_message": "done"})
cx_abort = ev({"type": "turn_aborted", "turn_id": "invented", "reason": "invented"})
cx_said = ev({"type": "agent_message", "message": "an invented reply"})
cx_limit = ev({"type": "error", "codex_error_info": "usage_limit_exceeded",
               "message": "You've hit your usage limit."})
# ⛔ THE TRAP: routine quota telemetry, on essentially every turn.
cx_telemetry = ev({"type": "token_count",
                   "rate_limits": {"primary": {"used_percent": 12.5, "window_minutes": 300}},
                   "info": {"total_token_usage": {"input_tokens": 10}}})

check("codex: a finished turn", T.classify_records([cx_started, cx_done]), "TURN_ENDED")
check("codex: a started turn is mid-turn", T.classify_records([cx_done, cx_started]), "MIDTURN")
check("codex: an aborted turn has ended", T.classify_records([cx_started, cx_abort]), "TURN_ENDED")
check("codex: a real usage limit", T.classify_records([cx_done, cx_limit]), "RATE_LIMITED")

# ⛔⛔ THE ONE THAT MATTERS MOST. Telemetry settles nothing, and must never outrank
#    the turn that actually finished — otherwise the wake plane freezes a whole CLI.
check("codex: quota TELEMETRY is not a rate limit",
      T.classify_records([cx_started, cx_said, cx_telemetry]), "TURN_ENDED")
check("codex: telemetry alone settles nothing",
      T.classify_records([cx_telemetry]), "EMPTY")
check("codex: telemetry is not a verdict on its own",
      T.classify_record(cx_telemetry), None)

# ⚠ Newest wins: a limit hit three turns ago must not outrank a turn that finished
#   since, or a row recovers and is still treated as frozen.
check("a limit older than a finished turn does not outrank it",
      T.classify_records([cx_limit, cx_started, cx_done]), "TURN_ENDED")

# ── antigravity: a flat step log ────────────────────────────────────────────
agy_said = {"type": "PLANNER_RESPONSE", "content": "an invented reply",
            "thinking": "scratchpad"}
agy_cmd = {"type": "RUN_COMMAND", "status": "DONE", "content": "invented command"}
agy_user = {"type": "USER_INPUT", "content": "invented question"}
agy_overload = {"type": "ERROR_MESSAGE",
                "content": "Error: The model API is currently overloaded."}

check("antigravity: a finished turn", T.classify_records([agy_user, agy_said]), "TURN_ENDED")
check("antigravity: an action means the turn is running",
      T.classify_records([agy_said, agy_cmd]), "MIDTURN")
check("antigravity: waiting on the model", T.classify_records([agy_said, agy_user]), "MIDTURN")
# ⛔ An OVERLOAD is not an exhausted account: one clears on its own, the other does
#    not, and freezing a row's wake plane for the first is a self-inflicted outage.
check("antigravity: an overloaded API is NOT a rate limit",
      T.classify_record(agy_overload), None)

# ── the remote probe is not a second copy ───────────────────────────────────
probe = T.remote_probe_source()
for fn in (T.classify_record, T.classify_records, T.prose_of):
    body = inspect.getsource(fn)
    if body.strip() not in probe:
        FAILURES.append(f"{fn.__name__} is not spliced into the remote probe verbatim — "
                        f"the far machine is running a different brain, which is exactly "
                        f"the drift this design removes")
compile(probe, "<remote-probe>", "exec")   # it has to actually run over there

# An unmeasured CLI settles nothing rather than being guessed at.
check("an unmeasured record shape yields EMPTY",
      T.classify_records([{"kind": "reply", "body": "invented unknown shape"}]), "EMPTY")

if FAILURES:
    print("FAIL")
    for f in FAILURES:
        print("  ⛔", f)
    sys.exit(1)
print("ok — one classifier, every measured CLI, and the remote probe is generated from it")
