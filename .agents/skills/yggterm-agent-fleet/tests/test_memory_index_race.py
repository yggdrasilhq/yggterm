#!/usr/bin/env python3
"""[memory-index-race] indexed-cache transaction coherence: a consumer's
snapshot must span cache refresh AND rows.

    python3 tests/test_memory_index_race.py [path-to-ygg-memory.py]

THE HOLE (lane ygg4/memory-index-race, measured against main@73f2bdad):
_ji_refresh committed its own transaction; _ji_read / version_for_digest /
get_latest_seq then opened a separate snapshot. An invalidator — any second
SQLite connection running _ji_invalidate — committing between the two left
the reader a bumped generation with zero rows: a healthy-looking EMPTY
history while the authoritative naive journal still returned every record.
Through the publish path (latest_version_for -> read_journal_entries) that
empty view resolved an update's causal parent to None, silently forking the
lineage even when the expected-digest CAS passed. _ji_refresh and _ji_install
also chose the generation BEFORE taking the write transaction, so two
invalidations landing in that gap moved it backwards.

Fix under test: one coherent write transaction per indexed consumer — the
consumer BEGINs IMMEDIATE, refresh joins the caller's transaction (no nested
BEGIN/COMMIT), generation/anchors/rows/seq commit together, rollback on
failure; _ji_install picks its generation inside its own transaction.

Faults are injected ONLY into a sandbox root — never the live DB. Scratch
stays off /tmp (repo law): fixtures live under ~/.yggterm/scratchpad/ and
are removed on success.
"""

import hashlib
import importlib.util
import json
import os
import shutil
import sys
import threading
import time
from pathlib import Path

DEFAULT_SRC = Path(__file__).resolve().parent.parent / "ygg-memory.py"
SCRATCH = Path.home() / ".yggterm" / "scratchpad"
NS = "-fixture"
DOOR = "fixture.md"

failures = []


def check(cond, msg):
    print(("  ok  " if cond else "  FAIL") + " " + msg)
    if not cond:
        failures.append(msg)
    return cond


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("ygg_memory_race_under_test", str(path))
    module = importlib.util.module_from_spec(spec)
    sys.modules["ygg_memory_race_under_test"] = module
    spec.loader.exec_module(module)
    return module


def sandbox(name: str) -> Path:
    SCRATCH.mkdir(parents=True, exist_ok=True)
    root = SCRATCH / f"memory-index-race-{name}-{os.getpid()}"
    if root.exists():
        shutil.rmtree(root)
    root.mkdir(parents=True)
    return root


def release(root: Path):
    shutil.rmtree(root, ignore_errors=True)


def make_record(seq: int, origin: str = "test-origin") -> dict:
    body = f"content of fixture revision {seq}\n"
    return {
        "seq": seq,
        "event_id": f"evt-{seq:06d}",
        "origin": origin,
        "ts": 1793360000 + seq,
        "iso": f"2026-10-30T00:{seq:02d}:00+00:00",
        "ns": NS,
        "file": DOOR,
        "kind": "user",
        "action": "upsert",
        "summary": f"fixture revision {seq}",
        "digest": hashlib.sha256(body.encode()).hexdigest(),
        "version_id": f"event:fixture-{seq}",
    }


def write_journal(module, root: Path, records: list) -> Path:
    jpath = module.get_journal_path(root)
    jpath.parent.mkdir(parents=True, exist_ok=True)
    jpath.write_text(
        "".join(json.dumps(r, sort_keys=True) + "\n" for r in records),
        encoding="utf-8",
    )
    return jpath


def prime(module, root: Path, count: int = 2) -> list:
    records = [make_record(i + 1) for i in range(count)]
    write_journal(module, root, records)
    assert len(module.read_journal_entries(root)) == count, "warm priming read failed"
    return records


def identity(entries: list) -> list:
    return [(r.get("seq"), r.get("ns"), r.get("file"), r.get("version_id")) for r in entries]


class _InvalidationRig:
    """Injects _ji_invalidate from a second SQLite connection at one seam of
    the next indexed consumer, on its FIRST connection only. Seams:
      commit — invalidation lands after refresh's commit, before the
               consumer's snapshot (the measured production gap);
      begin  — invalidation lands BEFORE the consumer's first
               "BEGIN IMMEDIATE" takes the write lock (stale prepared meta);
      begin2 — like begin, but TWO invalidations (generation-backwards probe
               for the install path).
    The rig never touches anything outside the sandbox root."""

    def __init__(self, module, seam: str):
        self.m = module
        self.seam = seam
        self.armed = True
        self.fired = False
        self.real_connect = module._ji_connect
        self.root = None
        module._ji_connect = self._connect

    def _connect(self, root: Path):
        if self.root is None:
            self.root = root
        conn = self.real_connect(root)
        if self.armed and not self.fired:
            self.fired = True
            return _FaultProxy(conn, self)
        return conn

    def _fire(self, times: int):
        self.armed = False  # the invalidator's own connect must be real
        for _ in range(times):
            self.m._ji_invalidate(self.root)
        # re-arm not needed: the rig fires exactly once per consumer call;
        # callers rearm explicitly between scenarios.
        self.fired_count = times

    def disarm(self):
        self.armed = False
        self.m._ji_connect = self.real_connect


class _FaultProxy:
    def __init__(self, conn, rig: _InvalidationRig):
        self._conn = conn
        self._rig = rig
        self._fired = False

    def __getattr__(self, name):
        return getattr(self._conn, name)

    def commit(self):
        self._conn.commit()
        if not self._fired:
            self._fired = True
            if self._rig.seam == "commit":
                self._rig._fire(1)

    def execute(self, sql, *args, **kwargs):
        if not self._fired and "BEGIN IMMEDIATE" in sql and self._rig.seam in ("begin", "begin2"):
            self._fired = True
            self._rig._fire(2 if self._rig.seam == "begin2" else 1)
        return self._conn.execute(sql, *args, **kwargs)


def faulted(module, seam: str):
    return _InvalidationRig(module, seam)


def meta_gen(module, root: Path) -> int:
    conn = module._ji_connect(root)
    try:
        return int(module._ji_meta_get(conn).get("gen", "0") or 0)
    finally:
        conn.close()


# --------------------------------------------------------------------------
# the two deterministic invalidation gaps, plus the three consumers


def t_commit_gap_indexed_read(m):
    print("\n[t] commit-gap invalidation: indexed read must equal the naive journal")
    root = sandbox("commit-gap-read")
    try:
        prime(m, root, 2)
        naive = identity(m._read_journal_entries_naive(root))
        rig = faulted(m, "commit")
        try:
            got = m.read_journal_entries(root)
        finally:
            rig.disarm()
        check(rig.fired, "fault fired at the refresh-commit seam")
        check(identity(got) == naive == [(1, NS, DOOR, "event:fixture-1"),
                                         (2, NS, DOOR, "event:fixture-2")],
              f"indexed read survives the gap (got {len(got)} records)")
        check(len(m.read_journal_entries(root)) == 2, "post-fault read stays correct")
    finally:
        release(root)


def t_begin_gap_indexed_read(m):
    print("\n[t] begin-gap invalidation: stale prepared meta must not win")
    root = sandbox("begin-gap-read")
    try:
        prime(m, root, 2)
        rig = faulted(m, "begin")
        try:
            got = m.read_journal_entries(root)
        finally:
            rig.disarm()
        naive = m._read_journal_entries_naive(root)
        check(len(got) == 2, f"indexed read survives a pre-BEGIN invalidation (got {len(got)})")
        check(identity(got) == identity(naive), "indexed equals naive after the gap")
    finally:
        release(root)


def t_seq_and_version_roundtrip(m):
    print("\n[t] get_latest_seq / version_for_digest roundtrip under the gap")
    root = sandbox("seq-version-roundtrip")
    try:
        records = prime(m, root, 2)
        rig = faulted(m, "commit")
        try:
            seq = m.get_latest_seq(root)
        finally:
            rig.disarm()
        check(seq == 2, f"get_latest_seq returns 2 under the gap (got {seq})")
        rig = faulted(m, "commit")
        try:
            ver = m.version_for_digest(root, NS, DOOR, records[-1]["digest"])
        finally:
            rig.disarm()
        check(ver == "event:fixture-2", f"version_for_digest resolves under the gap (got {ver})")
        check(m.version_for_digest(root, NS, DOOR, "0" * 64) is None,
              "unknown digest still resolves to None (roundtrip negative)")
    finally:
        release(root)


def t_publish_update_keeps_parent(m):
    print("\n[t] real publish create->update under the gap keeps the causal parent")
    root = sandbox("publish-parent")
    try:
        door = root / "namespaces" / NS / DOOR
        door.parent.mkdir(parents=True, exist_ok=True)
        door.write_text("revision 1\n", encoding="utf-8")
        created = m.append_journal_entry(root, NS, DOOR, "user", "upsert", "zcode", "create")
        check(created["seq"] == 1 and created["version_id"], "create published with a version")
        rig = faulted(m, "commit")
        try:
            door.write_text("revision 2\n", encoding="utf-8")
            updated = m.append_journal_entry(root, NS, DOOR, "user", "upsert", "zcode", "update")
        finally:
            rig.disarm()
        check(updated["base_version"] == created["version_id"],
              f"update keeps its causal parent under the gap (base={updated.get('base_version')!r})")
        naive = m._read_journal_entries_naive(root)
        check(len(naive) == 2, "journal holds exactly create+update (no fork rows)")
    finally:
        release(root)


def t_install_generation_never_moves_backwards(m):
    print("\n[t] _ji_install generation never moves backwards across invalidations")
    root = sandbox("install-gen")
    try:
        records = prime(m, root, 2)
        gen0 = meta_gen(m, root)
        jpath = m.get_journal_path(root)
        covered = jpath.stat().st_size
        offsets = [0]
        pos = 0
        for line in jpath.read_bytes().splitlines(keepends=True):
            pos += len(line)
            offsets.append(pos)
        offsets = offsets[:-1]
        rig = faulted(m, "begin2")
        try:
            m._ji_install(root, records, offsets, covered)
        finally:
            rig.disarm()
        gen1 = meta_gen(m, root)
        # two invalidations each bump the generation before install takes the
        # lock; a correct install reads its base INSIDE the transaction and
        # lands at gen0+3. The old pre-BEGIN read reused the stale gen0+1,
        # silently rewinding the two invalidations.
        check(gen1 >= gen0 + 3, f"generation moved forward past both invalidations ({gen0} -> {gen1})")
        check(len(m.read_journal_entries(root)) == 2,
              "installed rows are still readable after the double invalidation")
    finally:
        release(root)


def t_indexed_vs_naive_matrix(m):
    print("\n[t] indexed vs naive equivalence: cold, invalidate, rewrite, truncate, multi-origin")
    root = sandbox("equivalence")
    try:
        jpath = write_journal(m, root, [make_record(1), make_record(2)])

        def both(label, expect):
            got = m.read_journal_entries(root)
            nv = m._read_journal_entries_naive(root)
            check(identity(got) == identity(nv) == expect, f"{label}: indexed==naive=={expect}")

        both("cold first read (fresh process equivalent)",
             [(1, NS, DOOR, "event:fixture-1"), (2, NS, DOOR, "event:fixture-2")])
        m._ji_invalidate(root)
        both("after invalidation",
             [(1, NS, DOOR, "event:fixture-1"), (2, NS, DOOR, "event:fixture-2")])
        rewritten = [make_record(3), make_record(4), make_record(5)]
        write_journal(m, root, rewritten)
        m._ji_invalidate(root)
        both("after full rewrite + invalidation",
             [(3, NS, DOOR, "event:fixture-3"),
              (4, NS, DOOR, "event:fixture-4"),
              (5, NS, DOOR, "event:fixture-5")])
        jpath.write_bytes(b"")
        both("after truncate to empty", [])
        dup = [make_record(10, "origin-a"), make_record(10, "origin-b"), make_record(11)]
        write_journal(m, root, dup)
        m._ji_invalidate(root)
        got = m.read_journal_entries(root)
        origins = sorted(r["origin"] for r in got if r.get("seq") == 10)
        check(len(got) == 3 and origins == ["origin-a", "origin-b"],
              f"duplicate-seq multi-origin rows both survive (got {len(got)}, origins={origins})")
        nv = m._read_journal_entries_naive(root)
        check(identity(got) == identity(nv), "duplicate-seq indexed equals naive")
    finally:
        release(root)


def t_concurrent_readers_and_invalidators(m):
    print("\n[t] concurrent readers + invalidators: bounded, no empty history, gen monotonic")
    root = sandbox("concurrency")
    try:
        prime(m, root, 2)
        stop = threading.Event()
        errors = []
        gens = []
        gen_lock = threading.Lock()

        def reader():
            try:
                while not stop.is_set():
                    out = m.read_journal_entries(root)
                    if len(out) < 2:
                        errors.append(f"reader saw {len(out)} records (< primed base)")
                        return
            except Exception as exc:  # noqa: BLE001
                errors.append(f"reader raised: {exc}")

        def invalidator():
            try:
                while not stop.is_set():
                    m._ji_invalidate(root)
                    with gen_lock:
                        gens.append(meta_gen(m, root))
            except Exception as exc:  # noqa: BLE001
                errors.append(f"invalidator raised: {exc}")

        threads = [threading.Thread(target=reader, daemon=True) for _ in range(3)]
        threads += [threading.Thread(target=invalidator, daemon=True) for _ in range(2)]
        started = time.monotonic()
        for t in threads:
            t.start()
        time.sleep(3.0)
        stop.set()
        for t in threads:
            t.join(timeout=20)
        elapsed = time.monotonic() - started
        check(not any(t.is_alive() for t in threads), "all threads finished inside the bound")
        check(not errors, f"no reader observed an empty history ({errors[:2]})")
        with gen_lock:
            monotonic = all(b >= a for a, b in zip(gens, gens[1:]))
        check(monotonic and len(gens) > 10,
              f"observed generations never moved backwards ({len(gens)} samples)")
        print(f"  receipt: {elapsed:.2f}s wall, {len(gens)} invalidation samples")
        check(len(m.read_journal_entries(root)) == 2, "post-storm read still correct")
    finally:
        release(root)


def t_latency_receipt(m):
    print("\n[t] latency receipt (informational, loose sanity bound only)")
    root = sandbox("latency")
    try:
        write_journal(m, root, [make_record(i + 1) for i in range(2000)])
        m.read_journal_entries(root)  # warm build
        t0 = time.monotonic()
        got = m.read_journal_entries(root)
        t_idx = time.monotonic() - t0
        t0 = time.monotonic()
        nv = m._read_journal_entries_naive(root)
        t_nv = time.monotonic() - t0
        print(f"  receipt: warm indexed read {t_idx * 1000:.1f} ms, "
              f"naive scan {t_nv * 1000:.1f} ms, records={len(got)}")
        check(len(got) == len(nv) == 2000, "2000-record read is complete on both paths")
        # Latency is informational; correctness is independent of host load.
    finally:
        release(root)


def t_scalar_read_commits_refresh(m):
    print("\n[t] scalar lookups commit derived refresh, not just return correct values")
    for consumer in ("version", "sequence"):
        root = sandbox("scalar-commit-" + consumer)
        try:
            row = make_record(1)
            write_journal(m, root, [row])
            if consumer == "version":
                m.version_for_digest(root, row["ns"], row["file"], row["digest"])
            else:
                m.get_latest_seq(root)
            conn = m._ji_connect(root)
            try:
                meta = m._ji_meta_get(conn)
                count = conn.execute("SELECT COUNT(*) FROM entries").fetchone()[0]
                check(int(meta.get("covered", 0)) == m.get_journal_path(root).stat().st_size
                      and count == 1, consumer + " commits refresh for subsequent processes")
            finally: conn.close()
        finally: release(root)


def main() -> int:
    src = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_SRC
    m = load_module(src)
    print(f"module under test: {src}")
    for t in (t_commit_gap_indexed_read, t_begin_gap_indexed_read,
              t_seq_and_version_roundtrip, t_publish_update_keeps_parent,
              t_install_generation_never_moves_backwards, t_indexed_vs_naive_matrix,
              t_concurrent_readers_and_invalidators, t_scalar_read_commits_refresh, t_latency_receipt):
        t(m)
    if failures:
        print(f"\n{len(failures)} FAILURES")
        for f in failures:
            print(f"  - {f}")
        return 1
    print("\nall index-race locks green")
    return 0


if __name__ == "__main__":
    sys.exit(main())
