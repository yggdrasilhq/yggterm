#!/usr/bin/env python3
"""The screen normalizer must match through BOTH spellings of an escape.

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

    # 2. Doubled boot residue must read residue-only through either spelling —
    #    a False here wedges the row forever (refused every tick, boots
    #    refunded, subscription still reading healthy).
    twice = b.BOOT_TEXT + b.BOOT_TEXT
    residue_real = ("earlier conversation output\n❯ "
                    + twice.replace(" ", "\x1b[1C", 40) + "\n")
    for name, screen in (("real-bytes", residue_real),
                         ("literal-escaped", literalize(residue_real))):
        pre = b._plain_screen(screen)
        if not b._composer_is_boot_residue_only(pre, b.BOOT_TEXT[:27]):
            failures.append(f"doubled boot residue NOT residue-only through {name} spelling")

    # 3. An owner's words before the copies must still refuse — the cleaner may
    #    never eat a human's draft, whatever the spelling did to the screen.
    drafted = ("❯ please hold this thought about the ledger " + b.BOOT_TEXT + "\n")
    if b._composer_is_boot_residue_only(b._plain_screen(drafted), b.BOOT_TEXT[:27]):
        failures.append("owner draft before the copy was NOT refused")

    if failures:
        print("⛔ %d failed: %s" % (len(failures), "; ".join(failures)))
        return 1
    print("✅ plain-screen normalizer: both spellings match, owner drafts still refuse")
    return 0


if __name__ == "__main__":
    sys.exit(main())
