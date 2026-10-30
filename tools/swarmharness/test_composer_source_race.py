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

The repair: the tick validates generation, size AND lineage off ONE
handle after every read and before every commit; a flapping axis is a
typed defer (page discarded, cursor/anchor never advance), and the new
lineage is rebound only through the explicit lineage-change reset on a
later pass.

Tests the swap in both windows — before the read's open (census→read)
and after the read returns (read→validation) — plus controls: a stable
append keeps the current lineage with plain progression, and genuinely
distinct lineages still make a pattern eligible.

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


def swap_patch(src, twin, when):
    """Patch read_page so the path is replaced by the byte-identical
    twin inode either BEFORE the original open (`when="before-open"`)
    or AFTER the original read returns (`when="after-read"`). One
    replacement only — the swap is a physical event, not a loop."""
    real = cp.read_page
    fired = {"v": False}

    def swapping(path, cursor=0, max_bytes=262144):
        if not fired["v"]:
            fired["v"] = True
            if when == "before-open":
                os.replace(twin, src)
            page = real(path, cursor, max_bytes)
            if when == "after-read":
                os.replace(twin, src)
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


def main():
    t_r1_swap_before_open()
    t_r2_swap_after_read()
    t_r3_truncate_mid_read()
    t_c1_stable_append_control()
    t_c2_two_lineage_eligibility_control()
    print(f"\n{len(PASS)} passed, {len(FAIL)} failed")
    if FAIL:
        print("FAILED:", *FAIL, sep="\n  - ")
    return 1 if FAIL else 0


if __name__ == "__main__":
    sys.exit(main())
