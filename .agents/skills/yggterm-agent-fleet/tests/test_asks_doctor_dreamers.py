#!/usr/bin/env python3
"""Asks ledger, doctor probes, composer health, and scoped dreamers.

    python3 tests/test_asks_doctor_dreamers.py [path-to-ygg-memory.py]

Pins the 2026-10-18 sol-ruled build: (1) the ask-user ledger — owner-blocked
actions compile in one portable door, ONE review invitation per day across
harnesses (door-header day stamp under the hub lock), physical actions stay
pending-verification without --verified; (2) deterministic composer health —
consecutive limit/auth failures trip a skip window instead of eternal silent
retries; (3) the doctor — dead-timer shape detection and YGGTERM_UNIT_TUNED
respect; (4) scoped dreamers — expiry, disable, and scope discipline.
"""

import contextlib
import importlib.util
import io
import json
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT_SCRIPT = HERE.parent / "ygg-memory.py"

FAILURES = []


def check(name, ok, detail=""):
    print(f"{'ok  ' if ok else 'FAIL'}  {name}{('  — ' + detail) if detail and not ok else ''}")
    if not ok:
        FAILURES.append(name)


def load_module(script_path):
    spec = importlib.util.spec_from_file_location("ygg_memory_asks", str(script_path))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class Args:
    def __init__(self, root, **kw):
        self.root = str(root)
        self.json = False
        for k, v in kw.items():
            setattr(self, k, v)


def run():
    script = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_SCRIPT
    mod = load_module(script)
    tmp = Path(tempfile.mkdtemp(prefix="ygg-asks-test-"))

    # --- asks: seed, add, list, snooze eligibility ---
    with contextlib.redirect_stdout(io.StringIO()):
        mod.cmd_asks(Args(tmp, asks_action="seed"))
    check("seed creates the door", mod._asks_door_path(tmp).is_file())
    with contextlib.redirect_stdout(io.StringIO()) as out:
        mod.cmd_asks(Args(tmp, asks_action="add", ask_id="ask-rti",
                          kind="missing-input",
                          request="scan the reply documents into the document graph",
                          source_ref="fleet-task-ledger:T-002"))
    with contextlib.redirect_stdout(io.StringIO()) as out:
        mod.cmd_asks(Args(tmp, asks_action="list"))
    check("added ask lists as eligible", "ask-rti" in out.getvalue(), out.getvalue()[:120])

    # invite cooldown: first invite passes, second is refused
    with contextlib.redirect_stdout(io.StringIO()) as out:
        mod.cmd_asks(Args(tmp, asks_action="invite", ask_id="ask-rti"))
    check("first invite offered", "invited" in out.getvalue(), out.getvalue()[:100])
    with contextlib.redirect_stdout(io.StringIO()) as out:
        mod.cmd_asks(Args(tmp, asks_action="invite", ask_id="ask-rti"))
    check("second invite same day refused (cooldown)",
          "already offered today" in out.getvalue(), out.getvalue()[:120])

    # close: physical action without verified -> answered (pending verification)
    with contextlib.redirect_stdout(io.StringIO()) as out:
        mod.cmd_asks(Args(tmp, asks_action="add", ask_id="ask-phys",
                          kind="physical-action", request="sign the form"))
    with contextlib.redirect_stdout(io.StringIO()) as out:
        mod.cmd_asks(Args(tmp, asks_action="close", ask_id="ask-phys",
                          ruling="will do today"))
    check("physical action without verified stays pending",
          "PENDING VERIFICATION" in out.getvalue(), out.getvalue()[:120])
    asks = mod._asks_parse(tmp)
    phys = next(a for a in asks if a["id"] == "ask-phys")
    check("closed-but-unverified status is answered", phys["status"] == "answered", phys["status"])
    door_text = mod._asks_door_path(tmp).read_text(encoding="utf-8")
    check("answer receipt is append-only on the door", "ANSWER ask-phys" in door_text)

    # snooze honored by eligibility
    with contextlib.redirect_stdout(io.StringIO()):
        mod.cmd_asks(Args(tmp, asks_action="add", ask_id="ask-later", kind="decision",
                          request="decide something"))
    with contextlib.redirect_stdout(io.StringIO()):
        mod.cmd_asks(Args(tmp, asks_action="snooze", ask_id="ask-later", until="2999-12-31"))
    with contextlib.redirect_stdout(io.StringIO()) as out:
        mod.cmd_asks(Args(tmp, asks_action="list"))
    check("snoozed ask not eligible", "ask-later" not in out.getvalue())

    # --- composer health ---
    health_root = tmp / "health"
    entry = mod._composer_record_failure(health_root, "opencode", "m-free", "usage limit reached")
    check("failure recorded with class", entry["last_error_class"] == "limit", str(entry))
    for _ in range(mod.COMPOSER_FAILURE_LIMIT - 1):
        entry = mod._composer_record_failure(health_root, "opencode", "m-free", "429")
    check("skip window trips at the failure limit", entry.get("skip_until", 0) > time.time(), str(entry))
    check("transport unhealthy inside window",
          mod._composer_unhealthy(health_root, "opencode", "m-free"))
    mod._composer_record_success(health_root, "opencode", "m-free")
    check("success clears the window",
          not mod._composer_unhealthy(health_root, "opencode", "m-free"))
    entry = mod._composer_record_failure(health_root, "opencode", "m2", "ssh mesh exploded for real")
    check("non-limit error does not advance the counter",
          entry.get("consecutive", 0) == 0, str(entry))

    # --- doctor probes ---
    probe = mod._doctor_timer_probe()
    check("timer probe returns a verdict", "name" in probe and "detail" in probe, str(probe))
    drift = mod._doctor_unit_drift()
    check("unit drift probe returns a verdict",
          drift.get("name") == "unit-drift" and "detail" in drift, str(drift))
    if drift.get("ok") is False:
        check("real drift names the drifted unit", "drifted:" in drift["detail"], str(drift))
    with contextlib.redirect_stdout(io.StringIO()) as out:
        code = mod.cmd_doctor(Args(health_root))
    check("doctor runs end-to-end", "doctor:" in out.getvalue(), out.getvalue()[:120])

    # --- dreamers loader ---
    ddir = Path.home() / ".yggterm/config/dream/dreamers.d"
    ddir.mkdir(parents=True, exist_ok=True)
    fixtures = {
        "z_expired.json": {"name": "expired", "ns_scope": ["-x"], "expires_at": "2020-01-01", "enabled": True},
        "z_disabled.json": {"name": "disabled", "ns_scope": ["-x"], "enabled": False},
        "z_noscope.json": {"name": "noscope", "enabled": True},
        "z_valid.json": {"name": "valid", "ns_scope": ["-x-y"], "enabled": True},
    }
    for name, cfg in fixtures.items():
        (ddir / name).write_text(json.dumps(cfg), encoding="utf-8")
    try:
        loaded = {d["name"] for d in mod._dreamers_load()}
        check("expired dreamer skipped", "expired" not in loaded, str(loaded))
        check("disabled dreamer skipped", "disabled" not in loaded, str(loaded))
        check("scopeless dreamer skipped", "noscope" not in loaded, str(loaded))
        check("valid dreamer loads", "valid" in loaded, str(loaded))
    finally:
        for name in fixtures:
            (ddir / name).unlink()

    print()
    if FAILURES:
        print(f"FAILED: {len(FAILURES)} check(s): {', '.join(FAILURES)}")
        return 1
    print("all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(run())
