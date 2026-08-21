#!/usr/bin/env python3
"""A transcript is read from its TAIL, and only shapes that were measured are parsed.

    python3 tests/test_transcript_reads_are_bounded.py

⛔⛔ THE HOLE THIS PINS, measured 2026-08-22. Three callers read a transcript with
`[json.loads(l) for l in open(path)]` — the whole file, every line parsed into
memory. That was survivable only because they could reach ONE CLI's store, whose
largest file on this fleet is 36 MB. Teaching the lookup every CLI's store put a
**1,481 MB** file in front of the same line, on a timer, and one of the three runs
OVER SSH — so the gigabytes would have been allocated on someone else's laptop.

⚠ **The distribution is the trap.** The p95 codex transcript is 5.4 MB; the maximum
is 274x that. A bound chosen by looking at a typical file is wrong by two orders of
magnitude exactly where it matters — a long-running agent row.

⚠ And the second half: a bounded read that cannot FIND the prose is not a fix, it
is the same blank with better manners. So the window escalates once, and that is
pinned here too.

Every fixture is invented. The record shapes are real.
"""
import importlib.util
import json
import shutil
import sys
import tempfile
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


# The three record shapes, as read off real stores.
def cc(text):
    return {"type": "assistant", "message": {"content": [{"type": "text", "text": text}]}}


def codex(text):
    return {"type": "response_item",
            "payload": {"type": "message", "role": "assistant",
                        "content": [{"type": "output_text", "text": text}]}}


def agy(text):
    return {"type": "PLANNER_RESPONSE", "content": text, "thinking": "scratchpad, not speech"}


NOISE = {"type": "response_item",
         "payload": {"type": "function_call", "name": "invented_tool",
                     "arguments": "x" * 4000}}

tmp = Path(tempfile.mkdtemp(prefix="ygg-tail-fixture-"))
try:
    for name, maker in (("claude-code", cc), ("codex", codex), ("antigravity", agy)):
        path = tmp / f"{name}.jsonl"
        with open(path, "w") as handle:
            handle.write(json.dumps(maker("THE-OLDEST-WORDS")) + "\n")
            for _ in range(700):                      # ~2.8 MB of tool noise
                handle.write(json.dumps(NOISE) + "\n")
            handle.write(json.dumps(maker("THE-NEWEST-WORDS")) + "\n")
        size = path.stat().st_size
        if size <= T.TAIL_BYTES:
            FAILURES.append(f"{name}: fixture is only {size}B, smaller than the window "
                            f"it is meant to exercise — it proves nothing")
        check(f"{name}: the newest words are found", T.last_prose(str(path)),
              "THE-NEWEST-WORDS")

    # ⛔ THE BOUND ITSELF. A marker at the very START of a file far larger than both
    #    windows must NOT come back — if it does, something read the whole file.
    big = tmp / "far-too-big.jsonl"
    with open(big, "w") as handle:
        handle.write(json.dumps(cc("BEYOND-EVERY-WINDOW")) + "\n")
        for _ in range(5000):                         # ~20 MB, past the escalation
            handle.write(json.dumps(NOISE) + "\n")
    check("a record beyond both windows is not reached", T.last_prose(str(big)), "")
    if big.stat().st_size <= T.TAIL_BYTES_ESCALATED:
        FAILURES.append("the oversize fixture does not exceed the escalated window, "
                        "so it cannot demonstrate the bound")

    # ⚖ THE ESCALATION. Prose past the first window but inside the second must still
    #    be found, or the bound has simply replaced a crash with a wrong answer.
    mid = tmp / "past-the-first-window.jsonl"
    with open(mid, "w") as handle:
        handle.write(json.dumps(cc("INSIDE-THE-SECOND-WINDOW")) + "\n")
        for _ in range(900):                          # ~3.6 MB: past window 1, inside 2
            handle.write(json.dumps(NOISE) + "\n")
    if not (T.TAIL_BYTES < mid.stat().st_size < T.TAIL_BYTES_ESCALATED):
        FAILURES.append("the escalation fixture is not between the two windows, so it "
                        "does not test the escalation")
    check("prose past the first window is still found by the second",
          T.last_prose(str(mid)), "INSIDE-THE-SECOND-WINDOW")

    # A shape nobody has measured returns nothing rather than guessing — a wrong
    # answer here feeds a stall verdict.
    unknown = tmp / "unmeasured.jsonl"
    unknown.write_text(json.dumps({"kind": "reply", "body": "invented unknown shape"}) + "\n")
    check("an unmeasured record shape yields no prose", T.last_prose(str(unknown)), "")
    check("thinking is not mistaken for speech",
          T.prose_of({"type": "PLANNER_RESPONSE", "content": "", "thinking": "x"}), None)
    check("a codex tool call is not mistaken for speech", T.prose_of(NOISE), None)

    # A partial first record is dropped, not repaired.
    check("a truncated leading record does not become a phantom",
          [r for r in T.tail_records(str(big), 1000) if "BEYOND" in json.dumps(r)], [])
finally:
    shutil.rmtree(tmp, ignore_errors=True)

if FAILURES:
    print("FAIL")
    for f in FAILURES:
        print("  ⛔", f)
    sys.exit(1)
print("ok — transcript reads are bounded, escalate once, and parse only measured shapes")
