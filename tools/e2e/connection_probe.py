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
                             an error frame. The serving half (trace + live
                             row) measured GREEN 2026-09-29 on build bfe5ed19;
                             the binding half measured RED the same day (the
                             spawned command resumed the ABSENT requested id,
                             rows show kept it) — this scenario is that RED
                             baseline until the minted-bound compose reaches
                             the row.

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


def reap(key, cwd=None):
    """Close the row the way the click path does, then evict the corpse record.

    THE [11.213] TEARDOWN LAW: `rows despawn` is a corpse-record eviction —
    it has no teardown and lawfully REFUSES a live runtime ([11.74]). The
    user-path close is `server remove` (RemoveSession despawn=None ->
    close_live_session_row: tombstone + PTY teardown + row removal). The
    despawn rides AFTER it, on the corpse.

    Returns the live holders the CLOSE itself left behind (before any by-hand
    escalation) — the [11.195] no-live-holder measurement."""
    yggterm(["server", "remove", key], timeout=60)
    post_close = live_holder_pids(cwd) if cwd is not None else []
    if cwd is not None:
        for _ in range(3):
            holders = live_holder_pids(cwd)
            if not holders:
                break
            for pid in holders:
                run(["kill", "-9", str(pid)], timeout=30)
            time.sleep(1.0)
    yggterm(["rows", "despawn", key], timeout=60)
    return post_close


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
        post_close_holders = reap(key, cwd)
        if post_close_holders:
            return sc.fail(
                f"the close left live CLI holders {post_close_holders} — the "
                "[11.195] no-live-holder-survives-into-the-resume law is broken "
                "(escalation-killed for the record; the mint leg stays red until "
                "the close kills its own CLI)"
            )
        # 4. The store surgery: the cwd's conversation leaves the store, so
        # the candidate search answers None — the vouch has nothing to serve
        # and the definitive miss falls through to the mint.
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
            return sc.fail(
                "CLI on screen but NO ensure_definitive_miss_fresh_start trace — "
                "the row connected through some other arm; the mint falsifier "
                "stays unexercised"
            )
        return sc.fail(
            "no CLI after 150s and no named refusal — stderr="
            f"{refused_stderr[:240]!r} screen={text[-160:]!r}"
        )
    finally:
        reap(key)
        db_delete_conversations_for_dir(str(cwd))
        stderr_path.unlink(missing_ok=True)


SCENARIOS = [
    scenario_preflight,
    scenario_fresh_start,
    scenario_resume_store_present,
    scenario_rebirth_uuid_vouch,
    scenario_store_absent_refuses,
    scenario_defmiss_fresh_start_mint,
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
