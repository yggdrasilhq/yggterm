#!/usr/bin/env python3
"""Composer live-tail repair — focused hermetic tests for CSR-06.

An append-stream source can never be CLOSED: its EOF is merely the
current EOF. A trailing record without its terminator is PENDING — the
composer retains the bounded unfinished byte POSITION (never the raw
bytes), mints nothing from incomplete bytes, claims no coverage over
them, and when a later append terminates the record it is mined EXACTLY
ONCE with its full raw span — the same fault occurrence, at the same
byte span, with the same fingerprint that the static final bytes yield.
Closed-snapshot semantics survive only behind the EXPLICIT trusted
closed contract (read_page closed=True) — never inferred from idleness.

Hermetic: tempfile under the caller's TMPDIR (disk-backed), no network,
no model calls, no live state. Python-only.

Run:  python3 tools/swarmharness/test_composer_live_tail.py
"""
import inspect
import importlib.util
import json
import shutil
import sys
import tempfile
import types
from pathlib import Path
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import composer as cp  # noqa: E402

PASS, FAIL = [], []


def check(name, cond, detail=""):
    (PASS if cond else FAIL).append(name)
    tag = "ok  " if cond else "FAIL"
    line = f"{tag} {name}"
    if detail and not cond:
        line += f" — {detail}"
    print(line)


def fresh(tag):
    return Path(tempfile.mkdtemp(prefix=f"composer-livetail-{tag}-"))


FAULT = b"NameError: name 'widget' is not defined\n"
PENDING = FAULT[:-1]  # the same record, writer mid-write (no terminator)


def live_src(src):
    """Enroll src as a LIVE append stream under the EXPLICIT CSR-06
    declaration — closedness is declared, never inferred; a plain path
    would enroll the historical finite-snapshot contract instead."""
    return [{"path": str(src), "closed": False}]


def fault_rows(state):
    return [o for o in cp.load_observations(state)
            if o["detector_id"] == "sig-nameerror-typo"]


def cursor_of(state, key):
    return json.loads((state / "cursors.json").read_text())[key]


def anchors_of(state):
    p = state / "anchors.json"
    return json.loads(p.read_text()) if p.exists() else {}


def static_reference(w, payload, tag="static"):
    """One tick over the identical final bytes in a fresh state — the
    CSR-06 ground truth every live scenario must reproduce. The static
    file is a finite closed snapshot (plain path = historical closed
    contract): its final tail is emitted at eof."""
    src = w / f"{tag}.log"
    src.write_bytes(payload)
    cp.tick({"queue_run": [str(src)]}, state_dir=str(w / f"{tag}-state"),
            max_pages=10)
    rows = fault_rows(w / f"{tag}-state")
    return [(o["span"], o["fingerprint"]) for o in rows], len(rows)


def t_c06_live_tail_completed_matches_static():
    """The signed CSR-06 counterexample: a NameError record arrives as an
    unterminated prefix, a later append completes it. The stream must
    mine the SAME fault occurrence (count, byte span, fingerprint) as the
    static final bytes, exactly once — pre-repair this recorded 0."""
    w = fresh("core")
    state = w / "state"
    src = w / "live.log"
    src.write_bytes(PENDING)
    r1 = cp.tick({"queue_run": live_src(src)}, state_dir=str(state),
                 max_pages=10)
    check("L1-1 tick over the bare prefix mints NOTHING from incomplete "
          "bytes", len(fault_rows(state)) == 0,
          json.dumps(len(fault_rows(state))))
    check("L1-2 the pending pass claims NO coverage (the record is not "
          "consumed)", r1["coverage_complete"] == 0,
          json.dumps(r1["coverage_complete"]))
    check("L1-3 the pending pass NAMES the retained position (typed, not "
          "silent)", any("live tail pending" in n for n in r1["notes"]),
          json.dumps(r1["notes"])[:200])
    cur1 = (json.loads((state / "cursors.json").read_text())
            .get("queue_run::" + str(src), {}).get("cursor", 0)
            if (state / "cursors.json").exists() else 0)
    check("L1-4 the cursor retained the bounded unfinished POSITION (the "
          "record's start byte — at the file head it is the trivial 0)",
          cur1 == 0, json.dumps(cur1))
    src.write_bytes(FAULT)  # the append completes the record
    r2 = cp.tick({"queue_run": live_src(src)}, state_dir=str(state),
                 max_pages=10)
    rows = fault_rows(state)
    ref, ref_n = static_reference(w, FAULT)
    check("L1-5 the completed record is mined EXACTLY ONCE with the "
          "static fault occurrence (span+fingerprint identical)",
          len(rows) == 1 and ref_n == 1
          and [(o["span"], o["fingerprint"]) for o in rows] == ref,
          json.dumps({"live": [(o["span"], o["fingerprint"])
                               for o in rows], "static": ref}))
    check("L1-6 after completion the stream cursor reaches the true EOF",
          cursor_of(state, "queue_run::" + str(src))["cursor"]
          == src.stat().st_size,
          json.dumps(cursor_of(state, "queue_run::" + str(src))))
    check("L1-7 the completed pass stops noting the pending tail",
          not any("live tail pending" in n for n in r2["notes"]),
          json.dumps(r2["notes"])[:200])
    # exactly-once under replay: repeated ticks add no duplicate rows
    for _ in range(2):
        cp.tick({"queue_run": live_src(src)}, state_dir=str(state),
                max_pages=10)
    check("L1-8 repeated ticks after completion stay exactly-once (v2 "
          "dedupe replay is a no-op)", len(fault_rows(state)) == 1,
          json.dumps(len(fault_rows(state))))
    # the persistent detector state carried the POSITION, never the bytes
    cursors_text = (state / "cursors.json").read_text()
    check("L1-9 the persistent cursor never persisted the pending raw "
          "bytes", PENDING.decode() not in cursors_text
          and len(cursors_text) < 2048,
          f"cursors.json held {len(cursors_text)} bytes")
    shutil.rmtree(w)


def t_c06_multiple_appends():
    """A record completed across THREE appends: intermediate ticks stay
    empty and honest; the terminating append mines the whole record."""
    w = fresh("multiappend")
    state = w / "state"
    src = w / "live.log"
    pieces = [b"NameError: na", b"me 'widget' is", b" not defined\n"]
    src.write_bytes(pieces[0])
    obs_after = []
    coverage = []
    cp.tick({"queue_run": live_src(src)}, state_dir=str(state), max_pages=10)
    obs_after.append(len(fault_rows(state)))
    coverage.append(cp.tick({"queue_run": live_src(src)}, state_dir=str(state),
                            max_pages=10)["coverage_complete"])
    src.write_bytes(b"".join(pieces[:2]))
    cp.tick({"queue_run": live_src(src)}, state_dir=str(state), max_pages=10)
    obs_after.append(len(fault_rows(state)))
    src.write_bytes(FAULT)
    rep = cp.tick({"queue_run": live_src(src)}, state_dir=str(state),
                  max_pages=10)
    obs_after.append(len(fault_rows(state)))
    ref, ref_n = static_reference(w, FAULT)
    check("L2-1 no observation is minted while the record is incomplete "
          "across appends", obs_after[:2] == [0, 0],
          json.dumps(obs_after))
    check("L2-2 the third append completes the record and it is mined "
          "exactly once, identical to static",
          obs_after[2] == 1 and ref_n == 1
          and [(o["span"], o["fingerprint"])
               for o in fault_rows(state)] == ref,
          json.dumps({"live": obs_after, "static": ref}))
    check("L2-3 coverage is claimed only once the record terminated "
          "(earlier passes claimed none; the final pass claims the "
          "consumed view)", coverage == [0] and rep["coverage_complete"] == 1,
          json.dumps({"mid": coverage, "final": rep["coverage_complete"]}))
    shutil.rmtree(w)


def t_c06_crlf_split_across_append():
    """A final live CR stays ambiguous until a following byte arrives.

    It must not consume detector state and later parse LF as another
    record. Closed immutable snapshots still accept a final bare CR.
    """
    w = fresh("crlf")
    state = w / "state"
    src = w / "live.log"
    body = FAULT[:-1]                       # the fault line without its LF
    split = body.index(b"defined")          # split the actual fault
    src.write_bytes(body[:split])           # content before the CR exists
    r1 = cp.tick({"queue_run": live_src(src)}, state_dir=str(state),
                 max_pages=10)
    check("L3-1 the content split before the CR is PENDING (no "
          "terminator has arrived; nothing is minted from incomplete "
          "bytes)", len(fault_rows(state)) == 0
          and any("live tail pending" in n for n in r1["notes"]),
          json.dumps([len(fault_rows(state)), r1["notes"][:2]]))
    src.write_bytes(body + b"\r")
    cp.tick({"queue_run": live_src(src)}, state_dir=str(state), max_pages=10)
    check("L3-2 ambiguous final live CR retains the whole record",
          not fault_rows(state))
    src.write_bytes(body + b"\r\n")
    cp.tick({"queue_run": live_src(src)}, state_dir=str(state), max_pages=10)
    rows = fault_rows(state)
    ref, ref_n = static_reference(w, body + b"\r\n")
    check("L3-3 completing LF emits exactly the identical static record",
          len(rows) == ref_n == 1
          and [(o["span"], o["fingerprint"]) for o in rows] == ref)
    # variant: the split lands BETWEEN records — record boundary \r\n
    w2 = fresh("crlf-between")
    state2 = w2 / "state"
    src2 = w2 / "live.log"
    static_pair = b"$ ls\r\n" + FAULT
    src2.write_bytes(b"$ ls\r")
    cp.tick({"queue_run": live_src(src2)}, state_dir=str(state2), max_pages=10)
    src2.write_bytes(static_pair)
    cp.tick({"queue_run": live_src(src2)}, state_dir=str(state2), max_pages=10)
    rows2 = fault_rows(state2)
    ref2, ref2_n = static_reference(w2, static_pair)
    check("L3-4 a \\r\\n split between records still yields the static "
          "fault occurrence at its exact offset, exactly once",
          len(rows2) == 1 and ref2_n == 1
          and [(o["span"], o["fingerprint"]) for o in rows2] == ref2,
          json.dumps({"live": [(o["span"], o["fingerprint"])
                               for o in rows2], "static": ref2}))
    shutil.rmtree(w)
    shutil.rmtree(w2)


def t_c06_invalid_utf8_split_across_append():
    """A multi-byte UTF-8 character split mid-sequence across appends,
    inside the fault line. The pending prefix must NOT be decoded or
    detected (no replacement-char fragment); the completed record decodes
    once from full bytes and matches the static fingerprint exactly."""
    w = fresh("utf8split")
    state = w / "state"
    src = w / "live.log"
    full = "故障 NameError: name 'w' is not defined\n".encode("utf-8")
    cut = full.index("NameError".encode()) - 2  # inside the 3-byte char
    # (the byte before "NameError" is the ASCII space; one byte further
    # back is the last continuation byte of the multi-byte character)
    assert cut > 0 and (full[cut] & 0xC0) == 0x80  # cut is a continuation
    src.write_bytes(full[:cut])
    page = cp.read_page(src, 0, 262144, closed=False)
    check("L4-1 a mid-codepoint pending prefix is HELD undecoded (no "
          "replacement-char fragment is emitted)",
          page[0] == "" and page[1] == 0 and page[2] is False
          and page.raw == b"",
          json.dumps([page[0][:40], page[1], page[2]]))
    cp.tick({"queue_run": live_src(src)}, state_dir=str(state), max_pages=10)
    check("L4-2 the split-prefix tick mints nothing",
          len(fault_rows(state)) == 0, json.dumps(len(fault_rows(state))))
    src.write_bytes(full)
    cp.tick({"queue_run": live_src(src)}, state_dir=str(state), max_pages=10)
    rows = fault_rows(state)
    ref, ref_n = static_reference(w, full)
    check("L4-3 the completed record decodes once from full bytes and "
          "matches the static occurrence exactly",
          len(rows) == 1 and ref_n == 1
          and [(o["span"], o["fingerprint"]) for o in rows] == ref,
          json.dumps({"live": [(o["span"], o["fingerprint"])
                               for o in rows], "static": ref}))
    shutil.rmtree(w)


def t_c06_oversized_tail_drop_bounded():
    """Oversized unfinished bytes never advance a live frontier.

    The reference is the IDENTICAL final static bytes, not a substituted
    small record. Records larger than bounded lookahead remain held;
    their suffix can never be reinterpreted as a new record.
    """
    w = fresh("oversized")
    state = w / "state"
    src = w / "live.log"
    big = b"x" * 70000
    src.write_bytes(big)
    page = cp.read_page(src, 0, 262144, closed=False)
    check("L5-1 oversized live prefix retains its start without evidence",
          page[0] == "" and page[1] == 0 and not page[2]
          and page.raw == b"")
    r1 = cp.tick({"queue_run": live_src(src)}, state_dir=str(state),
                 max_pages=10)
    check("L5-2 oversized incomplete record claims no complete coverage",
          r1["coverage_complete"] == 0 and r1["new_tail_bytes"] == 0
          and r1["pages_read"] == 0
          and any("live tail pending" in n for n in r1["notes"]))
    check("L5-3 incomplete raw bytes are never persisted as detector state",
          len(fault_rows(state)) == 0 and not anchors_of(state))
    with src.open("ab") as f:
        f.write(FAULT)
    cp.tick({"queue_run": live_src(src)}, state_dir=str(state), max_pages=10)
    reference, ref_n = static_reference(w, big + FAULT)
    rows = fault_rows(state)
    check("L5-4 completed oversized record matches identical static bytes",
          len(rows) == ref_n == 1
          and [(o["span"], o["fingerprint"]) for o in rows] == reference
          and rows[0]["span"] == "bytes@0")
    cp.tick({"queue_run": live_src(src)}, state_dir=str(state), max_pages=10)
    check("L5-5 completed oversized record remains exactly once",
          len(fault_rows(state)) == 1)
    # A record beyond one lookahead page is not claimed consumed even if
    # a terminator exists outside the bounded window. No suffix evidence.
    huge = w / "huge.log"
    huge.write_bytes(b"x" * 300000 + FAULT)
    huge_state = w / "huge-state"
    for _ in range(2):
        held = cp.tick({"queue_run": live_src(huge)},
                       state_dir=str(huge_state), max_pages=10)
    check("L5-6 beyond-lookahead record stays held with bounded no progress",
          held["coverage_complete"] == 0 and held["pages_read"] == 0
          and not fault_rows(huge_state) and not anchors_of(huge_state)
          and any("live tail pending" in n for n in held["notes"]))
    shutil.rmtree(w)


def t_c06_crash_replay_pending():
    """Crash replay with a pending tail. Variant A: the anchor write
    crashes after the cursor committed at the pending position — the
    next pass recovers via the explicit anchor-absent reset, re-derives
    prior evidence (dedupe no-op), holds the pending record again, and
    the completed record is still mined exactly once. Variant B: the
    anchor write crashes on a LATER pass, leaving the cursor ahead of
    the anchored frontier across the completed record — the CSR-03 gap
    reconcile replays it with no duplicate evidence."""
    w = fresh("crash")
    state = w / "state"
    src = w / "live.log"
    head = b"$ ok\n"
    src.write_bytes(head + PENDING)
    with patch.object(cp, "_save_anchors",
                      side_effect=OSError("injected anchor write crash")):
        try:
            cp.tick({"queue_run": live_src(src)}, state_dir=str(state),
                    max_pages=10)
        except OSError:
            pass
    crashed = cursor_of(state, "queue_run::" + str(src))["cursor"]
    check("L6-1 the injected crash reproduces the post-commit state: "
          "cursor at the pending position, no anchor saved",
          crashed == len(head)
          and "queue_run::" + str(src) not in anchors_of(state),
          json.dumps({"cursor": crashed,
                      "anchors": sorted(anchors_of(state))}))
    rep = cp.tick({"queue_run": live_src(src)}, state_dir=str(state),
                  max_pages=10)
    check("L6-2 recovery names the reset explicitly and holds the "
          "pending record again (no silent reuse, no consumption)",
          any("reset" in n or "re-scan" in n for n in rep["notes"])
          and any("live tail pending" in n for n in rep["notes"])
          and len(fault_rows(state)) == 0,
          json.dumps(rep["notes"])[:300])
    src.write_bytes(head + FAULT + b"$ after\n")
    cp.tick({"queue_run": live_src(src)}, state_dir=str(state), max_pages=10)
    rows = fault_rows(state)
    ref, ref_n = static_reference(w, head + FAULT + b"$ after\n")
    check("L6-3 across the crash and its recovery the completed record "
          "is mined exactly once, identical to static",
          len(rows) == 1 and ref_n == 1
          and [(o["span"], o["fingerprint"]) for o in rows] == ref,
          json.dumps({"live": [(o["span"], o["fingerprint"])
                               for o in rows], "static": ref}))

    # Variant B: cursor AHEAD of the anchored frontier across the record
    w2 = fresh("crash-gap")
    state2 = w2 / "state"
    src2 = w2 / "live.log"
    src2.write_bytes(head + PENDING)
    cp.tick({"queue_run": live_src(src2)}, state_dir=str(state2),
            max_pages=10)  # clean pass: cursor at pending start, anchor saved
    src2.write_bytes(head + FAULT + b"$ after\n")
    with patch.object(cp, "_save_anchors",
                      side_effect=OSError("injected anchor write crash")):
        try:
            cp.tick({"queue_run": live_src(src2)}, state_dir=str(state2),
                    max_pages=10)
        except OSError:
            pass
    gap_cur = cursor_of(state2, "queue_run::" + str(src2))["cursor"]
    gap_anchor = anchors_of(state2)["queue_run::" + str(src2)]["size"]
    check("L6-4 the injected later crash leaves the cursor ahead of the "
          "anchored frontier (the anchor stays at the consumed "
          "frontier per L9 — the gap spans the completed record)",
          gap_cur == len(head + FAULT + b"$ after\n")
          and gap_anchor == len(head),
          json.dumps({"cursor": gap_cur, "anchor": gap_anchor}))
    rep2 = cp.tick({"queue_run": live_src(src2)}, state_dir=str(state2),
                   max_pages=10)
    rows2 = fault_rows(state2)
    ref2, ref2_n = static_reference(w2, head + FAULT + b"$ after\n")
    check("L6-5 the gap reconcile names the crash gap and evidence stays "
          "exactly-once, identical to static",
          any("crash gap" in n for n in rep2["notes"])
          and len(rows2) == 1 and ref2_n == 1
          and [(o["span"], o["fingerprint"]) for o in rows2] == ref2,
          json.dumps({"notes": rep2["notes"][:3],
                      "live": [(o["span"], o["fingerprint"])
                               for o in rows2]}))
    gap_replays = []
    for _ in range(2):
        rep3 = cp.tick({"queue_run": live_src(src2)}, state_dir=str(state2),
                       max_pages=10)
        gap_replays += [n for n in rep3["notes"] if "crash gap" in n]
    check("L6-6 the reconcile is crash-replay idempotent (one-time gap "
          "notes, stable evidence)",
          not gap_replays and len(fault_rows(state2)) == 1,
          json.dumps({"notes": gap_replays, "rows": len(
              fault_rows(state2))}))
    shutil.rmtree(w)
    shutil.rmtree(w2)


def t_c06_never_complete_no_false_coverage():
    """A pending record the writer never terminates stays pending
    honestly: zero observations, the position stable, no coverage claim,
    no busy loop, no raw content persisted."""
    w = fresh("never")
    state = w / "state"
    src = w / "live.log"
    src.write_bytes(b"$ ok\n" + PENDING)
    cursors_seen, coverage_seen, notes_seen, rows_seen = [], [], [], []
    for _ in range(3):
        rep = cp.tick({"queue_run": live_src(src)}, state_dir=str(state),
                      max_pages=10)
        cursors_seen.append(
            cursor_of(state, "queue_run::" + str(src))["cursor"])
        coverage_seen.append(rep["coverage_complete"])
        notes_seen.append(any("live tail pending" in n for n in rep["notes"]))
        rows_seen.append(len(fault_rows(state)))
    check("L7-1 the never-terminated record mines nothing across passes",
          rows_seen == [0, 0, 0], json.dumps(rows_seen))
    check("L7-2 the cursor stays at the retained position every pass "
          "(stable, bounded, no drift)",
          cursors_seen == [len(b"$ ok\n")] * 3, json.dumps(cursors_seen))
    check("L7-3 no coverage is ever claimed over the unconsumed bytes",
          coverage_seen == [0, 0, 0], json.dumps(coverage_seen))
    check("L7-4 every pass names the pending tail (honest reporting, no "
          "silent stall)", all(notes_seen), json.dumps(notes_seen))
    cursors_text = (state / "cursors.json").read_text()
    check("L7-5 the pending raw bytes were never persisted in detector "
          "state", PENDING.decode() not in cursors_text
          and len(cursors_text) < 2048,
          f"cursors.json held {len(cursors_text)} bytes")
    shutil.rmtree(w)


def t_c06_closed_contract_explicit():
    """The closed-snapshot semantics survive behind the EXPLICIT
    contract only: default and closed=True emit the final unterminated
    tail (finite closed-source EOF handling preserved — T31-5); "
    closed=False holds the record; closedness is a parameter, never an
    inference from idleness."""
    w = fresh("closed")
    src = w / "snap.log"
    src.write_bytes(b"err one\nunterminated tail")
    size = src.stat().st_size
    d1 = cp.read_page(src, 0, 262144)
    d2 = cp.read_page(src, d1[1], 262144)
    c1 = cp.read_page(src, 0, 262144, closed=True)
    c2 = cp.read_page(src, c1[1], 262144, closed=True)
    l1 = cp.read_page(src, 0, 262144, closed=False)
    l2 = cp.read_page(src, l1[1], 262144, closed=False)
    check("L8-1 default read_page keeps the closed-snapshot semantics "
          "(existing controls preserved: the final unterminated tail "
          "is emitted at eof — page 2 — done at EOF, cursor at size)",
          d1[0] == "err one\n" and d1[1] == 8 and d1[2] is False
          and d2[0] == "unterminated tail" and d2[1] == size
          and d2[2] is True, json.dumps([d1[:3], d2[:3]]))
    check("L8-2 the EXPLICIT closed contract is byte-identical to the "
          "historical default", (c1[0], c1[1], c1[2]) == (d1[0], d1[1],
          d1[2]) and (c2[0], c2[1], c2[2]) == (d2[0], d2[1], d2[2]),
          json.dumps([c1[:3], c2[:3]]))
    check("L8-3 the live contract holds the tail: the terminated "
          "record is emitted the same, the unterminated tail is HELD "
          "(nothing emitted, position retained at the record start, "
          "not done)",
          l1[0] == "err one\n" and l1[1] == 8 and l1[2] is False
          and l2[0] == "" and l2[1] == 8 and l2[2] is False
          and l2.raw == b"", json.dumps([l1[:3], l2[:3]]))
    sig = inspect.signature(cp.read_page).parameters["closed"]
    check("L8-4 the contract is an explicit parameter with the "
          "historical default", sig.default is True,
          repr(sig.default))
    # static equivalence of a completed live read
    src.write_bytes(b"err one\nunterminated tail\n")
    t4, c4, d4, _ = cp.read_page(src, 8, 262144, closed=False)
    check("L8-5 a later append completes the held record: the live read "
          "from the retained position yields the full record byte-exact",
          t4 == "unterminated tail\n" and c4 == src.stat().st_size
          and d4 is True, json.dumps([t4, c4, d4]))
    shutil.rmtree(w)


def t_c06_anchor_frontier_honesty():
    """The anchor, its frontier detector checkpoint and the coverage
    accounting stop at the consumed frontier: unconsumed pending bytes
    are never silently validated or claimed."""
    w = fresh("frontier")
    state = w / "state"
    src = w / "live.log"
    head = b"$ ok\n"
    src.write_bytes(head + PENDING)
    r1 = cp.tick({"queue_run": live_src(src)}, state_dir=str(state),
                 max_pages=10)
    key = "queue_run::" + str(src)
    cur = cursor_of(state, key)["cursor"]
    an = anchors_of(state)[key]
    consumed = head + PENDING
    check("L9-1 the anchor frontier equals the cursor at the pending "
          "record's start (the unconsumed bytes are outside the anchor)",
          an["size"] == cur == len(head)
          and an["head_sha"] == cp._sha(consumed[:len(head)]),
          json.dumps({"anchor": an["size"], "cursor": cur}))
    check("L9-2 anchor chunks cover only consumed bytes — none reaches "
          "into the pending record",
          all(c["start"] + c["len"] <= cur for c in an["chunks"]),
          json.dumps(an["chunks"]))
    det = an.get("det") or {}
    check("L9-3 the frontier detector checkpoint carries no content and "
          "no state minted from the pending bytes",
          set(det) <= {"cmd_fps", "pending_fps", "att_offs"}
          and all(isinstance(v, dict) and all(
              isinstance(k, str) and len(k) <= 16 for k in v)
              for v in [det.get("cmd_fps", {})]),
          json.dumps(det)[:120])
    check("L9-4 the pending pass claims no coverage_complete",
          r1["coverage_complete"] == 0, json.dumps(r1["coverage_complete"]))
    src.write_bytes(head + FAULT)
    r2 = cp.tick({"queue_run": live_src(src)}, state_dir=str(state),
                 max_pages=10)
    an2 = anchors_of(state)[key]
    check("L9-5 after completion the anchor frontier covers exactly the "
          "consumed bytes and the growth was accepted as append",
          an2["size"] == len(FAULT) + len(head)
          and cursor_of(state, key)["cursor"] == src.stat().st_size
          and any("prefix progression" in n for n in r2["notes"]),
          json.dumps({"anchor": an2["size"],
                      "cursor": cursor_of(state, key)["cursor"],
                      "notes": [n for n in r2["notes"]
                                if "progression" in n][:1]}))
    shutil.rmtree(w)


def t_c06_operational_manifest():
    """The real timer boundary must protect legacy plain-path enrollments."""
    w = fresh("operational")
    spec = importlib.util.spec_from_file_location(
        "composer_live_tail_runner", HERE / "composer-tick.py")
    runner = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(runner)
    runner.MANIFEST, runner.STATE, runner.LOG = (
        w / "sources.json", w / "state", w / "tick.jsonl")
    src = w / "live.log"
    src.write_bytes(PENDING)
    runner.MANIFEST.write_text(json.dumps(
        {"sources": {"queue_run": [str(src)]}}))
    with patch.dict(sys.modules, {"composer_review":
                                 types.SimpleNamespace(main=lambda: 0)}):
        runner.main()
        first = json.loads(runner.LOG.read_text().splitlines()[0])
        check("L10-1 real timer legacy enrollment holds unfinished bytes",
              not fault_rows(runner.STATE) and first["coverage_complete"] == 0
              and any("live tail pending" in n for n in first["notes"]))
        with src.open("ab") as f:
            f.write(b"\n")
        runner.main()
    ref, count = static_reference(w, FAULT)
    check("L10-2 real timer append recovers the static occurrence",
          count == 1 and [(o["span"], o["fingerprint"])
                          for o in fault_rows(runner.STATE)] == ref)
    invalid = cp.census({"queue_run": [{"path": str(src)}, 17]})
    check("L10-3 malformed declarations are typed gaps, never guessed",
          len(invalid["gaps"]) == 2 and not invalid["sources"])
    shutil.rmtree(w)


def t_c06_crlf_carried_hunt():
    w = fresh("crlf-hunt")
    source, state = w / "live", w / "state"
    prefix = b"$ build\r\n$ build\r"
    suffix = (b"\nnoise-one\r\nnoise-two\r\n"
              b"NameError: name 'widget' is not defined\r\n")
    source.write_bytes(prefix)
    first = cp.tick({"queue_run": live_src(source)}, state_dir=state)
    check("CRLF hunt pending terminator cannot claim complete coverage",
          first["coverage_complete"] == 0
          and any("live tail pending" in n for n in first["notes"]))
    with source.open("ab") as stream:
        stream.write(suffix)
    cp.tick({"queue_run": live_src(source)}, state_dir=state)
    reference_state = w / "reference"
    cp.tick({"queue_run": [str(source)]}, state_dir=reference_state)
    def observations(path):
        return sorted((o["detector_id"], o["span"], o["fingerprint"])
                      for o in cp.load_observations(path))
    actual, expected = observations(state), observations(reference_state)
    check("CRLF split preserves carried command hunt and signature",
          actual == expected and len(actual) == 2
          and any(o[0] == "cmd-failure-loop" and o[1] == "bytes@40"
                  for o in actual), repr((actual, expected)))
    # Exercise a finite page boundary immediately after CR as well.
    page_state = w / "page-state"
    cp.tick({"queue_run": [str(source)]}, state_dir=page_state,
            max_bytes_per_source=8, max_pages=20)
    check("CRLF page lookahead preserves exact static detector records",
          observations(page_state) == expected)
    shutil.rmtree(w)


def main():
    t_c06_live_tail_completed_matches_static()
    t_c06_multiple_appends()
    t_c06_crlf_split_across_append()
    t_c06_invalid_utf8_split_across_append()
    t_c06_oversized_tail_drop_bounded()
    t_c06_crash_replay_pending()
    t_c06_never_complete_no_false_coverage()
    t_c06_closed_contract_explicit()
    t_c06_anchor_frontier_honesty()
    t_c06_operational_manifest()
    t_c06_crlf_carried_hunt()
    print(f"\n{len(PASS)} passed, {len(FAIL)} failed")
    if FAIL:
        print("FAILED:", *FAIL, sep="\n  - ")
    return 1 if FAIL else 0


if __name__ == "__main__":
    sys.exit(main())
