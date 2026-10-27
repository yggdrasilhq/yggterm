import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import time

import pytest

HERE = Path(__file__).parent
YGGTERM = HERE.parent.parent
CANON = YGGTERM / ".agents/skills/yggterm-agent-fleet/ygg-memory.py"

LEDGER = """### [T-901] fixture task
- size: small ; value: 1 ; est: x
- status: open
"""


def isolated_env(tmp_path):
    """Isolated publisher root + outbox + a PATH wrapper that forwards
    ygg-memory.py to the canonical source under the test root."""
    root = tmp_path / "memory-root"
    outbox_dir = tmp_path / "outbox"
    wrapper = tmp_path / "bin"
    wrapper.mkdir(parents=True, exist_ok=True)
    fwd = wrapper / "ygg-memory.py"
    fwd.write_text("#!/usr/bin/env python3\n"
                   "import os, sys\n"
                   f"os.execv(sys.executable, [sys.executable, '{CANON}'] + sys.argv[1:])\n")
    fwd.chmod(fwd.stat().st_mode | stat.S_IEXEC)
    env = dict(os.environ)
    env["YGGTERM_MEMORY_ROOT"] = str(root)
    env["YGG_BOOKING_OUTBOX"] = str(outbox_dir / "intents.jsonl")
    env["PATH"] = f"{wrapper}:{env.get('PATH', '')}"
    return env, root, outbox_dir


def write_ledger(tmp_path):
    ledger = tmp_path / "fleet-task-ledger.md"
    ledger.write_text(LEDGER)
    return ledger


def test_c2_concurrent_bookings_cas_loser_retained(tmp_path, monkeypatch):
    """R1 reproduction at the real publisher: two staged plan processes race
    the same base; the loser must be retained pending, never a second
    positive receipt, and the door must carry exactly one done line."""
    env, root, outbox_dir = isolated_env(tmp_path)
    ledger = write_ledger(tmp_path)
    door_dir = root / "namespaces/_global"
    door_dir.mkdir(parents=True, exist_ok=True)
    (door_dir / "fleet-task-ledger.md").write_text(LEDGER)
    barrier = tmp_path / "barrier"
    barrier.write_text("go")

    def racer():
        # Each racer stages BEFORE the barrier releases: both read the same
        # pre-race door bytes, then publish concurrently.
        code = (
            "import sys, time, pathlib\n"
            f"led = pathlib.Path('{ledger}')\n"
            "base = led.read_text()\n"
            f"while not pathlib.Path('{barrier}').exists():\n"
            "    time.sleep(0.005)\n"
            "assert led.read_text() == base, 'base drifted before stage'\n"
            "time.sleep(0.02)\n"
            "sys.argv=['plan.py','--done','T-901','--proof','p-'+str(time.time()),"
            "'--by','worker',"
            f"'--ledger','{ledger}']\n"
            f"sys.path.insert(0,'{HERE}')\n"
            "import plan\n"
            "sys.exit(plan.main())\n")
        return subprocess.Popen([sys.executable, "-c", code],
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                text=True, env=env)

    a, b = racer(), racer()
    ra = a.communicate(timeout=120)
    rb = b.communicate(timeout=120)
    outs = (ra[0] + rb[0])
    # Exactly one positive booking receipt across both racers.
    receipts = [l for l in outs.splitlines() if l.startswith("BOOKING_RECEIPT ")]
    assert len(receipts) == 1, outs
    assert "PUBLISH_BASE_CHANGED" in (ra[1] + rb[1] + outs)
    door = (root / "namespaces/_global/fleet-task-ledger.md").read_text()
    assert door.count("done(") == 1
    intents = [json.loads(l) for l in
               (outbox_dir / "intents.jsonl").read_text().splitlines() if l.strip()]
    states = sorted(r["state"] for r in intents)
    assert states == ["done", "pending"]
    # Replay of the retained loser NEVER double-books or downgrades (C5/C2):
    # the winner's verified completion is preserved; the loser's cross-proof
    # intent stays RETAINED pending (visible BOOKKEEPING_BACKLOG debt, bounded
    # backoff) — its reconciliation is a reviewed act, never a silent rewrite.
    sys.path.insert(0, str(HERE))
    import booking_outbox
    with monkeypatch.context() as m:
        # The replay's plan.py subprocess must publish into the ISOLATED root.
        m.setenv("YGGTERM_MEMORY_ROOT", str(root))
        m.setenv("PATH", env["PATH"])
        booking_outbox.replay_once(path=outbox_dir / "intents.jsonl",
                                   clock=lambda: time.time() + 600)
    intents = [json.loads(l) for l in
               (outbox_dir / "intents.jsonl").read_text().splitlines() if l.strip()]
    assert {i["state"] for i in intents} == {"done", "pending"}
    assert next(i for i in intents if i["state"] == "done")["receipt_ref"]
    loser = next(i for i in intents if i["state"] == "pending")
    assert loser["attempts"] >= 1 and loser["next_attempt_at"] is not None
    door = (root / "namespaces/_global/fleet-task-ledger.md").read_text()
    assert door.count("done(") == 1


def test_c3_terminal_intents_hold_planning(tmp_path):
    env, root, outbox_dir = isolated_env(tmp_path)
    outbox_dir.mkdir(parents=True, exist_ok=True)
    intents = outbox_dir / "intents.jsonl"
    ledger = write_ledger(tmp_path)
    ledger.write_text(LEDGER +
                      "### [T-902] queue fixture\n- size: small ; value: 1 ; est: x\n- status: open\n"
                      "## §BATCH\n"
                      "- **T-903 [moderate] (open) — Batch brief fixture.** verbatim body\n")
    def intent(task, state):
        with intents.open("a") as f:
            f.write(json.dumps({"intent_id": f"id-{task}-{state}", "task": task,
                                "proof": "p", "by": "w", "ts": 1, "state": state,
                                "attempts": 0, "next_attempt_at": 1, "ledger": None,
                                "day": None, "receipt_ref": None}) + "\n")
    def plan_json():
        return json.loads(subprocess.run(
            [sys.executable, str(HERE / "plan.py"), "--json", "--ledger", str(ledger)],
            capture_output=True, text=True, env=env, timeout=60).stdout)
    # No intents: T-901/902/903 all offered.
    def offered(p):
        ids = {t["id"] for t in p["codex_queue"]["tasks"]}
        ids |= {t["id"] for t in p["daily_batch"]["tasks"]}
        ids |= {t["id"] for t in p["manual_fire"]}
        return ids
    assert {"T-901", "T-902", "T-903"} <= offered(plan_json())
    # Pending intent holds T-901 out of every executable projection.
    intent("T-901", "pending")
    p = plan_json()
    assert [t["id"] for t in p["booking_held"]] == ["T-901"]
    assert "T-901" not in offered(p)
    # The §BATCH task itself is held too (R14: terminal intent on the BATCH row).
    intent("T-903", "pending")
    p = plan_json()
    assert "T-903" in {t["id"] for t in p["booking_held"]}
    assert "T-903" not in {t["id"] for t in p["daily_batch"]["tasks"]}
    # A DONE intent also holds a spuriously reopened row (never re-executes).
    intent("T-902", "done")
    p = plan_json()
    held = {t["id"] for t in p["booking_held"]}
    assert held == {"T-901", "T-902", "T-903"}
    assert "T-902" not in offered(p)
    # Corrupt terminal-intent storage holds planning entirely (fail closed).
    intents.write_text('{"task": "T-9", "state": "pend')
    r = subprocess.run([sys.executable, str(HERE / "plan.py"), "--json",
                        "--ledger", str(ledger)],
                       capture_output=True, text=True, env=env, timeout=60)
    assert r.returncode == 1 and "planning held" in r.stdout


def test_c2_publisher_rejects_stale_expected_digest(tmp_path):
    env, root, outbox_dir = isolated_env(tmp_path)
    ledger = write_ledger(tmp_path)
    door_dir = root / "namespaces/_global"
    door_dir.mkdir(parents=True, exist_ok=True)
    door = door_dir / "fleet-task-ledger.md"
    door.write_text(LEDGER)
    stale = __import__("hashlib").sha256(b"old-base").hexdigest()
    good = __import__("hashlib").sha256(LEDGER.encode()).hexdigest()
    def publish(digest):
        return subprocess.run(
            [sys.executable, str(CANON), "publish", "--ns=_global",
             "--as", "fleet-task-ledger.md", "--file", str(ledger),
             "--expected-digest", digest, "--json"],
            capture_output=True, text=True, env=env, timeout=60)
    r = publish(stale)
    assert r.returncode != 0 and "PUBLISH_BASE_CHANGED" in (r.stdout + r.stderr)
    door.write_text(LEDGER + "\n<!-- evolved -->\n")
    r = publish(good)
    assert r.returncode != 0 and "PUBLISH_BASE_CHANGED" in (r.stdout + r.stderr)
    r = publish(__import__("hashlib").sha256((LEDGER + "\n<!-- evolved -->\n").encode()).hexdigest())
    assert r.returncode == 0
