#!/usr/bin/env python3
"""A turn that stopped far short of its own session's habit is a CUT turn.

    python3 tests/test_turn_cut_short.py [--babysit <path to ygg-babysit.py>]

⭐ WHY THIS CLASSIFIER EXISTS. A row whose turn is cut mid-flight — a daemon
swap re-resuming it on a fresh pty — is alive, at rest, and sitting at an
ordinary composer. No screen tells it apart from a row that simply finished, so
every screen classifier in the fleet reads it as healthy and idle. The one
signal that survives is the SHAPE OF THE OUTPUT: the turn stops after a line or
two, from a session that had been producing real work.

⛔ THE COMPARISON IS AGAINST THE SESSION'S OWN HISTORY, never a fleet-wide
constant. Sessions differ by an order of magnitude in how much they write, so a
fixed threshold is simultaneously too eager for a terse row and blind for a
verbose one — and the remedy here is to TYPE INTO A ROW, which is the act this
fleet has paid for most when it was aimed wrongly.

Every fixture below is invented.
"""
import argparse
import importlib.util
import json
import sys
import tempfile
from pathlib import Path


def load_babysit(path):
    spec = importlib.util.spec_from_file_location("babysit_under_test", path)
    mod = importlib.util.module_from_spec(spec)
    argv, sys.argv = sys.argv, [str(path)]
    try:
        spec.loader.exec_module(mod)
    finally:
        sys.argv = argv
    return mod


def transcript(tmp, turn_lengths, final_length, with_tool_use=False):
    """A JSONL whose ended assistant turns have the given text lengths."""
    path = Path(tmp) / "session.jsonl"
    rows = []
    for length in list(turn_lengths) + [final_length]:
        rows.append({"type": "user", "message": {"content": "go on"}})
        content = [{"type": "text", "text": "w" * length}]
        if with_tool_use and length == final_length:
            content.append({"type": "tool_use", "name": "Bash", "input": {}})
        rows.append({"type": "assistant", "message": {"content": content}})
    path.write_text("\n".join(json.dumps(r) for r in rows))
    return path


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--babysit",
                    default=str(Path(__file__).resolve().parent.parent / "ygg-babysit.py"))
    a = ap.parse_args()
    bs = load_babysit(a.babysit)
    failures = []

    def check(name, got, want):
        if got != want:
            failures.append(f"{name}: got {got!r}, want {want!r}")

    with tempfile.TemporaryDirectory() as tmp:
        # A session that habitually writes ~2000 chars, then stops after one line.
        cut, why = bs.turn_cut_short(transcript(tmp, [2000] * 6, 40))
        check("a one-line turn after six long ones is cut", cut, True)
        if cut and "40 chars" not in why:
            failures.append(f"the reason must quote the measurement, got {why!r}")

        # ⛔ The same short turn from a session that is ALWAYS short is not a
        # signal — it is that session's normal voice, and waking it would be
        # typing into a row on the strength of a fleet-wide average.
        cut, _ = bs.turn_cut_short(transcript(tmp, [50] * 6, 40))
        check("a terse session's short turn is not cut", cut, False)

        # A full-length final turn is an ordinary finished turn.
        cut, _ = bs.turn_cut_short(transcript(tmp, [2000] * 6, 1800))
        check("a full-length final turn is not cut", cut, False)

        # ⛔ Too little history is NOT evidence of anything. A session with three
        # turns has no habit to be out of character with, and guessing here is
        # how a watchdog wakes a row that was doing exactly what it should.
        cut, _ = bs.turn_cut_short(transcript(tmp, [2000] * 2, 40))
        check("a session with no established habit is never called cut", cut, False)

        # A record carrying a tool_use is one STEP of a turn, not a finished
        # turn: its text length says nothing about what the turn produced, and
        # counting it would call every tool-heavy session cut.
        cut, _ = bs.turn_cut_short(transcript(tmp, [2000] * 6, 40, with_tool_use=True))
        check("a mid-turn tool_use record is not counted as a short turn", cut, False)

        # An unreadable or absent transcript must answer "no", never raise and
        # never invent — the caller is a watchdog that types.
        cut, _ = bs.turn_cut_short(Path(tmp) / "does-not-exist.jsonl")
        check("a missing transcript is not evidence of a cut", cut, False)
        torn = Path(tmp) / "torn.jsonl"
        torn.write_text('{"type": "assistant", "message": {"conte')
        cut, _ = bs.turn_cut_short(torn)
        check("a torn transcript is not evidence of a cut", cut, False)

    if failures:
        print("FAIL")
        for f in failures:
            print("  -", f)
        return 1
    print("ok — turn_cut_short reads a session against its own habit")
    return 0


if __name__ == "__main__":
    sys.exit(main())
