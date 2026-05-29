#!/usr/bin/env python3
"""Migrate yggterm-generated UUIDv4 codex live sessions to real Codex ULIDs.

Background: `start_remote_codex_session` at yggterm-server/src/lib.rs:3708
generates a UUIDv4 and uses it as the live-session identity. Codex on the
remote machine never sees this UUID — it creates its own ULID for the
transcript JSONL. After any restart, yggterm tries to `resume-codex
<yggterm-UUIDv4>` and the remote wrapper bails with "saved Codex session
is no longer available on this machine."

This script reads `~/.yggterm/server-state.json`, finds Codex live sessions
whose id is a UUIDv4 (not a ULID — ULIDs start with `019` since codex's
timestamp prefix), looks up `remote_machines[*].sessions` for the most
recent codex session with the same `(machine, cwd)`, and rewrites the
live session entry to use the codex ULID instead.

Usage:
    python3 migrate_uuidv4_codex_live_sessions.py             # dry-run preview
    python3 migrate_uuidv4_codex_live_sessions.py --apply     # commit changes

The original file is backed up to server-state.json.pre-uuid-migration.json
on --apply. Run BEFORE relaunching yggterm so the migrated identities are
loaded fresh.
"""
import argparse
import json
import shutil
import sys
from pathlib import Path

STATE_PATH = Path.home() / ".yggterm" / "server-state.json"
BACKUP_PATH = STATE_PATH.with_suffix(".pre-uuid-migration.json")


def is_codex_ulid(session_id: str) -> bool:
    """Codex ULIDs (UUIDv7-style with codex timestamp prefix) start with 019."""
    return isinstance(session_id, str) and session_id.startswith("019")


def is_uuidv4(session_id: str) -> bool:
    """Random UUIDv4: 8-4-4-4-12 hex format, NOT starting with codex's 019."""
    if not isinstance(session_id, str):
        return False
    parts = session_id.split("-")
    if len(parts) != 5 or [len(p) for p in parts] != [8, 4, 4, 4, 12]:
        return False
    if session_id.startswith("019"):
        return False
    try:
        for p in parts:
            int(p, 16)
        return True
    except ValueError:
        return False


def machine_key_from_session_path(session_path: str) -> str | None:
    """remote-session://<machine>/<id> -> <machine>"""
    prefix = "remote-session://"
    if not session_path.startswith(prefix):
        return None
    tail = session_path[len(prefix) :]
    return tail.split("/", 1)[0] if "/" in tail else None


def find_replacement_ulid(
    state: dict,
    machine: str,
    cwd: str,
    excluded_ulids: set[str],
) -> tuple[str, int] | None:
    """Return (replacement_ulid, modified_epoch) for the most recent codex
    session at (machine, cwd) that is NOT already used by another live row.
    Excluded set prevents collapsing two distinct live-session rows onto a
    single codex transcript when both share (machine, cwd)."""
    candidates = []
    for rm in state.get("remote_machines", []):
        # Match the machine by ssh_target or machine_key.
        if rm.get("machine_key") != machine and rm.get("ssh_target") != machine:
            continue
        for sess in rm.get("sessions", []):
            if sess.get("cwd") != cwd:
                continue
            sid = sess.get("session_id", "")
            if not is_codex_ulid(sid):
                continue
            if sid in excluded_ulids:
                continue
            candidates.append((sid, sess.get("modified_epoch", 0)))
    if not candidates:
        return None
    candidates.sort(key=lambda x: x[1], reverse=True)
    return candidates[0]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--apply", action="store_true", help="commit changes")
    parser.add_argument(
        "--state", default=str(STATE_PATH), help=f"override state path (default {STATE_PATH})"
    )
    args = parser.parse_args()
    state_path = Path(args.state)
    backup_path = state_path.with_suffix(".pre-uuid-migration.json")

    if not state_path.exists():
        print(f"error: state file not found: {state_path}", file=sys.stderr)
        sys.exit(1)

    with state_path.open() as f:
        state = json.load(f)

    # Collect all ULIDs already used by other live_sessions so we don't
    # collapse two distinct rows onto one codex transcript.
    existing_live_ulids: set[str] = set()
    for sess in state.get("live_sessions", []):
        sid = sess.get("id", "")
        if is_codex_ulid(sid):
            existing_live_ulids.add(sid)
    # As migration plans entries one-by-one, also reserve ULIDs we pick
    # along the way so two UUIDv4 rows at the same (machine, cwd) don't
    # race for the same replacement.
    claimed_ulids: set[str] = set()
    plan: list[dict] = []
    for idx, sess in enumerate(state.get("live_sessions", [])):
        kind = sess.get("kind")
        if kind != "codex":
            continue
        sid = sess.get("id", "")
        if not is_uuidv4(sid):
            continue
        spath = sess.get("key", "")
        machine = machine_key_from_session_path(spath)
        cwd = sess.get("cwd")
        if not machine or not cwd:
            plan.append(
                {
                    "idx": idx,
                    "title": sess.get("title"),
                    "old_id": sid,
                    "new_id": None,
                    "reason": f"unable to derive machine/cwd (machine={machine!r}, cwd={cwd!r})",
                }
            )
            continue
        excluded = existing_live_ulids | claimed_ulids
        replacement = find_replacement_ulid(state, machine, cwd, excluded)
        if not replacement:
            plan.append(
                {
                    "idx": idx,
                    "title": sess.get("title"),
                    "old_id": sid,
                    "new_id": None,
                    "machine": machine,
                    "cwd": cwd,
                    "reason": (
                        "no unclaimed codex ULID at (machine, cwd) "
                        f"(excluded={len(excluded)})"
                    ),
                }
            )
            continue
        claimed_ulids.add(replacement[0])
        plan.append(
            {
                "idx": idx,
                "title": sess.get("title"),
                "old_id": sid,
                "new_id": replacement[0],
                "machine": machine,
                "cwd": cwd,
                "modified_epoch": replacement[1],
            }
        )

    print(f"=== Migration plan ({len(plan)} candidates) ===")
    actionable = [p for p in plan if p.get("new_id")]
    for p in plan:
        if p.get("new_id"):
            print(
                f"  [{p['idx']}] {p['title']!r}: {p['old_id']} -> {p['new_id']} "
                f"(machine={p['machine']}, cwd={p['cwd']})"
            )
        else:
            print(f"  [{p['idx']}] {p['title']!r}: {p['old_id']} -> SKIP ({p['reason']})")
    print(f"=== {len(actionable)} actionable, {len(plan) - len(actionable)} skipped ===")

    if not args.apply:
        print("\nDry-run only. Re-run with --apply to commit.")
        return

    if not actionable:
        print("Nothing to apply.")
        return

    shutil.copy(state_path, backup_path)
    print(f"Backed up to {backup_path}")

    rewrites = 0
    for p in actionable:
        idx = p["idx"]
        sess = state["live_sessions"][idx]
        machine = p["machine"]
        new_id = p["new_id"]
        old_key = sess["key"]
        new_key = f"remote-session://{machine}/{new_id}"
        sess["id"] = new_id
        sess["key"] = new_key
        # Also rewrite live_session_order entries pointing at the old key.
        if isinstance(state.get("live_session_order"), list):
            state["live_session_order"] = [
                new_key if k == old_key else k for k in state["live_session_order"]
            ]
        # Rewrite keep_alive_sessions map keys if present.
        ka = state.get("keep_alive_sessions")
        if isinstance(ka, dict) and old_key in ka:
            ka[new_key] = ka.pop(old_key)
        rewrites += 1

    with state_path.open("w") as f:
        json.dump(state, f, indent=2)
    print(f"Wrote {state_path} with {rewrites} rebinds.")


if __name__ == "__main__":
    main()
