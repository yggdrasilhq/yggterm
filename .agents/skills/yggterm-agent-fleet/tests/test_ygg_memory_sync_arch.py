#!/usr/bin/env python3
"""[sync-arch phase 1] journal index + writer lock falsifiers.

    python3 tests/test_ygg_memory_sync_arch.py [path-to-ygg-memory.py]

Pins (sol design consult 2026-10-27):
  1. indexed == naive parity under duplicate seqs across origins, malformed
     lines and repeated digests (seq is NOT a key; (gen, off) is);
  2. the quadratic is dead: after first refresh a lookup parses a bounded
     subset, never the whole journal (counted, not timed);
  3. a legacy in-place rewrite that keeps the inode is DETECTED (anchors)
     and rebuilt, never mis-served;
  4. partial-tail repair before append; short-write abort truncates back;
  5. concurrent appends without any global lock keep seqs unique and the
     journal line-valid; merge invalidates the index (gen bump);
  6. a corrupt index degrades to the naive scan, never wrong answers.
"""

import importlib.util
import json
import os
import random
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path

MODULE_PATH = Path(__file__).resolve().parent.parent / "ygg-memory.py"


def load(path):
    spec = importlib.util.spec_from_file_location("ym", str(path))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def write_journal(path: Path, records):
    path.write_text("".join(json.dumps(r) + "\n" for r in records), encoding="utf-8")


def rand_records(rng, n, nses=("a", "b", "_global")):
    out = []
    for i in range(n):
        out.append(
            {
                "seq": rng.choice([1, 2, 3]),  # duplicate seqs across origins
                "event_id": f"e{i}",
                "origin": rng.choice(["h1", "h2"]),
                "ts": rng.randrange(1000),
                "ns": rng.choice(nses),
                "file": f"f{rng.randrange(6)}.md",
                "kind": "note",
                "action": "upsert",
                "harness": "zcode",
                "target_harness": rng.choice(["all", "zcode", "codex"]),
                "summary": f"s{i} " + "x" * rng.randrange(80),
                "digest": rng.choice(["d1", "d2", None]),
                "version_id": f"v{i}",
            }
        )
    return out


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail and not ok else ""))
    return ok


def main():
    mod = load(sys.argv[1] if len(sys.argv) > 1 else MODULE_PATH)
    results = []
    rng = random.Random(7)

    # 1. parity across every filter -------------------------------------------
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        recs = rand_records(rng, 800)
        recs[10].pop("ns")  # record without ns key
        blob = "".join(json.dumps(r) + "\n" for r in recs) + "not json\n\n  \n"
        (root / "journal.jsonl").write_text(blob, encoding="utf-8")
        cases = [
            {},
            {"namespace": "a"},
            {"after_seq": 2},
            {"namespace": "b", "after_seq": 1},
            {"target_harness": "zcode"},
            {"namespace": "_global", "target_harness": "all"},
        ]
        for kw in cases:
            got = mod.read_journal_entries(root, **kw)
            want = mod._read_journal_entries_naive(root, **kw)
            results.append(check(f"parity {kw}", got == want, f"{len(got)} vs {len(want)}"))
        v_got = mod.version_for_digest(root, "a", "f3.md", "d1")
        v_want = mod.version_for_digest.__wrapped__(root, "a", "f3.md", "d1") if hasattr(
            mod.version_for_digest, "__wrapped__"
        ) else None
        # naive version_for_digest is the original body; recompute by hand:
        found = None
        for record in mod._read_journal_entries_naive(root, namespace="a"):
            if record.get("file") == "f3.md" and record.get("digest") == "d1":
                found = record.get("version_id")
        results.append(check("version_for_digest parity", v_got == found, f"{v_got} vs {found}"))
        results.append(
            check("latest_seq parity", mod.get_latest_seq(root) == mod._get_latest_seq_naive(root))
        )

    # 2. the quadratic is dead (counted parses, not timing) --------------------
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        big = rand_records(random.Random(1), 5000)
        (root / "journal.jsonl").write_text(
            "".join(json.dumps(r) + "\n" for r in big), encoding="utf-8"
        )
        real_loads = json.loads
        counter = {"n": 0, "bytes": 0}

        def counting(s):
            counter["n"] += 1
            counter["bytes"] += len(s) if isinstance(s, (str, bytes)) else 0
            return real_loads(s)

        json.loads = counting
        try:
            mod.version_for_digest(root, "a", "f3.md", "d1")
            first = dict(counter)
            counter["n"] = counter["bytes"] = 0
            mod.version_for_digest(root, "a", "f3.md", "d1")
            second = dict(counter)
        finally:
            json.loads = real_loads
        results.append(
            check(
                "second lookup is bounded, not quadratic",
                second["bytes"] < first["bytes"] / 4,
                f"first={first['bytes']}B second={second['bytes']}B",
            )
        )

    # 3. legacy in-place rewrite (same inode, no registration) is detected -----
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        jp = root / "journal.jsonl"
        a = rand_records(random.Random(2), 50)
        write_journal(jp, a)
        assert mod.read_journal_entries(root)  # build index
        b = list(reversed(rand_records(random.Random(3), 50)))  # re-sorted, different
        write_journal(jp, b)  # same path, no gen bump, size may differ
        got = mod.read_journal_entries(root)
        want = mod._read_journal_entries_naive(root)
        results.append(check("in-place rewrite rebuilt", got == want))

    # 4. partial tail repaired, then append works ------------------------------
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        jp = root / "journal.jsonl"
        write_journal(jp, rand_records(random.Random(4), 10))
        with open(jp, "a", encoding="utf-8") as f:
            f.write('{"seq": 99, "trunc')  # crash-style partial line
        mod.append_journal_entry(root, "a", "x.md", "note", "upsert", "zcode", "t4")
        recs = mod.read_journal_entries(root)
        results.append(
            check(
                "partial tail repaired + append",
                all(r.get("seq") != 99 for r in recs)
                and sum(1 for r in recs if r.get("summary") == "t4") == 1
                and recs == mod._read_journal_entries_naive(root),
            )
        )

    # 5. concurrent lock-free appends: unique seqs, valid lines -----------------
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        (root / "journal.jsonl").write_text("", encoding="utf-8")
        body = "".join(
            "m.append_journal_entry(root,'w'+sys.argv[3],'f.md','note','upsert',"
            "'zcode','p%d-'+sys.argv[3]);" % i
            for i in range(30)
        )
        runner = (
            "import importlib.util,sys,time;"
            "spec=importlib.util.spec_from_file_location('ym',sys.argv[1]);"
            "m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);"
            "from pathlib import Path;root=Path(sys.argv[4]);"
            "time.sleep(float(sys.argv[2]));"
            + body
        )
        t0 = time.time()
        procs = [
            subprocess.Popen(
                [sys.executable, "-c", runner, str(MODULE_PATH), str(max(0.0, 0.4 - w * 0.13)), str(w), str(root)],
                stderr=subprocess.PIPE,
            )
            for w in range(4)
        ]
        errs = [p.communicate(timeout=120)[1] for p in procs]
        rcs = [p.returncode for p in procs]
        lines = [json.loads(x) for x in (root / "journal.jsonl").read_text().splitlines() if x.strip()]
        seqs = [r["seq"] for r in lines]
        results.append(
            check(
                "4-way concurrent appends",
                rcs == [0, 0, 0, 0] and len(lines) == 120 and len(set(seqs)) == 120,
                f"rcs={rcs} lines={len(lines)} uniq={len(set(seqs))} err={errs[0][-200:] if rcs != [0,0,0,0] else ''}",
            )
        )
        results.append(
            check(
                "concurrent journal == naive",
                mod.read_journal_entries(root) == mod._read_journal_entries_naive(root),
            )
        )

    # 5b. merge bumps generation, read stays correct ----------------------------
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        jp = root / "journal.jsonl"
        write_journal(jp, rand_records(random.Random(5), 30))
        mod.read_journal_entries(root)
        peer_recs = rand_records(random.Random(6), 20)
        for k, r in enumerate(peer_recs):
            r["event_id"] = f"peer{k}"
        peer = "\n".join(json.dumps(r) for r in peer_recs)
        mod._merge_journals(jp, peer)
        got = mod.read_journal_entries(root)
        results.append(
            check(
                "merge rebuild + parity",
                got == mod._read_journal_entries_naive(root) and len(got) == 50,
                f"n={len(got)}",
            )
        )

    # 6. corrupt index degrades to naive ----------------------------------------
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        write_journal(root / "journal.jsonl", rand_records(random.Random(8), 25))
        (root / "journal-index.db").write_bytes(b"this is not sqlite at all" * 100)
        got = mod.read_journal_entries(root)
        results.append(
            check(
                "corrupt index -> naive fallback",
                got == mod._read_journal_entries_naive(root),
            )
        )

    # R1: same-size MIDDLE rewrite is detected (head + tail anchors unchanged)
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        jp = root / "journal.jsonl"
        a = rand_records(random.Random(11), 40)
        write_journal(jp, a)
        mod.read_journal_entries(root)
        b = [dict(r) for r in a]
        b[20]["version_id"] = "BAD"  # same-size middle mutation
        write_journal(jp, b)
        got = mod.read_journal_entries(root)
        results.append(
            check(
                "same-size middle rewrite detected",
                got == mod._read_journal_entries_naive(root)
                and any(r.get("version_id") == "BAD" for r in got),
            )
        )
        # R1b: a stale max_seq must not survive a rebuild after truncation
        lines = jp.read_text().splitlines()
        jp.write_text("\n".join(lines[:5]) + "\n", encoding="utf-8")
        results.append(
            check(
                "max_seq decreases after truncation",
                mod.get_latest_seq(root) == max(int(json.loads(l)["seq"]) for l in lines[:5]),
            )
        )

    # R3: unchanged refreshes preserve anchors — gen stays flat across reads
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        write_journal(root / "journal.jsonl", rand_records(random.Random(12), 30))
        gens = []
        for _ in range(5):
            mod.read_journal_entries(root)
            conn = mod._ji_connect(root)
            gens.append(mod._ji_meta_get(conn).get("gen"))
            conn.close()
        results.append(check("unchanged refresh keeps gen flat", len(set(gens)) == 1, str(gens)))

    # R2b: a long (100KB) unparsed tail is repaired without losing records
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        jp = root / "journal.jsonl"
        write_journal(jp, rand_records(random.Random(13), 10))
        with open(jp, "ab") as f:
            f.write(b"x" * 100_000)  # >64KB debris, no newline
        mod.append_journal_entry(root, "a", "x.md", "note", "upsert", "zcode", "t-long")
        recs = mod.read_journal_entries(root)
        results.append(
            check(
                "long tail repaired, records kept",
                len(recs) == 11
                and sum(1 for r in recs if r.get("summary") == "t-long") == 1
                and recs == mod._read_journal_entries_naive(root),
            )
        )

    # R6a: a valid unterminated final record is read, then newline-completed
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        jp = root / "journal.jsonl"
        write_journal(jp, rand_records(random.Random(14), 5))
        with open(jp, "a", encoding="utf-8") as f:
            f.write(json.dumps({"seq": 6, "ns": "a", "file": "u.md", "digest": None,
                                "version_id": "v-unterminated", "target_harness": "all"}))
        got = mod.read_journal_entries(root)
        seen = any(r.get("version_id") == "v-unterminated" for r in got)
        mod.append_journal_entry(root, "a", "y.md", "note", "upsert", "zcode", "t-after")
        still = mod._read_journal_entries_naive(root)
        results.append(
            check(
                "unterminated final record read + preserved",
                seen
                and any(r.get("version_id") == "v-unterminated" for r in still)
                and any(r.get("summary") == "t-after" for r in still),
            )
        )

    # R6b: digest lookup excludes seq<=0 rows exactly like the naive scan
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        recs = rand_records(random.Random(15), 20)
        recs[7]["seq"] = 0
        write_journal(root / "journal.jsonl", recs)
        results.append(
            check(
                "digest lookup skips seq=0",
                mod.version_for_digest(root, recs[7]["ns"], recs[7]["file"], recs[7]["digest"])
                == (
                    [r.get("version_id") for r in mod._read_journal_entries_naive(
                        root, namespace=recs[7]["ns"])
                      if r.get("file") == recs[7]["file"] and r.get("digest") == recs[7]["digest"]] or [None]
                )[-1],
            )
        )

    # R5: hammer appends + merges concurrently — no record may be lost
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        (root / "journal.jsonl").write_text("", encoding="utf-8")
        body = (
            "from pathlib import Path\n"
            "root=Path(sys.argv[3])\n"
            "for i in range(15):\n"
            "    m.append_journal_entry(root,'h','f.md','note','upsert','zcode',"
            "'hammer-'+sys.argv[2]+str(i))\n"
            "    if i%5==0:\n"
            "        m._merge_journals(root/'journal.jsonl','')\n"
        )
        runner = (
            "import importlib.util,sys;"
            "spec=importlib.util.spec_from_file_location('ym',sys.argv[1]);"
            "m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);" + body
        )
        procs = [
            subprocess.Popen([sys.executable, "-c", runner, str(MODULE_PATH), str(w), str(root)],
                             stderr=subprocess.PIPE)
            for w in range(3)
        ]
        rcs = [p.wait(timeout=180) for p in procs]
        recs = mod.read_journal_entries(root)
        hammers = [r for r in recs if str(r.get("summary", "")).startswith("hammer-")]
        results.append(
            check(
                "append+merge hammer loses nothing",
                rcs == [0, 0, 0] and len(hammers) == 45 and len(set(r["summary"] for r in hammers)) == 45,
                f"rcs={rcs} hammers={len(hammers)}",
            )
        )

    print(f"\n{'ALL PASS' if all(results) else 'FAILURES PRESENT'} ({sum(results)}/{len(results)})")
    return 0 if all(results) else 1


if __name__ == "__main__":
    sys.exit(main())
