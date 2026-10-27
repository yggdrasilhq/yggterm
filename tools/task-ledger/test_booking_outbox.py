import fcntl
import json
import os
from pathlib import Path
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).parent))
import booking_outbox as outbox


def test_intent_fsync_survives_writer_kill(tmp_path):
    path = tmp_path / "intents.jsonl"
    script = ("import sys,time; sys.path.insert(0,sys.argv[1]); "
              "from booking_outbox import write_intent; "
              "write_intent('T-901','proof','worker',path=sys.argv[2]); "
              "print('durable',flush=True); time.sleep(120)")
    process = subprocess.Popen([sys.executable, "-c", script, str(Path(__file__).parent), str(path)], stdout=subprocess.PIPE, text=True)
    try:
        assert process.stdout.readline().strip() == "durable"
        process.kill()
        process.wait(timeout=5)
        assert outbox.read_intents(path)[0]["state"] == "pending"
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()


def test_idempotency_and_success_receipt(tmp_path):
    path = tmp_path / "intents.jsonl"
    one = outbox.write_intent("T-901", "proof", "worker", path=path, ts=100)
    two = outbox.write_intent("T-901", "proof", "worker", path=path, ts=999)
    assert one == two
    assert len(outbox.read_intents(path)) == 1
    seen = []
    def publish(row):
        seen.append(row["intent_id"])
        return "receipt:test-publication"
    outbox.replay_once(path=path, publish=publish, clock=lambda: 1000)
    row = outbox.read_intents(path)[0]
    assert row["state"] == "done" and row["receipt_ref"] == "receipt:test-publication"
    outbox.replay_once(path=path, publish=publish, clock=lambda: 2000)
    assert seen == [one["intent_id"]]


def test_failure_backoff_and_missing_receipt_stay_pending(tmp_path):
    path = tmp_path / "intents.jsonl"
    outbox.write_intent("T-901", "proof", "worker", path=path, ts=100)
    def fail(row):
        raise RuntimeError("unavailable")
    outbox.replay_once(path=path, publish=fail, clock=lambda: 100)
    row = outbox.read_intents(path)[0]
    assert (row["state"], row["attempts"], row["next_attempt_at"]) == ("pending", 1, 105)
    assert outbox.replay_once(path=path, publish=fail, clock=lambda: 104) == []
    outbox.replay_once(path=path, publish=lambda row: None, clock=lambda: 105)
    row = outbox.read_intents(path)[0]
    assert row["state"] == "pending" and row["next_attempt_at"] == 120
    assert row["receipt_ref"] is None


def test_plan_lock_failure_retained_and_replay_verified(tmp_path):
    ledger = tmp_path / "ledger.md"
    ledger.write_text("### [T-901] synthetic task\n- size: small ; value: 2 ; est: x\n- route: codex\n- status: open\n\n## §BOOK — one-liners of finished work\n")
    path = tmp_path / "outbox/intents.jsonl"
    bindir = tmp_path / "bin"
    bindir.mkdir()
    hublock = tmp_path / "hub.lock"
    runner = bindir / "ygg-memory.py"
    runner.write_text('''import fcntl,hashlib,json,os,pathlib,sys
with open(os.environ["TEST_HUB_LOCK"],"a") as f:
    try: fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB)
    except BlockingIOError:
        print("hub lock held"); sys.exit(9)
    source=sys.argv[sys.argv.index("--file")+1]
    pathlib.Path(os.environ["TEST_LEDGER"]).write_text(pathlib.Path(source).read_text())
    digest=hashlib.sha256(pathlib.Path(source).read_bytes()).hexdigest()
    print(json.dumps({"status":"ok","record":{"ns":"_global","file":"fleet-task-ledger.md","digest":digest,"event_id":"synthetic-publication"}}))
''')
    runner.chmod(0o755)
    env = {**os.environ, "PATH": str(bindir), "YGG_BOOKING_OUTBOX": str(path),
           "TEST_HUB_LOCK": str(hublock), "TEST_LEDGER": str(ledger)}
    cmd = [sys.executable, str(Path(__file__).with_name("plan.py")), "--ledger", str(ledger),
           "--done", "T-901", "--proof", "proof-901", "--by", "test-worker"]
    with hublock.open("a") as held:
        fcntl.flock(held, fcntl.LOCK_EX)
        result = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=5)
        assert result.returncode != 0
        assert "RETAINED in the outbox" in result.stdout
        assert outbox.read_intents(path)[0]["state"] == "pending"
        assert "status: open" in ledger.read_text()
    result = subprocess.run([sys.executable, str(Path(__file__).with_name("booking_outbox.py")),
                             "--path", str(path), "replay", "--once"],
                            env=env, capture_output=True, text=True, timeout=5)
    assert result.returncode == 0, result.stderr
    row = outbox.read_intents(path)[0]
    assert row["state"] == "done" and row["receipt_ref"].startswith("ygg-memory:event:")
    assert "status: done(proof-901)" in ledger.read_text()
    # Simulate death after publish but before outbox ack: exact replay works.
    outbox.update_intent(path, row["intent_id"], state="pending", next_attempt_at=0)
    result = subprocess.run(cmd + ["--from-outbox"], env=env, capture_output=True, text=True, timeout=5)
    assert result.returncode == 0 and "BOOKING_RECEIPT" in result.stdout
    assert ledger.read_text().count("- 20") == 1


def test_zero_exit_without_positive_receipt_is_not_booked(tmp_path, monkeypatch):
    import types
    row = {"task": "T-901", "proof": "proof", "by": "worker"}
    monkeypatch.setattr(outbox.subprocess, "run", lambda *a, **kw: types.SimpleNamespace(returncode=0, stdout="finished\n"))
    import pytest
    with pytest.raises(RuntimeError, match="without a matching booking receipt"):
        outbox.publish_intent(row)


def test_append_during_publish_is_preserved(tmp_path):
    path = tmp_path / "intents.jsonl"
    outbox.write_intent("T-901", "proof", "worker", path=path, ts=1)
    def publish(row):
        outbox.write_intent("T-902", "other", "worker", path=path, ts=1)
        return "receipt:901"
    outbox.replay_once(path=path, publish=publish, clock=lambda: 2)
    assert [r["state"] for r in outbox.read_intents(path)] == ["done", "pending"]
