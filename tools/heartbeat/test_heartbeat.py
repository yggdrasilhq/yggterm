import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import time

import pytest

sys.path.insert(0, str(Path(__file__).parent))
import heartbeat as hb


class Clock:
    def __init__(self):
        self.now = 1_800_000_000
    def __call__(self):
        return self.now
    def advance(self, seconds):
        self.now += seconds


class Schedule:
    """Monotonic schedule clock for C7 scheduler tests."""
    def __init__(self):
        self.now = 0.0
    def __call__(self):
        return self.now
    def advance(self, seconds):
        self.now += seconds


def drain(c):
    """Drive the loop until retained reader futures complete (C7)."""
    for _ in range(2000):
        if not c._jobs:
            break
        c.tick(schedule=False)


def catalog(clock, remaining=None, end_delta=600, full_allowed=False):
    return {"glm53_full_allowed": full_allowed, "balance": "unavailable-from-cli",
            "plans": [{"plan_id": "zcode-v3-start-plan-synthetic", "status": "active",
                       "ends_at": clock() + end_delta,
                       "entitlements": [{"capabilities": ["model:glm-5.3-flash"],
                                         "grant_units": 300_000_000,
                                         "effective_at": clock() - 60,
                                         "remaining": remaining}]}]}


def setup(tmp_path, clock, reader, publisher=None):
    ledger = tmp_path / "ledger.md"
    ledger.write_text("### [T-901] fixture\n- size: small ; value: 1 ; est: x\n- status: open\n")
    config = {"planes": [{"id": "start-plan-grant", "reader": {"cmd": ["catalog"], "parse": None}, "host_class": "server"}],
              "ledger_path": str(ledger), "outbox_path": str(tmp_path / "outbox.jsonl"),
              "queue_reader": {"cmd": ["queue"]}, "watchers": []}
    posts = []
    def publish(event):
        posts.append(event.copy())
        return "ACK-fixture"
    c = hb.Controller(config, tmp_path / "health.json", clock=clock, reader=reader, publisher=publisher or publish)
    return c, posts


def test_new_flash_grant_found_in_one_urgent_pass_and_remaining_unknown(tmp_path):
    clock = Clock()
    data = {"plans": []}
    c, _ = setup(tmp_path, clock, lambda _: data)
    c.urgent()
    clock.advance(60)
    data = catalog(clock)
    c.urgent()
    observed = c.state["planes"]["start-plan-grant"]
    assert observed["observed_at"] == clock()
    assert observed["quality"] == "verified"
    assert observed["obs"]["quality"] == "verified-metadata"
    assert observed["obs"]["grants"][0]["remaining"] is None
    assert {f["kind"] for f in c.state["findings"] if f.get("active")} >= {"GRANT_NEW", "EXPIRY_AT_RISK"}
    assert next(f for f in c.state["findings"] if f["kind"] == "EXPIRY_AT_RISK")["detail"] == "remaining unreadable"


def test_expiring_balance_and_expired_or_changed_grants(tmp_path):
    clock = Clock()
    data = catalog(clock, remaining=123)
    c, _ = setup(tmp_path, clock, lambda _: data)
    c.urgent()
    assert any(f["kind"] == "EXPIRY_AT_RISK" for f in c.state["findings"])
    clock.advance(601)
    c.urgent()
    assert any(f["kind"] == "GRANT_EXPIRED" and f["active"] for f in c.state["findings"])
    data = catalog(clock, end_delta=1000)
    c.urgent()
    assert any(f["kind"] == "GRANT_NEW" and f["active"] for f in c.state["findings"])


def test_reader_failure_sanitized_unknown_and_recovers_without_crash(tmp_path):
    clock = Clock()
    sched = Schedule()
    def fail(_):
        raise RuntimeError("401 Authorization: Bearer super-secret https://private.test/token")
    c, _ = setup(tmp_path, clock, fail)
    c.schedule_clock = sched
    c.tick()
    drain(c)
    state = c.state["planes"]["start-plan-grant"]
    assert state["quality"] == "unavailable" and state["obs"] is None
    assert "authentication" in state["error"]
    assert "super-secret" not in c.path.read_text() and "private.test" not in c.path.read_text()
    assert any(f["kind"] == "AUTH_BLOCKED" for f in c.state["findings"])
    c.reader = lambda _: catalog(clock)
    clock.advance(60)
    sched.advance(60)
    c.tick()
    drain(c)
    assert c.state["planes"]["start-plan-grant"]["quality"] == "verified"


def test_hysteresis_once_alarm_recovery_and_restart(tmp_path):
    clock = Clock()
    c, posts = setup(tmp_path, clock, lambda _: {})
    obs = [c.finding("RUNNER_BROKEN", "watcher", "missing")]
    for _ in range(2):
        c.reconcile_findings("cheap", obs)
        c.flush()
        c.deliver()
    assert posts == [] and not c.state["findings"][0]["alarmed"]
    c.reconcile_findings("cheap", obs)
    c.deliver()
    assert len(posts) == 1 and posts[0]["transition"] == "ALARM"
    c.reconcile_findings("cheap", obs)
    c.deliver()
    assert len(posts) == 1
    restarted = hb.Controller(c.config, c.path, clock=clock, reader=c.reader,
                              publisher=lambda e: posts.append(e) or "ACK-recovery")
    for _ in range(2):
        restarted.reconcile_findings("cheap", [])
        restarted.deliver()
    assert len(posts) == 1
    restarted.reconcile_findings("cheap", [])
    restarted.deliver()
    assert len(posts) == 2 and posts[-1]["transition"] == "RECOVERY"
    assert not restarted.state["findings"][0]["alarmed"]


def test_failed_alarm_keyed_sink_retry_and_duplicate_rejection(tmp_path, monkeypatch):
    # C6: idempotency lives at the sink. First attempt fails BEFORE any
    # append; the retry posts the same retained transition id; the sink
    # rejects a duplicate key with a conflicting payload; an old sink that
    # rejects the flag holds the alarm (never an unkeyed fallback).
    clock = Clock()
    appended = []
    def sink(cmd, capture_output=True, text=True, timeout=None):
        argv = list(cmd)
        if "--idempotency-key" not in argv:
            raise SystemExit("unknown argument")  # pre-C6 sink
        key = argv[argv.index("--idempotency-key") + 1]
        body = argv[argv.index("--body") + 1]
        for row in appended:
            if row["key"] == key:
                if row["body"] != body:
                    raise SystemExit("idempotency payload conflict")
                return subprocess.CompletedProcess(argv, 0, stdout="posted ACK-retained (retained)\n", stderr="")
        appended.append({"key": key, "body": body})
        return subprocess.CompletedProcess(argv, 0, stdout="posted ACK-new to infra/meta\n", stderr="")
    monkeypatch.setattr(hb.subprocess, "run", sink)
    c, _ = setup(tmp_path, clock, lambda _: {})
    c.publisher = hb.BoardPublisher({})
    obs = [c.finding("RUNNER_BROKEN", "watcher", "missing")]
    for _ in range(3):
        c.reconcile_findings("cheap", obs)
    c.deliver()
    event = c.state["notifications"][0]
    assert event["receipt_ref"] == "ACK-new" and len(appended) == 1
    # A retry of the same transition (crash between append and ACK) is safe.
    event["receipt_ref"] = None
    c.deliver()
    assert event["receipt_ref"] == "ACK-retained" and len(appended) == 1
    # An old sink that rejects the flag holds: no unkeyed fallback post.
    def old_sink(cmd, **k):
        if "--idempotency-key" in list(cmd):
            return subprocess.CompletedProcess(cmd, 2, stdout="", stderr="unknown option")
        raise AssertionError("unkeyed fallback attempted")
    monkeypatch.setattr(hb.subprocess, "run", old_sink)
    event["receipt_ref"] = None
    clock.advance(60)
    c.deliver()
    assert event["receipt_ref"] is None and event["attempts"] >= 1


def test_atomic_health_survives_crash_before_replace(tmp_path, monkeypatch):
    path = tmp_path / "health.json"
    hb.atomic_json(path, {"as_of": 1, "remaining": None})
    def crash(*a, **k):
        raise OSError("simulated crash")
    monkeypatch.setattr(hb.os, "replace", crash)
    with pytest.raises(OSError):
        hb.atomic_json(path, {"as_of": 2})
    assert json.loads(path.read_text()) == {"as_of": 1, "remaining": None}


def test_scripted_json_reader_parse_failure_and_timeout(tmp_path):
    script = tmp_path / "read.py"
    script.write_text('import json; print(json.dumps({"quota":{"remaining":None}}))')
    reader = {"cmd": [sys.executable, str(script)], "timeout_s": 1, "parse": "quota"}
    assert hb.run_json(reader) == {"remaining": None}
    script.write_text("import time; time.sleep(10)")
    with pytest.raises(subprocess.TimeoutExpired):
        hb.run_json({**reader, "timeout_s": 0.1})


def test_cadences_do_not_count_cached_urgent_observations(tmp_path):
    clock = Clock()
    sched = Schedule()
    calls = []
    def reader(r):
        calls.append(r["cmd"][0])
        if r["cmd"] == ["queue"]:
            return {"serve": {"alive": True}, "routes": {"fixture": {"counts": {"queued": 1}}}}
        return catalog(clock)
    c, posts = setup(tmp_path, clock, reader)
    c.schedule_clock = sched
    c.tick()
    drain(c)
    first_count = next(f["count"] for f in c.state["findings"] if f["kind"] == "EXPIRY_AT_RISK")
    sched.advance(15)
    c.tick()
    drain(c)
    # Urgent future still pending: no reader call, cached finding not recounted.
    assert calls.count("catalog") == 1 and calls.count("queue") == 1
    assert next(f["count"] for f in c.state["findings"] if f["kind"] == "EXPIRY_AT_RISK") == first_count
    sched.advance(45)
    c.tick()
    drain(c)
    assert calls.count("catalog") == 2 and calls.count("queue") == 1
    sched.advance(240)
    c.tick()
    drain(c)
    assert calls.count("queue") == 2
    # Slow reader never delays the cheap cadence (C7/R5): with a blocking
    # urgent future outstanding, cheap passes still fire on schedule.
    class Slow:
        def __call__(self, r):
            time.sleep(0.3)
            return {"serve": {"alive": True}, "routes": {}}
    (tmp_path / "s2").mkdir()
    c2, _ = setup(tmp_path / "s2", clock, Slow())
    c2.schedule_clock = sched
    starts = []
    real_cheap = c2.cheap
    def timed_cheap():
        starts.append(sched())
        real_cheap()
    c2.cheap = timed_cheap
    c2.tick()               # starts cheap + slow urgent future
    for _ in range(4):
        sched.advance(15)
        c2.tick()
    gaps = [b - a for a, b in zip(starts, starts[1:])]
    assert all(g <= 15 + 0.5 for g in gaps), gaps
    c2.close()


def test_failed_pass_observes_in_failure_domain_and_never_recounts_cached(tmp_path):
    # C7/R10: a crashed pass lands in its own failure domain; a reader-level
    # failure recounts the plane's real QUOTA_UNKNOWN but NEVER counts or
    # clear-credits the cached expiry finding.
    clock = Clock()
    sched = Schedule()
    state = {"fail": False}
    def reader(_):
        if state["fail"]:
            raise RuntimeError("reader down")
        return catalog(clock)
    c, posts = setup(tmp_path, clock, reader)
    c.schedule_clock = sched
    c.tick(); drain(c)
    risk = next(f for f in c.state["findings"] if f["kind"] == "EXPIRY_AT_RISK")
    baseline = risk["count"]
    state["fail"] = True
    for _ in range(3):
        sched.advance(60)
        c.tick(); drain(c)
    row = next(f for f in c.state["findings"] if f["kind"] == "EXPIRY_AT_RISK")
    assert row["count"] == baseline and row["clear_count"] == 0 and row["active"]
    assert not any(f["id"] == "RUNNER_BROKEN:urgent-pass" for f in c.state["findings"])
    # A crashed pass (not a reader-level failure) lands in urgent-failure.
    c.config["planes"] = None
    for _ in range(3):
        sched.advance(60)
        c.tick(); drain(c)
    fail_row = next(f for f in c.state["findings"] if f["id"] == "RUNNER_BROKEN:urgent-pass")
    assert fail_row["count"] >= 3 and fail_row["alarmed"]
    assert any(p["kind"] == "RUNNER_BROKEN" and p["transition"] == "ALARM" for p in posts)


def test_local_watchers_bookkeeping_and_full_reconciliation(tmp_path):
    clock = Clock()
    c, _ = setup(tmp_path, clock, lambda _: {"serve": {"alive": False}, "routes": {}})
    c.config["watchers"] = [{"id": "fixture", "as_of": clock() - 100, "pid": 99, "boot_id": "test"}]
    c.pid_alive = lambda pid, boot: False
    Path(c.config["outbox_path"]).write_text(json.dumps({"state": "pending"}) + "\n")
    c.cheap()
    assert c.state["booking_pending"] == 1
    assert {f["kind"] for f in c.state["findings"]} >= {"RUNNER_BROKEN", "BOOKKEEPING_BACKLOG"}
    Path(c.config["ledger_path"]).write_text(Path(c.config["ledger_path"]).read_text() +
        "### [T-902] claimed fixture\n- status: claimed(worker, today)\nmonitor-conflict\n")
    c.full()
    assert {f["kind"] for f in c.state["findings"]} >= {"FEED_STARVED", "RUNNER_BROKEN"}


def test_scope_conflict_and_free_route_never_manufacture_quota(tmp_path):
    clock = Clock()
    raw = catalog(clock)
    other = copy.deepcopy(raw["plans"][0])
    other["entitlements"][0]["grant_units"] = 12
    raw["plans"].append(other)
    obs, quality = hb.normalize("start-plan-grant", raw)
    assert quality == "conflicting"
    free, quality = hb.normalize("free-tier", {"available": True})
    assert quality == "unavailable" and free["remaining"] is None
    c, _ = setup(tmp_path, clock, lambda _: raw)
    c.urgent()
    assert any(f["kind"] == "QUOTA_SCOPE_CONFLICT" for f in c.state["findings"])


def test_sigterm_flushes_service_state_with_only_fixture_readers(tmp_path):
    clock = Clock()
    c, _ = setup(tmp_path, clock, lambda _: {})
    catalog_script = tmp_path / "catalog.py"
    catalog_script.write_text("import json; print(" + repr(json.dumps(catalog(clock))) + ")")
    queue_script = tmp_path / "queue.py"
    queue_script.write_text('print(\'{"serve":{"alive":true},"routes":{"fixture":{"counts":{"queued":1}}}}\')')
    config = c.config
    config["planes"][0]["reader"] = {"cmd": [sys.executable, str(catalog_script)], "timeout_s": 1}
    config["queue_reader"] = {"cmd": [sys.executable, str(queue_script)], "timeout_s": 1}
    config_path = tmp_path / "planes.json"
    config_path.write_text(json.dumps(config))
    proc = subprocess.Popen([sys.executable, str(Path(hb.__file__)), "--config", str(config_path), "--state", str(c.path)],
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        deadline = time.monotonic() + 5
        while not c.path.exists() and time.monotonic() < deadline:
            time.sleep(0.01)
        assert c.path.exists()
        proc.terminate()
        _, stderr = proc.communicate(timeout=5)
        assert proc.returncode == 0, stderr
        state = json.loads(c.path.read_text())
        assert state["as_of"] is not None and state["loop"]["last_cheap"] is not None
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()


def test_c8_structured_auth_and_incomplete_windows(tmp_path):
    # Structured wallet auth refusal -> AUTH_BLOCKED, not generic unknown.
    clock = Clock()
    sched = Schedule()
    c, _ = setup(tmp_path, clock, lambda _: {})
    c.schedule_clock = sched
    c.config["planes"] = [
        {"id": "autoclaw-wallet", "reader": {"cmd": ["wallet"]}, "host_class": "server"},
        {"id": "coding-plan", "reader": {"cmd": ["usage"]}, "host_class": "server"}]
    data = {"wallet": {"kind": "access_credential_rejected", "code": 410000},
            "usage": {"windows": [{}]}}
    c.reader = lambda r: data[r["cmd"][0]]
    c.tick(); drain(c)
    assert any(f["kind"] == "AUTH_BLOCKED" and f["id"].endswith("autoclaw-wallet")
               for f in c.state["findings"])
    assert c.state["planes"]["autoclaw-wallet"]["quality"] == "unavailable"
    # Missing numeric counters -> QUOTA_UNKNOWN, never verified, no invented zeros.
    coding = c.state["planes"]["coding-plan"]
    assert coding["quality"] == "unavailable" and coding["obs"] is None
    assert any(f["kind"] == "QUOTA_UNKNOWN" and f["id"].endswith("coding-plan")
               for f in c.state["findings"])
    # Complete windows still verify.
    c.reader = lambda r: {"windows": [{"window": "5h", "remaining": 10, "usage": 1,
                                       "number": 300, "nextResetTime": clock()}]}
    clock.advance(60); sched.advance(60); c.tick(); drain(c)
    assert c.state["planes"]["coding-plan"]["quality"] == "verified"


def test_c9_start_plan_aliases_recognized(tmp_path):
    clock = Clock()
    base = catalog(clock)
    for mutate in (
        lambda p: p.update(plan_id="zcode-v3-START_PLAN-1028"),
        lambda p: p.update(plan_id="zcode-v3-start plan-1028"),
        lambda p: p.update(plan_id="grant-x", name="Dev Day bonus"),
    ):
        raw = {"plans": [dict(base["plans"][0])]}
        mutate(raw["plans"][0])
        obs, quality = hb.normalize("start-plan-grant", raw)
        assert len(obs["grants"]) == 1 and obs["quality"] == "verified-metadata" and quality == "verified"
    # Unrelated plan -> no grant; malformed plan row -> unavailable (holds).
    obs, quality = hb.normalize("start-plan-grant", {"plans": [dict(base["plans"][0], plan_id="coding-plus")]})
    assert obs["grants"] == []
    with pytest.raises(ValueError):
        hb.normalize("start-plan-grant", {"plans": ["garbage"]})
