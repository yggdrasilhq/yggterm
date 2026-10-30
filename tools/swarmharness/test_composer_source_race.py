#!/usr/bin/env python3
"""Composer source-identity race — focused hermetic conformance tests
(CSR-05: census-to-read inode replacement must never manufacture a
second incident).

The counterexample (parent probe `inode-race`): a byte-identical inode
swap between census and read_page passes the generation-token-only
validation, so tick N publishes the census lineage for bytes read off a
different inode; tick N+1 census rebinds to the new lineage through the
designed reset and re-mines the same bytes — two "independent" incident
lineages for one physical swap, eligible=true.

The repair: every page carries `.rid` — the (dev, ino, size) fstat'ed on
the ONE handle that read it — and the tick commits only when that read
lineage equals the census lineage; the post-read pathname check
(generation, size AND lineage off ONE handle) is retained for the
read→validation window. A flapping axis is a typed defer (page
discarded, cursor/anchor never advance), and the new lineage is rebound
only through the explicit lineage-change reset on a later pass. The
validation→commit sub-window stays unlocked (no pathname lock); the next
pass's checks catch it — stated, never claimed closed.

Tests the swap in all three windows — before the read's open (census→read),
inside the read with the census inode restored before validation
(read-restore ABA: byte-identical and different-bytes), and after the
read returns (read→validation) — plus controls: a stable append keeps the
current lineage with plain progression, and genuinely distinct lineages
still make a pattern eligible.

Hermetic: tempdirs only, no network, no model calls, no live state.
Run:  python3 tools/swarmharness/test_composer_source_race.py
"""
import json
import os
import shutil
import sys
import tempfile
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
    return Path(tempfile.mkdtemp(prefix=f"composer-source-race-{tag}-"))


CLEAN = "2026-10-30 board sync ok\n" * 8
FAULT = "bash: -c: line 1: syntax error near unexpected token `('\n"


def rows_for(state):
    return cp.load_observations(state)


def fault_lineages(state):
    return {o["lineage_id"] for o in rows_for(state)
            if o["detector_id"] == "cmd-failure-loop"
            or o["detector_id"].startswith("sig-")}


def _anchor_substance(anchor):
    """Anchor content minus volatile fields (bytes or a file path): the
    per-pass rewrite round-trips through _load_anchors, which rebuilds
    entries without the `at` timestamp — compare what the anchor CLAIMS,
    not when."""
    body = json.loads(
        Path(anchor).read_text() if isinstance(anchor, Path) else anchor)
    for entry in body.values():
        entry.pop("at", None)
    return body


def swap_patch(src, twin, when, parked=None, captured=None):
    """Patch read_page so the path is replaced by the byte-identical
    twin inode either BEFORE the original open (`when="before-open"`),
    AFTER the original read returns (`when="after-read"`), or ONLY
    INSIDE the read (`when="read-restore"` — the twin is placed for the
    open and the census inode is restored before read_page returns, so
    validation opens the census inode: the CSR-05 EDITOR-01 restore
    geometry). One replacement cycle only — the swap is a physical
    event, not a loop. `captured`, when a dict, receives the first real
    page (with its .rid read-handle identity)."""
    real = cp.read_page
    fired = {"v": False}
    parked = parked or src.parent / (src.name + ".parked")

    def swapping(path, cursor=0, max_bytes=262144):
        if not fired["v"]:
            fired["v"] = True
            if when == "before-open":
                os.replace(twin, src)
            elif when == "read-restore":
                os.replace(src, parked)
                os.replace(twin, src)
            page = real(path, cursor, max_bytes)
            if captured is not None:
                captured["page"] = page
            if when == "after-read":
                os.replace(twin, src)
            elif when == "read-restore":
                os.replace(src, twin)
                os.replace(parked, src)
            return page
        return real(path, cursor, max_bytes)

    return patch.object(cp, "read_page", swapping)


def eligible_fault_patterns(state):
    pats = cp.group_patterns(rows_for(state))
    return [p for p in pats if p["eligible"]]


# =====================================================================
# R1 — census→read byte-identical inode swap fabricates 2 lineages
# =====================================================================

def t_r1_swap_before_open():
    w = fresh("r1")
    state = w / "state"
    src = w / "board.jsonl"
    src.write_text(CLEAN)

    cp.tick({"board": [str(src)]}, state_dir=state)
    cur0 = json.loads((state / "cursors.json").read_text())[
        "board::" + str(src)]
    lineage_a = cur0["lineage_id"]
    check("R1-0 baseline tick commits the census identity durably",
          bool(lineage_a) and rows_for(state) == []
          and cp.generation(src) == cur0["generation"],
          json.dumps({"lineage": lineage_a, "rows": len(rows_for(state))}))

    # fleet appends the fault bytes, then the path is swapped to a
    # byte-identical twin inode INSIDE tick 1's first read_page call —
    # after census (still the old inode) and before the page's open
    appended = FAULT * 4
    with src.open("a") as f:
        f.write(appended)
    twin = w / "twin.jsonl"
    twin.write_bytes(src.read_bytes())

    with swap_patch(src, twin, "before-open"):
        r1 = cp.tick({"board": [str(src)]}, state_dir=state)

    flap = [n for n in r1["notes"] if "identity flapped mid-read" in n]
    check("R1-1 the swap is a typed identity defer, not a silent pass",
          bool(flap) and r1["gaps"] >= 1,
          json.dumps(r1["notes"])[:240])
    check("R1-2 the flapped page publishes NOTHING: no observations, "
          "no cursor advance, no anchor blessing",
          rows_for(state) == [],
          json.dumps([o["dedupe_key"][:12] for o in rows_for(state)]))

    # tick 2: census rebinds through the designed lineage-change reset —
    # one physical swap, ONE incident lineage
    r2 = cp.tick({"board": [str(src)]}, state_dir=state)
    lins = fault_lineages(state)
    elig = eligible_fault_patterns(state)
    check("R1-3 after the rebind the same fault is ONE lineage, not two",
          len(lins) == 1 and lineage_a not in lins,
          json.dumps({"lineages": sorted(lins)})[:140])
    check("R1-4 the swap never fabricates an eligible pattern",
          elig == [],
          json.dumps([p["pattern_id"][:8] for p in elig]))
    check("R1-5 the rebind went through the explicit lineage reset",
          any("lineage changed" in n for n in r2["notes"]),
          json.dumps(r2["notes"])[:200])
    shutil.rmtree(w)


# =====================================================================
# R2 — read→validation window: swap AFTER the page is read
# =====================================================================

def t_r2_swap_after_read():
    w = fresh("r2")
    state = w / "state"
    src = w / "board.jsonl"
    src.write_text(CLEAN)
    cp.tick({"board": [str(src)]}, state_dir=state)

    with src.open("a") as f:
        f.write(FAULT * 4)
    twin = w / "twin.jsonl"
    twin.write_bytes(src.read_bytes())

    with swap_patch(src, twin, "after-read"):
        r1 = cp.tick({"board": [str(src)]}, state_dir=state)

    flap = [n for n in r1["notes"] if "identity flapped mid-read" in n
            and "lineage" in n]
    check("R2-1 a swap landing after the read but before validation is "
          "still caught (lineage re-checked off one handle)",
          bool(flap) and rows_for(state) == [],
          json.dumps(r1["notes"])[:240])
    r2 = cp.tick({"board": [str(src)]}, state_dir=state)
    check("R2-2 next pass mines once under the rebound lineage",
          len(fault_lineages(state)) == 1
          and eligible_fault_patterns(state) == [],
          json.dumps(r2["notes"])[:200])
    shutil.rmtree(w)


# =====================================================================
# R3 — mid-read truncation keeps the typed defer (no fabricated page)
# =====================================================================

def t_r3_truncate_mid_read():
    w = fresh("r3")
    state = w / "state"
    src = w / "board.jsonl"
    src.write_text(CLEAN + FAULT * 40)  # multi-page body
    real = cp.read_page
    fired = {"v": False}

    def truncating(path, cursor=0, max_bytes=262144):
        if not fired["v"]:
            fired["v"] = True
            os.truncate(str(src), 64)  # shrink UNDER the reading tick
        return real(path, cursor, max_bytes)

    with patch.object(cp, "read_page", truncating):
        r1 = cp.tick({"board": [str(src)]}, state_dir=state)
    flap = [n for n in r1["notes"] if "identity flapped mid-read" in n]
    check("R3-1 truncation under the read defers with the typed identity "
          "note (generation+size axes), page discarded",
          bool(flap) and any("generation" in n for n in flap),
          json.dumps(r1["notes"])[:240])
    shutil.rmtree(w)


# =====================================================================
# C1 — control: stable append keeps the CURRENT lineage, no flap
# =====================================================================

def t_c1_stable_append_control():
    w = fresh("c1")
    state = w / "state"
    src = w / "board.jsonl"
    src.write_text(CLEAN)
    cp.tick({"board": [str(src)]}, state_dir=state)
    cur0 = json.loads((state / "cursors.json").read_text())[
        "board::" + str(src)]
    lineage_a = cur0["lineage_id"]

    with src.open("a") as f:  # the same inode grows — no swap
        f.write(FAULT * 4)
    r1 = cp.tick({"board": [str(src)]}, state_dir=state)

    check("C1-1 a stable append progresses with NO identity-flap note",
          not any("identity flapped" in n for n in r1["notes"])
          and any("tail bytes to read" in n for n in r1["notes"]),
          json.dumps(r1["notes"])[:240])
    check("C1-2 appended evidence carries the CURRENT census lineage "
          "and the cursor is complete",
          fault_lineages(state) == {lineage_a},
          json.dumps({"lineages": sorted(fault_lineages(state))})[:140])
    cur1 = json.loads((state / "cursors.json").read_text())[
        "board::" + str(src)]
    check("C1-3 control cursor consumed the appended tail",
          cur1["cursor"] == src.stat().st_size,
          json.dumps({"cursor": cur1["cursor"],
                      "size": src.stat().st_size}))
    shutil.rmtree(w)


# =====================================================================
# C2 — control: genuinely distinct lineages still make a pattern eligible
# =====================================================================

def t_c2_two_lineage_eligibility_control():
    w = fresh("c2")
    state = w / "state"
    s1 = w / "alpha.jsonl"
    s2 = w / "beta.jsonl"
    s1.write_text(FAULT * 2)
    s2.write_text(FAULT * 2)
    cp.tick({"board": [str(s1), str(s2)]}, state_dir=state)
    lins = fault_lineages(state)
    check("C2-1 two distinct stable sources yield two incident lineages",
          len(lins) == 2 and len({s1, s2}) == 2,
          json.dumps({"lineages": sorted(lins)})[:140])
    elig = eligible_fault_patterns(state)
    check("C2-2 real recurrence across two lineages stays eligible "
          "(the repair blocks fabrication, not detection)",
          len(elig) == 1 and elig[0]["independent_incidents"] == 2,
          json.dumps([p["independent_incidents"] for p in elig]))
    shutil.rmtree(w)


# =====================================================================
# R4 — read-restore ABA, byte-identical: the page must answer for its
# OWN read handle, not for what the path holds at validation time
# =====================================================================

def t_r4_read_restore_byte_identical_aba():
    """Editor geometry (CSR05-EDITOR-01): rename the census inode aside,
    place a byte-identical twin ONLY while read_page's own handle reads
    it, restore the census inode before validation. The path-only check
    then matches the census — the old code committed the twin's bytes
    under the census lineage. The read identity must reject it."""
    w = fresh("r4")
    state = w / "state"
    src = w / "board.jsonl"
    parked = w / "parked.jsonl"
    src.write_text(CLEAN)
    cp.tick({"board": [str(src)]}, state_dir=state)
    lineage_a = json.loads((state / "cursors.json").read_text())[
        "board::" + str(src)]["lineage_id"]
    cursors_before = (state / "cursors.json").read_bytes()
    anchors_before = (state / "anchors.json").read_bytes()

    with src.open("a") as f:
        f.write(FAULT * 4)
    twin = w / "twin.jsonl"
    twin.write_bytes(src.read_bytes())  # byte-identical, OWN inode
    lineage_b = cp.source_identity(twin)[2]
    check("R4-0 fixture: byte-identical twin, distinct physical lineage",
          lineage_a != lineage_b and twin.read_bytes() == src.read_bytes(),
          json.dumps({"a": lineage_a, "b": lineage_b}))

    seen = {}
    with swap_patch(src, twin, "read-restore", parked=parked,
                    captured=seen):
        r1 = cp.tick({"board": [str(src)]}, state_dir=state)

    rid_lineage = cp._sha(f"{seen['page'].rid[0]}:{seen['page'].rid[1]}")[:16]
    flap = [n for n in r1["notes"] if "read-lineage" in n]
    check("R4-1 the page was really read from the twin's handle while the "
          "path held the census inode (old check was blind here)",
          rid_lineage == lineage_b
          and cp.source_identity(src)[2] == lineage_a,
          json.dumps({"rid": rid_lineage, "path": lineage_a}))
    check("R4-2 the read-identity mismatch is a typed defer",
          bool(flap) and r1["gaps"] >= 1,
          json.dumps(r1["notes"])[:240])
    check("R4-3 nothing from the foreign read is published: zero "
          "observations, cursor never advances, anchor never blesses the "
          "twin bytes (substantive anchor fields equal baseline)",
          rows_for(state) == []
          and (state / "cursors.json").read_bytes() == cursors_before
          and _anchor_substance(state / "anchors.json")
          == _anchor_substance(anchors_before),
          json.dumps([o["dedupe_key"][:12] for o in rows_for(state)]))

    # expose the twin for the next pass: the designed lineage rebind
    # mines B once, under B — never a second incident attributed to A
    os.replace(src, parked)
    os.replace(twin, src)
    r2 = cp.tick({"board": [str(src)]}, state_dir=state)
    lins = fault_lineages(state)
    check("R4-4 the later rebind mines the twin ONCE under ITS lineage",
          len(lins) == 1 and lineage_b in lins and lineage_a not in lins,
          json.dumps({"lineages": sorted(lins)})[:140])
    check("R4-5 one physical swap still never fabricates an eligible "
          "pattern", eligible_fault_patterns(state) == [],
          json.dumps([p["independent_incidents"]
                      for p in eligible_fault_patterns(state)]))
    check("R4-6 the rebind went through the explicit lineage reset",
          any("lineage changed" in n for n in r2["notes"]),
          json.dumps(r2["notes"])[:200])
    shutil.rmtree(w)


# =====================================================================
# R5 — read-restore ABA, different bytes: a fault-free census inode is
# never slandered with another inode's faults
# =====================================================================

def t_r5_restore_different_bytes_aba():
    """Same-size different-bytes control: fault-free A is censused, a
    faulty twin B is read, A restored before validation. The faults
    belong to B and must not be committed under A's lineage."""
    w = fresh("r5")
    state = w / "state"
    src = w / "board.jsonl"
    parked = w / "parked.jsonl"
    src.write_text("x" * (len(FAULT) - 1) + "\n")  # A: fault-free, |A|=|FAULT|
    lineage_a = cp.source_identity(src)[2]
    twin = w / "twin.jsonl"
    twin.write_text(FAULT)                          # B: faulty, same size
    lineage_b = cp.source_identity(twin)[2]
    assert lineage_a != lineage_b
    assert cp.source_identity(src)[1] == cp.source_identity(twin)[1]

    seen = {}
    with swap_patch(src, twin, "read-restore", parked=parked,
                    captured=seen):
        r1 = cp.tick({"board": [str(src)]}, state_dir=state)

    rid_lineage = cp._sha(f"{seen['page'].rid[0]}:{seen['page'].rid[1]}")[:16]
    check("R5-1 the faults were read off B's handle; validation-time path "
          "was the census inode (path-only check saw nothing)",
          rid_lineage == lineage_b
          and cp.source_identity(src)[2] == lineage_a
          and rows_for(state) == [],
          json.dumps({"rid": rid_lineage, "path": lineage_a,
                      "rows": len(rows_for(state))}))
    check("R5-2 the defer is typed on the read identity and the fault-free "
          "census lineage never appears in evidence",
          any("read-lineage" in n for n in r1["notes"])
          and fault_lineages(state) == set(),
          json.dumps(r1["notes"])[:240])

    # exposing B mines its own faults under ITS lineage — genuine
    # detection, correct provenance, still not eligible alone
    os.replace(src, parked)
    os.replace(twin, src)
    cp.tick({"board": [str(src)]}, state_dir=state)
    lins = fault_lineages(state)
    check("R5-3 the twin's own faults later land under the twin's lineage "
          "only", len(lins) == 1 and lineage_b in lins,
          json.dumps({"lineages": sorted(lins)})[:140])
    check("R5-4 and the repair blocks detection of nothing: the twin IS "
          "mined once exposed", len(rows_for(state)) >= 1
          and eligible_fault_patterns(state) == [],
          json.dumps({"rows": len(rows_for(state))}))
    shutil.rmtree(w)


def main():
    t_r1_swap_before_open()
    t_r2_swap_after_read()
    t_r3_truncate_mid_read()
    t_r4_read_restore_byte_identical_aba()
    t_r5_restore_different_bytes_aba()
    t_c1_stable_append_control()
    t_c2_two_lineage_eligibility_control()
    print(f"\n{len(PASS)} passed, {len(FAIL)} failed")
    if FAIL:
        print("FAILED:", *FAIL, sep="\n  - ")
    return 1 if FAIL else 0


if __name__ == "__main__":
    sys.exit(main())
