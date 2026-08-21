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

    # 5. ⛔⛔ A BALANCE IS NOT A WINDOW. One row out of credits must not stand the
    #    whole fleet down: no timer clears a balance, so every probe re-arms a
    #    blackout over every other campaign. Reported live with 23 subscribers
    #    unwakeable behind one row's billing state.
    balance = ("You're out of usage credits. Run /usage-credits to keep using "
               "the model or /model to switch models.")
    if not b.refusal_is_a_balance_not_a_window(balance):
        failures.append("an exhausted CREDIT BALANCE was read as a timed quota window")

    # 6. …and the conservative direction, which is the one that must not drift:
    #    an ordinary session limit is still a window, still account-wide, and
    #    still stands the fleet down, because there waiting really does clear it.
    for window in ("You've hit your session limit. Try again at 3pm.",
                   "Rate limited. Please try again later.",
                   ""):
        if b.refusal_is_a_balance_not_a_window(window):
            failures.append(f"a timed window was misread as a balance: {window[:40]!r}")

    # 7. ⛔ EVERY REFUSAL THIS FILE CAN RETURN MUST BE IN BOTH TABLES. One decides
    #    whether the row is charged a wake it never received, the other whether
    #    anybody is ever told it is stuck. A guard that learns a new refusal and
    #    updates neither is how a lane loses its whole budget in silence — and it
    #    has happened twice, the second time to the lane that had just read the
    #    comment warning about the first.
    import re as _re
    src = Path(args.booter).read_text()
    returned = set(_re.findall(r'return "(refused-[a-z-]+)"', src))
    refunded = set(_re.findall(r'"(refused-[a-z-]+)"', src[src.index("if via in ("):]
                               [:src[src.index("if via in ("):].index(")")]))
    for name in sorted(returned - refunded):
        failures.append(f"{name} is returned but NOT refunded — it is charged as a boot")
    for name in sorted(returned - set(b.STANDING_REFUSAL_ESCALATE_AFTER)):
        failures.append(f"{name} is returned but has no escalation threshold — silent forever")

    if failures:
        print("⛔ %d failed: %s" % (len(failures), "; ".join(failures)))
        return 1
    print("✅ both spellings match; composer read as a ROW; a balance is not a window; every refusal is in both tables")
    return 0


if __name__ == "__main__":
    sys.exit(main())
