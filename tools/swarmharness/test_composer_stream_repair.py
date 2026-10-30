#!/usr/bin/env python3
"""Composer append-stream repair — focused hermetic conformance tests.

Owner-authorized MODERATE repair lane (composer-stream-repair, base
e7cdfdef). Covers the six acceptance criteria that live in the composer
kernel: append-only progression with retained detector/cursor progress (F1),
stable logical observation identity with explicit v1->v2 store migration
(F2), sampled-token integrity with typed truncation / lineage / interior
rewrite reconciliation (F6), and crash/concurrency-consistent durable
commits. Kernel-only: no review side (sibling lane), no network, no model
calls, no private session bodies, no live-state writes.

Run:  python3 tools/swarmharness/test_composer_stream_repair.py
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import uuid
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
    return Path(tempfile.mkdtemp(prefix=f"composer-repair-{tag}-"))


FAULT = "bash: -c: line 1: syntax error near unexpected token `('\n"
FAULT2 = "NameError: name 'zz' is not defined\n"


def cursor_of(state, key):
    return json.loads((state / "cursors.json").read_text())[key]


def anchors_of(state):
    p = state / "anchors.json"
    return json.loads(p.read_text()) if p.exists() else {}


def reset_notes(rep):
    return [n for n in rep["notes"] if any(
        m in n for m in ("re-scanning", "reset", "re-scan", "mutated",
                         "truncated"))]


# =====================================================================
# A1 — append-only progression retains progress; finite prefix completes
# =====================================================================

def t_a1_progression_and_completion():
    w = fresh("a1")
    state = w / "state"
    src = w / "board.jsonl"
    src.write_text(FAULT * 200)
    sources = {"board": [str(src)]}

    r1 = cp.tick(sources, state_dir=state, max_bytes_per_source=2048,
                 max_pages=2)
    c1 = cursor_of(state, "board::" + str(src))["cursor"]

    # fleet appends between ticks
    appended = FAULT * 50
    with src.open("a") as f:
        f.write(appended)

    r2 = cp.tick(sources, state_dir=state, max_bytes_per_source=2048,
                 max_pages=10)
    c2 = cursor_of(state, "board::" + str(src))["cursor"]
    check("A1-1 appended source progresses from the stored cursor, no reset",
          c2 > c1 and not reset_notes(r2)
          and not any("generation changed" in n for n in r2["notes"]),
          json.dumps({"c1": c1, "c2": c2, "notes": r2["notes"]})[:220])
    check("A1-2 the pass consumes only previously-unread bytes "
          "(old prefix never re-read)",
          r2["new_tail_bytes"] == src.stat().st_size - c1
          and c2 == src.stat().st_size,
          json.dumps({"new_tail_bytes": r2["new_tail_bytes"],
                      "size": src.stat().st_size, "c1": c1})[:160])
    check("A1-3 progression names its integrity coverage explicitly",
          any("integrity: full" in n or "integrity: sampled" in n
              for n in r2["notes"]),
          json.dumps(r2["notes"])[:220])

    # detector state carries ACROSS the append boundary: command lines end
    # the old prefix, the error lands in the appended tail -> one loop
    w2 = fresh("a1state")
    st2 = w2 / "state"
    f2 = w2 / "loop.jsonl"
    f2.write_text("filler line\n$ retry-cmd\n$ retry-cmd\n")
    cp.tick({"board": [str(f2)]}, state_dir=st2, max_bytes_per_source=4096,
            max_pages=10)
    with f2.open("a") as f:
        f.write("NameError: name 'q' is not defined\n")
    r = cp.tick({"board": [str(f2)]}, state_dir=st2, max_bytes_per_source=4096,
                max_pages=10)
    loops = [o for o in cp.load_observations(st2)
             if o["detector_id"] == "cmd-failure-loop"]
    check("A1-4 detector adjacency state survives the append boundary",
          len(loops) == 1,
          json.dumps([o["detector_id"] for o in cp.load_observations(st2)]))

    # finite prefix completes under a fair bounded pass budget
    passes = 0
    while passes < 100:
        rep = cp.tick(sources, state_dir=state, max_bytes_per_source=2048,
                      max_pages=2)
        passes += 1
        if rep["coverage_complete"] and rep["pages_read"] == 0 \
                and rep["new_tail_bytes"] == 0:
            break
    check("A1-5 finite enrolled prefix completes under bounded passes",
          rep["coverage_complete"] == 1
          and cursor_of(state, "board::" + str(src))["cursor"]
          == src.stat().st_size and passes < 100,
          json.dumps({"passes": passes, "coverage": rep["coverage_complete"],
                      "cursor": cursor_of(state, "board::" + str(src))["cursor"],
                      "size": src.stat().st_size})[:200])
    check("A1-6 completed source reports closed coverage, zero backlog",
          rep["backlog_remaining"] == 0 and rep["coverage_complete"] == 1,
          json.dumps({k: rep[k] for k in ("backlog_remaining",
                                          "coverage_complete")}))
    shutil.rmtree(w)
    shutil.rmtree(w2)


def t_a1_progress_vs_backlog_distinction():
    w = fresh("a1mix")
    state = w / "state"
    big = w / "big.jsonl"
    small = w / "small.jsonl"
    big.write_text(FAULT * 400)      # larger than one pass budget
    small.write_text("plain line\n" + FAULT)
    # small source first: it closes within the pass budget while the big
    # one lands in backlog — both states visible in one report
    sources = {"board": [str(small), str(big)]}
    r1 = cp.tick(sources, state_dir=state, max_bytes_per_source=2048,
                 max_pages=3)
    check("A1-7 pass separates closed snapshot coverage from new tail bytes",
          r1["coverage_complete"] == 1 and r1["new_tail_bytes"] > 0
          and r1["backlog_remaining"] == 1,
          json.dumps({k: r1[k] for k in ("coverage_complete",
                                         "new_tail_bytes",
                                         "backlog_remaining")}))
    shutil.rmtree(w)


# =====================================================================
# A2 — logical observation identity, migration, denominators
# =====================================================================

def t_a2_identity_and_migration():
    # same occurrence (same bytes, same span) re-derived after an explicit
    # reset must dedupe to ONE row across generations (F2 defect shape)
    w = fresh("a2replay")
    state = w / "state"
    src = w / "b.jsonl"
    src.write_text(FAULT * 20)
    sources = {"board": [str(src)]}
    cp.tick(sources, state_dir=state, max_bytes_per_source=8192, max_pages=10)
    n_before = len(cp.load_observations(state))
    # corrupt the cursor -> typed reset -> full re-read of identical bytes
    (state / "cursors.json").write_text("GARBAGE{")
    rep = cp.tick(sources, state_dir=state, max_bytes_per_source=8192,
                  max_pages=10)
    n_after = len(cp.load_observations(state))
    check("A2-1 identical replay after explicit reset adds ZERO rows "
          "(generation excluded from identity)",
          n_after == n_before and any("reset" in n for n in rep["notes"]),
          json.dumps({"before": n_before, "after": n_after})[:120])

    # appends of the SAME fault text: new offsets stay distinct evidence,
    # but the DENOMINATOR stays lineage-scoped (honest incidents)
    with src.open("a") as f:
        f.write(FAULT * 5)
    cp.tick(sources, state_dir=state, max_bytes_per_source=8192, max_pages=10)
    rows = cp.load_observations(state)
    pats = cp.group_patterns(rows, scanned_lineages=1)
    sig = next(p for p in pats if p["detector_id"] == "sig-ssh-argv-join")
    check("A2-2 distinct occurrence offsets remain distinct evidence; "
          "incidents stay lineage-scoped",
          len(rows) == n_before + 5 and sig["independent_incidents"] == 1,
          json.dumps({"rows": len(rows), "incidents":
                      sig["independent_incidents"]})[:160])

    # identity is v2 (generation-independent); provenance (generation)
    # stays on the row
    o1 = cp.make_observation("sig-ssh-argv-join", "medium", "board",
                             "genA", "bytes@0", "fp", "lin1")
    o2 = cp.make_observation("sig-ssh-argv-join", "medium", "board",
                             "genB", "bytes@0", "fp", "lin1")
    check("A2-3 same occurrence under different generations shares one "
          "v2 identity while keeping its generation provenance",
          o1["dedupe_key"] == o2["dedupe_key"]
          and o1["dedupe_key"].startswith("v2-")
          and o1["generation"] == "genA" and o2["generation"] == "genB",
          json.dumps([o1["dedupe_key"][:16], o1["generation"],
                      o2["generation"]]))

    # changed content at the same offset stays distinct evidence
    o3 = cp.make_observation("sig-ssh-argv-join", "medium", "board",
                             "genB", "bytes@0", "fp-other", "lin1")
    check("A2-4 changed content at the same offset is a distinct identity",
          o3["dedupe_key"] != o1["dedupe_key"], "")

    # ---- explicit v1 -> v2 migration ----
    w3 = fresh("a2mig")
    st3 = w3 / "state"
    st3.mkdir(parents=True)
    # a live-shaped v1 store: one logical fault, 3 generations, v1 keys
    v1_rows = []
    for i, gen in enumerate(("sz100:aa:bb", "sz200:cc:dd", "sz300:ee:ff")):
        v1_rows.append({"detector_id": "sig-ssh-argv-join",
                        "detector_version": "det-v3", "severity": "medium",
                        "status": "unverified-symptom", "source_kind": "board",
                        "generation": gen, "lineage_id": "lin1",
                        "span": "bytes@0", "fingerprint": "fp",
                        "at": f"2026-10-2{i}T00:00:00Z",
                        "dedupe_key": cp._sha(cp._canonical(
                            ["board", "lin1", gen, "bytes@0",
                             "sig-ssh-argv-join", "det-v3", "fp"]))})
    (st3 / "observations.jsonl").write_text(
        "".join(cp._canonical(r) + "\n" for r in v1_rows))
    rep = cp.tick({"board": [str(w3 / "any.jsonl")]}, state_dir=st3,
                  max_bytes_per_source=8192, max_pages=1)
    rows3 = cp.load_observations(st3)
    meta = json.loads((st3 / "observations.meta.json").read_text())
    check("A2-5 v1 store migrates EXPLICITLY: generation-era duplicates "
          "coalesce to the earliest row, counts reported",
          len(rows3) == 1 and rows3[0]["generation"] == "sz100:aa:bb"
          and meta["identity_version"] == 2
          and any("migrated to identity v2" in n and "coalesced 2" in n
                  for n in rep["notes"]),
          json.dumps([len(rows3),
                      [n for n in rep["notes"] if "v2" in n][:1]])[:260])
    rep2 = cp.tick({"board": [str(w3 / "any.jsonl")]}, state_dir=st3,
                   max_bytes_per_source=8192, max_pages=1)
    check("A2-6 migration is one-time (meta marker); no repeated notes",
          not any("migrated" in n for n in rep2["notes"]),
          json.dumps(rep2["notes"])[:160])

    # foreign detector-version rows are preserved verbatim, never faked
    w4 = fresh("a2for")
    st4 = w4 / "state"
    st4.mkdir(parents=True)
    foreign = {"detector_id": "sig-x", "detector_version": "det-v0",
               "severity": "low", "status": "unverified-symptom",
               "source_kind": "board", "generation": "g", "lineage_id": "l",
               "span": "bytes@0", "fingerprint": "f", "at": "t",
               "dedupe_key": "legacy"}
    (st4 / "observations.jsonl").write_text(cp._canonical(foreign) + "\n")
    cp.tick({"board": [str(w4 / "any.jsonl")]}, state_dir=st4,
            max_bytes_per_source=8192, max_pages=1)
    rows4 = cp.load_observations(st4)
    check("A2-7 foreign-version rows preserved verbatim through migration",
          len(rows4) == 1 and rows4[0]["dedupe_key"] == "legacy"
          and rows4[0]["detector_version"] == "det-v0",
          json.dumps(rows4)[:200])
    for d in (w, w3, w4):
        shutil.rmtree(d)


# =====================================================================
# A3 — integrity: truncation, lineage, hardlinks, interior rewrite,
#      typed bounded outcomes
# =====================================================================

def t_a3_integrity():
    w = fresh("a3")
    state = w / "state"
    src = w / "src.jsonl"
    src.write_text(FAULT * 300)
    sources = {"board": [str(src)]}
    cp.tick(sources, state_dir=state, max_bytes_per_source=8192,
            max_pages=10)

    # truncation: typed explicit reconcile, never a silent skip
    data = src.read_bytes()
    src.write_bytes(data[:len(data) // 2])
    rep = cp.tick(sources, state_dir=state, max_bytes_per_source=8192,
                  max_pages=10)
    check("A3-1 truncation detected and reconciled with a typed reset",
          any("truncated" in n for n in rep["notes"])
          and cursor_of(state, "board::" + str(src))["cursor"]
          == len(data) // 2,
          json.dumps(reset_notes(rep)[:1])[:160])

    # same-size interior rewrite (F6/CSR-10): the control now states its
    # geometry honestly — records are EQUAL WIDTH (60 bytes replaced by
    # 60 bytes, no size delta), the fixture fits ONE verification budget
    # (65,536 x 4 >= 240,000), and the assertions include the invariants
    # the rewrite depends on: equal byte length and an unchanged
    # generation token. This is the original short-prefix audit control.
    w2 = fresh("a3rewrite")
    st2 = w2 / "state"
    f2 = w2 / "mid.jsonl"
    clean = ("x" * 59 + "\n").encode()                 # 60-byte records
    f2.write_bytes(clean * 4000)                       # 240,000 B
    tok0 = cp.generation(f2)
    cp.tick({"board": [str(f2)]}, state_dir=st2, max_bytes_per_source=65536,
            max_pages=10)                              # fully consumed
    body = bytearray(f2.read_bytes())
    bad = ("NameError: name 'zz' is not defined".ljust(59) + "\n").encode()
    start, width = 100_000, 60 * 900
    check("A3-2p falsifier geometry is honest: equal-width records, "
          "rewrite inside one verification budget",
          len(bad) == 60 and len(body) == 240_000
          and start + width <= len(body) and width <= 65536 * 4,
          json.dumps({"record": len(bad), "size": len(body),
                      "span": [start, start + width]})[:160])
    body[start:start + width] = bad * 900              # SAME size, interior
    f2.write_bytes(bytes(body))
    check("A3-2q rewrite is size-preserving and the sampled generation "
          "token cannot see it (token-blind control)",
          f2.stat().st_size == 240_000 and cp.generation(f2) == tok0,
          json.dumps({"size": f2.stat().st_size,
                      "tok_blind": cp.generation(f2) == tok0})[:140])
    rep2 = cp.tick({"board": [str(f2)]}, state_dir=st2,
                   max_bytes_per_source=65536, max_pages=10)
    loops = [o for o in cp.load_observations(st2)
             if o["detector_id"] == "sig-nameerror-typo"]
    check("A3-2 same-size interior rewrite detected despite an unchanged "
          "generation token; injected faults observed",
          len(loops) == 900
          and any("consumed-prefix mutation" in n for n in rep2["notes"]),
          json.dumps({"faults": len(loops),
                      "notes": [n[:60] for n in reset_notes(rep2)[:1]]})[:200])

    # long-prefix rotation counterexample (CSR-01/CSR-10): the rewrite
    # begins BEYOND one verification budget, so only an ADVANCING
    # rotation — never a single budget-sized pass — can see it; the
    # position geometry is asserted exactly
    w2b = fresh("a3rotate")
    st2b = w2b / "state"
    f2b = w2b / "long.jsonl"
    long_clean = ("x" * 63 + "\n").encode()            # 64-byte records
    f2b.write_bytes(long_clean * 16384)                # 1,048,576 B
    tokL = cp.generation(f2b)
    cp.tick({"board": [str(f2b)]}, state_dir=st2b,
            max_bytes_per_source=65536, max_pages=32)
    bodyL = bytearray(f2b.read_bytes())
    badL = ("NameError: name 'zz' is not defined".ljust(63) + "\n").encode()
    offL = 700_032                                     # inside chunk 10
    bodyL[offL:offL + 64] = badL
    f2b.write_bytes(bytes(bodyL))
    check("A3-2r long-prefix rewrite is same-size, token-blind, and sits "
          "beyond four verification budgets",
          f2b.stat().st_size == 1_048_576 and cp.generation(f2b) == tokL
          and offL >= 65536 * 4 * 2,
          json.dumps({"size": f2b.stat().st_size,
                      "tok_blind": cp.generation(f2b) == tokL,
                      "off": offL})[:160])
    positionsL = []
    for _ in range(8):
        cp.tick({"board": [str(f2b)]}, state_dir=st2b,
                max_bytes_per_source=65536, max_pages=32)
        positionsL.append(
            anchors_of(st2b)["board::" + str(f2b)]["verify_pos"])
    loopsL = [o for o in cp.load_observations(st2b)
              if o["detector_id"] == "sig-nameerror-typo"]
    check("A3-2s rotation beyond one verification budget: position "
          "advances (4 chunks/pass, cyclic) and the fault is seen "
          "within one sweep",
          positionsL == [4, 8, 0, 4, 8, 12, 0, 4] and len(loopsL) == 1,
          json.dumps({"positions": positionsL,
                      "faults": len(loopsL)})[:220])
    for d in (w2b,):
        shutil.rmtree(d)

    # path resurrection: same path, NEW inode, byte-identical content
    w3 = fresh("a3inode")
    st3 = w3 / "state"
    f3 = w3 / "r.jsonl"
    f3.write_text("plain\n")
    cp.tick({"board": [str(f3)]}, state_dir=st3, max_bytes_per_source=8192,
            max_pages=10)
    tmp = w3 / "newbody"
    tmp.write_text("plain\n")
    os.replace(tmp, f3)
    rep3 = cp.tick({"board": [str(f3)]}, state_dir=st3,
                   max_bytes_per_source=8192, max_pages=10)
    check("A3-3 replaced inode at the same path fires the lineage-changed "
          "reset (typed)",
          any("lineage changed" in n for n in rep3["notes"]),
          json.dumps(reset_notes(rep3)[:1])[:140])

    # hard links collapse to ONE physical source (one lineage, one incident)
    h1, h2 = w3 / "h1.jsonl", w3 / "h2.jsonl"
    h1.write_text("$ c\n$ c\n" + FAULT)
    if h2.exists() or h2.is_symlink():
        h2.unlink()
    os.link(h1, h2)
    lin1, lin2 = cp.lineage_id(h1), cp.lineage_id(h2)
    tok1, size1, lin3 = cp.source_identity(h1)
    check("A3-4 hard links and one-fd identity collapse to one lineage",
          lin1 == lin2 == lin3 and size1 == h1.stat().st_size
          and tok1.startswith("sz"),
          json.dumps([lin1, lin2, lin3]))
    st4 = w3 / "st4"
    rep4 = cp.tick({"board": [str(h1), str(h2)]}, state_dir=st4,
                   max_bytes_per_source=8192, max_pages=10)
    rows4 = cp.load_observations(st4)
    pats4 = cp.group_patterns(rows4, scanned_lineages=2)
    sig4 = next((p for p in pats4 if p["detector_id"] == "sig-ssh-argv-join"),
                None)
    check("A3-5 hard-linked enrollments are one lineage: one incident, "
          "never a false second",
          sig4 is not None and sig4["independent_incidents"] == 1,
          json.dumps(sig4)[:160] if sig4 else "no pattern")

    # typed bounded outcomes: corrupt anchors file -> explicit note, no crash
    w5 = fresh("a3corrupt")
    st5 = w5 / "state"
    f5 = w5 / "c.jsonl"
    f5.write_text(FAULT * 5)
    cp.tick({"board": [str(f5)]}, state_dir=st5, max_bytes_per_source=8192,
            max_pages=10)
    (st5 / "anchors.json").write_text("NOT JSON{{{")
    rep5 = cp.tick({"board": [str(f5)]}, state_dir=st5,
                   max_bytes_per_source=8192, max_pages=10)
    check("A3-6 corrupt anchor store: explicit typed note, source re-scan, "
          "no crash, no silent skip",
          any("anchors.json unreadable" in n or "invalid" in n.lower()
              for n in rep5["notes"]) and rep5["pages_read"] >= 1,
          json.dumps(rep5["notes"])[:200])

    # invalid UTF-8: replacement decode with an explicit note; offsets raw
    w6 = fresh("a3utf")
    st6 = w6 / "state"
    f6 = w6 / "u.jsonl"
    f6.write_bytes(b"ok\xff\n" + FAULT.encode())
    rep6 = cp.tick({"board": [str(f6)]}, state_dir=st6,
                   max_bytes_per_source=8192, max_pages=10)
    check("A3-7 invalid UTF-8 decodes detection-only with an explicit note",
          any("utf8 replacement" in n for n in rep6["notes"])
          and len(cp.load_observations(st6)) == 1,
          json.dumps([n for n in rep6["notes"] if "utf8" in n])[:160])

    # oversized record: an UNTERMINATED record past the carry bound is
    # dropped alone with its in-content marker; the cursor passes it and
    # the following fault is still observed
    w7 = fresh("a3big")
    st7 = w7 / "state"
    f7 = w7 / "big.jsonl"
    f7.write_bytes(b"Z" * 70000 + b"rest" + b"\n" + FAULT.encode())
    rep7 = cp.tick({"board": [str(f7)]}, state_dir=st7,
                   max_bytes_per_source=65536, max_pages=10)
    rows7 = cp.load_observations(st7)
    cur7 = cursor_of(st7, "board::" + str(f7))["cursor"]
    check("A3-8 oversized record dropped alone with its in-content marker; "
          "the pass still observes the following fault",
          cur7 > 70000 and len(rows7) == 1
          and rep7["coverage_complete"] == 1,
          json.dumps({"cursor": cur7, "rows": len(rows7)})[:120])

    # partial trailing line: emitted at EOF, done only at cursor EOF
    w8 = fresh("a3tail")
    st8 = w8 / "state"
    f8 = w8 / "t.jsonl"
    f8.write_bytes(b"err one\nunterminated tail")
    cp.tick({"board": [str(f8)]}, state_dir=st8, max_bytes_per_source=8192,
            max_pages=10)
    check("A3-9 unterminated tail record observed; coverage closes at EOF",
          rep8_coverage(st8, f8),
          "coverage flag")
    for d in (w, w2, w3, w5, w6, w7, w8):
        shutil.rmtree(d)


def rep8_coverage(st8, f8):
    rep = cp.tick({"board": [str(f8)]}, state_dir=st8,
                  max_bytes_per_source=8192, max_pages=10)
    return rep["coverage_complete"] == 1 \
        and cursor_of(st8, "board::" + str(f8))["cursor"] == f8.stat().st_size


# =====================================================================
# A4 — atomic durable commits: crash between evidence and cursor,
#      concurrent ticks
# =====================================================================

def t_a4_crash_and_concurrency():
    # crash injection: the cursor write fails AFTER observations landed;
    # the next tick must re-read, dedupe, and converge with no lost or
    # duplicated evidence
    w = fresh("a4crash")
    state = w / "state"
    src = w / "f.jsonl"
    src.write_text(FAULT * 30 + FAULT2 * 3)
    sources = {"board": [str(src)]}
    real_write = cp._atomic_write_json

    def crash_on_cursors(path, obj):
        if Path(path).name == "cursors.json":
            raise OSError("simulated crash before cursor durable write")
        return real_write(path, obj)

    cp._atomic_write_json = crash_on_cursors
    crashed = False
    try:
        cp.tick(sources, state_dir=state, max_bytes_per_source=8192,
                max_pages=10)
    except OSError:
        crashed = True
    finally:
        cp._atomic_write_json = real_write
    n_after_crash = len(cp.load_observations(state))
    rep = cp.tick(sources, state_dir=state, max_bytes_per_source=8192,
                  max_pages=10)
    n_final = len(cp.load_observations(state))
    cur = cursor_of(state, "board::" + str(src))["cursor"]
    check("A4-1 crash between evidence and cursor leaves evidence intact; "
          "replay converges with no duplicated rows",
          crashed and n_after_crash > 0 and n_final == n_after_crash
          and cur == src.stat().st_size and rep["coverage_complete"] == 1,
          json.dumps({"crashed": crashed, "after": n_after_crash,
                      "final": n_final, "cursor": cur})[:180])

    # concurrent ticks: two independent processes serialize on the state
    # lock; the union of their commits stays consistent
    w2 = fresh("a4conc")
    state2 = w2 / "state"
    src2 = w2 / "g.jsonl"
    src2.write_text(FAULT * 200)
    child = (
        "import sys; sys.path.insert(0, %r); import composer as cp; "
        "cp.tick({'board': [%r]}, state_dir=%r, max_bytes_per_source=8192, "
        "max_pages=10)"
        % (str(HERE), str(src2), str(state2)))
    procs = [subprocess.Popen([sys.executable, "-c", child],
                              cwd=str(HERE),
                              stdout=subprocess.DEVNULL,
                              stderr=subprocess.PIPE) for _ in range(2)]
    errs = [p.communicate(timeout=120)[1] for p in procs]
    rcs = [p.returncode for p in procs]
    rows = cp.load_observations(state2)
    cur = cursor_of(state2, "board::" + str(src2))["cursor"]
    keys = [o["dedupe_key"] for o in rows]
    check("A4-2 concurrent ticks serialize; evidence and cursor consistent "
          "(no lost rows, no duplicate keys, cursor at EOF)",
          all(rc == 0 for rc in rcs) and len(keys) == len(set(keys))
          and len(rows) == 200 and cur == src2.stat().st_size,
          json.dumps({"rcs": rcs, "rows": len(rows), "cursor": cur,
                      "err": [e.decode()[-160:] for e in errs if e]})[:260])
    shutil.rmtree(w)
    shutil.rmtree(w2)


# =====================================================================
# A5 — static controls (the original audit probes, re-expressed against
#      the repaired kernel)
# =====================================================================

def t_a5_static_controls():
    # staticreplay control: identical replay is stable and reset-free
    w = fresh("a5static")
    state = w / "state"
    src = w / "s.jsonl"
    src.write_text(FAULT * 400)
    sources = {"board": [str(src)]}
    counts, resets = [], []
    for _ in range(2):
        rep = cp.tick(sources, state_dir=state, max_bytes_per_source=8192,
                      max_pages=10)
        counts.append(len(cp.load_observations(state)))
        resets.append(sum(1 for n in rep["notes"] if "generation changed" in n))
    check("A5-1 static source: replay dedupes to a stable row set with "
          "zero resets (control holds)",
          counts[0] == counts[1] and resets == [0, 0],
          json.dumps({"rows": counts, "resets": resets}))

    # inflation control: an appended source no longer re-mints the SAME
    # occurrence; growth equals only the genuinely new occurrences
    with src.open("a") as f:
        f.write(FAULT * 10)
    rep = cp.tick(sources, state_dir=state, max_bytes_per_source=8192,
                  max_pages=10)
    check("A5-2 append grows the store by exactly the NEW occurrences "
          "(no reset-driven re-mint)",
          len(cp.load_observations(state)) == counts[1] + 10
          and not reset_notes(rep),
          json.dumps({"rows": len(cp.load_observations(state)),
                      "expected": counts[1] + 10}))
    shutil.rmtree(w)


def t_csr02_fair_scheduling():
    """CSR-02: a hoarding first source — one that grows faster than the
    page budget drains it — cannot permanently starve a later enrolled
    prefix. Probe-mirror geometry: one-page total budget, cohort of two,
    the big source appends 2KiB every pass. The persisted round-robin
    scheduler must give the later prefix its page chance within one
    cohort round while the big source keeps progressing."""
    w = fresh("csr02")
    state = w / "state"
    big = w / "big.jsonl"
    later = w / "later.jsonl"
    clean = ("x" * 63 + "\n").encode()
    big.write_bytes(clean * 128)
    later.write_bytes(FAULT2.encode())                 # 37 B, one record
    sources = {"board": [str(big), str(later)]}
    cursors = []
    for _ in range(8):
        rep = cp.tick(sources, state_dir=state, max_bytes_per_source=1024,
                      max_pages=1, claim_cap=0)
        cur = json.loads((state / "cursors.json").read_text())
        cursors.append([cur.get("board::" + str(big), {}).get("cursor"),
                        cur.get("board::" + str(later), {}).get("cursor")])
        with big.open("ab") as f:
            f.write(clean * 32)                        # grows 2 KiB/pass
    rows = cp.load_observations(state)
    later_size = later.stat().st_size
    check("C2-1 the later enrolled prefix is served within one cohort "
          "round despite the hoarding source (cursor reaches its EOF)",
          cursors[1][1] == later_size,
          json.dumps({"cursors": cursors})[:260])
    check("C2-2 the later prefix's fault is OBSERVED, not just passed "
          "over",
          any(o["detector_id"] == "sig-nameerror-typo" for o in rows),
          json.dumps([o["detector_id"] for o in rows])[:160])
    check("C2-3 the hoarding source still progresses (fairness adds "
          "service, it does not stall the big source)",
          cursors[-1][0] > cursors[0][0],
          json.dumps({"first": cursors[0][0], "last": cursors[-1][0]})[:140])
    check("C2-4 the later prefix stays covered after being served "
          "(closed snapshot: cursor pinned at its EOF; backlog stays "
          "honest about the still-growing source)",
          all(c[1] == later_size for c in cursors[1:])
          and rep["backlog_remaining"] == 1,
          json.dumps({"later": [c[1] for c in cursors],
                      "backlog": rep["backlog_remaining"]})[:220])
    check("C2-5 the scheduler state is persisted and bounded",
          json.loads((state / "scheduler.json").read_text())
          .get("rr") is not None
          and 0 <= json.loads(
              (state / "scheduler.json").read_text()).get("rr", -1) < 2,
          json.dumps(json.loads(
              (state / "scheduler.json").read_text()))[:100])
    shutil.rmtree(w)


def t_csr03_crash_gap_reconciled():
    """CSR-03: the cursor commits per page, the anchor saves once per
    pass — a crash between the two leaves the cursor AHEAD of the
    anchored frontier. Recovery must detect the mismatch and re-read the
    gap (fresh detector state, dedupe keeping prior evidence); the
    previously-uncovered interval must become anchored and rotation-
    verified. Probe-mirror geometry: cursor 131072, anchor 65536."""
    w = fresh("csr03")
    state = w / "state"
    src = w / "crash.jsonl"
    clean = ("x" * 63 + "\n").encode()                 # 64-byte records
    src.write_bytes(clean * 8192)                      # 512 KiB
    cp.tick({"board": [str(src)]}, state_dir=state,
            max_bytes_per_source=65536, max_pages=1)
    with patch.object(cp, "_save_anchors",
                      side_effect=OSError("injected anchor write crash")):
        try:
            cp.tick({"board": [str(src)]}, state_dir=state,
                    max_bytes_per_source=65536, max_pages=1)
        except OSError:
            pass
    crashed_cur = cursor_of(state, "board::" + str(src))["cursor"]
    crashed_anchor = anchors_of(state)["board::" + str(src)]
    check("C3-0 the injected crash reproduces the cursor-ahead state: "
          "cursor 131072, anchored frontier 65536, chunk [65536,131072) "
          "absent",
          crashed_cur == 131_072 and crashed_anchor["size"] == 65_536
          and not any(ch["start"] == 65_536 for ch in crashed_anchor["chunks"]),
          json.dumps({"cursor": crashed_cur, "anchor": crashed_anchor["size"],
                      "chunks": [c["start"] for c in
                                 crashed_anchor["chunks"]]})[:200])
    rep = cp.tick({"board": [str(src)]}, state_dir=state,
                  max_bytes_per_source=65536, max_pages=16)
    an = anchors_of(state)["board::" + str(src)]
    covered = any(ch["start"] <= 96_000 < ch["start"] + ch["len"]
                  for ch in an["chunks"])
    check("C3-1 recovery NAMES the crash gap explicitly (never a silent "
          "skip)",
          any("crash gap" in n for n in rep["notes"]),
          json.dumps(rep["notes"])[:220])
    check("C3-2 the previously-uncovered interval is ANCHORED after "
          "recovery (contiguous chunk coverage from 0)",
          covered and an["chunks"] == sorted(an["chunks"], key=lambda c: c["start"])
          and an["chunks"][0]["start"] == 0,
          json.dumps([c["start"] for c in an["chunks"]])[:200])
    rows0 = cp.load_observations(state)
    check("C3-3 no evidence was manufactured or lost across the crash "
          "and its reconciliation (clean prefix mines zero rows)",
          len(rows0) == 0, json.dumps(len(rows0)))
    # fault INSIDE the recovered interval; the anchor rotation must see it
    body = bytearray(src.read_bytes())
    bad = ("NameError: name 'widget' is not defined".ljust(63)
           + "\n").encode()
    body[96_000:96_064] = bad
    src.write_bytes(bytes(body))
    found = 0
    gap_notes = []
    for _ in range(8):
        rep2 = cp.tick({"board": [str(src)]}, state_dir=state,
                       max_bytes_per_source=65536, max_pages=16)
        gap_notes += [n for n in rep2["notes"] if "crash gap" in n]
        found = len([o for o in cp.load_observations(state)
                     if o["detector_id"] == "sig-nameerror-typo"])
        if found:
            break
    check("C3-4 the injected fault inside the recovered interval is seen "
          "within the rotation bound",
          found == 1, json.dumps({"rows": found})[:100])
    # crash-replay idempotence: converged passes emit no new gap notes,
    # no new rows
    for _ in range(2):
        rep3 = cp.tick({"board": [str(src)]}, state_dir=state,
                       max_bytes_per_source=65536, max_pages=16)
        gap_notes += [n for n in rep3["notes"] if "crash gap" in n]
    rows3 = cp.load_observations(state)
    check("C3-5 recovery is crash-replay idempotent: the gap reconcile is "
          "one-time (no repeated notes) and evidence stays stable",
          not gap_notes and len(rows3) == 1
          and cursor_of(state, "board::" + str(src))["cursor"]
          == src.stat().st_size,
          json.dumps({"gap_notes": gap_notes, "rows": len(rows3)})[:200])

    # CSR-03 EDITOR-01 (round-1 editor finding): the ANCHORED PREFIX's
    # detector context must survive recovery. A count-1 command at 65408
    # retains an adjacent error offset (65472) inside the anchored prefix;
    # a crash leaves the cursor ahead; the recurring command in the gap
    # re-read must still close the loop at the retained offset —
    # identically to an uncrashed control. The frontier detector
    # checkpoint (saved inside the anchor's atomic write) is the
    # mechanism; fresh-state wiping was the round-1 defect.
    rec = ("x" * 63 + "\n").encode()                   # clean 64B record
    cmd = ("$ retry-cmd".ljust(63) + "\n").encode()
    err = ("NameError: name 'widget' is not defined".ljust(63)
           + "\n").encode()

    def boundary_fixture(path):
        recs = [rec] * (196608 // 64)
        recs[65408 // 64] = cmd                        # count-1 command
        recs[65472 // 64] = err                        # adjacent error
        recs[131072 // 64] = cmd                       # recurrence in gap
        path.write_bytes(b"".join(recs))

    w_ctrl, w_crash = fresh("csr03ctrl"), fresh("csr03crash")
    boundary_fixture(w_ctrl / "b.jsonl")
    boundary_fixture(w_crash / "b.jsonl")
    for _ in range(3):                                 # uncrashed control
        cp.tick({"board": [str(w_ctrl / "b.jsonl")]}, state_dir=w_ctrl / "st",
                max_bytes_per_source=65536, max_pages=1)
    ctrl = [o for o in cp.load_observations(w_ctrl / "st")
            if o["detector_id"] == "cmd-failure-loop"]
    cp.tick({"board": [str(w_crash / "b.jsonl")]}, state_dir=w_crash / "st",
            max_bytes_per_source=65536, max_pages=1)
    with patch.object(cp, "_save_anchors",
                      side_effect=OSError("injected anchor write crash")):
        try:
            cp.tick({"board": [str(w_crash / "b.jsonl")]},
                    state_dir=w_crash / "st",
                    max_bytes_per_source=65536, max_pages=1)
        except OSError:
            pass
    for _ in range(3):                                 # recovery passes
        repc = cp.tick({"board": [str(w_crash / "b.jsonl")]},
                       state_dir=w_crash / "st",
                       max_bytes_per_source=65536, max_pages=16)
    crash_rows = cp.load_observations(w_crash / "st")
    loops_crash = [o for o in crash_rows
                   if o["detector_id"] == "cmd-failure-loop"]
    check("C3-6 frontier checkpoint preserves anchored-prefix detector "
          "context across a crash-gap recovery: the loop observation is "
          "IDENTICAL to the uncrashed control (same retained span and "
          "command fingerprint; distinct fixture inodes give distinct "
          "lineages, so dedupe keys differ by construction)",
          len(ctrl) == 1 and len(loops_crash) == 1
          and ctrl[0]["span"] == loops_crash[0]["span"] == "bytes@65472"
          and ctrl[0]["fingerprint"] == loops_crash[0]["fingerprint"],
          json.dumps({"ctrl": [(o["span"], o["fingerprint"]) for o in ctrl],
                      "crash": [(o["span"], o["fingerprint"])
                                for o in loops_crash],
                      "notes": [n[:70] for n in repc["notes"][:2]]})[:300])
    check("C3-7 no duplicated evidence across the crash boundary "
          "(dedupe keeps the signature row once)",
          len([o for o in crash_rows
               if o["detector_id"] == "sig-nameerror-typo"]) == 1
          and len(crash_rows) == len(cp.load_observations(w_ctrl / "st")),
          json.dumps({"crash_rows": len(crash_rows)})[:120])
    for d in (w, w_ctrl, w_crash):
        shutil.rmtree(d)


def t_csr03_reset_invalidates_checkpoint():
    """CSR-03 EDITOR-02 (round-2 editor finding): a frontier checkpoint
    is valid only while its anchored prefix remains valid. Gap capture
    happens BEFORE rotation verification, so a prefix mutation detected
    after capture must invalidate the checkpoint — a stale checkpoint
    surviving the reset would manufacture recurrence observations from
    discarded-prefix context (the editor's falsifier geometry)."""
    w = fresh("csr03rst")
    rec = ("x" * 63 + "\n").encode()
    cmd = ("$ retry-cmd".ljust(63) + "\n").encode()
    err = ("NameError: name 'widget' is not defined".ljust(63)
           + "\n").encode()
    state = w / "st"
    src = w / "b.jsonl"

    def write(prefix_cmd=True):
        recs = [rec] * (196608 // 64)
        if prefix_cmd:
            recs[65408 // 64] = cmd
            recs[65472 // 64] = err
        recs[131072 // 64] = cmd                # recurrence in the gap
        src.write_bytes(b"".join(recs))

    write(prefix_cmd=True)
    cp.tick({"board": [str(src)]}, state_dir=state,
            max_bytes_per_source=65536, max_pages=1)   # anchor + checkpoint
    with patch.object(cp, "_save_anchors",
                      side_effect=OSError("injected anchor write crash")):
        try:
            cp.tick({"board": [str(src)]}, state_dir=state,
                    max_bytes_per_source=65536, max_pages=1)
        except OSError:
            pass
    # replace the anchored prefix (same size) BEFORE recovery: the
    # command/error pair is gone, only the gap command remains
    write(prefix_cmd=False)
    repc = cp.tick({"board": [str(src)]}, state_dir=state,
                   max_bytes_per_source=65536, max_pages=1)
    reset_notes_r = reset_notes(repc)
    for _ in range(10):
        cp.tick({"board": [str(src)]}, state_dir=state,
                max_bytes_per_source=65536, max_pages=1)
    rows = cp.load_observations(state)
    loops = [o for o in rows if o["detector_id"] == "cmd-failure-loop"]
    sigs = [o for o in rows if o["detector_id"] == "sig-nameerror-typo"]
    check("C3-8 a reset after gap capture invalidates the checkpoint: "
          "the replaced prefix's mutation is announced FRESH and no "
          "recurrence evidence from the discarded prefix survives "
          "(no fabricated loop; at most the ORIGINAL pre-crash "
          "signature row is retained as history — never re-derived, "
          "since the replacement bytes contain no error line)",
          any("mutated" in n.lower() or "re-scanning" in n
              for n in reset_notes_r)
          and len(loops) == 0
          and all(o["span"] == "bytes@65472" for o in sigs)
          and len(sigs) <= 1,
          json.dumps({"notes": [n[:80] for n in reset_notes_r],
                      "loops": len(loops),
                      "sig_spans": [o["span"] for o in sigs]})[:300])
    shutil.rmtree(w)


def t_csr10_control_strength():
    """CSR-10: the long-prefix same-size control must FAIL a wrong-
    rotation implementation. Proof by construction: under the historical
    non-advancing rotation (verify_pos never moves — the CSR-01 defect),
    the SAME scenario never sees the fault; restore the advancing
    rotation and the fault is seen within one sweep. The control is not
    weakened acceptance — it discriminates."""
    w = fresh("csr10")
    state = w / "state"
    src = w / "long.jsonl"
    clean = ("x" * 63 + "\n").encode()
    src.write_bytes(clean * 16384)                     # 1 MiB, 16 chunks
    cp.tick({"board": [str(src)]}, state_dir=state,
            max_bytes_per_source=65536, max_pages=32)
    body = bytearray(src.read_bytes())
    bad = ("NameError: name 'widget' is not defined".ljust(63)
           + "\n").encode()
    body[700_032:700_096] = bad                        # same-size, chunk 10
    src.write_bytes(bytes(body))
    tok = cp.generation(src)
    real_verify = cp._verify_prefix_chunks

    def stuck_rotation(path, anchor, budget_bytes):
        ok, mismatch, vbytes, _ = real_verify(
            path, {"chunks": anchor.get("chunks") or [],
                   "verify_pos": 0}, budget_bytes)
        return ok, mismatch, vbytes, 0                 # position never moves

    cp._verify_prefix_chunks = stuck_rotation
    try:
        for _ in range(8):
            cp.tick({"board": [str(src)]}, state_dir=state,
                    max_bytes_per_source=65536, max_pages=32)
    finally:
        cp._verify_prefix_chunks = real_verify
    stuck_rows = [o for o in cp.load_observations(state)
                  if o["detector_id"] == "sig-nameerror-typo"]
    check("C10-1 under the historical non-advancing rotation the fault "
          "stays UNSEEN for 8 passes (the control would fail it)",
          len(stuck_rows) == 0 and cp.generation(src) == tok,
          json.dumps({"rows": len(stuck_rows)})[:100])
    seen = 0
    for _ in range(8):
        cp.tick({"board": [str(src)]}, state_dir=state,
                max_bytes_per_source=65536, max_pages=32)
        seen = len([o for o in cp.load_observations(state)
                    if o["detector_id"] == "sig-nameerror-typo"])
        if seen:
            break
    check("C10-2 the SAME scenario under correct advancing rotation sees "
          "the fault within one sweep (green is earned, not assumed)",
          seen == 1, json.dumps({"rows": seen})[:100])
    shutil.rmtree(w)


def main():
    t_a1_progression_and_completion()
    t_a1_progress_vs_backlog_distinction()
    t_a2_identity_and_migration()
    t_a3_integrity()
    t_a4_crash_and_concurrency()
    t_a5_static_controls()
    t_csr02_fair_scheduling()
    t_csr03_crash_gap_reconciled()
    t_csr03_reset_invalidates_checkpoint()
    t_csr10_control_strength()
    print(f"\n{len(PASS)} passed, {len(FAIL)} failed")
    if FAIL:
        print("FAILED:", *FAIL, sep="\n  - ")
    return 1 if FAIL else 0


if __name__ == "__main__":
    sys.exit(main())
