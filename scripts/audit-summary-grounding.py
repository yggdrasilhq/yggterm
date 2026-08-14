#!/usr/bin/env python3
"""Find generated session summaries that describe work the session never did.

WHY A SPECIAL DETECTOR IS NEEDED. The obvious test — "does the summary appear in
the session" — is worthless, because paraphrasing is the summariser's whole job;
run that way it condemns every summary ever generated, which is exactly the
measurement this tool replaced. What a paraphrase cannot do is invent the
SUBJECT. So the test here is: take the concrete words the summary commits to
(its nouns, identifiers and technical terms) and ask how many of them occur
anywhere in the session at all. An honest paraphrase reuses the session's
vocabulary even when it rewrites the sentences. A summary written from an empty
context has no vocabulary to reuse, so it borrows one from nowhere in
particular, and the overlap collapses.

WHAT THE SESSION SAID has exactly one owner, and it is not this script:

    yggterm-headless server remote generation-context <transcript.jsonl>

That verb is the same context the generator itself is given. Re-deriving it in
Python would be a second decoder that could drift from the real one — which is
the defect family this tool exists to catch.

CLEARING WHAT IT FINDS. An ungrounded summary does not heal on its own: the
generator returns the cached one unless it is forced or the cache looks like
junk, and a fluent invention looks like neither. So `--clear` deletes the
offending rows (summary, its timeline, and the title if that is ungrounded too)
and the next copy scan regenerates them from the real transcript. Nothing
hand-written is touched.

USAGE
    scripts/audit-summary-grounding.py                    # audit the local store
    scripts/audit-summary-grounding.py --json             # machine-readable
    scripts/audit-summary-grounding.py --limit 50
    scripts/audit-summary-grounding.py --clear            # delete what it found
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
from pathlib import Path

# Measured on a live store, 2026-08-14, against the context this script actually
# uses and with the stemming below in force: 70 known-fabricated summaries
# scored at most 0.53, while generated summaries of sessions the generator could
# read had a 10th percentile of 0.80 and a median of 1.00. The threshold sits
# inside that gap rather than at either edge. ⚠ Re-measure it whenever the rule
# or the haystack changes — a threshold quoted from a different measurement than
# the one the tool performs is worse than no threshold at all.
GROUNDED_THRESHOLD = 0.65

# A summary shorter than this commits to too little for the ratio to mean
# anything; one stray word would swing it.
MIN_ANCHORS = 4

STOP = set(
    """
the a an and or but if then than that this these those with without within into onto from for to of
in on at by as is are was were be been being do does did done have has had having will would shall
should can could may might must not no nor so such only also more most less least other another
some any all both each few many much own same very just now current currently next step objective
goal primary main session work working task tasks issue issues blocker blockers progress finding
findings verified confirm confirmed confirms complete completed completing finish finished finishing
remain remaining remains implement implemented implementation update updated updating fix fixed
fixes add added adding run ran running test tests testing use used using make made new old
across after before while during still yet however therefore because since when where which who
what how why we our us you your they their it its there here about over under between among
first second third last final latest recent code file files line lines change changes changed
error errors fail fails failed failure output input value values state states time times
""".split()
)

TOKEN = re.compile(r"[A-Za-z][A-Za-z0-9_.\-/]{2,}")
# The generator opens with a date stamp of its own; it is scaffolding, not a
# claim about the session, so it never counts for or against grounding.
DATE_STAMP = re.compile(r"^\s*\**\s*\d{4}[^A-Za-z]{0,12}\d{2}[^A-Za-z]{0,12}\d{2}.{0,20}?[–—-]\s*")


# Suffixes stripped before an anchor is looked for, longest first.
#
# ⚠ Without this the detector fails on exactly the summaries it is supposed to
# pass: a session that "evicts" the key gets summarised as "evicting" it, and a
# surface-form comparison scores that honest paraphrase as an invention. Changing
# the word IS the job; changing the subject is the defect.
SUFFIXES = ("ations", "ation", "ings", "ing", "ies", "ed", "es", "s")
STEM_FLOOR = 4


def stem(word: str) -> str:
    for suffix in SUFFIXES:
        if word.endswith(suffix) and len(word) - len(suffix) >= STEM_FLOOR:
            return word[: -len(suffix)]
    return word


def anchors(summary: str) -> set[str]:
    """The concrete things a summary asserts exist, as stems."""
    body = DATE_STAMP.sub("", summary)
    found = set()
    for raw in TOKEN.findall(body):
        token = raw.strip("./-_").lower()
        if len(token) >= 4 and token not in STOP:
            found.add(stem(token))
    return found


def grounding(summary: str, session_text: str) -> tuple[float, list[str]] | None:
    """Fraction of the summary's anchors that occur in the session."""
    found = anchors(summary)
    if len(found) < MIN_ANCHORS:
        return None
    haystack = session_text.lower()
    missing = sorted(anchor for anchor in found if anchor not in haystack)
    return 1 - len(missing) / len(found), missing


def session_text(headless: str, transcript: Path) -> str:
    out = subprocess.run(
        [headless, "server", "remote", "generation-context", str(transcript)],
        capture_output=True,
        text=True,
        timeout=120,
    )
    return out.stdout if out.returncode == 0 else ""


def find_headless() -> str | None:
    local = Path(__file__).resolve().parent.parent / "target" / "release" / "yggterm-headless"
    if local.exists():
        return str(local)
    return shutil.which("yggterm-headless")


def index_transcripts(roots: list[Path]) -> dict[str, Path]:
    uuid = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
    index: dict[str, Path] = {}
    for root in roots:
        if not root.exists():
            continue
        for path in root.rglob("*.jsonl"):
            for found in uuid.findall(path.name):
                current = index.get(found)
                if current is None or path.stat().st_size > current.stat().st_size:
                    index[found] = path
    return index


# ===== the detector's own proof, on invented sessions =====
#
# A detector only ever observed passing has not been tested. Both arms are
# required: one that must FIRE and one that must stay QUIET. A rule that flags
# everything would satisfy the first arm alone and be useless.
SELFTEST_SESSION = """PRIMARY USER GOALS:
- work out why the widget cache keeps a stale key after a rename

RECENT SUBSTANTIVE TURNS:
USER: the rename path leaves the old cache key behind, find it
ASSISTANT: rename_widget writes the new key but never evicts the old one, so
lookups by the previous name keep resolving. The eviction belongs in
WidgetCache::rename, beside the insert.
USER: add a regression test that fails without the eviction
ASSISTANT: added; it renames a cached widget and asserts the old key misses.
USER: does anything else serve the stale entry
ASSISTANT: only the lookup path, and it stops serving it once the rename evicts.
The cache no longer holds an entry under a name that was found by the old key."""

SELFTEST_CASES = [
    (
        "grounded paraphrase of the session",
        "The objective is to stop the widget cache serving a stale entry after a "
        "rename. WidgetCache::rename was found to insert the new key without "
        "evicting the previous one, so lookups by the old name still resolve, and "
        "a regression test now covers the eviction.",
        False,
    ),
    (
        "fluent summary of a session that never happened",
        "The current objective is to integrate the new payment reconciliation "
        "middleware into the existing ledger service. The settlement endpoint was "
        "implemented and the nightly batch verified against the sandbox gateway. "
        "The remaining blocker is that webhook signatures fail validation under "
        "the rotated merchant key.",
        True,
    ),
]


def selftest() -> int:
    failures = []
    for name, summary, should_fire in SELFTEST_CASES:
        scored = grounding(summary, SELFTEST_SESSION)
        if scored is None:
            failures.append(f"{name}: too few anchors to judge — the case is vacuous")
            continue
        ratio, _missing = scored
        fired = ratio < GROUNDED_THRESHOLD
        verdict = "ok" if fired == should_fire else "WRONG"
        print(f"  [{verdict}] {name}: grounded={ratio:.2f} fired={fired}")
        if fired != should_fire:
            failures.append(
                f"{name}: expected fired={should_fire}, got {fired} (grounded={ratio:.2f})"
            )
    if failures:
        for failure in failures:
            print(f"SELFTEST FAILED: {failure}", file=sys.stderr)
        return 1
    print("summary-grounding detector selftest passed")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--home", default=os.path.expanduser("~/.yggterm"))
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--json", action="store_true")
    parser.add_argument(
        "--selftest",
        action="store_true",
        help="prove the detector both fires and stays quiet, on invented sessions",
    )
    parser.add_argument(
        "--clear",
        action="store_true",
        help="delete the ungrounded copy so the next scan regenerates it",
    )
    args = parser.parse_args()

    if args.selftest:
        return selftest()

    headless = find_headless()
    if headless is None:
        print(
            "no yggterm-headless on PATH or in target/release — it is the one "
            "owner of what a session said, and this audit is meaningless "
            "without it",
            file=sys.stderr,
        )
        return 2

    database = Path(args.home) / "session-titles.db"
    if not database.exists():
        print(f"no title store at {database}", file=sys.stderr)
        return 2

    home = Path(os.path.expanduser("~"))
    index = index_transcripts([home / ".codex", home / ".codex-litellm", home / ".claude" / "projects"])

    connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
    # A hand-written summary is the owner's own words, not generated copy. It is
    # allowed to say whatever he meant it to say, and scoring it against the
    # transcript flags him for paraphrasing himself.
    rows = connection.execute(
        "SELECT session_id, summary, model FROM session_summaries "
        "WHERE COALESCE(source, '') <> 'manual' AND COALESCE(model, '') <> 'manual'"
    ).fetchall()

    ungrounded, checked, skipped = [], 0, 0
    for session_id, summary, model in rows:
        transcript = index.get(session_id)
        if transcript is None:
            skipped += 1
            continue
        text = session_text(headless, transcript)
        if len(text) < 200:
            skipped += 1
            continue
        scored = grounding(summary, text)
        if scored is None:
            skipped += 1
            continue
        checked += 1
        ratio, missing = scored
        if ratio < GROUNDED_THRESHOLD:
            # The title is written from the same context by the same call, so it
            # is judged here too — but only when it is generated, and only when
            # it commits to enough to judge.
            title_row = connection.execute(
                "SELECT title FROM session_titles WHERE session_id = ?1 "
                "AND COALESCE(source, '') <> 'manual' AND COALESCE(model, '') <> 'manual'",
                (session_id,),
            ).fetchone()
            title_scored = grounding(title_row[0], text) if title_row else None
            ungrounded.append(
                {
                    "session_id": session_id,
                    "model": model,
                    "grounded": round(ratio, 3),
                    "absent_from_session": missing[:12],
                    "title_ungrounded": bool(
                        title_scored and title_scored[0] < GROUNDED_THRESHOLD
                    ),
                }
            )
        if args.limit and checked >= args.limit:
            break

    ungrounded.sort(key=lambda item: item["grounded"])
    if args.json:
        print(json.dumps({"checked": checked, "skipped": skipped, "ungrounded": ungrounded}, indent=2))
    else:
        print(f"checked {checked} summaries ({skipped} skipped: no transcript, or too short)")
        print(f"ungrounded (below {GROUNDED_THRESHOLD}): {len(ungrounded)}")
        for item in ungrounded:
            print(
                f"  {item['grounded']:.2f}  {item['session_id']}  {item['model']}"
                f"{'  [title too]' if item['title_ungrounded'] else ''}\n"
                f"        absent from the session: {', '.join(item['absent_from_session'])}"
            )

    if args.clear and ungrounded:
        connection.close()
        writable = sqlite3.connect(database)
        titles_cleared = 0
        for item in ungrounded:
            session_id = item["session_id"]
            writable.execute(
                "DELETE FROM session_summaries WHERE session_id = ?1", (session_id,)
            )
            writable.execute(
                "DELETE FROM session_summary_timeline WHERE session_id = ?1", (session_id,)
            )
            if item["title_ungrounded"]:
                writable.execute(
                    "DELETE FROM session_titles WHERE session_id = ?1 "
                    "AND COALESCE(source, '') <> 'manual' AND COALESCE(model, '') <> 'manual'",
                    (session_id,),
                )
                titles_cleared += 1
        writable.commit()
        writable.close()
        print(
            f"\ncleared {len(ungrounded)} summaries and {titles_cleared} titles; "
            "the next copy scan regenerates them from the transcript"
        )
    elif args.clear:
        print("\nnothing to clear")

    return 1 if ungrounded else 0


if __name__ == "__main__":
    raise SystemExit(main())
