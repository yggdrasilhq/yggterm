#!/usr/bin/env python3
"""THE E2E CONNECTION PROBE for the yggterm agent-row chain ([11.187] law).

Owner directive 2026-09-27: "an end to end connection probing of all errors
MUST pass for you to call the issue fixed. If this E2E test passes and I still
see the error, our probing LIED and should be critically evaluated, diagnosed
and fixed."

So this probe drives the REAL daemon on THIS host through every connection
path that has ever failed, asserts on the row's own PTY screen (the only
instrument that cannot lie about what the user would see), and reaps every
probe row it creates. Scenario ↔ defect map:

  preflight                  the [11.182] ops condition: no live stuck holder
                             pins the managed-CLI install lock
  fresh_start_connects       spawn → ensure → PTY → CLI visible (the base chain)
  resume_store_present       [11.184]: store-vouched resume with NO runtime
                             record — reproduced live failing 20:11 IST
                             ("no terminal spec"); must CONNECT
  rebirth_uuid_vouch         [11.183]: a store-absent re-birth uuid over a cwd
                             whose store holds a conversation — must land on
                             THAT conversation (agy_store_candidate_vouch in
                             the trace) and CONNECT, never refuse forever
  store_absent_refuses       the DESIGNED refusal: a store-absent uuid with no
                             store candidate must refuse by name (honest),
                             never fabricate a fresh conversation silently

A scenario that cannot run (no store, no CLI) is SKIP with the reason; a
scenario that asserts and fails is FAIL. Exit code 0 only when nothing failed.
"""

import argparse
import json
import shutil
import os
import pathlib
import re
import subprocess
import sys
import time
import uuid

HOME = pathlib.Path(os.path.expanduser("~"))
AGY_DB = HOME / ".gemini/antigravity-cli/conversation_summaries.db"
LOCK_FILE = HOME / ".yggterm/managed-cli-install.lock"
YTRACE = HOME / ".yggterm/ytrace.jsonl"
PROBE_CWD_ROOT = pathlib.Path("/tmp/yggterm-connection-probe")

REFUSAL_WORDS = (
    "Error: yggterm:",
    "no terminal spec for session",
    "no longer available on this machine",
    "could not be created",
    "terminal session not found",
)
CLI_MARKERS = (
    "Antigravity CLI",
    "? for shortcuts",  # the composer footer — the signed-in TUI
    "Google AI",  # the welcome header's account line
)


def yggterm_binary():
    """The dev ssh non-login shell prunes ~/.local/bin (documented dev PATH
    gotcha), so resolve the binary explicitly."""
    candidates = [
        os.environ.get("YGGTERM_BIN"),
        str(HOME / ".yggterm/bin/yggterm"),
        str(HOME / ".local/bin/yggterm"),
    ]
    for candidate in candidates:
        if candidate and pathlib.Path(candidate).exists():
            return candidate
    which = shutil.which("yggterm")
    return which or "yggterm"


YGGTERM_BIN = None


def run(args, timeout=120):
    return subprocess.run(
        args, capture_output=True, text=True, timeout=timeout, check=False
    )


def yggterm(args, timeout=120):
    return run([YGGTERM_BIN, "server", *args], timeout=timeout)


def yggterm_detached(args):
    """The start/resume wrapper IS the row's living bridge — it never exits
    while the row is connected, so a probe must not wait on it. Spawn it
    detached, own the process via the session-id kill in reap()."""
    return subprocess.Popen(
        [YGGTERM_BIN, "server", *args],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        stdin=subprocess.DEVNULL,
        start_new_session=True,
    )


class Scenario:
    def __init__(self, name):
        self.name = name
        self.outcome = "SKIP"
        self.evidence = ""

    def fail(self, evidence):
        self.outcome, self.evidence = "FAIL", evidence
        return self

    def ok(self, evidence=""):
        self.outcome, self.evidence = "PASS", evidence
        return self


def screen(key, timeout=30):
    answer = yggterm(["terminal", "screen", key], timeout=timeout)
    return answer.stdout if answer.returncode == 0 else ""


def screen_ok(key):
    text = screen(key)
    if not text.strip():
        return None, text
    for word in REFUSAL_WORDS:
        if word in text:
            return False, text
    for marker in CLI_MARKERS:
        if marker in text:
            return True, text
    return None, text


def reap(key):
    """Kill the row's runtime processes, then despawn the record."""
    ident = key.split("://")[-1]
    run(["pkill", "-f", ident], timeout=30)
    time.sleep(1.0)
    run(["pkill", "-9", "-f", ident], timeout=30)
    yggterm(["rows", "despawn", key], timeout=60)


def poll_until(fn, deadline_s, interval_s=3.0):
    last = (None, "")
    end = time.monotonic() + deadline_s
    while time.monotonic() < end:
        last = fn()
        if last[0] is not None:
            return last
        time.sleep(interval_s)
    return last


def scenario_preflight():
    sc = Scenario("preflight_no_stuck_lock_holder")
    if not LOCK_FILE.exists():
        return sc.ok("no lock file at all")
    # THE [11.182] PRE-FLIGHT: test the FLOCK, not the file — a wedged holder
    # pins the flock while the file sits empty (measured on dev 2026-09-27:
    # fuser showed the refresh walker holding the lock for ~7h with a stuck
    # `mimo upgrade` child, file 0 bytes, Aug 9 mtime).
    import fcntl

    handle = open(LOCK_FILE, "r")
    try:
        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        fcntl.flock(handle, fcntl.LOCK_UN)
        return sc.ok("flock is free")
    except OSError:
        pass
    finally:
        handle.close()
    # A holder INSIDE its step deadline is not a wedge — the [11.182] fix
    # guarantees release at MANAGED_CLI_INSTALL_STEP_TIMEOUT_SECS (900s).
    # Fail only past the deadline + margin; name the holder either way.
    age_s = None
    try:
        holder_json = json.loads(LOCK_FILE.read_text().strip() or "{}")
        acquired_at = holder_json.get("acquired_at_ms")
        if acquired_at:
            age_s = max(0, time.time() - acquired_at / 1000)
    except (ValueError, OSError):
        pass
    fuser = run(["fuser", "-v", str(LOCK_FILE)], timeout=30)
    fuser = run(["fuser", "-v", str(LOCK_FILE)], timeout=30)
    # fuser prints the bare pids first (stdout) and the annotated table after
    # (stderr) — read the pid list first, fall back to the table.
    match = re.search(r"^\s*(\d+)\s*$", fuser.stdout, re.MULTILINE) or re.search(
        r"(\d+)\s+F", fuser.stdout + fuser.stderr
    )
    pid = int(match.group(1)) if match else None
    holder_text = "an unresolvable holder"
    if pid and pathlib.Path(f"/proc/{pid}").exists():
        cmdline = (pathlib.Path(f"/proc/{pid}/cmdline").read_text(errors="replace")
                   .replace("\0", " ").strip())
        holder_text = f"pid {pid} ({cmdline[:140]})"
    if age_s is not None and age_s <= 900 + 120:
        return sc.ok(
            f"lock busy with a holder inside its step deadline ({holder_text}, "
            f"age {int(age_s)}s ≤ 900s+margin) — the [11.182] deadline bounds it"
        )
    sc.fail(
        f"the [11.182] ops condition is LIVE: the managed-CLI install lock is "
        f"flock-held by {holder_text} "
        + (f"for {int(age_s)}s, PAST the 900s step deadline — " if age_s is not None else "")
        + "clear the stuck holder before the chain can be trusted"
    )
    return sc


def scenario_fresh_start():
    sc = Scenario("fresh_start_connects")
    session_id = str(uuid.uuid4())
    cwd = PROBE_CWD_ROOT / session_id[:8]
    cwd.mkdir(parents=True, exist_ok=True)
    key = f"agy-runtime://{session_id}"
    try:
        yggterm_detached(["remote", "start-agy", session_id, str(cwd)])
        connected, text = poll_until(lambda: screen_ok(key), deadline_s=150)
        if connected is True:
            return sc.ok(f"screen shows the CLI ({len(text)} chars)")
        if connected is False:
            return sc.fail(f"refusal painted on the row screen: {text[:300]!r}")
        return sc.fail(f"no CLI on screen after 150s; screen tail: {text[-300:]!r}")
    finally:
        reap(key)


PRESENCE_DIR = HOME / ".gemini/antigravity-cli/presence"


def presence_held(conversation_id):
    """True when a LIVE process holds the conversation's presence lock — the
    agy CLI marks an open conversation there, and the fd-based holder arm
    (linux_proc_pids_holding_session_path) will rightly refuse a second
    resume. A probe must not pick a held conversation and then report the
    holder guard as a failure."""
    lock = PRESENCE_DIR / f"{conversation_id}.lock"
    if not lock.exists():
        return False
    answer = run(["fuser", str(lock)], timeout=30)
    return answer.returncode == 0 and bool(answer.stdout.strip())


def store_conversations():
    """(id, workspace_dir) for every live, non-killed, UNHELD conversation,
    newest first."""
    if not AGY_DB.exists():
        return []
    import sqlite3

    conn = sqlite3.connect(f"file:{AGY_DB}?mode=ro", uri=True)
    rows = conn.execute(
        "select conversation_id, workspace_uris, last_user_input_time, "
        "last_modified_time, killed from conversation_summaries"
    ).fetchall()
    conn.close()
    out = []
    for cid, ws, last_input, modified, killed in rows:
        if killed or presence_held(cid):
            continue
        try:
            dirs = json.loads(ws)
        except (TypeError, ValueError):
            continue
        recency = last_input if not str(last_input).startswith("0001") else modified
        for d in dirs or []:
            if d.startswith("file://") and pathlib.Path(d[7:]).is_dir():
                out.append((recency, cid, d[7:]))
    out.sort(reverse=True)
    return [(cid, d) for _, cid, d in out]


def live_runtime_keys():
    """The agy-runtime keys this daemon currently holds (so the probe never
    'resumes' a session that is already alive)."""
    answer = yggterm(["connect", "--list"], timeout=60)
    return set(re.findall(r"agy-runtime://[0-9a-f-]+", answer.stdout))


def scenario_resume_store_present():
    sc = Scenario("resume_store_present_connects_11184")
    conversations = store_conversations()
    if not conversations:
        sc.evidence = "no agy store on this host"
        return sc
    held = live_runtime_keys()
    target = next(
        (cid, d) for cid, d in conversations if f"agy-runtime://{cid}" not in held
    )
    session_id, workspace = target
    key = f"agy-runtime://{session_id}"
    try:
        yggterm_detached(
            ["remote", "resume-agy", session_id, workspace, "--require-existing"]
        )
        connected, text = poll_until(lambda: screen_ok(key), deadline_s=150)
        if connected is True:
            return sc.ok(f"store-vouched resume connected ({len(text)} chars)")
        if connected is False:
            return sc.fail(f"refusal painted: {text[:300]!r}")
        return sc.fail(f"no CLI after 150s; tail: {text[-300:]!r}")
    finally:
        reap(key)


def trace_vouched_id(requested_id, since_bytes):
    """The id the wrapper ladder vouched for `requested_id`, if it fired —
    the ROW lives under the VOUCHED id (the ensure keys the runtime by the
    id it is handed), so a probe that polls only the requested key watches
    an empty terminal while the real row connects next to it."""
    if not YTRACE.exists():
        return None
    found = None
    with open(YTRACE, "rb") as handle:
        handle.seek(since_bytes)
        for line in handle:
            if b"agy_store_candidate_vouch" not in line:
                continue
            try:
                event = json.loads(line)
            except ValueError:
                continue
            payload = event.get("payload", {})
            if payload.get("requested_id") == requested_id:
                found = payload.get("vouched_id")
    return found


def trace_has_vouch(requested_id, since_bytes):
    if not YTRACE.exists():
        return False
    with open(YTRACE, "rb") as handle:
        handle.seek(since_bytes)
        for line in handle:
            if b"agy_store_candidate_vouch" not in line:
                continue
            if requested_id.encode() in line:
                return True
    return False


def scenario_rebirth_uuid_vouch():
    sc = Scenario("rebirth_uuid_lands_on_store_candidate_11183")
    conversations = store_conversations()
    if not conversations:
        sc.evidence = "no agy store on this host"
        return sc
    fresh_id = str(uuid.uuid4())  # the store has never held this — by construction
    _, workspace = conversations[0]  # the store's newest conversation's cwd
    key = f"agy-runtime://{fresh_id}"
    try:
        offset = YTRACE.stat().st_size if YTRACE.exists() else 0
        yggterm_detached(
            ["remote", "resume-agy", fresh_id, workspace, "--require-existing"]
        )
        def both_keys():
            ok, text = screen_ok(key)
            if ok is not None:
                return ok, text
            vouched_id = trace_vouched_id(fresh_id, offset)
            if vouched_id:
                return screen_ok(f"agy-runtime://{vouched_id}")
            return None, text

        connected, text = poll_until(both_keys, deadline_s=150)
        vouched = trace_has_vouch(fresh_id, offset)
        if connected is True and vouched:
            return sc.ok(f"re-birth uuid {fresh_id[:8]}… vouched onto a store conversation")
        if connected is True and not vouched:
            return sc.fail(
                "row connected but NO agy_store_candidate_vouch trace — it may be "
                "fabricating a fresh conversation under the birth id (the [11.183] "
                "silent-fabrication half)"
            )
        if connected is False:
            if "no longer available" in text:
                return sc.fail(
                    f"the ladder did not fire; the [11.165] gate refused: {text[:240]!r}"
                )
            return sc.fail(f"refusal painted: {text[:240]!r}")
        return sc.fail(f"no CLI after 150s; tail: {text[-240:]!r}")
    finally:
        vouched_id = trace_vouched_id(fresh_id, offset)
        reap(key)
        if vouched_id:
            reap(f"agy-runtime://{vouched_id}")


def scenario_store_absent_refuses():
    sc = Scenario("store_absent_no_candidate_refuses_honestly")
    fresh_id = str(uuid.uuid4())
    lonely = PROBE_CWD_ROOT / f"lonely-{fresh_id[:8]}"
    lonely.mkdir(parents=True, exist_ok=True)
    conversations = {cid for cid, _ in store_conversations()}
    if fresh_id in conversations:  # cannot happen; keeps the assert honest
        return sc
    answer = yggterm(
        ["remote", "resume-agy", fresh_id, str(lonely), "--require-existing"],
        timeout=120,
    )
    combined = (answer.stdout + answer.stderr).strip()
    if "no longer available on this machine" in combined:
        return sc.ok("refused by name; nothing was fabricated")
    key = f"agy-runtime://{fresh_id}"
    time.sleep(5)
    text = screen(key)
    reap(key)
    if REFUSAL_WORDS[0] in text or "no longer available" in text:
        return sc.ok("refused (painted on the row, caught by the classifier wording)")
    return sc.fail(
        f"neither the wrapper nor the peer refused by name; stdout/stderr: "
        f"{combined[:200]!r}; screen: {text[:200]!r}"
    )


SCENARIOS = [
    scenario_preflight,
    scenario_fresh_start,
    scenario_resume_store_present,
    scenario_rebirth_uuid_vouch,
    scenario_store_absent_refuses,
]


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument(
        "--only", action="append", choices=[fn.__name__ for fn in SCENARIOS],
        help="run a subset of scenarios",
    )
    parser.add_argument(
        "--json", action="store_true", help="machine-readable report on stdout"
    )
    args = parser.parse_args()

    global YGGTERM_BIN
    YGGTERM_BIN = yggterm_binary()
    PROBE_CWD_ROOT.mkdir(parents=True, exist_ok=True)
    report = []
    for fn in SCENARIOS:
        if args.only and fn.__name__ not in args.only:
            continue
        try:
            scenario = fn()
        except Exception as error:  # noqa: BLE001 — a probe reports, never crashes
            scenario = Scenario(fn.__name__)
            scenario.outcome, scenario.evidence = "FAIL", f"probe error: {error}"
        report.append(
            {"scenario": scenario.name, "outcome": scenario.outcome,
             "evidence": scenario.evidence}
        )
        print(f"[{scenario.outcome:4}] {scenario.name}: {scenario.evidence}",
              flush=True)
    failed = [row for row in report if row["outcome"] == "FAIL"]
    if args.json:
        print(json.dumps(report, indent=2))
    print(f"\n{len(report)-len(failed)}/{len(report)} scenarios passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
