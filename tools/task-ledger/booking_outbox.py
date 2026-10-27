#!/usr/bin/env python3
"""Retained completion intents. This lock is independent of the memory hub.

No intent is removed. Replay requires plan's positive, task-bound receipt;
zero exit without a receipt is uncertain and remains pending.
"""
import argparse
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


def default_path():
    return Path(os.environ.get("YGG_BOOKING_OUTBOX", str(
        Path.home() / ".yggterm/swarm/booking-outbox/intents.jsonl")))


@contextlib.contextmanager
def locked(path):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    with path.with_suffix(".lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        yield


def read_intents(path):
    try:
        return [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]
    except FileNotFoundError:
        return []


def sync_dir(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def replace_intents(path, rows):
    fd, tmp = tempfile.mkstemp(prefix=".intents-", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as f:
            for row in rows:
                f.write(json.dumps(row, sort_keys=True) + "\n")
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, path)
        sync_dir(path.parent)
    finally:
        if os.path.exists(tmp):
            os.unlink(tmp)


def write_intent(task, proof, by, *, path=None, ts=None, ledger=None, day=None):
    path = Path(path) if path is not None else default_path()
    ts = time.time() if ts is None else ts
    identity = [task, proof, by]
    with locked(path):
        rows = read_intents(path)
        # Repeated terminal publication must not mint a new obligation.
        for row in rows:
            if [row["task"], row["proof"], row["by"]] == identity:
                return row
        key = json.dumps(identity + [int(ts)], separators=(",", ":"))
        row = {"intent_id": hashlib.sha256(key.encode()).hexdigest(),
               "task": task, "proof": proof, "by": by, "ts": ts,
               "state": "pending", "attempts": 0, "next_attempt_at": ts,
               "ledger": str(ledger) if ledger else None, "day": day, "receipt_ref": None}
        # Logical JSONL append, atomically committed: a killed writer cannot
        # poison the tail (C4). replace_intents fsyncs candidate and parent.
        replace_intents(path, rows + [row])
        return row


def update_intent(path, intent_id, **changes):
    path = Path(path)
    with locked(path):
        rows = read_intents(path)
        for row in rows:
            if row["intent_id"] == intent_id:
                # Terminal done is absorbing (C5): a late failed replay must
                # never downgrade a verified completion or lose its receipt.
                if row.get("state") == "done" and changes.get("state") == "pending":
                    return row
                row.update(changes)
                replace_intents(path, rows)
                return row
        raise ValueError("unknown intent")


def publish_intent(row):
    cmd = [sys.executable, str(Path(__file__).with_name("plan.py")),
           "--done", row["task"], "--proof", row["proof"], "--by", row["by"],
           "--from-outbox"]
    if row.get("ledger"):
        cmd += ["--ledger", row["ledger"]]
    if row.get("day"):
        cmd += ["--day", row["day"]]
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=130)
    if result.returncode != 0:
        raise RuntimeError(f"plan failed (exit {result.returncode})")
    for line in result.stdout.splitlines():
        if line.startswith("BOOKING_RECEIPT "):
            receipt = json.loads(line[len("BOOKING_RECEIPT "):])
            if (receipt.get("task"), receipt.get("proof"), receipt.get("by")) == (row["task"], row["proof"], row["by"]) and receipt.get("receipt_ref"):
                return receipt["receipt_ref"]
    raise RuntimeError("plan exited without a matching booking receipt")


def replay_once(*, path=None, publish=publish_intent, clock=time.time):
    path = Path(path) if path is not None else default_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    # One replay writer. Never hold the append lock while calling the hub.
    with path.with_suffix(".replay-lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return []
        results = []
        with locked(path):
            pending = read_intents(path)
        for row in pending:
            if row["state"] != "pending" or row.get("next_attempt_at", 0) > clock():
                continue
            attempts = row.get("attempts", 0) + 1
            try:
                receipt = publish(row)
                if not isinstance(receipt, str) or not receipt.strip():
                    raise RuntimeError("missing publication receipt")
                changes = dict(state="done", receipt_ref=receipt, done_at=clock(),
                               attempts=attempts, next_attempt_at=None, last_error=None)
            except Exception as e:
                # Retain bounded diagnostics only; command output may contain secrets.
                changes = dict(state="pending", attempts=attempts,
                               next_attempt_at=clock() + (5, 15, 30, 60)[min(attempts - 1, 3)],
                               last_error=type(e).__name__)
            results.append(update_intent(path, row["intent_id"], **changes))
        return results


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--path", type=Path, default=default_path())
    sub = ap.add_subparsers(dest="verb", required=True)
    intent = sub.add_parser("intent").add_subparsers(dest="action", required=True).add_parser("write")
    for key in ("task", "proof", "by"):
        intent.add_argument("--" + key, required=True)
    replay = sub.add_parser("replay")
    replay.add_argument("--once", action="store_true")
    args = ap.parse_args()
    if args.verb == "intent":
        print(json.dumps(write_intent(args.task, args.proof, args.by, path=args.path)))
        return 0
    while True:
        try:
            print(json.dumps(replay_once(path=args.path)), flush=True)
        except Exception as e:
            print(f"booking replay held: {type(e).__name__}", file=sys.stderr, flush=True)
        if args.once:
            return 0
        time.sleep(5)


if __name__ == "__main__":
    sys.exit(main())
