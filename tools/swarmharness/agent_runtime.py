#!/usr/bin/env python3
"""agent_runtime — portable metadata bridge for the ygg4 worker identity plane.

This module is the NON-Rust half of the `server workers` contract (see
docs/ygg4-daemon-contract.md): the role taxonomy tokens, the exact argv each
verb takes against `yggterm-headless`, and a validator for the JSON records
the daemon home stores. It executes NOTHING by itself — no daemon call, no
spawn — so a seat without a yggterm install can still shape requests and
check responses, and a fresh seed can carry this file without assuming a
system Python beyond the interpreter already running it.

Laws mirrored from crates/yggterm-server/src/worker_identity.rs (keep in
step; the Rust module is SSOT):

- launch token IS the idempotency key: a retried register returns the
  existing record verbatim; role and parent are immutable per token.
- retire is veto-first on a live pid or a pinned row; --force overrides and
  the override is recorded.
- unprovided optional fields are explicitly unknown (absent/null), never
  guessed.
- a context successor is a record fact (role + parent_token), never a GUI
  seat.

Self-test: `python3 agent_runtime.py selftest` — pure functions only, no
subprocess, no network, no writes outside the caller's cwd.
"""

from __future__ import annotations

import json
import shlex
import re
import sys

WORKER_IDENTITY_SCHEMA = "ygg4-worker-identity/2"

#: The stable role tokens, exactly as the Rust wire format and the CLI accept
#: them (kebab). The underscore spellings are also accepted by the parser but
#: callers SHOULD emit the kebab form.
WORKER_ROLES = (
    "worker",
    "group",
    "orchestrator",
    "sub-orchestrator",
    "context-successor",
)

#: Roles that may legally carry a parent_token. A plain worker CAN be
#: parented (it was spawned by somebody); the taxonomy requires the edge on
#: sub-orchestrator and context-successor and allows it elsewhere.
PARENT_REQUIRED_ROLES = ("sub-orchestrator", "context-successor")

_TOKEN_SAFE = re.compile(r"^[A-Za-z0-9._:\-]+$")


def normalize_role(raw: str) -> str:
    """Normalize a role spelling (kebab or underscore) to the kebab token.

    Raises ValueError for anything that is not a role — an unknown role is a
    caller bug, not a default.
    """
    normalized = raw.strip().replace("-", "_").lower()
    mapping = {role.replace("-", "_"): role for role in WORKER_ROLES}
    if normalized not in mapping:
        raise ValueError(
            f"unknown role {raw!r}; expected one of {', '.join(WORKER_ROLES)}"
        )
    return mapping[normalized]


def validate_launch_token(token: str) -> str:
    """Refuse a token that cannot travel as a record filename unchanged."""
    if not token or not _TOKEN_SAFE.match(token):
        raise ValueError(
            f"launch token {token!r} must match [A-Za-z0-9._:-]+ and be non-empty"
        )
    return token


def build_register_argv(
    launch_token: str,
    role: str,
    *,
    session_marker: str | None = None,
    session_id: str | None = None,
    harness: str | None = None,
    title: str | None = None,
    source: str | None = None,
    parent_token: str | None = None,
    pid: int | None = None,
    pinned: bool = False,
    headless: str = "yggterm-headless",
) -> list[str]:
    """The exact argv for an idempotent worker registration.

    The token and role are the identity; every other field is an observation
    the caller actually made. Unobserved fields are OMITTED (the record's
    explicit unknown), never sent as empty strings — an empty title is not an
    unknown, it is a wrong value.
    """
    validate_launch_token(launch_token)
    role = normalize_role(role)
    argv = [
        headless,
        "server",
        "workers",
        "register",
        "--launch-token",
        launch_token,
        "--role",
        role,
    ]
    optional = {
        "--session-marker": session_marker,
        "--session-id": session_id,
        "--harness": harness,
        "--title": title,
        "--source": source,
        "--parent-token": parent_token,
    }
    for flag, value in optional.items():
        if value:
            argv += [flag, str(value)]
    if pid is not None:
        argv += ["--pid", str(int(pid))]
    if pinned:
        argv.append("--pinned")
    return argv


def build_ls_argv(
    role: str | None = None, *, headless: str = "yggterm-headless"
) -> list[str]:
    argv = [headless, "server", "workers", "ls", "--json"]
    if role:
        argv += ["--role", normalize_role(role)]
    return argv


def build_retire_argv(
    launch_token: str,
    reason: str,
    *,
    force: bool = False,
    headless: str = "yggterm-headless",
) -> list[str]:
    validate_launch_token(launch_token)
    argv = [
        headless,
        "server",
        "workers",
        "retire",
        "--launch-token",
        launch_token,
        "--reason",
        reason,
    ]
    if force:
        argv.append("--force")
    return argv


def shell_quote_command(command: str) -> str:
    """POSIX single-quote a worker command for a `sh -c` composition.

    Mirrors worker_identity::shell_quote_single in Rust: the daemon composes
    `exec <shell> -c '<command>'`, so the CALLER does not pre-quote — this
    helper exists for bridges that build display/audit lines, and its output
    must match the daemon's byte for byte.
    """
    return "'" + command.replace("'", "'\\''") + "'"


def build_spawn_argv(
    launch_token: str,
    role: str,
    *,
    parent_token: str | None = None,
    cwd: str | None = None,
    command: str | None = None,
    headless: str = "yggterm-headless",
) -> list[str]:
    """The exact argv for a DAEMON-ATTESTED worker launch.

    Spawn goes through the live daemon (`EnsureShellSession` carrying the
    worker spec): the daemon derives the session id from the token, runs the
    command in its own PTY, and writes the identity record itself. Idempotent
    by token — a retried spawn re-ensures the SAME row, never a second one.
    """
    validate_launch_token(launch_token)
    role = normalize_role(role)
    argv = [
        headless,
        "server",
        "workers",
        "spawn",
        "--launch-token",
        launch_token,
        "--role",
        role,
    ]
    for flag, value in (("--parent-token", parent_token), ("--cwd", cwd), ("--command", command)):
        if value:
            argv += [flag, str(value)]
    return argv


def build_attach_argv(
    launch_token: str, *, headless: str = "yggterm-headless"
) -> list[str]:
    """The exact argv for the GUI-absent reattach: same token, same PTY."""
    validate_launch_token(launch_token)
    return [headless, "server", "workers", "attach", "--launch-token", launch_token]


def parse_register_outcome(stdout: str) -> dict:
    """Parse `register --json` output and assert the idempotency shape.

    A caller that retried an ambiguous network create MUST see
    `outcome == "existing"` on the retry — never a second "created" — and the
    record's created_at_ms must match the first answer. This parser refuses a
    malformed answer rather than letting a caller proceed on a guess.
    """
    outcome = json.loads(stdout)
    if "outcome" not in outcome or "record" not in outcome:
        raise ValueError(f"register output missing outcome/record: {stdout[:200]!r}")
    if outcome["outcome"] not in ("created", "existing"):
        raise ValueError(f"unknown register outcome {outcome['outcome']!r}")
    validate_record(outcome["record"])
    return outcome


def parse_retire_outcome(stdout: str) -> dict:
    outcome = json.loads(stdout)
    if outcome.get("outcome") not in ("retired", "already_retired", "vetoed"):
        raise ValueError(f"unknown retire outcome {outcome.get('outcome')!r}")
    return outcome


def validate_record(record: dict) -> dict:
    """Validate one WorkerIdentityRecord per the ygg4-worker-identity/1 schema.

    Absent optional keys ARE the explicit-unknown form; present-but-null is
    tolerated for bridges that JSON-padded. `schema`, `launch_token`, `role`,
    timestamps and `pinned` are mandatory — a record without them is not this
    schema and must not be trusted as one.
    """
    for key in ("schema", "launch_token", "role", "created_at_ms", "updated_at_ms"):
        if key not in record:
            raise ValueError(f"worker record missing required key {key!r}")
    if record["schema"] != WORKER_IDENTITY_SCHEMA:
        raise ValueError(
            f"worker record schema {record['schema']!r} is not {WORKER_IDENTITY_SCHEMA!r}"
        )
    validate_launch_token(record["launch_token"])
    normalize_role(record["role"])
    if record["role"] in PARENT_REQUIRED_ROLES and not record.get("parent_token"):
        raise ValueError(
            f"role {record['role']!r} requires parent_token (the hierarchy edge is "
            "the record fact; a parentless sub-orchestrator is a lie)"
        )
    for key in ("created_at_ms", "updated_at_ms"):
        value = record[key]
        if not isinstance(value, int) or value < 0:
            raise ValueError(f"{key} must be a non-negative integer, got {value!r}")
    if not isinstance(record.get("pinned", False), bool):
        raise ValueError("pinned must be a boolean")
    # Ownership attestation (schema /2): present-or-absent, never a bare
    # string. daemon-attested is execution evidence; caller-asserted is a
    # statement — consumers must weigh them differently and retire --force
    # may not certify a caller-asserted pid.
    ownership = record.get("ownership")
    if ownership is not None:
        kind = ownership.get("kind") if isinstance(ownership, dict) else None
        if kind not in ("daemon-attested", "caller-asserted"):
            raise ValueError(
                f"ownership kind {kind!r} must be daemon-attested or caller-asserted"
            )
        if kind == "daemon-attested" and "at_ms" not in ownership:
            raise ValueError("daemon-attested ownership requires at_ms")
        if kind == "caller-asserted" and "at_ms" not in ownership:
            raise ValueError("caller-asserted ownership requires at_ms")
    return record


def selftest() -> int:
    failures: list[str] = []

    def check(name: str, condition: bool, detail: str = "") -> None:
        if not condition:
            failures.append(f"{name}: {detail}")

    # Taxonomy
    check("roles count", len(WORKER_ROLES) == 5)
    check("normalize kebab", normalize_role("sub-orchestrator") == "sub-orchestrator")
    check("normalize underscore", normalize_role("context_successor") == "context-successor")
    try:
        normalize_role("nonsense")
        check("unknown role refused", False, "ValueError not raised")
    except ValueError:
        check("unknown role refused", True)

    # argv shape
    argv = build_register_argv(
        "wave-1", "worker", harness="zcode", parent_token="orch-1", pid=4242
    )
    check(
        "register argv",
        argv[:6] == ["yggterm-headless", "server", "workers", "register",
                     "--launch-token", "wave-1"],
        str(argv),
    )
    check("role token in argv", "--role" in argv and "worker" in argv, str(argv))
    check("pid in argv", argv[-2:] == ["--pid", "4242"], str(argv))
    check("unknowns omitted", "--title" not in argv and "--session-id" not in argv, str(argv))

    pinned_argv = build_register_argv("w2", "orchestrator", pinned=True)
    check("pinned flag", pinned_argv[-1] == "--pinned", str(pinned_argv))

    try:
        build_register_argv("bad token with spaces", "worker")
        check("bad token refused", False, "ValueError not raised")
    except ValueError:
        check("bad token refused", True)

    # spawn/attach argv + quoting parity with the Rust composer
    spawn = build_spawn_argv("wave-1", "worker", parent_token="orch-1", command="echo hi")
    check(
        "spawn argv",
        spawn[:6] == ["yggterm-headless", "server", "workers", "spawn",
                      "--launch-token", "wave-1"],
        str(spawn),
    )
    check("spawn carries command", spawn[-2:] == ["--command", "echo hi"], str(spawn))
    check(
        "shell quoting parity",
        shlex.split(shell_quote_command("it's")) == ["it's"],
        shell_quote_command("it's"),
    )
    attach = build_attach_argv("wave-1")
    check("attach argv", attach == ["yggterm-headless", "server", "workers", "attach",
                                    "--launch-token", "wave-1"], str(attach))

    check("ls argv", build_ls_argv()[-1] == "--json")
    check("ls role filter", build_ls_argv("group")[-2:] == ["--role", "group"])

    # Record validation — the Rust module's fixture shape.
    record = {
        "schema": WORKER_IDENTITY_SCHEMA,
        "launch_token": "wave-1",
        "role": "worker",
        "harness": "zcode",
        "parent_token": "orch-1",
        "pinned": False,
        "created_at_ms": 1000,
        "updated_at_ms": 1000,
    }
    check("valid record", validate_record(dict(record)) == record)
    no_unknowns = validate_record({**record, "title": None})
    check("null tolerated as unknown", no_unknowns["title"] is None)
    bad_schema = {**record, "schema": "ygg4-worker-identity/0"}
    try:
        validate_record(bad_schema)
        check("schema mismatch refused", False, "ValueError not raised")
    except ValueError:
        check("schema mismatch refused", True)
    orphan = {**record, "role": "sub-orchestrator"}
    del orphan["parent_token"]
    try:
        validate_record(orphan)
        check("parentless sub-orchestrator refused", False, "ValueError not raised")
    except ValueError:
        check("parentless sub-orchestrator refused", True)

    # Outcome parsing — the idempotency shape.
    created = {
        "outcome": "created",
        "record": dict(record),
    }
    check("created parses", parse_register_outcome(json.dumps(created))["outcome"] == "created")
    existing = {
        "outcome": "existing",
        "record": dict(record),
    }
    check("existing parses", parse_register_outcome(json.dumps(existing))["outcome"] == "existing")
    try:
        parse_register_outcome(json.dumps({"outcome": "duplicated", "record": record}))
        check("unknown outcome refused", False, "ValueError not raised")
    except ValueError:
        check("unknown outcome refused", True)

    vetoed = {"outcome": "vetoed", "record": dict(record), "reason": "pinned"}
    check("veto parses", parse_retire_outcome(json.dumps(vetoed))["outcome"] == "vetoed")

    if failures:
        for failure in failures:
            print(f"FAIL {failure}", file=sys.stderr)
        print(f"selftest: {len(failures)} failure(s)", file=sys.stderr)
        return 1
    print("selftest: all worker identity bridge checks passed")
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 2 or sys.argv[1] != "selftest":
        print("usage: agent_runtime.py selftest", file=sys.stderr)
        sys.exit(2)
    sys.exit(selftest())
