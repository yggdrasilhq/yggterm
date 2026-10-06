#!/usr/bin/env python3
"""[mem-dream 3/4] the auto-dreamer's selection discipline + the canary probes.

    python3 tests/test_memory_dream_auto.py [path-to-ygg-memory.py]

⛔ THE HOLES THIS PINS (gemini round-4, node
2026-10-06-yggterm-memory-dreaming-round4-auto-and-completeness.md):
(1) PHANTOM AUTOMATION — a dream trigger that blocks the 60s replication
tick, stampedes all hosts, or re-stages briefs across a quota window is
theater. The auto-dreamer must be one-namespace-per-invocation, cooldown +
quiescence + pending filtered, backoff-stamped on failure, and never raise.
(2) AMNESIAC WATERMARKS — a per-host scratch watermark makes a foreign-host
dream re-collect from seq 0; the journal's dream-origin NOW.md records are
the fleet floor. (3) The canary: a client regenerator re-bloating MEMORY.md
(sitting 10's 1409-line incident) must never recur silently.
"""

import datetime
import importlib.util
import json
import subprocess
import sys
import tempfile
import time
from pathlib import Path


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("ygg_memory_under_test", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def write_journal(root: Path, records: list) -> None:
    journal = root / "journal.jsonl"
    journal.parent.mkdir(parents=True, exist_ok=True)
    with open(journal, "a", encoding="utf-8") as handle:
        for record in records:
            handle.write(json.dumps(record) + "\n")


def main() -> int:
    default = Path(__file__).resolve().parent.parent / "ygg-memory.py"
    module = load_module(Path(sys.argv[1]) if len(sys.argv) > 1 else default)
    failures = []

    def check(condition, message):
        if condition:
            print(f"  ok: {message}")
        else:
            failures.append(message)
            print(f"  FAIL: {message}")

    print("[1] the journal-derived watermark floor")
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        write_journal(
            root,
            [
                {"seq": 10, "ts": time.time() - 4000, "ns": "ns-x", "file": "door.md", "origin": "jojo", "summary": "sitting write"},
                {"seq": 11, "ts": time.time() - 3900, "ns": "ns-x", "file": "NOW.md", "origin": "dreamer", "summary": "dream rollup [consumed seq 9]"},
                {"seq": 12, "ts": time.time() - 3800, "ns": "other", "file": "NOW.md", "origin": "dreamer", "summary": "foreign ns dream"},
                {"seq": 13, "ts": time.time() - 3700, "ns": "ns-x", "file": "door.md", "origin": "jojo", "summary": "later non-dream write"},
            ],
        )
        watermark = module.load_dream_watermark(root, "ns-x")
        check(watermark["last_seq"] == 9, "a foreign host's dream publish floors the scratch watermark at its journal seq")
        watermark = module.load_dream_watermark(root, "never-dreamed")
        check(watermark["last_seq"] == 0, "an undreamed ns starts at zero")

    print("[2] the auto-dreamer's filters")
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        dream_dir = root / "dream" / "ns-hot"
        dream_dir.mkdir(parents=True)
        (dream_dir / "watermark.json").write_text(
            json.dumps({"ns": "ns-hot", "last_seq": 5, "last_dream_ts": None, "dreams": []}),
            encoding="utf-8",
        )
        write_journal(root, [{"seq": 6, "ts": time.time() - 3600, "ns": "ns-hot", "file": "d.md", "origin": "jojo", "summary": "write"}])

        class Args:
            model = "gpt-6.1-sol"
            effort = "high"

        # Backoff stamp: any reason, within the hour → quiet return.
        module._dream_auto_backoff(root, "composer quota capped")
        message = module._dream_auto(root, "", Args())
        check("backing off" in message, "an active backoff stamp defers the auto-dream quietly")
        # Expire the stamp; a hot ns with an old write is eligible — but the
        # composer subprocess must fail softly (no codex in the test env path
        # resolves) and stamp the hour backoff rather than raise.
        (root / "dream" / "auto-backoff.json").write_text(
            json.dumps({"ts": time.time() - 3700, "reason": "stale"}), encoding="utf-8"
        )
        try:
            message = module._dream_auto(root, "", Args())
            check(
                isinstance(message, str),
                "the auto-dreamer returns a message instead of raising on composer failure",
            )
        except SystemExit as error:
            check(False, f"the auto-dreamer must never exit-hard (got {error})")
        check((root / "dream" / "auto-backoff.json").is_file(), "composer failure stamps the 1h backoff")
        # Cooldown: a recent dream defers candidacy.
        (root / "dream" / "auto-backoff.json").unlink(missing_ok=True)
        (dream_dir / "watermark.json").write_text(
            json.dumps(
                {
                    "ns": "ns-hot",
                    "last_seq": 5,
                    "last_dream_ts": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                    "dreams": [],
                }
            ),
            encoding="utf-8",
        )
        (root / "dream" / "auto-backoff.json").unlink(missing_ok=True)
        message = module._dream_auto(root, "", Args())
        check("no candidate" in message, "a namespace inside the 2h cooldown is not a candidate")
        # Quiescence: a write 30s ago defers candidacy even past cooldown.
        (root / "dream" / "auto-backoff.json").unlink(missing_ok=True)
        (dream_dir / "watermark.json").write_text(
            json.dumps({"ns": "ns-hot", "last_seq": 5, "last_dream_ts": None, "dreams": []}),
            encoding="utf-8",
        )
        write_journal(root, [{"seq": 7, "ts": time.time() - 30, "ns": "ns-hot", "file": "d.md", "origin": "jojo", "summary": "live write"}])
        message = module._dream_auto(root, "", Args())
        check("no candidate" in message, "a live write inside quiescence defers candidacy")
        # Pending manifest: a staged dream belongs to its stager.
        (root / "dream" / "auto-backoff.json").unlink(missing_ok=True)
        write_journal(root, [{"seq": 8, "ts": time.time() - 3600, "ns": "ns-hot", "file": "d.md", "origin": "jojo", "summary": "older write"}])
        (dream_dir / "pending").mkdir(exist_ok=True)
        (dream_dir / "pending" / "MANIFEST.json").write_text("{}", encoding="utf-8")
        message = module._dream_auto(root, "", Args())
        check("no candidate" in message, "a pending staged dream is left for its stager")

    print("[3] the canary probes")
    with tempfile.TemporaryDirectory() as tmp:
        home = Path(tmp)
        projects = home / ".zcode" / "cli" / "memories" / "projects" / "p1" / "memory"
        projects.mkdir(parents=True)
        (projects / "MEMORY.md").write_text("# Memory index\n" + "\n".join(f"- line {i}" for i in range(500)), encoding="utf-8")
        (projects.parent / "memory-archive").mkdir()
        root = home / ".yggterm" / "memory"
        ns_dir = root / "namespaces" / "ns-a"
        ns_dir.mkdir(parents=True)
        (ns_dir / "NOW.md").write_text("x" * 4000, encoding="utf-8")
        dream_dir = root / "dream" / "ns-a"
        dream_dir.mkdir(parents=True)
        (dream_dir / "watermark.json").write_text(
            json.dumps({"ns": "ns-a", "last_seq": 9, "last_dream_ts": None, "dreams": [{"seq": 5}, {"seq": 3}]}),
            encoding="utf-8",
        )
        original_home = Path.home
        module.Path.home = staticmethod(lambda: home)
        module.DEFAULT_MEMORY_ROOT = root
        try:
            import types

            canary_args = types.SimpleNamespace(root=str(root), json=False, harness="zcode", ns=None)

            import io
            import contextlib

            stderr = io.StringIO()
            try:
                with contextlib.redirect_stderr(stderr):
                    module.cmd_canary(canary_args)
                check(False, "a bloated index + missing pointer + oversized NOW.md + non-monotonic watermark must exit 1")
            except SystemExit as error:
                check(error.code == 1, "the canary exits 1 on violations")
                text = stderr.getvalue()
                check("lines > 400" in text, "the index blowout is named")
                check("ARCHIVE-INDEX.md pointer is missing" in text, "the missing archive pointer is named")
                check("over budget" in text, "the oversized NOW.md is named")
                check("not monotonic" in text, "the non-monotonic watermark is named")
            # Green path: fix every violation.
            (projects / "MEMORY.md").write_text("# Memory index\n", encoding="utf-8")
            (projects / "ARCHIVE-INDEX.md").write_text("# Archive index\n", encoding="utf-8")
            (ns_dir / "NOW.md").write_text("ok", encoding="utf-8")
            (dream_dir / "watermark.json").write_text(
                json.dumps({"ns": "ns-a", "last_seq": 9, "last_dream_ts": None, "dreams": [{"seq": 3}, {"seq": 5}]}),
                encoding="utf-8",
            )
            try:
                with contextlib.redirect_stdout(io.StringIO()):
                    module.cmd_canary(canary_args)
                check(True, "the green path exits 0")
            except SystemExit:
                check(False, "the green path must not exit 1")
        finally:
            module.Path.home = original_home

    if failures:
        print(f"\n{len(failures)} FAILURES")
        return 1
    print("\nall auto-dream + canary locks green")
    return 0


if __name__ == "__main__":
    sys.exit(main())
