#!/usr/bin/env python3
"""The screen normalizer, and the composer read as a ROW rather than a screen.

    python3 tests/test_booter_plain_screen.py [--booter <path to ygg-booter.py>]

⛔ THE HOLE THIS PINS, measured 2026-08-21 on four wedged rows at once: the
read-buffer arm's stdout is a JSON envelope, and consumed raw every escape
byte arrives as the six literal characters ``\\u001b[…``. The real-byte CSI
regex never fires, tokens split mid-word, the residue cleaner's length cap
blows on the inflation — so four live rows were refused every tick forever —
and the choice-prompt guard, whose whole job is to stop a bare Enter from
selecting a billing option, silently matched nothing. Every fixture here is
invented; the boot text itself ships in this repository.
"""
import argparse
import importlib.util
import sys
from pathlib import Path


def load_booter(path):
    spec = importlib.util.spec_from_file_location("booter_under_test", path)
    mod = importlib.util.module_from_spec(spec)
    argv, sys.argv = sys.argv, [str(path), "status"]
    try:
        spec.loader.exec_module(mod)
    except SystemExit:
        pass
    finally:
        sys.argv = argv
    return mod


def literalize(text):
    """Real escape bytes -> the six-character literal spelling, real blanks ->
    literal ``\\n`` — the shape a raw JSON envelope hands over."""
    return text.replace("\x1b", "\\u001b").replace("\n", "\\n")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--booter",
                    default=Path(__file__).resolve().parent.parent / "ygg-booter.py")
    args = ap.parse_args()
    b = load_booter(args.booter)
    failures = []

    # 1. A choice prompt must be found through EITHER spelling — the unsafe
    #    direction is a miss, which lets a bare Enter select a billing option.
    prompt_real = ("You have hit your session limit.\n"
                   "\x1b[1m1. Stop\x1b[0m\x1b[3Cand\x1b[2Cwait\n"
                   "2. Switch to a team account\n\x1b[7m❯\x1b[0m ")
    for name, screen in (("real-bytes", prompt_real),
                         ("literal-escaped", literalize(prompt_real))):
        low = b._plain_screen(screen).lower()
        if not any(m in low for m in b.CHOICE_PROMPT_MARKERS):
            failures.append(f"choice prompt MISSED through {name} spelling")

    # 2. ⛔⛔ THE JAM THIS ENDS. A boot that WORKED stays on the screen as a
    #    delivered transcript entry, and the agent CLI draws that entry behind
    #    the SAME glyph the composer uses. The old reader flattened the whole
    #    screen to one line and found the boot text after a `❯` — so the row
    #    read "residue in the composer" forever and no clear could ever satisfy
    #    it. Measured across 19 rows and 434 consecutive refusals.
    delivered = [
        "  earlier conversation output",
        "❯ " + b.BOOT_TEXT[:70],
        "",
        "● and the reply the row already gave to it",
        "✻ Churned for 50s",
        "─" * 60,
        "❯",
        "─" * 60,
        "  ⏵⏵ bypass permissions on (shift+tab to cycle) · ← 1 agent",
    ]
    if b._composer_from_grid(delivered) != "":
        failures.append("a DELIVERED boot in the transcript read as composer content: "
                        f"{b._composer_from_grid(delivered)!r}")

    # 3. The composer's own content is still read, including when it wraps.
    drafted = [
        "● some earlier output",
        "─" * 60,
        "❯ please hold this thought about the",
        "  ledger until tomorrow",
        "─" * 60,
        "  ⏵⏵ bypass permissions on (shift+tab to cycle)",
    ]
    if b._composer_from_grid(drafted) != "please hold this thought about the ledger until tomorrow":
        failures.append(f"a wrapped composer draft was misread: {b._composer_from_grid(drafted)!r}")

    # 4. No composer drawn at all is NOT an empty composer — one may be typed
    #    into and the other may not, and returning "" for both is how a watcher
    #    types into a modal.
    mid_output = ["● running the sweep", "  Ran 1 shell command", "  ...still going"]
    if b._composer_from_grid(mid_output) is not None:
        failures.append("a mid-output screen reported a composer")

    if failures:
        print("⛔ %d failed: %s" % (len(failures), "; ".join(failures)))
        return 1
    print("✅ choice prompt matches through both spellings; the composer is read as a ROW")
    return 0


if __name__ == "__main__":
    sys.exit(main())
