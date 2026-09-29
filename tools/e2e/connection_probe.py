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
  defmiss_fresh_start_mint   [11.206]: the SAME shape behind a REMEMBERED
                             close (a row reopened after a proper close) must
                             MINT a fresh start — trace
                             ensure_definitive_miss_fresh_start, the row
                             RE-POINTED to a fresh yggterm-minted conversation
                             id ([11.193] binding: rows show id != requested)
                             — and CONNECT, never the pre-fix 14 ms bail into
                             an error frame. RED baseline 2026-09-29 on build
                             bfe5ed19 (serving half green, binding half red);
                             FULLY GREEN the same day on build 7b399aba (the
                             [11.212] close-out run: trace fired, rows show
                             re-pointed off the requested id, row live) — the
                             minted-bound compose reaches the row.
  startborn_remote_corpse    [11.213] live proof: a START-BORN remote-agy row
                             (launch action `start-*` — minted ONLY by the GUI
                             Fresh-Start flow, so this scenario speaks the
                             GUI's own wire request `start_remote_agent_session`
                             to the daemon socket) is built into a controlled
                             handover-dead corpse (the peer stream pump frozen
                             SIGSTOP, the peer CLI child killed: bridge alive,
                             peer runtime gone — the exact shape a daemon
                             handover leaves), then re-mounted twice: mount 1
                             must dispatch the [11.213] liveness worker and —
                             because the start-born class SKIPS the fail-open
                             store ask (lane 11213-store-failopen) — answer
                             `remount_peer_liveness_verdict {instrument:
                             alive_ask_gone}` from the strict
                             `agent-runtime-alive` verb; mount 2 must SPEND it:
                             `remote_reuse_refused_peer_dead_remount`, the
                             named refusal, teardown of the leaked bridge, and
                             NO daemon_owned_fast_ready_on_first_meaningful_
                             output after the kill. RED on record: the
                             store-failopen falsifier run on d755bfd0 measured
                             the pre-skip disease on this exact shape — the
                             worker vouched the corpse ALIVE from the [11.165]
                             fail-open store answer (`instrument: null`) and
                             the gate stayed silent for the row's lifetime.

A scenario that cannot run (no store, no CLI, no peer machine) is SKIP with
the reason; a scenario that asserts and fails is FAIL. Exit code 0 only when
nothing failed.
"""

import argparse
import collections
import glob
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


def yggterm_detached(args, stderr_path=None):
    """The start/resume wrapper IS the row's living bridge — it never exits
    while the row is connected, so a probe must not wait on it. Spawn it
    detached, own the process via the session-id kill in reap().

    stderr_path: capture the wrapper's stderr. The detached spawn owns no
    PTY, so a PRE-ENSURE refusal (the [11.165] wrapper gate, the [11.197]
    tombstone guard) paints nowhere else on the machine — without the
    capture a refused rebirth reads as a silently blank row and the
    scenario misfiles a designed refusal as a dead row (measured 2026-09-29
    on jojo: rebirth FAILed "no CLI after 150s" on what was the [11.197]
    guard's named refusal, written to /dev/null)."""
    err = open(stderr_path, "wb") if stderr_path else subprocess.DEVNULL
    try:
        return subprocess.Popen(
            [YGGTERM_BIN, "server", *args],
            stdout=subprocess.DEVNULL,
            stderr=err,
            stdin=subprocess.DEVNULL,
            start_new_session=True,
        )
    finally:
        if stderr_path:
            err.close()


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


def live_holder_pids(cwd):
    """Live CLI pids whose process cwd is the row's cwd — the holder set.

    THE [11.213] IDENTITY LESSON (measured 2026-09-29): the birth compose is
    `agy '--dangerously-skip-permissions'` — the CLI's argv NEVER carries the
    row uuid, so `pkill -f <uuid>` matched nothing, the kill silently missed,
    and `rows despawn` then lawfully refused the still-live runtime. The one
    identity the holder cannot hide is its working directory."""
    pids = []
    for proc in pathlib.Path("/proc").iterdir():
        if not proc.name.isdigit():
            continue
        try:
            if os.path.realpath(proc / "cwd") == os.path.realpath(cwd):
                pids.append(int(proc.name))
        except OSError:
            continue
    return pids


def daemon_identity():
    """The (pid, build) of the default daemon — the rotation guard baseline.
    During an active merge wave the ygg-ci tick rolls the host daemon every
    few minutes; a scenario that straddles a swap is not evidence (its row
    and its close can land in different daemons)."""
    answer = yggterm(["server", "daemons"], timeout=30)
    for line in answer.stdout.splitlines():
        if line.strip().startswith("*"):
            parts = line.split()
            if len(parts) >= 3:
                return (parts[1], parts[2])
    return None


def reap(key, cwd=None):
    """Close the row the way the click path does, then evict the corpse record.

    THE [11.213] TEARDOWN LAW: `rows despawn` is a corpse-record eviction —
    it has no teardown and lawfully REFUSES a live runtime ([11.74]). The
    user-path close is `server remove` (RemoveSession despawn=None ->
    close_live_session_row: tombstone + PTY teardown + row removal). The
    despawn rides AFTER it, on the corpse.

    Returns the live holders still standing after the settle window (before
    any by-hand escalation) — the [11.195] no-live-holder measurement.

    ⛔ THE SETTLE WINDOW IS PART OF THE MEASUREMENT: the close tears the PTY
    down synchronously (the session leader is awaited), but the CLI grandchild
    dies from the master drop a few hundred ms LATER. A scan at T+0 counts a
    dying holder as a survivor — measured 2026-09-29: `closed terminal runtime`
    with the pair gone 3 s later, while a T+0 scan named two live pids."""
    # yggterm() already prepends `server` -- a bare [server, remove, ...]
    # here became `server server remove` and the close NEVER ran (rc=1,
    # "unsupported server command: server"), which re-created the exact
    # never-closed-row shape the [11.195] assert exists to catch.
    answer = yggterm(["remove", key], timeout=60)
    close_message = (
        "rc=" + str(answer.returncode)
        + " out=" + answer.stdout.strip()[:150]
        + " err=" + answer.stderr.strip()[:150]
    )
    post_close = []
    if cwd is not None:
        deadline = time.monotonic() + 5.0
        while time.monotonic() < deadline:
            post_close = live_holder_pids(cwd)
            if not post_close:
                break
            time.sleep(0.5)
    if cwd is not None:
        for _ in range(3):
            holders = live_holder_pids(cwd)
            if not holders:
                break
            for pid in holders:
                run(["kill", "-9", str(pid)], timeout=30)
            time.sleep(1.0)
    yggterm(["rows", "despawn", key], timeout=60)
    return post_close, close_message


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


def trace_has_event(requested_id, since_bytes, event_name):
    """Exact event-name match — `trace_has_vouch`'s needle is a prefix of
    `agy_store_candidate_vouch_refused_tombstoned`, so it cannot tell a
    vouch from a refusal."""
    if not YTRACE.exists():
        return False
    needle = event_name.encode()
    with open(YTRACE, "rb") as handle:
        handle.seek(since_bytes)
        for line in handle:
            if needle not in line:
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
    stderr_path = PROBE_CWD_ROOT / f"rebirth-{fresh_id[:8]}.stderr"
    try:
        offset = YTRACE.stat().st_size if YTRACE.exists() else 0
        proc = yggterm_detached(
            ["remote", "resume-agy", fresh_id, workspace, "--require-existing"],
            stderr_path=str(stderr_path),
        )
        def both_keys():
            ok, text = screen_ok(key)
            if ok is not None:
                return ok, text
            vouched_id = trace_vouched_id(fresh_id, offset)
            if vouched_id:
                return screen_ok(f"agy-runtime://{vouched_id}")
            return None, text

        def wrapper_refused_early():
            # A pre-ensure refusal exits the wrapper before any row is
            # born — no screen will ever appear, so stop the poll and read
            # the captured stderr instead.
            if proc.poll() is None:
                return None
            try:
                err = stderr_path.read_text(errors="replace")
            except OSError:
                return False
            return "no longer available" in err

        deadline = time.monotonic() + 150
        connected, text = None, ""
        while time.monotonic() < deadline:
            connected, text = both_keys()
            if connected is not None or wrapper_refused_early():
                break
            time.sleep(3)
        refused_stderr = ""
        try:
            refused_stderr = stderr_path.read_text(errors="replace").strip()
        except OSError:
            pass
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
        # No row was born and the poll ended. Either the wrapper refused by
        # name (captured stderr — the only place a detached refusal paints),
        # or the row truly died in silence, which is the defect this arm
        # exists to catch.
        witness = trace_has_event(
            fresh_id, offset, "agy_store_candidate_vouch_refused_tombstoned"
        )
        if "no longer available" in refused_stderr and witness:
            return sc.ok(
                "designed [11.197] tombstone guard: this cwd's newest candidate is a "
                "remembered close, refused by name pre-ensure (captured stderr); the "
                "ladder consulted the store and enforced the guard — its bind arm is "
                "not exercised by this cwd"
            )
        if "no longer available" in refused_stderr:
            return sc.fail(
                "wrapper refused without the tombstone witness — the ladder never "
                f"consulted the store: {refused_stderr[:240]!r}"
            )
        return sc.fail(
            "no CLI after 150s and no named refusal in the wrapper's stderr — a "
            f"silent death: stderr={refused_stderr[:240]!r} screen={text[-160:]!r}"
        )
    finally:
        vouched_id = trace_vouched_id(fresh_id, offset)
        reap(key)
        if vouched_id:
            reap(f"agy-runtime://{vouched_id}")
        stderr_path.unlink(missing_ok=True)


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


def db_conversations_for_dir(directory):
    """(conversation_id, killed) for every summaries row whose
    workspace_uris names `directory` — what the vouch ladder's candidate
    search reads (exact `file://<cwd>` element, the [11.183] tightening)."""
    import sqlite3

    if not AGY_DB.exists():
        return []
    conn = sqlite3.connect(f"file:{AGY_DB}?mode=ro", uri=True)
    try:
        rows = conn.execute(
            "select conversation_id, workspace_uris, killed "
            "from conversation_summaries"
        ).fetchall()
    finally:
        conn.close()
    out = []
    for cid, ws, killed in rows:
        try:
            dirs = json.loads(ws)
        except (TypeError, ValueError):
            continue
        if f"file://{directory}" in (dirs or []):
            out.append((cid, killed))
    return out


def db_delete_conversations_for_dir(directory):
    """The scenario's own store surgery, scoped to its own probe cwd: delete
    every summaries row bound to `directory`, return the ids removed."""
    import sqlite3

    if not AGY_DB.exists():
        return []
    conn = sqlite3.connect(str(AGY_DB), timeout=10)
    removed = []
    try:
        for cid, _killed in db_conversations_for_dir(directory):
            conn.execute(
                "delete from conversation_summaries where conversation_id = ?",
                (cid,),
            )
            removed.append(cid)
        conn.commit()
    finally:
        conn.close()
    return removed


def rows_show_text(key):
    answer = yggterm(["rows", "show", key], timeout=60)
    return (answer.stdout + answer.stderr).strip()


def scenario_defmiss_fresh_start_mint():
    sc = Scenario("defmiss_fresh_start_mint_11206")
    session_id = str(uuid.uuid4())  # the requested id — never in the store by construction
    cwd = PROBE_CWD_ROOT / f"mint-{session_id[:8]}"
    cwd.mkdir(parents=True, exist_ok=True)
    key = f"agy-runtime://{session_id}"
    stderr_path = PROBE_CWD_ROOT / f"mint-{session_id[:8]}.stderr"
    try:
        # 1. Birth a real row in the dedicated cwd.
        yggterm_detached(["remote", "start-agy", session_id, str(cwd)])
        connected, text = poll_until(lambda: screen_ok(key), deadline_s=120)
        if connected is not True:
            return sc.fail(f"birth leg never mounted a CLI; screen: {text[-200:]!r}")
        # 2. Give the store time to bind the birth's conversation to this cwd
        # (a turnless birth may never write one — then the cwd is already
        # candidate-less and step 4's surgery removes nothing).
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline and not db_conversations_for_dir(str(cwd)):
            time.sleep(3)
        # 3. Close the row the way the click path does: `server remove`
        # (close_live_session_row: tombstone + PTY teardown + row removal),
        # THEN rows despawn evicts the corpse record. THE [11.195] LAW,
        # asserted: the close itself must leave no live CLI holder — a
        # survivor orphans into the resume's holder wait.
        daemon_at_birth = daemon_identity()
        post_close_holders, close_message = reap(key, cwd)
        if post_close_holders:
            rotated = daemon_identity() != daemon_at_birth
            invalid = (
                " — INVALID RUN: the daemon rotated mid-scenario, the row "
                "and its close landed in different daemons"
                if rotated
                else ""
            )
            return sc.fail(
                f"the close left live CLI holders {post_close_holders} \u2014 the "
                "[11.195] no-live-holder-survives-into-the-resume law is broken "
                f"(close said: {close_message!r}; escalation-killed for the "
                "record; the mint leg stays red until the close kills its own "
                f"CLI{invalid})"
            )
        removed = db_delete_conversations_for_dir(str(cwd))
        # 5. The click — the same verb chain a GUI row-open drives: wrapper
        # gate (passes: the close IS remembered) -> ensure (definitive miss,
        # no candidate) -> the [11.190] minted fresh start.
        offset = YTRACE.stat().st_size if YTRACE.exists() else 0
        proc = yggterm_detached(
            ["remote", "resume-agy", session_id, str(cwd), "--require-existing"],
            stderr_path=str(stderr_path),
        )

        def minted():
            ok, text = screen_ok(key)
            if ok is None:
                return None, text
            # Screen alive AND the mint named in the trace — either alone
            # could be a different arm (a vouch this scenario tried to
            # starve, or a trace the screen never followed).
            if trace_has_event(session_id, offset, "ensure_definitive_miss_fresh_start"):
                return True, text
            return None, text

        def wrapper_refused_early():
            if proc.poll() is None:
                return None
            try:
                err = stderr_path.read_text(errors="replace")
            except OSError:
                return False
            return "no longer available" in err

        deadline = time.monotonic() + 150
        connected, text = None, ""
        while time.monotonic() < deadline:
            connected, text = minted()
            if connected is not None or wrapper_refused_early():
                break
            time.sleep(3)
        refused_stderr = ""
        try:
            refused_stderr = stderr_path.read_text(errors="replace").strip()
        except OSError:
            pass
        if connected is True:
            # THE [11.193] BINDING, the half the Fresh Start stamp names: the
            # minted fresh start composes `--conversation <fresh uuid>` and
            # RE-POINTS the row's id onto it — rows show must answer an id
            # that is NOT the store-absent requested one. Measured RED
            # 2026-09-29 on build bfe5ed19: the serving half landed (trace +
            # live row) but the spawned command resumed the ABSENT requested
            # id and rows show kept it — the minted-bound compose never
            # reached the row. Split verdicts, never launder one into the
            # other.
            shown = rows_show_text(key)
            id_field = ""
            for line in shown.splitlines():
                line = line.strip()
                if line.startswith('"id":'):
                    id_field = line.split('"id":')[1].strip(" \",")
                    break
            bound = bool(id_field) and id_field != session_id
            if bound:
                return sc.ok(
                    "store-absent remembered-closed row with no cwd candidate MINTED "
                    f"and BOUND a fresh start (trace fired; rows show id re-pointed "
                    f"{session_id[:8]}… -> {id_field[:8]}…; {len(text)} chars)"
                )
            return sc.fail(
                "SERVING half green, BINDING half red: ensure_definitive_miss_fresh_start "
                f"traced and the row connected, but rows show answers id="
                f"{id_field or '(none)'} — the row still wears the store-absent "
                f"requested id {session_id[:8]}…, so the minted-bound compose "
                "([11.190]/[11.193]) did not reach the row (the spawned command "
                "resumed the absent id; store surgery removed "
                f"{len(removed)} conversation(s))"
            )
        if "no longer available" in refused_stderr or "no longer available" in text:
            return sc.fail(
                "the ladder refused instead of minting — remembered close did not "
                f"pass the wrapper or the miss was not definitive: stderr="
                f"{refused_stderr[:240]!r} screen={text[:160]!r}"
            )
        if connected is False:
            return sc.fail(f"refusal painted on the row screen: {text[:300]!r}")
        alive = screen_ok(key)[0]
        if alive:
            # ⛔ THE ARM IS NAMED OR THE VERDICT IS WORTHLESS (measured
            # 2026-09-29, the [11.213] grind): this branch fired three times
            # with the wrapper's stderr hidden, and each read-through of the
            # trace re-derived by hand what one stderr dump would have said.
            return sc.fail(
                "CLI on screen but NO ensure_definitive_miss_fresh_start trace — "
                "the row connected through some other arm; the mint falsifier "
                f"stays unexercised (wrapper exit={proc.poll()} stderr={refused_stderr[:400]!r})"
            )
        return sc.fail(
            "no CLI after 150s and no named refusal — stderr="
            f"{refused_stderr[:240]!r} screen={text[-160:]!r}"
        )
    finally:
        reap(key)
        db_delete_conversations_for_dir(str(cwd))
        stderr_path.unlink(missing_ok=True)


def daemon_socket():
    """The live daemon's unix socket. The daemon's listening name carries the
    version (server-<ver>.sock) and every older-name compatibility symlink is
    re-pointed at it on each rotation — so the majority realpath of the
    server-*.sock family IS the serving socket (measured on dev 2026-09-29:
    dozens of legacy links, one target)."""
    counts = collections.Counter()
    for link in glob.glob(str(HOME / ".yggterm/server-*.sock")):
        try:
            counts[os.path.realpath(link)] += 1
        except OSError:
            continue
    if not counts:
        return None
    return counts.most_common(1)[0][0]


def wire_request(request, timeout=60):
    """One newline-JSON request to the daemon socket — the SAME wire every CLI
    verb and the GUI speak (ClientRequestEnvelope; an anonymous envelope
    serializes as the bare request). The [11.213] scenario needs the one
    request no CLI verb reaches: `start_remote_agent_session`, the GUI
    Fresh-Start flow's own birth, and the only plane that mints a START-BORN
    remote row (launch action `start-*`). Real daemon verb, real compose, no
    fabricated state."""
    import socket as pysocket

    sock_path = daemon_socket()
    if not sock_path:
        raise RuntimeError("no daemon socket resolved under ~/.yggterm")
    stream = pysocket.socket(pysocket.AF_UNIX, pysocket.SOCK_STREAM)
    stream.settimeout(timeout)
    try:
        stream.connect(sock_path)
        stream.sendall((json.dumps(request) + "\n").encode())
        buf = b""
        while b"\n" not in buf:
            chunk = stream.recv(65536)
            if not chunk:
                break
            buf += chunk
    finally:
        stream.close()
    line = buf.decode(errors="replace").strip().splitlines()
    if not line:
        raise RuntimeError("empty daemon answer")
    return json.loads(line[0])


def ssh_run(machine, script, timeout=30):
    return run(
        [
            "ssh",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=5",
            machine,
            script,
        ],
        timeout=timeout,
    )


PEER_YGGTERM = "$HOME/.yggterm/bin/yggterm"


def remote_holder_pids(machine, cwd):
    """Live pids on `machine` whose cwd is `cwd` — the peer-side holder walk,
    the [11.213] identity lesson applied over ssh (the CLI's argv never
    carries the row uuid; the working directory cannot hide)."""
    script = (
        "for p in /proc/[0-9]*/cwd; do tgt=$(readlink \"$p\" 2>/dev/null); "
        f'if [ "$tgt" = "{cwd}" ]; then echo $(basename $(dirname "$p")); fi; done'
    )
    answer = ssh_run(machine, script)
    return [int(x) for x in answer.stdout.split() if x.isdigit()]


def peer_runtime_alive(machine, session_uuid):
    """The strict peer instrument, asked where it lives: `agent-runtime-alive`
    on the PEER (the worker sshes there too). Strict-parse of the last JSON
    line; None = unreadable (transport/old binary), never False."""
    answer = ssh_run(
        machine,
        f"{PEER_YGGTERM} server remote agent-runtime-alive "
        f"agy-runtime://{session_uuid}",
        timeout=45,
    )
    for line in reversed(answer.stdout.splitlines()):
        line = line.strip()
        if line.startswith("{"):
            try:
                value = json.loads(line)
            except ValueError:
                continue
            alive = value.get("alive")
            if isinstance(alive, bool):
                return alive
    return None


def bridge_pids(session_uuid):
    """Local pids of the row's ssh bridge (launch command carries the uuid in
    `ssh <peer> ... start-agy <uuid> ...`). This is `still_running`'s source:
    the daemon PTY's child."""
    pids = []
    for proc in pathlib.Path("/proc").iterdir():
        if not proc.name.isdigit():
            continue
        try:
            cmdline = (proc / "cmdline").read_bytes().replace(b"\0", b" ")
        except OSError:
            continue
        text = cmdline.decode(errors="replace")
        if session_uuid in text and "start-agy" in text:
            pids.append(int(proc.name))
    return pids


def trace_payloads(event_name, since_bytes):
    """Every payload of `event_name` appended since `since_bytes`."""
    if not YTRACE.exists():
        return []
    needle = event_name.encode()
    out = []
    with open(YTRACE, "rb") as handle:
        handle.seek(since_bytes)
        for line in handle:
            if needle not in line:
                continue
            try:
                event = json.loads(line)
            except ValueError:
                continue
            if (event.get("name") or "").endswith(event_name):
                out.append(event.get("payload", {}))
    return out


def scenario_startborn_remote_corpse():
    sc = Scenario("startborn_remote_corpse_refuses_11213")
    this_host = os.uname().nodename
    peer = None
    for candidate in ("oc", "dev", "jojo", "practice"):
        if candidate == this_host:
            continue
        capable = ssh_run(
            candidate,
            f"test -x {PEER_YGGTERM} && command -v agy >/dev/null",
            timeout=20,
        )
        if capable.returncode == 0:
            peer = candidate
            break
    if not peer:
        sc.evidence = (
            "no peer machine with yggterm + agy reachable — a cross-machine "
            "corpse needs one; nothing was born"
        )
        return sc

    session_cwd = PROBE_CWD_ROOT / f"sb-{uuid.uuid4().hex[:8]}"
    key = None
    session_uuid = None
    oneshot_pid = None
    kill_offset = None
    daemon_at_birth = None
    try:
        ssh_run(peer, f"mkdir -p '{session_cwd}'", timeout=20)
        # 1. THE BIRTH — the GUI Fresh-Start wire request, the only mint of a
        # start-born remote row (launch action `start-*`).
        answer = wire_request(
            {
                "kind": "start_remote_agent_session",
                "session_kind": "antigravity",
                "target": peer,
                "prefix": None,
                "cwd": str(session_cwd),
                "title_hint": None,
                "terminal_appearance": None,
                "insert_after": None,
                "outline_prefix": None,
                "launch_options": None,
            }
        )
        message = answer.get("message") or ""
        key = next(
            (token for token in message.split() if token.startswith("remote-agy://")),
            None,
        )
        if not key:
            return sc.fail(
                f"the wire birth did not name a row key: {json.dumps(answer)[:300]}"
            )
        session_uuid = key.rsplit("/", 1)[1]
        daemon_at_birth = daemon_identity()
        # 2. The birth leg must mount the CLI (the ring now holds real output).
        connected, text = poll_until(lambda: screen_ok(key), deadline_s=150)
        if connected is not True:
            return sc.fail(
                f"birth leg never mounted a CLI on the row; screen: {text[-200:]!r}"
            )
        # 3. CORPSIFY — the handover-dead shape: bridge ALIVE, peer runtime
        # GONE. Freeze the peer stream pump first (a handover orphans the pump
        # mid-stream; the frozen pump never sees the EOF a child death sends),
        # then kill the CLI child by cwd — the argv never carries the uuid.
        kill_offset = YTRACE.stat().st_size if YTRACE.exists() else 0
        oneshot_answer = ssh_run(
            peer, f"pgrep -f 'start-agy {session_uuid}' | head -1", timeout=20
        )
        try:
            oneshot_pid = int(oneshot_answer.stdout.strip().split()[0])
        except (ValueError, IndexError):
            oneshot_pid = None
        if oneshot_pid:
            ssh_run(peer, f"kill -STOP {oneshot_pid}", timeout=20)
        for pid in remote_holder_pids(peer, str(session_cwd)):
            ssh_run(peer, f"kill -9 {pid}", timeout=20)
        alive = peer_runtime_alive(peer, session_uuid)
        if alive is not False:
            return sc.fail(
                f"corpsify failed: the peer ({peer}) answers "
                f"agent-runtime-alive = {alive!r} (want False) — the runtime "
                "still lives, so the verdict could only vouch"
            )
        bridge_deadline = time.monotonic() + 10
        while time.monotonic() < bridge_deadline and not bridge_pids(session_uuid):
            time.sleep(0.5)
        if not bridge_pids(session_uuid):
            return sc.fail(
                "the local bridge died with the peer child — still_running "
                "false sends the ensure down the RESTART arm and the re-mount "
                "liveness gate is unreachable by construction; the corpse "
                "shape (childless-alive bridge) did not form"
            )
        # 4. RE-MOUNT 1 — the reuse arm pays the [11.213] gate: the worker
        # dispatches, the START-BORN class skips the fail-open store ask, and
        # the strict alive verb answers gone → verdict alive_ask_gone.
        yggterm(["connect", key], timeout=60)

        def verdict_for_row():
            payloads = [
                p
                for p in trace_payloads("remount_peer_liveness_verdict", kill_offset)
                if p.get("path") == key
            ]
            return (payloads[-1], "") if payloads else (None, "")

        verdict = poll_until(verdict_for_row, deadline_s=90, interval_s=4.0)[0]
        if verdict is None:
            return sc.fail(
                "re-mount 1 never dispatched the remount liveness worker (no "
                "remount_peer_liveness_verdict in 90s) — the gate did not fire"
            )
        instrument = verdict.get("instrument")
        if instrument != "alive_ask_gone":
            return sc.fail(
                f"THE PRE-SKIP DISEASE (the red shape, on record from the "
                f"store-failopen falsifier run on d755bfd0): the worker "
                f"answered instrument={instrument!r} vouched="
                f"{verdict.get('vouched')!r} — the fail-open store ask vouched "
                "the corpse alive; the start-born skip is NOT live on this "
                "daemon"
            )
        # 5. RE-MOUNT 2 — the spend: refused healable + teardown.
        answer = yggterm(["connect", key], timeout=60)
        combined = (answer.stdout + answer.stderr).strip()
        spent = any(
            p.get("path") == key
            for p in trace_payloads("remote_reuse_refused_peer_dead_remount", kill_offset)
        )
        refusal_named = "peer session gone" in combined
        if not (spent or refusal_named):
            return sc.fail(
                "the landed verdict was never spent: re-mount 2 saw neither "
                f"remote_reuse_refused_peer_dead_remount nor the named refusal "
                f"(rc={answer.returncode} out={answer.stdout[:120]!r} "
                f"err={answer.stderr[:160]!r})"
            )
        # 6. THE NEGATIVE — no daemon-owned fast-ready may bless the corpse
        # after the kill (the disease's paint: retained bytes read as a live
        # birth's first meaningful output).
        fast_ready = [
            p
            for p in trace_payloads(
                "daemon_owned_fast_ready_on_first_meaningful_output", kill_offset
            )
            if key in json.dumps(p)
        ]
        # 7. THE TEARDOWN — the Q5 wedge's actual harm is the corpse SESSION
        # staying held (live_runtime_held blocks the [11.214] restore flow)
        # with the bridge leaking behind it. The frozen pump holds the ssh
        # open by construction, so measure in two honest steps: first the row
        # must leave the daemon's live set, then — the pump unfrozen, the
        # construction artifact gone — the bridge must die from the master
        # drop (the [11.195] settle law).
        row_gone_deadline = time.monotonic() + 15
        while time.monotonic() < row_gone_deadline:
            listing = yggterm(["connect", "--list"], timeout=60).stdout
            if key not in listing:
                break
            time.sleep(1.0)
        row_still_held = key in yggterm(["connect", "--list"], timeout=60).stdout
        if oneshot_pid:
            ssh_run(peer, f"kill -CONT {oneshot_pid} 2>/dev/null", timeout=20)
        bridge_deadline = time.monotonic() + 15
        while time.monotonic() < bridge_deadline and bridge_pids(session_uuid):
            time.sleep(0.5)
        leaked = bridge_pids(session_uuid)
        rotated = daemon_identity() != daemon_at_birth
        if rotated:
            return sc.fail(
                "INVALID RUN: the daemon rotated mid-scenario (the verdict and "
                "the spend landed in different daemons) — rerun on a settled "
                "daemon"
            )
        problems = []
        if fast_ready:
            problems.append(
                f"daemon_owned_fast_ready_on_first_meaningful_output fired for "
                f"the corpse {len(fast_ready)}x after the kill"
            )
        if row_still_held:
            problems.append(
                "the daemon still holds the refused row 15s after the spend "
                "(the Q5 wedge: live_runtime_held blocks the restore flow)"
            )
        if leaked:
            problems.append(
                f"the refused runtime leaked the bridge {leaked} (alive 15s "
                "after the pump unfroze — the Q5 bridge leak)"
            )
        if problems:
            return sc.fail("; ".join(problems))
        return sc.ok(
            f"start-born corpse on {peer}: mount 1 verdict alive_ask_gone (the "
            f"store-ask skip held), mount 2 spent it "
            f"({'trace fired' if spent else 'refusal named'}"
            f"{'; trace+refusal' if spent and refusal_named else ''}), bridge "
            "torn down, no fast-ready"
        )
    finally:
        # REAP — the row on this daemon, the corpse record, the peer's runtime
        # row, the frozen pump (CONT so it can see the closed stream, then
        # kill), any surviving holder, both cwd copies.
        if key:
            yggterm(["remove", key], timeout=60)
            yggterm(["rows", "despawn", key], timeout=60)
        if session_uuid:
            ssh_run(
                peer,
                f"{PEER_YGGTERM} server remove agy-runtime://{session_uuid}; "
                f"{PEER_YGGTERM} server rows despawn "
                f"agy-runtime://{session_uuid}",
                timeout=60,
            )
        if oneshot_pid:
            ssh_run(
                peer,
                f"kill -CONT {oneshot_pid} 2>/dev/null; "
                f"kill -9 {oneshot_pid} 2>/dev/null",
                timeout=20,
            )
        for pid in remote_holder_pids(peer, str(session_cwd)):
            ssh_run(peer, f"kill -9 {pid}", timeout=20)
        ssh_run(peer, f"rm -rf '{session_cwd}'", timeout=20)
        shutil.rmtree(session_cwd, ignore_errors=True)


def scenario_stillborn_resume_corpse():
    """THE [11.165] COMPLETION FALSIFIER — the stillborn-RESUME subclass
    (born from a store-absent resume, NOT start-born) used to vouch itself
    alive on the fail-open store answer, and the vouched set is never
    re-asked, so ONE fail-open silenced the [11.213] gate for the row's
    lifetime (measured live on d755bfd0: 5x bootstrap_reset, 14x
    resize_failed, 4x ghost_frame while the gate slept). The cure: the
    peer's session-exists verb answers THREE-VALUED and the worker falls
    through to the strict alive verb on the fail-open shape — while a
    DEFINITIVE store miss is the confident word that CLOSES the row (the
    [11.155] shape). This scenario births the corpse the cheap way (the
    stored-session open on a store-absent id), re-mounts twice, and asserts
    the full close chain: verdict instrument peer_store_gone, the
    remote_reuse_closed_peer_dead_remount spend, the row leaving the live
    set, and no daemon_owned_fast_ready ever blessing the corpse."""
    sc = Scenario("stillborn_resume_corpse_closes_11165")
    this_host = os.uname().nodename
    peer = None
    for candidate in ("oc", "dev", "jojo", "practice"):
        if candidate == this_host:
            continue
        capable = ssh_run(
            candidate,
            f"test -x {PEER_YGGTERM} && command -v agy >/dev/null",
            timeout=20,
        )
        if capable.returncode == 0:
            peer = candidate
            break
    if not peer:
        sc.evidence = (
            "no peer machine with yggterm + agy reachable — a cross-machine "
            "corpse needs one; nothing was born"
        )
        return sc

    session_uuid = f"11165-{uuid.uuid4()}"
    key = f"remote-agy://{peer}/{session_uuid}"
    kill_offset = YTRACE.stat().st_size if YTRACE.exists() else 0
    wrapper_pid = None
    daemon_at_birth = None
    try:
        # 1. THE BIRTH — the stored-session open (`server connect`) on a
        # store-absent id: the daemon-plane birth whose launch action is a
        # RESUME (the peer compose is `resume-agy <id> <cwd>
        # --require-existing`). Measured 2026-09-29 (the red experiment,
        # uuid 11165rb-a05d0028 on the pre-fix daemon): the peer's
        # [11.165] gate is fail-open so the resume proceeds, the ladder
        # finds no candidate, the wrapper hangs in the store-bind wait —
        # childless-alive — and the daemon holds the row with the
        # store-absent id. That hung wrapper IS the corpse's still_running
        # truth; no pump freeze is needed.
        answer = yggterm(["connect", key], timeout=120)
        if key not in (answer.stdout + answer.stderr):
            return sc.fail(
                f"the connect birth did not name the row: "
                f"{json.dumps(answer)[:200]}"
            )
        time.sleep(5)

        def stillborn_shape():
            """The wrapper alive + childless + the runtime dead — the corpse
            shape the reuse arm needs. Returns (wrapper_pid, error)."""
            wrapper_answer = ssh_run(
                peer, f"pgrep -f 'resume-agy {session_uuid}' | head -1", timeout=20
            )
            try:
                pid = int(wrapper_answer.stdout.strip().split()[0])
            except (ValueError, IndexError):
                return None, (
                    "the peer wrapper is gone — the stillborn shape is not "
                    f"held right now; search: {wrapper_answer.stdout!r}"
                )
            children = ssh_run(peer, f"pgrep -P {pid} 2>/dev/null", timeout=20)
            if children.stdout.strip():
                return None, (
                    f"the [11.183] ladder re-pointed the birth onto a live "
                    f"peer conversation (wrapper {pid} holds children "
                    f"{children.stdout.strip().splitlines()[:2]!r}) — the row "
                    "is a healthy resume the store genuinely holds, not a "
                    "stillborn corpse; nothing was falsified this run"
                )
            return pid, None

        # 2. THE STILLBORN SHAPE, verified the moment it holds. The bind
        # wait is nondeterministic (measured: it can expire well inside a
        # minute), and once the wrapper exits the next mount takes the
        # RESTART arm — the reuse arm and its [11.213] gate never run. So
        # the falsifier waits out the birth, remounts INSIDE the alive
        # window, and if a mount still took the restart arm it re-verifies
        # the shape (a restart re-hangs a fresh wrapper — the corpse
        # re-forms) and remounts again.
        wrapper_pid = None
        for _ in range(10):
            wrapper_pid, shape_error = stillborn_shape()
            if wrapper_pid is not None:
                break
            if "re-pointed" in (shape_error or ""):
                sc.evidence = shape_error
                return sc
            time.sleep(3)
        if wrapper_pid is None:
            return sc.fail(
                f"the stillborn shape never held after the birth: {shape_error}"
            )
        alive = peer_runtime_alive(peer, session_uuid)
        if alive is not False:
            return sc.fail(
                f"the stillborn shape did not form: the peer ({peer}) answers "
                f"agent-runtime-alive = {alive!r} (want False) — the runtime "
                "lives, so no store word could close it honestly"
            )
        daemon_at_birth = daemon_identity()
        # 3. THE RE-MOUNTS — the reuse arm pays the [11.213] gate; the
        # resume class keeps the store-first ask, and THE THREE-VALUED
        # ANSWER DECIDES: a definitive miss is the [11.155] confident NO.
        verdict = None
        for attempt in range(6):
            yggterm(["connect", key], timeout=60)

            def verdict_for_row():
                payloads = [
                    p
                    for p in trace_payloads(
                        "remount_peer_liveness_verdict", kill_offset
                    )
                    if p.get("path") == key
                ]
                return (payloads[-1], "") if payloads else (None, "")

            verdict = poll_until(verdict_for_row, deadline_s=60, interval_s=4.0)[0]
            if verdict is not None:
                break
            # No verdict: this mount most likely took the restart arm (the
            # bind wait expired mid-attempt). The restart re-hangs a fresh
            # wrapper — re-verify the corpse and mount again.
            wrapper_pid, shape_error = stillborn_shape()
            if wrapper_pid is None:
                if "re-pointed" in (shape_error or ""):
                    sc.evidence = shape_error
                    return sc
                time.sleep(5)
        if verdict is None:
            return sc.fail(
                f"no remount ever took the reuse arm for {key} (no "
                "remount_peer_liveness_verdict across 6 mounts) — the "
                "stillborn shape kept dissolving into restart-arm spawns or "
                "the worker's ask never landed"
            )
        instrument = verdict.get("instrument")
        if instrument != "peer_store_gone":
            if verdict.get("vouched") and instrument is None:
                return sc.fail(
                    "THE FAILOPEN DISEASE (the red shape, on record: the "
                    "store-failopen falsifier run on d755bfd0 and the 11165 "
                    "red experiment, uuid 11165rb-a05d0028): the worker "
                    "answered vouched=true from the fail-open store answer — "
                    "the peer answered without the definitive bit (old peer "
                    "binary?) or the three-valued fetch is not live on this "
                    "daemon"
                )
            return sc.fail(
                f"the store ask answered instrument={instrument!r} (want "
                "peer_store_gone, the definitive miss). alive_ask_gone means "
                "the peer binary predates the definitive bit — redeploy the "
                "fleet and rerun"
            )
        # 4. THE SPEND — one more reuse-window mount: the gate spends the
        # landed verdict as the CLOSE (tombstone + departure), not the
        # healable refusal — a definitive store absence is the confident
        # word, and the recovery door must have nothing to re-mount.
        spent = False
        refusal_named = False
        combined = ""
        for _ in range(3):
            answer = yggterm(["connect", key], timeout=60)
            combined = (answer.stdout + answer.stderr).strip()
            spent = any(
                p.get("path") == key
                for p in trace_payloads(
                    "remote_reuse_closed_peer_dead_remount", kill_offset
                )
            )
            refusal_named = "peer session gone" in combined
            if spent or refusal_named:
                break
            wrapper_pid, _shape_error = stillborn_shape()
            if wrapper_pid is None:
                time.sleep(5)
        if not (spent or refusal_named):
            return sc.fail(
                "the landed verdict was never spent: neither "
                f"remote_reuse_closed_peer_dead_remount nor the named "
                f"refusal across 3 mounts (last rc={answer.returncode} "
                f"out={answer.stdout[:120]!r} err={answer.stderr[:160]!r})"
            )
        # 5. THE CLOSE — the row leaves the daemon's live set.
        row_gone_deadline = time.monotonic() + 15
        while time.monotonic() < row_gone_deadline:
            listing = yggterm(["connect", "--list"], timeout=60).stdout
            if key not in listing:
                break
            time.sleep(1.0)
        row_still_held = key in yggterm(["connect", "--list"], timeout=60).stdout
        # 6. THE NEGATIVE — no daemon-owned fast-ready may bless the corpse
        # after the birth (the disease's paint: retained bytes read as a live
        # birth's first meaningful output).
        fast_ready = [
            p
            for p in trace_payloads(
                "daemon_owned_fast_ready_on_first_meaningful_output", kill_offset
            )
            if key in json.dumps(p)
        ]
        rotated = daemon_identity() != daemon_at_birth
        if rotated:
            return sc.fail(
                "INVALID RUN: the daemon rotated mid-scenario (the verdict "
                "and the spend landed in different daemons) — rerun on a "
                "settled daemon"
            )
        problems = []
        if fast_ready:
            problems.append(
                f"daemon_owned_fast_ready_on_first_meaningful_output fired "
                f"for the corpse {len(fast_ready)}x"
            )
        if row_still_held:
            problems.append(
                "the daemon still holds the closed row 15s after the spend "
                "(the close did not complete)"
            )
        if problems:
            return sc.fail("; ".join(problems))
        return sc.ok(
            f"stillborn-resume corpse on {peer}: mount 1 verdict "
            "peer_store_gone (the definitive store word — no fail-open "
            f"vouch), mount 2 spent it as the CLOSE "
            f"({'trace fired' if spent else 'refusal named'}"
            f"{'; trace+refusal' if spent and refusal_named else ''}), the "
            "row left the live set, no fast-ready"
        )
    finally:
        # REAP — the row on this daemon (a spent close already removed it;
        # both calls are idempotent), the tombstone record's peer twin, EVERY
        # hung wrapper the restart-arm churn may have stacked, any peer
        # runtime row.
        yggterm(["remove", key], timeout=60)
        yggterm(["rows", "despawn", key], timeout=60)
        ssh_run(
            peer,
            f"pkill -9 -f 'resume-agy {session_uuid}' 2>/dev/null; true",
            timeout=20,
        )
        ssh_run(
            peer,
            f"{PEER_YGGTERM} server remove agy-runtime://{session_uuid} "
            f"2>/dev/null; "
            f"{PEER_YGGTERM} server rows despawn "
            f"agy-runtime://{session_uuid} 2>/dev/null",
            timeout=60,
        )


SCENARIOS = [
    scenario_preflight,
    scenario_fresh_start,
    scenario_resume_store_present,
    scenario_rebirth_uuid_vouch,
    scenario_store_absent_refuses,
    scenario_defmiss_fresh_start_mint,
    scenario_startborn_remote_corpse,
    scenario_stillborn_resume_corpse,
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
