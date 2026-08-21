#!/usr/bin/env python3
"""A row's id is the one the daemon published, never a slice of its address.

    python3 tests/test_row_identity_is_the_rows_own_answer.py

⛔ THE HOLE THIS PINS, measured 2026-08-22 against a live row plane: the fleet
derived every row's identity as `full_path.rsplit("/", 1)[-1]`, which is correct
only when the address ENDS in the id. **246 of 281 session rows derived an id
that was not the session's id**, and five rows of one CLI collapsed onto a single
value, because that CLI spells identity as a directory and names every session
file the same thing.

⚠ It survived because the rows a lane touches most are LIVE, and a live row wears
a `scheme://host/<uuid>` address whose tail really is the id. Every row at rest is
a store path. So the slice is right in front of you and wrong further down the
tree — and the failures are silent: a lookup misses, a guard does not match, a set
gains a member named after a file.

Every fixture here is invented. The shapes are real; the ids and paths are not.
"""
import importlib.util
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(HERE))
spec = importlib.util.spec_from_file_location("ygg_rowarg", HERE / "ygg_rowarg.py")
rowarg = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rowarg)

FAILURES = []


def check(name, got, want):
    if got != want:
        FAILURES.append(f"{name}: got {got!r}, wanted {want!r}")


# ── The four address shapes a row can wear ──────────────────────────────────
# A live agent row: the address ends in the id, so the old slice agreed.
LIVE = {"session_id": "aaaaaaaa-1111-4aaa-8aaa-aaaaaaaaaaaa",
        "full_path": "remote-xx://buildbox/aaaaaaaa-1111-4aaa-8aaa-aaaaaaaaaaaa"}
# A row at rest whose store names the session's DIRECTORY and gives every
# session the same file name. This is the shape that collapsed.
NESTED_A = {"session_id": "bbbbbbbb-2222-4bbb-8bbb-bbbbbbbbbbbb",
            "full_path": "/home/u/.demo-cli/brain/bbbbbbbb-2222-4bbb-8bbb-bbbbbbbbbbbb"
                         "/.generated/logs/transcript_full.jsonl"}
NESTED_B = {"session_id": "cccccccc-3333-4ccc-8ccc-cccccccccccc",
            "full_path": "/home/u/.demo-cli/brain/cccccccc-3333-4ccc-8ccc-cccccccccccc"
                         "/.generated/logs/transcript_full.jsonl"}
# A row at rest whose file name DECORATES the id rather than being it.
DECORATED = {"session_id": "01234567-4444-4ddd-8ddd-dddddddddddd",
             "full_path": "/home/u/.demo-cli/sessions/2026/rollout-2026-01-02T03-04-05"
                          "-01234567-4444-4ddd.jsonl"}

check("a live row keeps the answer the slice already gave",
      rowarg.row_session_id(LIVE), LIVE["session_id"])
check("a nested store row reports its own id",
      rowarg.row_session_id(NESTED_A), NESTED_A["session_id"])
check("a decorated file name is not mistaken for the id",
      rowarg.row_session_id(DECORATED), DECORATED["session_id"])

# ⛔ THE COLLAPSE ITSELF. Two distinct rows must never share an identity — that
#    is what let one row's guard, seat and transcript stand in for another's.
ids = {rowarg.row_session_id(r) for r in (NESTED_A, NESTED_B)}
check("two rows of a same-file-name store stay two identities", len(ids), 2)
sliced = {r["full_path"].rsplit("/", 1)[-1] for r in (NESTED_A, NESTED_B)}
check("the fixture reproduces the defect it pins (old slice collapses them)",
      len(sliced), 1)

# A row with no id at all — a folder, a group, a machine — still needs a stable
# key, and there the address tail is the best available answer.
check("a row that carries no id falls back to its address tail",
      rowarg.row_session_id({"full_path": "/home/u/code/project"}), "project")

# ⛔ The two questions must not be confused: `bare_uuid` reads a COMMAND LINE,
#    where there is no row to ask, and must keep taking the last segment.
check("bare_uuid still answers the command-line question",
      rowarg.bare_uuid("remote-xx://buildbox/" + LIVE["session_id"]), LIVE["session_id"])

try:
    rowarg.row_session_id(LIVE["full_path"])
    FAILURES.append("row_session_id accepted a string: it must demand the row's JSON, "
                    "because a string is exactly what cannot answer this question")
except TypeError:
    pass

if FAILURES:
    print("FAIL")
    for f in FAILURES:
        print("  ⛔", f)
    sys.exit(1)
print(f"ok — row identity is the row's own answer ({6 - len(FAILURES)} checks)")
