#!/usr/bin/env python3
"""Composer observation-store repair — focused hermetic conformance tests.

Owner-authorized bounded lane (composer-store-repair, base 1f7103e7).
Covers exactly the two repair findings this unit owns:

  CSR-04 — before a migration replaces the live observation index, the
  EXACT original store bytes (every generation row, every old key, every
  corrupt line) survive in a NON-OVERWRITING content-addressed archive
  owned by the composer, fsynced ahead of the switch, with the
  migration's logical counts in a typed manifest that never rewrites
  provenance.

  CSR-07 — a store whose identity metadata names a NEWER composer
  version, an unrecognized version, or unreadable/malformed meta is
  HELD typed and non-destructively (never rewritten, never downgraded,
  sources deferred unread); a malformed CURRENT row is typed observable
  debt held verbatim in the store — classified BEFORE grouping, never a
  KeyError out of the pattern grouper, never a fabricated identity.

Kernel-only: no network, no model calls, no private session bodies, no
live-state writes. Scratch lives under ~/.yggterm/scratchpad (repo law:
/tmp is tmpfs). Run:  python3 tools/swarmharness/test_composer_store_repair.py
"""
import json
import shutil
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
    """Disk-backed scratch root per test (no /tmp — repo scratch law)."""
    root = Path.home() / ".yggterm" / "scratchpad" / "composer-store-repair"
    root.mkdir(parents=True, exist_ok=True)
    return Path(tempfile.mkdtemp(prefix=f"{tag}-{uuid.uuid4().hex[:8]}-",
                                 dir=root))


def store_bytes(state):
    return (state / "observations.jsonl").read_bytes()


def write_store(state, lines, meta=None):
    state.mkdir(parents=True, exist_ok=True)
    (state / "observations.jsonl").write_text(
        "".join(json.dumps(ln, sort_keys=True) + "\n" for ln in lines))
    if meta is not None:
        (state / "observations.meta.json").write_text(meta)


def v1_row(lineage, gen, fp="fp", detector="sig-nameerror-typo"):
    """A shape-valid row carrying a v1 (generation-embedded) key."""
    o = cp.make_observation(detector, "medium", "board", gen, "bytes@0",
                            fp, lineage)
    o["dedupe_key"] = "v1-" + cp._sha(
        cp._canonical(["board", lineage, gen, "bytes@0", fp]))
    return o


def malformed_row(extra=None):
    """JSON-valid CURRENT-version row missing detector_id (the exact
    CSR-07 counterexample: pre-repair it raised KeyError out of
    group_patterns)."""
    body = {"detector_version": cp.DETECTOR_VERSION, "dedupe_key": "broken"}
    if extra:
        body.update(extra)
    return body


# =====================================================================
# CSR-04 — exact original bytes archived BEFORE the index switch
# =====================================================================

def t_csr04_archive_byte_identical():
    w = fresh("csr04-archive")
    state = w / "state"
    rows = [v1_row("lineage-a", "g-old-1"), v1_row("lineage-a", "g-old-2"),
            {"detector_version": "det-v2", "note": "foreign row verbatim"}]
    write_store(state, rows + ["CORRUPT-LINE-NEVER-JSON"])
    original = store_bytes(state)

    notes = cp.migrate_observations(state)

    archive = (state / cp._ARCHIVE_DIR /
               f"{cp._sha(original)}.jsonl")
    check("CSR-04 archive exists at the content-addressed path",
          archive.exists(),
          f"missing {archive.name if archive.parent.exists() else 'dir'}")
    check("CSR-04 archived bytes are the EXACT original store bytes",
          archive.exists() and archive.read_bytes() == original)
    check("CSR-04 archive note names the digest and the archive dir",
          any("archived" in n and "sha256" in n for n in notes),
          json.dumps(notes)[:120])

    meta = json.loads((state / "observations.meta.json").read_text())
    check("CSR-04 meta switched to identity v2 after archive",
          meta.get("identity_version") == cp._OBS_IDENTITY_VERSION)

    live = cp.load_observations(state)
    check("CSR-04 migrated live store carries only v2-keyed current rows "
          "(generation duplicates coalesced, foreign row verbatim)",
          len(live) == 2
          and all(o["dedupe_key"].startswith("v2-")
                  for o in live if o["detector_version"] == "det-v3")
          and any(o["detector_version"] == "det-v2" for o in live),
          json.dumps([o.get("dedupe_key", o.get("detector_version"))
                      for o in live])[:120])

    manifest_path = (state / cp._ARCHIVE_DIR /
                     f"{cp._sha(original)}.manifest.json")
    m = json.loads(manifest_path.read_text()) if manifest_path.exists() \
        else {}
    check("CSR-04 manifest records the logical counts separately from "
          "provenance (rows/coalesce/corrupt/foreign/prior_meta)",
          m.get("rows_parsed") == 3 and
          m.get("coalesced_generation_duplicates") == 1 and
          m.get("rekeyed") == 1 and m.get("corrupt_lines") == 1 and
          m.get("foreign_version_rows") == 1 and
          m.get("quarantined_malformed") == 0 and
          m.get("prior_meta_raw") is None and
          m.get("sha256") == cp._sha(original),
          json.dumps(m)[:200])
    shutil.rmtree(w)


def t_csr04_quarantine_malformed_on_migration():
    w = fresh("csr04-quarantine")
    state = w / "state"
    rows = [v1_row("lineage-q", "g-1"), malformed_row()]
    write_store(state, rows)
    original = store_bytes(state)

    notes = cp.migrate_observations(state)

    check("CSR-04 migration quarantines the malformed row as typed debt "
          "(never rekeyed under a fabricated identity)",
          any("quarantined" in n for n in notes), json.dumps(notes)[:120])

    archive = (state / cp._ARCHIVE_DIR / f"{cp._sha(original)}.jsonl")
    check("CSR-04 quarantined row survives verbatim in the archive",
          archive.exists() and
          json.dumps(malformed_row(), sort_keys=True).encode()
          in archive.read_bytes())

    live = cp.load_observations(state)
    check("CSR-04 live store after migration has no identity-less row",
          len(live) == 1 and live[0]["dedupe_key"].startswith("v2-"))

    mpath = (state / cp._ARCHIVE_DIR / f"{cp._sha(original)}.manifest.json")
    m = json.loads(mpath.read_text()) if mpath.exists() else {}
    check("CSR-04 manifest carries the quarantine count and fault detail",
          m.get("quarantined_malformed") == 1 and
          any("detector_id" in f for f in m.get("quarantine_faults", [])),
          json.dumps(m)[:200])
    shutil.rmtree(w)


def t_csr04_replay_never_rewrites_provenance():
    w = fresh("csr04-replay")
    state = w / "state"
    write_store(state, [v1_row("lineage-r", "g-1")])
    original = store_bytes(state)
    cp.migrate_observations(state)

    archive = (state / cp._ARCHIVE_DIR / f"{cp._sha(original)}.jsonl")
    manifest = (state / cp._ARCHIVE_DIR /
                f"{cp._sha(original)}.manifest.json")
    archived_then = archive.read_bytes()
    manifest_then = manifest.read_bytes()

    # force a re-migration: the same digest path must be verified, and
    # neither the archive nor its manifest may be rewritten
    (state / "observations.meta.json").unlink()
    cp.migrate_observations(state)

    check("CSR-04 replay verifies the existing archive entry and never "
          "overwrites it", archive.read_bytes() == archived_then)
    check("CSR-04 replay never rewrites an existing manifest (append-only "
          "provenance, not mutable state)",
          manifest.read_bytes() == manifest_then)
    shutil.rmtree(w)


def t_csr04_compromised_archive_entry_holds_store():
    w = fresh("csr04-compromised")
    state = w / "state"
    rows = [v1_row("lineage-c", "g-1")]
    write_store(state, rows)
    original = store_bytes(state)
    digest = cp._sha(original)
    adir = state / cp._ARCHIVE_DIR
    adir.mkdir()
    (adir / f"{digest}.jsonl").write_bytes(b"tampered-provenance")

    notes = cp.migrate_observations(state)

    check("CSR-04 a compromised archive entry HOLDS the store (typed "
          "hold), never silently overwrites owned provenance",
          any("HELD" in n and "refusing to overwrite" in n for n in notes),
          json.dumps(notes)[:160])
    check("CSR-04 held migration leaves the live store byte-identical",
          store_bytes(state) == original)
    check("CSR-04 held migration writes no identity meta",
          not (state / "observations.meta.json").exists())
    shutil.rmtree(w)


# =====================================================================
# CSR-07 — typed non-destructive HOLD for unknown/malformed identity
# =====================================================================

def t_csr07_future_version_holds_and_defers_sources():
    w = fresh("csr07-future")
    state = w / "state"
    good = cp.make_observation("sig-nameerror-typo", "medium", "board",
                               "g-1", "bytes@0", "fp", "lineage-f")
    write_store(state, [good], meta='{"identity_version": 99}')
    before = store_bytes(state)
    meta_before = (state / "observations.meta.json").read_bytes()
    src = w / "board.jsonl"
    src.write_text("NameError: name 'zz' is not defined\n" * 5)

    notes = cp.migrate_observations(state)
    check("CSR-07 identity_version 99 (NEWER composer) is HELD by "
          "migration, never downgraded",
          any("HELD" in n and "NEWER" in n for n in notes),
          json.dumps(notes)[:160])
    check("CSR-07 held migration preserves store bytes exactly",
          store_bytes(state) == before)
    check("CSR-07 held migration preserves meta bytes exactly",
          (state / "observations.meta.json").read_bytes() == meta_before)
    check("CSR-07 held migration creates no archive (nothing replaced)",
          not (state / cp._ARCHIVE_DIR).exists())

    rep = cp.tick({"board": [str(src)]}, state_dir=state, claim_cap=0)
    held_note = [n for n in rep["notes"] if "HELD" in n]
    defer_note = [n for n in rep["notes"] if "deferred unread" in n]
    check("CSR-07 tick on a held store reports the typed hold",
          len(held_note) >= 1)
    check("CSR-07 held store defers its source UNREAD (no reads, no "
          "commits, bytes preserved)",
          len(defer_note) == 1 and rep["pages_read"] == 0
          and rep["observations"] == 0 and rep["backlog_remaining"] == 1,
          json.dumps({"pages": rep["pages_read"],
                      "obs": rep["observations"],
                      "backlog": rep["backlog_remaining"]})[:120])
    check("CSR-07 tick on a held store leaves observation bytes exactly "
          "as they were", store_bytes(state) == before)
    shutil.rmtree(w)


def t_csr07_unrecognized_and_malformed_meta_hold():
    held_metas = [
        ('{"identity_version": "2"}', "non-int version"),
        ('{"identity_version": 0}', "unrecognized version 0"),
        ('{"identity_version": -1}', "unrecognized negative version"),
        ('{"dedupe_identity": "v2"}', "meta without version field"),
        ('this is not json at all {', "unreadable meta"),
    ]
    for meta_text, label in held_metas:
        w = fresh(f"csr07-meta")
        state = w / "state"
        rows = [cp.make_observation("sig-nameerror-typo", "low", "board",
                                    "g-1", "bytes@0", "fp", "lineage-m")]
        write_store(state, rows, meta=meta_text)
        before = store_bytes(state)
        notes = cp.migrate_observations(state)
        check(f"CSR-07 {label} HOLDS the store typed (never reinterpreted)",
              len(notes) == 1 and "HELD" in notes[0]
              and store_bytes(state) == before,
              json.dumps(notes)[:160])
        shutil.rmtree(w)

    # recognized versions are NOT held: v1 migrates explicitly, v2 and
    # absent-meta (legacy) are business as usual
    w = fresh("csr07-recognized")
    state = w / "state"
    write_store(state, [v1_row("lineage-ok", "g-1")],
                meta='{"identity_version": 1}')
    notes = cp.migrate_observations(state)
    check("CSR-07 identity_version 1 is a recognized predecessor — the "
          "explicit migration proceeds (not held)",
          notes and "HELD" not in notes[0]
          and "migrated" in notes[0], json.dumps(notes)[:160])
    check("CSR-07 v1 migration still archives the original bytes first",
          any("archived" in n for n in notes))
    shutil.rmtree(w)


def t_csr07_malformed_current_row_typed_debt():
    """The CSR-07 headline regression: pre-repair, a JSON-valid det-v3
    row missing detector_id crashed tick with KeyError out of
    group_patterns. Post-repair: typed debt, store untouched."""
    w = fresh("csr07-debt")
    state = w / "state"
    good = cp.make_observation("sig-nameerror-typo", "medium", "board",
                               "g-1", "bytes@0", "fp", "lineage-d")
    bad_line = json.dumps(malformed_row(), sort_keys=True)
    write_store(state, [good, malformed_row()],
                meta='{"identity_version": 2}')
    bad_before = (state / "observations.jsonl").read_text().splitlines()
    bad_line_stored = [ln for ln in bad_before if "broken" in ln][0]
    src = w / "board.jsonl"
    src.write_text("NameError: name 'qq' is not defined\n" * 3)

    rep = cp.tick({"board": [str(src)]}, state_dir=state, claim_cap=0)

    debt_notes = [n for n in rep["notes"]
                  if "observation debt: malformed current row held" in n]
    check("CSR-07 malformed current row produces a TYPED debt note (not "
          "an exception)", len(debt_notes) == 1
          and "schema violation" in debt_notes[0]
          and "'detector_id'" in debt_notes[0],
          json.dumps(debt_notes)[:200])
    check("CSR-07 the tick COMPLETES and groups valid rows (pre-repair "
          "this was a KeyError out of group_patterns)",
          rep["observations"] >= 1, json.dumps(rep)[:160])
    stored_after = (state / "observations.jsonl").read_text()
    check("CSR-07 malformed row is HELD verbatim in the store — never "
          "rewritten, never dropped", bad_line_stored in stored_after
          and bad_line in stored_after)
    shutil.rmtree(w)


def t_csr07_debt_note_cap():
    w = fresh("csr07-cap")
    state = w / "state"
    write_store(state, [malformed_row({"dedupe_key": f"broken-{i}"})
                        for i in range(10)],
                meta='{"identity_version": 2}')
    rep = cp.tick({"board": [str(w / "missing.jsonl")]}, state_dir=state,
                  claim_cap=0)
    shown = [n for n in rep["notes"]
             if n.startswith("observation debt: malformed")]
    cap_notes = [n for n in rep["notes"] if "note cap" in n]
    check("CSR-07 debt notes are capped at the finite bound "
          f"({cp._MAX_DEBT_NOTES} shown)",
          len(shown) == cp._MAX_DEBT_NOTES,
          json.dumps(len(shown)))
    check("CSR-07 the overflow is COUNTED in a cap note (never silent)",
          len(cap_notes) == 1 and "2 more malformed current rows held"
          in cap_notes[0], json.dumps(cap_notes)[:160])
    shutil.rmtree(w)


def t_csr07_commit_survives_malformed_stored_row():
    w = fresh("csr07-commit")
    state = w / "state"
    bad_line = json.dumps({"detector_version": cp.DETECTOR_VERSION},
                          sort_keys=True)
    state.mkdir(parents=True, exist_ok=True)
    (state / "observations.jsonl").write_text(bad_line + "\n")

    obs = cp.make_observation("sig-nameerror-typo", "medium", "board",
                              "g-1", "bytes@0", "fp", "lineage-k")
    entry = {"source_kind": "board", "path": "/invented/board.jsonl",
             "generation": "g-1", "lineage_id": "lineage-k"}
    out = cp.commit_batch(state, entry, [obs], 128, {})

    check("CSR-07 commit_batch keys the uniqueness map from usable keys "
          "only — a keyless stored row never breaks the commit",
          out["appended"] == 1)
    stored = (state / "observations.jsonl").read_text().splitlines()
    check("CSR-07 the keyless row stays held verbatim next to the new "
          "row (append preserves prior lines byte-for-byte)",
          bad_line in stored and len(stored) == 2)
    shutil.rmtree(w)


def t_csr07_load_checked_classification():
    w = fresh("csr07-load")
    state = w / "state"
    good = cp.make_observation("sig-nameerror-typo", "medium", "board",
                               "g-1", "bytes@0", "fp", "lineage-l")
    forged = cp.make_observation("sig-nameerror-typo", "medium", "board",
                                 "g-2", "bytes@8", "fp", "lineage-l")
    forged["dedupe_key"] = "v2-forged"  # stale/forged key: schema-valid
    write_store(state, [good, forged, malformed_row(),
                        {"detector_version": "det-v2", "x": 1},
                        "NOT-JSON"])
    groupable, debts, corrupt = cp.load_observations_checked(state)
    check("CSR-07 load_observations_checked admits only valid current "
          "rows to grouping", len(groupable) == 1
          and groupable[0]["dedupe_key"] == good["dedupe_key"])
    check("CSR-07 forged-key and schema-violating rows become typed "
          "debt (counted, never grouped)",
          len(debts) == 2
          and any("dedupe key mismatch" in d for d in debts)
          and any("schema violation" in d for d in debts),
          json.dumps(debts)[:200])
    check("CSR-07 corrupt lines keep their counted note",
          corrupt == 1)
    shutil.rmtree(w)


# =====================================================================
# CSRSTORE-01..04 — critic round 2 repair unit (bounded findings)
# =====================================================================

def _csr_old_store(state):
    """The exact original CSR-04 legacy geometry: two generation-era rows
    plus a genuinely broken JSON line, written with CRLF/blank byte
    noise so byte preservation is distinguishable from normalization."""
    rows = [v1_row("lineage-s1", "g-1"), v1_row("lineage-s1", "g-2")]
    body = ("".join(json.dumps(r, sort_keys=True) + "\n" for r in rows)
            + 'CORRUPT-LINE-NEVER-JSON\n \r\n')
    state.mkdir(parents=True, exist_ok=True)
    (state / "observations.jsonl").write_bytes(body.encode())
    return body.encode()


def t_csrstore01_archive_fault_holds_whole_tick():
    """CSRSTORE-01: an archive-fault hold must propagate to the TICK —
    no source reads, no commits, no meta — and the exact original bytes
    must be preserved durably without relying on the compromised
    archive (pre-repair the tick recomputed held state and proceeded)."""
    w = fresh("csrstore01-tick")
    state = w / "state"
    original = _csr_old_store(state)
    digest = cp._sha(original)
    adir = state / cp._ARCHIVE_DIR
    adir.mkdir()
    (adir / f"{digest}.jsonl").write_bytes(b"compromised archive")
    src = w / "board.jsonl"
    src.write_text("NameError: name 'invented' is not defined\n")

    rep = cp.tick({"board": [str(src)]}, state_dir=state, claim_cap=0)

    check("CSRSTORE-01 an archive-fault hold stops the tick before any "
          "source read (pages_read 0)",
          rep["pages_read"] == 0, json.dumps(rep)[:160])
    check("CSRSTORE-01 a held tick commits nothing",
          rep["observations"] == 0 and rep["backlog_remaining"] == 1,
          json.dumps(rep)[:160])
    check("CSRSTORE-01 a held tick leaves the live store byte-identical",
          store_bytes(state) == original)
    check("CSRSTORE-01 a held tick writes no identity meta",
          not (state / "observations.meta.json").exists())
    recovery = adir / f"{digest}.recovery.jsonl"
    check("CSRSTORE-01 the hold preserves the exact original bytes "
          "durably OUTSIDE the compromised archive",
          recovery.exists() and recovery.read_bytes() == original
          and any("preserved durably" in n for n in rep["notes"]),
          json.dumps(rep["notes"])[:200])
    shutil.rmtree(w)


def t_csrstore01_hold_preserves_crlf_blank_bytes_exactly():
    w = fresh("csrstore01-crlf")
    state = w / "state"
    original = _csr_old_store(state)
    digest = cp._sha(original)
    adir = state / cp._ARCHIVE_DIR
    adir.mkdir()
    (adir / f"{digest}.jsonl").write_bytes(b"compromised archive")

    notes = cp.migrate_observations(state)

    check("CSRSTORE-01 CRLF/blank byte noise is never normalized away on "
          "a hold — a state file retains the EXACT original bytes",
          (adir / f"{digest}.recovery.jsonl").read_bytes() == original,
          json.dumps(notes)[:160])
    check("CSRSTORE-01 the compromised archive entry is never touched",
          (adir / f"{digest}.jsonl").read_bytes() == b"compromised archive")
    check("CSRSTORE-01 the held migration notes the recovery location",
          any(f"{cp._ARCHIVE_DIR}/{digest}.recovery.jsonl" in n
              for n in notes), json.dumps(notes)[:200])
    shutil.rmtree(w)


def t_csrstore02_held_store_never_opens_enrolled_source():
    """CSRSTORE-02: a held-store tick must gate BEFORE the content-
    reading census — no enrolled source is opened at all (pre-repair
    census's source_identity opened the source despite pages_read=0)."""
    w = fresh("csrstore02-noopen")
    state = w / "state"
    state.mkdir(parents=True)
    (state / "observations.meta.json").write_text('{"identity_version": 99}')
    (state / "observations.jsonl").write_bytes(b"")
    src = w / "board.jsonl"
    src.write_bytes(b"NameError: name 'invented' is not defined\n")

    reads = []
    original_open = Path.open

    def tracked_open(p, *args, **kwargs):
        if p == src:
            reads.append(args[0] if args else kwargs.get("mode", "r"))
        return original_open(p, *args, **kwargs)

    with patch.object(Path, "open", tracked_open):
        rep = cp.tick({"board": [str(src)]}, state_dir=state, claim_cap=0)

    check("CSRSTORE-02 a held store reports without ANY enrolled-source "
          "open (gate ahead of the content-reading census)",
          reads == [], json.dumps(reads))
    check("CSRSTORE-02 a held tick reads zero pages and defers unread",
          rep["pages_read"] == 0 and rep["observations"] == 0
          and any("deferred unread" in n for n in rep["notes"]),
          json.dumps(rep)[:160])
    check("CSRSTORE-02 a held tick creates no per-source state",
          not (state / "cursors.json").exists()
          and not (state / "anchors.json").exists())
    shutil.rmtree(w)


def t_csrstore03_corrupt_bytes_typed_debt_never_crash():
    """CSRSTORE-03: non-UTF8 records are typed corrupt debt — held
    schemas stay opaque, current stores never crash, migration archives
    the exact bytes (pre-repair each case raised UnicodeDecodeError)."""
    w = fresh("csrstore03-future")
    state = w / "state"
    state.mkdir(parents=True)
    (state / "observations.meta.json").write_text('{"identity_version": 99}')
    raw = b"\xfffuture-encoding\n"
    (state / "observations.jsonl").write_bytes(raw)
    src = w / "board.jsonl"
    src.write_bytes(b"NameError: name 'invented' is not defined\n")
    rep = cp.tick({"board": [str(src)]}, state_dir=state, claim_cap=0)
    check("CSRSTORE-03 a HELD future schema stays opaque — its non-UTF8 "
          "bytes are never parsed and never crash the tick",
          rep["pages_read"] == 0 and store_bytes(state) == raw,
          json.dumps(rep)[:160])
    shutil.rmtree(w)

    w = fresh("csrstore03-current")
    state = w / "state"
    good = cp.make_observation("sig-nameerror-typo", "medium", "board",
                               "g-1", "bytes@0", "fp", "lineage-x")
    corrupt = b"\xffcurrent-corrupt\n"
    payload = (cp._canonical(good) + "\n").encode() + corrupt
    state.mkdir(parents=True)
    (state / "observations.meta.json").write_text('{"identity_version": 2}')
    (state / "observations.jsonl").write_bytes(payload)
    src = w / "board.jsonl"
    src.write_bytes(b"NameError: name 'invented' is not defined\n")
    rep = cp.tick({"board": [str(src)]}, state_dir=state, claim_cap=0)
    check("CSRSTORE-03 corrupt non-UTF8 records in a CURRENT store are "
          "typed corrupt debt — the tick completes, never raises",
          any("corrupt lines skipped" in n for n in rep["notes"])
          and rep["pages_read"] >= 1, json.dumps(rep["notes"])[:200])
    check("CSRSTORE-03 the corrupt bytes survive the tick's append "
          "verbatim (never a lossy re-encode)",
          store_bytes(state).startswith(payload)
          and b"\xffcurrent-corrupt" in store_bytes(state))
    shutil.rmtree(w)

    w = fresh("csrstore03-legacy")
    state = w / "state"
    original = _csr_old_store(state) + b"\xff\n"
    (state / "observations.jsonl").write_bytes(original)
    notes = cp.migrate_observations(state)
    archive = (state / cp._ARCHIVE_DIR / f"{cp._sha(original)}.jsonl")
    check("CSRSTORE-03 a legacy store with a non-UTF8 record migrates "
          "without a decoding crash and archives the EXACT bytes",
          notes and archive.exists()
          and archive.read_bytes() == original,
          json.dumps(notes)[:160])
    shutil.rmtree(w)


def t_csrstore04_manifest_staged_before_switch():
    """CSRSTORE-04: the ledger is staged durably BEFORE the projection/
    meta switch, so a simulated interruption after staging replays into
    an existing valid manifest (pre-repair the crash left no ledger)."""
    w = fresh("csrstore04-crash")
    state = w / "state"
    original = _csr_old_store(state)
    digest = cp._sha(original)

    class SimulatedCrash(BaseException):
        pass

    with patch.object(cp, "_migration_manifest",
                      side_effect=SimulatedCrash):
        raised = False
        try:
            cp.migrate_observations(state)
        except SimulatedCrash:
            raised = True
    check("CSRSTORE-04 the interruption propagates (no swallowed crash)",
          raised)
    check("CSRSTORE-04 a crash at the ledger boundary leaves the live "
          "store and meta UNCHANGED (ledger staged before the switch)",
          store_bytes(state) == original
          and not (state / "observations.meta.json").exists())
    check("CSRSTORE-04 the archived provenance already exists at the "
          "crash boundary",
          (state / cp._ARCHIVE_DIR / f"{digest}.jsonl").read_bytes()
          == original)

    notes = cp.migrate_observations(state)
    manifest = (state / cp._ARCHIVE_DIR / f"{digest}.manifest.json")
    m = json.loads(manifest.read_text()) if manifest.exists() else {}
    check("CSRSTORE-04 recovery replays into an existing valid ledger "
          "(counts durable across the interruption)",
          manifest.exists() and m.get("rows_parsed") == 2
          and m.get("corrupt_lines") == 2 and m.get("rekeyed") == 1
          and m.get("coalesced_generation_duplicates") == 1
          and json.loads((state / "observations.meta.json").read_text()
                         ).get("identity_version") == 2,
          json.dumps(m)[:200])
    check("CSRSTORE-04 the archived provenance is byte-identical after "
          "recovery",
          (state / cp._ARCHIVE_DIR / f"{digest}.jsonl").read_bytes()
          == original and any("migrated" in n for n in notes))
    shutil.rmtree(w)


def t_csrstore04_replay_validates_manifest_never_trusts():
    """CSRSTORE-04: an exact-digest replay VALIDATES the existing ledger;
    a damaged ledger's bytes are quarantined and a valid ledger rebuilt
    (pre-repair any existing manifest was accepted unread)."""
    w = fresh("csrstore04-corrupt")
    state = w / "state"
    original = _csr_old_store(state)
    digest = cp._sha(original)
    cp.migrate_observations(state)
    mpath = state / cp._ARCHIVE_DIR / f"{digest}.manifest.json"
    damaged = b"NOT JSON: damaged manifest"
    mpath.write_bytes(damaged)
    (state / "observations.jsonl").write_bytes(original)
    (state / "observations.meta.json").unlink()

    notes = cp.migrate_observations(state)

    m = json.loads(mpath.read_text())
    check("CSRSTORE-04 replay rebuilds a VALID ledger after detecting "
          "the damage (validated, not silently accepted)",
          m.get("sha256") == digest and m.get("rows_parsed") == 2
          and m.get("corrupt_lines") == 2
          and any("ledger" in n and "damaged" in n for n in notes),
          json.dumps(notes)[:200])
    sides = list((state / cp._ARCHIVE_DIR).glob(
        f"{digest}.manifest.damaged-*.json"))
    check("CSRSTORE-04 the damaged ledger's exact bytes are quarantined "
          "beside it (never silently discarded)",
          len(sides) == 1 and sides[0].read_bytes() == damaged,
          json.dumps([p.name for p in sides]))
    check("CSRSTORE-04 the immutable archived provenance is untouched by "
          "the ledger repair",
          (state / cp._ARCHIVE_DIR / f"{digest}.jsonl").read_bytes()
          == original
          and json.loads((state / "observations.meta.json").read_text()
                         ).get("identity_version") == 2)

    # a VALID ledger on replay is left byte-identical (validated, not
    # rewritten)
    manifest_then = mpath.read_bytes()
    (state / "observations.meta.json").unlink()
    cp.migrate_observations(state)
    check("CSRSTORE-04 a validated ledger is never rewritten on replay",
          mpath.read_bytes() == manifest_then)
    shutil.rmtree(w)


def t_csrstore04_counts_and_quarantine_collisions():
    w = fresh("counts-collisions")
    state = w / "state"
    rows = [v1_row("invented", "g1"), v1_row("invented", "g2")]
    write_store(state, rows)
    original = store_bytes(state)
    cp.migrate_observations(state)
    digest = cp._sha(original)
    adir = state / cp._ARCHIVE_DIR
    mp = adir / (digest + ".manifest.json")
    valid = json.loads(mp.read_text())
    invalid = dict(valid, rows_parsed=999, rekeyed=666,
                   coalesced_generation_duplicates=888,
                   quarantine_faults=["invented incompatible fact"])
    damaged = json.dumps(invalid).encode()
    mp.write_bytes(damaged)
    # Both the old truncated name and new full-content-address slot are
    # occupied by unrelated immutable bytes. Neither can be overwritten.
    names = [adir / (digest + ".manifest.damaged-" +
                     cp._sha(damaged)[:12] + ".json"),
             adir / (digest + ".manifest.damaged-" +
                     cp._sha(damaged) + ".json")]
    for n, path in enumerate(names):
        path.write_bytes(("existing evidence " + str(n)).encode())
    before = {path: path.read_bytes() for path in names}
    (state / "observations.jsonl").write_bytes(original)
    (state / cp._STORE_META).unlink()
    notes = cp.migrate_observations(state)
    actual = json.loads(mp.read_text())
    check("CSRSTORE-04 replay derives exact counts from archived rows",
          all(actual[k] == valid[k] for k in cp._COUNT_KEYS)
          and actual["quarantine_faults"] == valid["quarantine_faults"]
          and any("contradicts archived" in n for n in notes))
    check("CSRSTORE-05 occupied quarantine evidence is never overwritten",
          all(path.read_bytes() == raw for path, raw in before.items()))
    check("CSRSTORE-05 new exact damaged bytes survive under another slot",
          any(path.read_bytes() == damaged
              for path in adir.glob("*.manifest.damaged-*.json")))
    preserved = {path: path.read_bytes() for path in adir.iterdir()
                 if path.is_file()}
    cp.migrate_observations(state)
    check("CSRSTORE-05 healthy replay preserves every archive object",
          all(path.read_bytes() == raw for path, raw in preserved.items()))
    (state / "observations.jsonl").write_bytes(original)
    (state / cp._STORE_META).unlink()
    mp.write_bytes(damaged)
    for number in range(8):
        suffix = "" if number == 0 else "-" + str(number)
        (adir / (digest + ".manifest.damaged-" + cp._sha(damaged)
                 + suffix + ".json")).write_bytes(
                     ("occupied slot " + str(number)).encode())
    objects = {path: path.read_bytes() for path in adir.iterdir()
               if path.is_file()}
    notes, held = cp._migrate_observations(state)
    check("CSRSTORE-05 exhausted recovery slots hold without evidence loss",
          held and store_bytes(state) == original
          and not (state / cp._STORE_META).exists()
          and all(path.read_bytes() == raw for path, raw in objects.items())
          and any("HELD" in n for n in notes))
    shutil.rmtree(w)


def main():
    t_csr04_archive_byte_identical()
    t_csr04_quarantine_malformed_on_migration()
    t_csr04_replay_never_rewrites_provenance()
    t_csr04_compromised_archive_entry_holds_store()
    t_csr07_future_version_holds_and_defers_sources()
    t_csr07_unrecognized_and_malformed_meta_hold()
    t_csr07_malformed_current_row_typed_debt()
    t_csr07_debt_note_cap()
    t_csr07_commit_survives_malformed_stored_row()
    t_csr07_load_checked_classification()
    t_csrstore01_archive_fault_holds_whole_tick()
    t_csrstore01_hold_preserves_crlf_blank_bytes_exactly()
    t_csrstore02_held_store_never_opens_enrolled_source()
    t_csrstore03_corrupt_bytes_typed_debt_never_crash()
    t_csrstore04_manifest_staged_before_switch()
    t_csrstore04_replay_validates_manifest_never_trusts()
    t_csrstore04_counts_and_quarantine_collisions()
    print(f"\n{len(PASS)} passed, {len(FAIL)} failed")
    if FAIL:
        print("FAILED:", *FAIL, sep="\n  - ")
    return 1 if FAIL else 0


if __name__ == "__main__":
    sys.exit(main())
