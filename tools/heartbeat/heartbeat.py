#!/usr/bin/env python3
"""Observation-only heartbeat. No allocation, execution, repair or LLM calls.

Findings are evidence, never launch authority. Unknown capacity stays null.
Only the canonical board publisher may append alarms; failed/uncertain posts
remain durable and are reconciled by their transition id before retry.
"""
import argparse
import concurrent.futures
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shlex
import signal
import socket
import subprocess
import sys
import tempfile
import time
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "task-ledger"))
from plan import parse_ledger
from booking_outbox import read_intents

CHEAP_S, URGENT_S, FULL_S = 15, 60, 300
PRODUCTS = {"start-plan-grant", "coding-plan", "autoclaw-wallet", "idle-allowance", "free-tier"}


def atomic_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix=".health-", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as f:
            json.dump(value, f, sort_keys=True, allow_nan=False)
            f.write("\n")
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, path)
        fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    finally:
        if os.path.exists(tmp):
            os.unlink(tmp)


def number(value):
    return value if isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) else None


def epoch(value):
    n = number(value)
    if n is not None:
        return n / 1000 if n > 100_000_000_000 else n
    if isinstance(value, str):
        from datetime import datetime
        try:
            return datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp()
        except ValueError:
            pass
    return None


def safe_error(error):
    """Diagnostics use a closed vocabulary, never raw provider text/headers."""
    text = str(error).lower()
    if any(x in text for x in ("401", "403", "unauthorized", "forbidden", "credential", "auth")):
        reason = "authentication rejected or unavailable"
    elif "timeout" in text or isinstance(error, subprocess.TimeoutExpired):
        reason = "reader timeout"
    elif isinstance(error, (json.JSONDecodeError, ValueError, KeyError, TypeError)):
        reason = "invalid or missing reader data"
    else:
        reason = "reader unavailable"
    return f"{type(error).__name__}: {reason}"


def run_json(reader):
    cmd = reader["cmd"]
    if not isinstance(cmd, list) or not cmd or not all(isinstance(x, str) for x in cmd):
        raise ValueError("reader cmd must be argv")
    host = reader.get("host")
    local = socket.gethostname().split(".")[0]
    if host and host not in ("local", "localhost", local, socket.gethostname()):
        if not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9._-]*", host):
            raise ValueError("invalid reader host")
        cmd = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", host,
               "exec " + shlex.join(cmd)]
    else:
        cmd = [os.path.expanduser(x) for x in cmd]
    timeout = min(10, max(0.1, float(reader.get("timeout_s", 10))))
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            text=True, start_new_session=True)
    try:
        stdout, _ = proc.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(proc.pid, signal.SIGKILL)
        proc.communicate()
        raise
    if proc.returncode:
        # Never store stderr: it can include authentication material.
        raise RuntimeError(f"reader exit {proc.returncode}")
    if len(stdout) > 1_048_576:
        raise ValueError("reader output too large")
    value = json.loads(stdout)
    path = reader.get("parse")
    for part in path.split(".") if path else []:
        value = value[int(part)] if isinstance(value, list) else value[part]
    return value


def normalize(plane_id, raw):
    if raw is None:
        raise ValueError("null observation")
    if isinstance(raw, dict) and raw.get("error"):
        raise RuntimeError(str(raw["error"]))
    if plane_id == "start-plan-grant":
        if not isinstance(raw, dict) or not isinstance(raw.get("plans"), list):
            raise ValueError("missing plans")
        if str(raw.get("note", "")).startswith("billing/current failed"):
            raise RuntimeError(raw["note"])
        grants, seen, conflicting = [], {}, False
        for plan in raw["plans"]:
            if "start-plan" not in str(plan.get("plan_id", "")):
                continue
            for ent in plan.get("entitlements", []):
                grant = {"id": str(plan["plan_id"]) + ":" + ",".join(ent.get("capabilities", [])),
                         "grant_units": number(ent.get("grant_units")),
                         "effective_at": epoch(ent.get("effective_at", plan.get("starts_at"))),
                         "ends_at": epoch(plan.get("ends_at")),
                         "remaining": number(ent.get("remaining")),
                         "status": plan.get("status")}
                # A model-specific grant is independent of full-model eligibility.
                if grant["grant_units"] is None or grant["effective_at"] is None or grant["ends_at"] is None:
                    raise ValueError("incomplete grant metadata")
                if grant["id"] in seen and seen[grant["id"]] != grant:
                    conflicting = True
                seen[grant["id"]] = grant
                if grant not in grants:
                    grants.append(grant)
        metadata_only = any(g["remaining"] is None for g in grants)
        return {"grants": grants, "remaining": None,
                "quality": "verified-metadata" if metadata_only else "verified"}, "conflicting" if conflicting else "verified"
    if plane_id == "coding-plan":
        if not isinstance(raw, dict) or not isinstance(raw.get("windows"), list) or not raw["windows"]:
            raise ValueError("missing quota windows")
        windows = [{key: (number(w.get(key)) if key != "window" else str(w.get(key, "unknown")))
                    for key in ("window", "remaining", "usage", "number", "nextResetTime")}
                   for w in raw["windows"]]
        return {"windows": windows}, "conflicting" if raw.get("scope_conflict") else "verified"
    if plane_id == "autoclaw-wallet":
        if not isinstance(raw, dict) or number(raw.get("totalPoints")) is None:
            raise ValueError("wallet unavailable")
        wallets = [{"scope": str(w.get("scope", "unknown")), "balance": number(w.get("balance"))}
                   for w in raw.get("wallets", [])]
        return {"totalPoints": number(raw["totalPoints"]), "wallets": wallets,
                "ends_at": None}, "conflicting" if raw.get("scope_conflict") else "verified"
    if plane_id == "idle-allowance":
        if not isinstance(raw, dict) or not isinstance(raw.get("canCreateNow"), bool):
            raise ValueError("missing idle availability")
        return {"canCreateNow": raw["canCreateNow"], "resetsAt": raw.get("resetsAt")}, "verified"
    if plane_id == "free-tier":
        # Queue route metadata is useful health, never evidence of free quota.
        if not isinstance(raw, dict):
            raise ValueError("missing free route metadata")
        return {"route_available": raw.get("available") if isinstance(raw.get("available"), bool) else None,
                "remaining": None, "quality": "verified-metadata",
                "error": "authoritative free-tier quota reader unavailable"}, "unavailable"
    raise ValueError("unregistered funding product")


class BoardPublisher:
    """Canonical msgboard transport with durable transition-id readback.

    A timeout is uncertain: the next attempt first checks the board. No
    readback means hold, not a second blind append. The controller is the
    single writer (service flock), so a successful transition posts once.
    """
    def __init__(self, config):
        self.config = config

    def __call__(self, event):
        board = self.config.get("alarm_board", "infra/meta")
        readback = Path(self.config.get("board_readback", str(
            Path.home() / "data/msggraph/boards" / board / "board.jsonl"))).expanduser()
        marker = f"heartbeat-transition:{event['id']}"
        # Reading is allowed; all mutation goes through the canonical verb.
        for line in readback.read_text().splitlines():
            row = json.loads(line)
            if marker in str(row.get("body", "")) and row.get("ack_token"):
                return row["ack_token"]
        if event.get("uncertain"):
            raise RuntimeError("uncertain publication requires reconciliation")
        # Fixed vocabulary only: no source content or reader diagnostics egress.
        body = f"{marker} {event['transition']} {event['kind']}"
        cmd = self.config.get("board_cmd", ["msgboard"]) + ["post", board,
               "--kind", "note", "--from-row", event["controller_id"],
               "--harness", "heartbeat", "--ttl-days", "14", "--body", body]
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=2)
        if result.returncode:
            raise RuntimeError("board publication failed")
        match = re.search(r"\bACK-[a-zA-Z0-9]+\b", result.stdout)
        if not match:
            raise RuntimeError("board publication has no receipt")
        return match.group(0)


class Controller:
    def __init__(self, config, state_path, *, clock=time.time, reader=run_json, publisher=None,
                 pid_alive=None):
        self.config, self.path, self.clock, self.reader = config, Path(state_path), clock, reader
        self.publisher = publisher or BoardPublisher(config)
        self.pid_alive = pid_alive or self._pid_alive
        if self.path.exists():
            self.state = json.loads(self.path.read_text())  # corrupt state holds at startup
        else:
            self.state = {"as_of": None, "controller_id": str(uuid.uuid4()), "planes": {},
                          "findings": [], "notifications": [],
                          "loop": {"last_cheap": None, "last_urgent": None, "last_full": None}}

    @staticmethod
    def _pid_alive(pid, boot_id):
        if not isinstance(pid, int) or isinstance(pid, bool) or pid <= 0 or not boot_id:
            return None
        try:
            boot = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
            if boot != boot_id:
                return None
            Path(f"/proc/{pid}").stat()
            return True
        except FileNotFoundError:
            return False
        except OSError:
            return None

    def flush(self):
        self.state["as_of"] = self.clock()
        atomic_json(self.path, self.state)

    def finding(self, kind, scope, detail, **extra):
        return {"id": f"{kind}:{scope}", "kind": kind, "detail": detail, **extra}

    def reconcile_findings(self, domain, observations):
        now = self.clock()
        current = {x["id"]: x for x in observations}
        rows = {x["id"]: x for x in self.state["findings"] if x["domain"] == domain}
        others = [x for x in self.state["findings"] if x["domain"] != domain]
        for key, obs in current.items():
            row = rows.setdefault(key, {**obs, "domain": domain, "first_seen": now,
                                       "count": 0, "clear_count": 0, "alarmed": False})
            row.update(detail=obs["detail"], last_seen=now, clear_count=0, active=True)
            row["count"] += 1
            if row["count"] >= 3 and not row["alarmed"] and obs.get("severity") != "info":
                row["alarmed"] = True
                self.transition(row, "ALARM")
        for key, row in rows.items():
            if key not in current:
                row["count"] = 0
                row["clear_count"] += 1
                row["active"] = False
                if row["clear_count"] >= 3 and row["alarmed"]:
                    row["alarmed"] = False
                    self.transition(row, "RECOVERY")
        self.state["findings"] = others + list(rows.values())

    def transition(self, finding, transition):
        seq = len(self.state["notifications"]) + 1
        controller = self.state["controller_id"]
        event_id = hashlib.sha256(f"{controller}:{seq}:{finding['id']}:{transition}".encode()).hexdigest()
        self.state["notifications"].append({"id": event_id, "controller_id": controller,
                "kind": finding["kind"], "transition": transition, "ts": self.clock(),
                "receipt_ref": None, "attempts": 0, "next_attempt_at": self.clock()})

    def deliver(self):
        started = time.monotonic()
        for event in self.state["notifications"]:
            if event["receipt_ref"] or event["next_attempt_at"] > self.clock():
                continue
            if time.monotonic() - started >= 2:
                break
            # Persist intent and uncertainty before the append can happen.
            uncertain = event.get("uncertain", False)
            event["uncertain"] = True
            self.flush()
            try:
                receipt = self.publisher({**event, "uncertain": uncertain})
                if not isinstance(receipt, str) or not receipt:
                    raise RuntimeError("missing alarm receipt")
                event["receipt_ref"] = receipt
                event["last_error"] = None
            except Exception as e:
                event["attempts"] += 1
                event["last_error"] = safe_error(e)
                event["next_attempt_at"] = self.clock() + 60
                print(f"HEARTBEAT_ALARM_PENDING {event['kind']} {event['id']}", file=sys.stderr)
            self.flush()

    def cheap(self):
        now, findings = self.clock(), []
        watchers = self.config.get("watchers", [])
        if not watchers:
            findings.append(self.finding("RUNNER_BROKEN", "observer-coverage", "no watchers registered; independent observer coverage unknown"))
        self.state["watchers"] = {}
        for watcher in watchers:
            wid = str(watcher["id"])
            try:
                health = json.loads(Path(watcher["health_path"]).expanduser().read_text()) if watcher.get("health_path") else watcher
                stamp = epoch(health.get("as_of"))
                if stamp is None or not 0 <= now - stamp <= watcher.get("max_age_s", 45):
                    findings.append(self.finding("RUNNER_BROKEN", wid + ":age", "watcher heartbeat missing or stale"))
                alive = self.pid_alive(health.get("pid"), health.get("boot_id"))
                fresh = stamp is not None and 0 <= now - stamp <= watcher.get("max_age_s", 45)
                self.state["watchers"][wid] = {"alive": alive, "fresh": fresh, "task": watcher.get("task")}
                if alive is not True:
                    findings.append(self.finding("RUNNER_BROKEN", wid + ":pid", "watcher liveness unverified"))
            except Exception:
                findings.append(self.finding("RUNNER_BROKEN", wid, "watcher health unavailable"))
        outbox = Path(self.config.get("outbox_path", str(Path.home() / ".yggterm/swarm/booking-outbox/intents.jsonl"))).expanduser()
        try:
            pending = [r for r in read_intents(outbox) if r.get("state") != "done"]
            self.state["booking_pending"] = len(pending)
            if pending:
                findings.append(self.finding("BOOKKEEPING_BACKLOG", "outbox", f"{len(pending)} retained booking intents"))
        except Exception:
            self.state["booking_pending"] = None
            findings.append(self.finding("BOOKKEEPING_BACKLOG", "outbox", "booking outbox unreadable"))
        self.reconcile_findings("cheap", findings)

    def urgent(self):
        # Deduplicate shared usage command; independent parses retain scopes.
        readers = {}
        for plane in self.config["planes"]:
            base = {k: v for k, v in plane["reader"].items() if k != "parse"}
            readers[json.dumps(base, sort_keys=True)] = base
        results = {}
        def read(key, value):
            try:
                return key, self.reader(value), None
            except Exception as e:
                return key, None, e
        with concurrent.futures.ThreadPoolExecutor(max_workers=5) as pool:
            futures = [pool.submit(read, k, v) for k, v in readers.items()]
            for future in concurrent.futures.as_completed(futures):
                key, raw, error = future.result()
                results[key] = (raw, error)
        findings, now = [], self.clock()
        for plane in self.config["planes"]:
            pid = plane["id"]
            base = {k: v for k, v in plane["reader"].items() if k != "parse"}
            raw, error = results[json.dumps(base, sort_keys=True)]
            prior = self.state["planes"].get(pid, {})
            try:
                if error:
                    raise error
                for part in (plane["reader"].get("parse") or "").split("."):
                    if part:
                        raw = raw[int(part)] if isinstance(raw, list) else raw[part]
                obs, quality = normalize(pid, raw)
                row = {"obs": obs, "observed_at": now, "quality": quality,
                       "error": obs.get("error"), "grant_events": prior.get("grant_events", [])}
            except Exception as e:
                row = {"obs": None, "observed_at": now, "quality": "unavailable",
                       "error": safe_error(e), "last_verified_obs": prior.get("obs") if prior.get("quality") == "verified" else prior.get("last_verified_obs"),
                       "grant_events": prior.get("grant_events", [])}
            self.state["planes"][pid] = row
            if row["quality"] == "unavailable":
                findings.append(self.finding("QUOTA_UNKNOWN", pid, row["error"]))
                if "authentication" in (row["error"] or ""):
                    findings.append(self.finding("AUTH_BLOCKED", pid, "funding reader authentication unavailable"))
            elif row["quality"] == "conflicting":
                findings.append(self.finding("QUOTA_SCOPE_CONFLICT", pid, "funding observations conflict; availability held"))
            if pid == "start-plan-grant" and row["quality"] == "verified":
                self.grant_findings(prior, row, findings, now)
        for missing in PRODUCTS - {p["id"] for p in self.config["planes"]}:
            self.state["planes"][missing] = {"obs": None, "observed_at": now,
                                           "quality": "unavailable", "error": "funding reader not registered"}
            findings.append(self.finding("QUOTA_UNKNOWN", missing, "funding reader not registered"))
        self.reconcile_findings("urgent", findings)

    def grant_findings(self, prior, row, findings, now):
        grants = row["obs"]["grants"]
        previous = (prior.get("obs") or prior.get("last_verified_obs") or {}).get("grants", [])
        old = {g["id"]: g for g in previous}
        events = [e for e in row["grant_events"] if now - e["at"] <= 180]
        for grant in grants:
            signature = {k: v for k, v in grant.items() if k != "remaining"}
            previous_signature = {k: v for k, v in old.get(grant["id"], {}).items() if k != "remaining"}
            if signature != previous_signature:
                events.append({"at": now, "kind": "GRANT_NEW", "scope": grant["id"], "detail": "new or changed grant metadata observed; admission remains gated"})
            ends, effective = grant["ends_at"], grant["effective_at"]
            if ends <= now:
                findings.append(self.finding("GRANT_EXPIRED", grant["id"], "grant expiry observed; availability held"))
            elif effective <= now and ends - now <= self.config.get("expiry_risk_s", 12 * 3600) and (grant["remaining"] is None or grant["remaining"] > 0):
                detail = "remaining unreadable" if grant["remaining"] is None else "verified remaining balance at risk"
                findings.append(self.finding("EXPIRY_AT_RISK", grant["id"], detail))
        present = {g["id"] for g in grants}
        for missing in old.keys() - present:
            events.append({"at": now, "kind": "GRANT_EXPIRED", "scope": missing, "detail": "grant removed from authoritative catalog; availability held"})
        row["grant_events"] = events
        findings.extend(self.finding(e["kind"], e["scope"], e["detail"]) for e in events)

    def full(self):
        findings = []
        try:
            text = Path(self.config["ledger_path"]).expanduser().read_text()
            tasks = parse_ledger(text)
            if not tasks and "T-" in text:
                raise ValueError("ledger parse failed")
            opened = [t for t in tasks if t.get("status", "").startswith("open")]
            claimed = [t for t in tasks if t.get("status", "").startswith("claimed")]
            self.state["ledger"] = {"open": len(opened), "claimed": len(claimed)}
            if not opened:
                findings.append(self.finding("SOURCE_EMPTY", "ledger", "no open source tasks; no work inferred", severity="info"))
            watchers = {w.get("task"): w for w in self.config.get("watchers", []) if w.get("task")}
            for task in claimed:
                watcher = watchers.get(task["id"], {})
                observed = self.state.get("watchers", {}).get(watcher.get("id"), {})
                live = observed.get("alive") is True and observed.get("fresh") is True
                if not live:
                    findings.append(self.finding("RUNNER_BROKEN", task["id"], "claimed task has no verified live watcher"))
            if re.search(r"monitor[-_ ]conflict", text, re.I):
                findings.append(self.finding("RUNNER_BROKEN", "monitor-conflict", "monitor conflict marker in ledger"))
        except Exception:
            self.state["ledger"] = None
            opened = None
            findings.append(self.finding("SOURCE_EMPTY", "ledger-unreadable", "ledger source unavailable; source freshness unknown"))
        for marker in self.config.get("monitor_conflict_paths", []):
            if Path(marker).expanduser().exists():
                findings.append(self.finding("RUNNER_BROKEN", "monitor-conflict", "monitor conflict marker present"))
        try:
            queue = self.reader(self.config["queue_reader"])
            if not isinstance(queue.get("serve", {}).get("alive"), bool) or not isinstance(queue.get("routes"), dict):
                raise ValueError("invalid queue status")
            counts = [r["counts"] for r in queue["routes"].values()]
            pending = sum(c.get(k, 0) for c in counts for k in ("queued", "retry_wait", "cooldown_wait"))
            running = sum(c.get("running", 0) for c in counts)
            self.state["queue"] = {"pending": pending, "running": running, "alive": queue["serve"]["alive"]}
            if not queue["serve"]["alive"]:
                findings.append(self.finding("RUNNER_BROKEN", "queue", "queue owner not live"))
            if opened and pending + running == 0:
                findings.append(self.finding("FEED_STARVED", "queue", "open source tasks with no pending or running queue work; eligibility unverified"))
            for route, value in queue["routes"].items():
                if "auth" in str(value.get("cooldown_reason", "")).lower():
                    findings.append(self.finding("AUTH_BLOCKED", route, "queue route reports authentication hold"))
        except Exception:
            self.state["queue"] = None
            findings.append(self.finding("RUNNER_BROKEN", "queue-reader", "queue observation unavailable"))
        self.reconcile_findings("full", findings)

    def tick(self):
        for domain, cadence in (("cheap", CHEAP_S), ("urgent", URGENT_S), ("full", FULL_S)):
            last = self.state["loop"]["last_" + domain]
            if last is None or self.clock() - last >= cadence or self.clock() < last:
                try:
                    getattr(self, domain)()
                    self.state["loop"]["last_" + domain] = self.clock()
                except Exception as e:
                    # Failed observation is not a clear sample for prior faults.
                    retained = [f for f in self.state["findings"] if f["domain"] == domain and f.get("active")]
                    self.reconcile_findings(domain, retained + [self.finding("RUNNER_BROKEN", domain + "-pass", safe_error(e))])
                self.flush()
        self.deliver()
        self.flush()


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--config", type=Path, default=Path.home() / ".yggterm/config/planes.json")
    ap.add_argument("--state", type=Path, default=Path.home() / ".yggterm/swarm/heartbeat/health.json")
    ap.add_argument("--once", action="store_true")
    args = ap.parse_args()
    config = json.loads(args.config.read_text())
    args.state.parent.mkdir(parents=True, exist_ok=True)
    with args.state.with_suffix(".lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            print("HEARTBEAT_OWNER_HELD", file=sys.stderr)
            return 1
        controller = Controller(config, args.state)
        stopping = False
        def stop(signum, frame):
            nonlocal stopping
            stopping = True
        signal.signal(signal.SIGTERM, stop)
        signal.signal(signal.SIGINT, stop)
        try:
            while not stopping:
                controller.tick()
                if args.once:
                    break
                # Short waits let SIGTERM flush promptly, even between due passes.
                time.sleep(1)
        finally:
            controller.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
